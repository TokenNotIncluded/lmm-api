#!/usr/bin/env python3
"""Bounded, opt-in diagnostics for an already-frozen Core resource case.

This is not acceptance. It never compiles, accepts a DSN, modifies the frozen
runner/test, changes the traffic gate, or adopts an existing PostgreSQL fixture.
The host psql client and observer run outside Core/PG/generator cgroups. The
observer's read-only PG backend necessarily consumes some of PG's own budget;
its CPU and query time are recorded. Pool-acquire time is not externally visible.

PostgreSQL 18 interfaces: /docs/18/monitoring-stats.html and
/docs/18/runtime-config-logging.html. Uses installed psql, not a new wire client.
"""

from __future__ import annotations

import argparse
import collections
import datetime as dt
import fcntl
import hashlib
import io
import json
import os
from pathlib import Path
import resource as usage
import secrets
import selectors
import signal
import stat
import subprocess
import sys
import tarfile
import time
from types import SimpleNamespace

sys.dont_write_bytecode = True
REVISION = "467a144fbdb399c784077baa714da1dc0159967e"
CORE_SHA = "3aa974e898ea1e1f39c7a52ea4fc406b5c299b5f8b5d6190009b2fed6f129a29"
MODEL_SHA = "5721e7bb9b565fdc25d4670c4228818b42357635ee0f5cb98128a82a47d6584c"
PREPARED_SHA = "e8efc6675b83c6d3ab7d85d2fb26f3da512c699960ab05a648c82fd903f41a2d"
SOURCE_BINDING_SHA = "09ae3fdfd5c9f53714dec07511adbc5b9fd57e78e9a5c6ae051b4a8ce9fcad0f"
INTERVAL = 0.250
MAX_SECONDS = 1100
CORE_LIMITS = {
    "cpu.max": "100000 100000",
    "memory.max": "268435456",
    "memory.swap.max": "0",
    "pids.max": "128",
}
PG_LIMITS = {**CORE_LIMITS, "memory.max": "536870912", "memory.swap.max": "536870912"}
GEN_LIMITS = {**PG_LIMITS, "memory.swap.max": "0"}
OBS_LIMITS = {
    **CORE_LIMITS,
    "cpu.max": "25000 100000",
    "memory.max": "134217728",
    "pids.max": "32",
}
LOG_OPTIONS = {
    "log_line_prefix": "%m [%p] %e ",
    "log_error_verbosity": "terse",
    "log_min_error_statement": "panic",
    "log_statement": "none",
    "log_parameter_max_length": "0",
    "log_parameter_max_length_on_error": "0",
}

# Only fixed stage labels leave PostgreSQL; query text and parameters never do.
# "ledger" cannot be divided into reserve/settle without an observed predecessor.
STAGE_CASE = """CASE
 WHEN lower(ltrim(query)) ~ '^(begin|set |start transaction)' THEN 'tx-setup'
 WHEN query LIKE '%pg_advisory_xact_lock%' AND query LIKE '%core_charging:%' THEN 'request-lock'
 WHEN lower(query) LIKE '%core_charging.requests%for update%' THEN 'request-load'
 WHEN lower(ltrim(query)) LIKE 'insert into core_charging.requests%' THEN 'reserve-insert'
 WHEN query LIKE '%core_model.checkpoint_output%' THEN 'output-checkpoint'
 WHEN lower(query) LIKE '%update core_charging.requests%settled%' THEN 'settle-update'
 WHEN lower(query) LIKE '%update core_charging.requests%input_tokens%' THEN 'usage-checkpoint'
 WHEN lower(query) LIKE '%usage_events%' THEN 'usage-event'
 WHEN query LIKE '%core_billing.post_ledger%' THEN 'ledger'
 WHEN lower(ltrim(query)) LIKE 'commit%' THEN 'commit'
 WHEN lower(ltrim(query)) LIKE 'rollback%' THEN 'rollback'
 WHEN query LIKE '%completed_streams%' THEN 'completed-streams'
 WHEN lower(ltrim(query)) LIKE 'select id from core_charging.requests%' AND lower(query) LIKE '%lease_until%' THEN 'recovery-candidate'
 ELSE 'other-runtime' END"""
WAIT_SQL = """WITH sampled AS MATERIALIZED (
 SELECT coalesce(json_agg(json_build_object(
  'pid',pid,'backend_start',backend_start,'state',state,'state_change',state_change,
  'query_start',query_start,'xact_start',xact_start,'wait_event_type',wait_event_type,
  'wait_event',wait_event,'query_id',query_id,'stage',STAGE,
  'blocking_pids',CASE WHEN wait_event_type='Lock' THEN pg_blocking_pids(pid) ELSE ARRAY[]::integer[] END)), '[]'::json) AS rows
 FROM pg_stat_activity WHERE backend_type='client backend' AND usename LIKE 'lmm_runtime_test_%'
) SELECT json_build_object('server_utc',clock_timestamp(),'observer_backend_pid',pg_backend_pid(),
 'server_wall_ms',extract(epoch FROM clock_timestamp()-statement_timestamp())*1000,
 'read_only',current_setting('default_transaction_read_only'),
 'statement_timeout',current_setting('statement_timeout'),'application_name',current_setting('application_name'),
 'backends',sampled.rows) FROM sampled;""".replace("STAGE", STAGE_CASE)


def now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def load(path):
    return json.loads(Path(path).read_text())


def save(path, value):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w") as stream:
        json.dump(value, stream, sort_keys=True, indent=2)
        stream.write("\n")


def private(path):
    path = Path(path)
    info = path.lstat()
    if (
        path.resolve() != path
        or not stat.S_ISDIR(info.st_mode)
        or info.st_uid != os.geteuid()
        or info.st_mode & 0o077
    ):
        raise ValueError("expected an existing private, owned, real directory")
    return path


def identity(pid):
    raw = Path(f"/proc/{pid}/stat").read_text()
    fields = raw[raw.rfind(")") + 2 :].split()
    return {
        "pid": pid,
        "start_ticks": int(fields[19]),
        "boot_id": Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
    }


def process(pid):
    try:
        raw = Path(f"/proc/{pid}/stat").read_text()
        fields = raw[raw.rfind(")") + 2 :].split()
        return {
            "identity": identity(pid),
            "cpu_ticks": int(fields[11]) + int(fields[12]),
            "rss_pages": int(fields[21]),
            "threads": int(fields[17]),
        }
    except OSError as error:
        return {"unavailable_errno": error.errno}


def own_cgroup(pid):
    rows = Path(f"/proc/{pid}/cgroup").read_text().splitlines()
    unified = [row[3:] for row in rows if row.startswith("0::")]
    if (
        len(unified) != 1
        or not unified[0].startswith("/")
        or ".." in Path(unified[0]).parts
    ):
        raise ValueError("expected one real cgroup-v2 membership")
    return Path("/sys/fs/cgroup") / unified[0].lstrip("/")


def cgroup(path, expected=None):
    path = Path(path)
    if not path.exists():
        return {"status": "absent"}
    if path.resolve() != path or not str(path).startswith("/sys/fs/cgroup/"):
        raise ValueError("invalid cgroup boundary")
    values = {}
    for name in [
        *CORE_LIMITS,
        "cpu.stat",
        "cpu.pressure",
        "io.stat",
        "io.pressure",
        "memory.current",
        "memory.peak",
        "memory.events",
        "memory.pressure",
        "pids.current",
    ]:
        try:
            values[name] = (path / name).read_text().strip()
        except OSError as error:
            values[name] = {"unavailable_errno": error.errno}
    if expected is not None and {k: values[k] for k in expected} != expected:
        raise ValueError("actual hard budget differs from the frozen budget")
    return {"status": "observed", "path": str(path), "values": values}


def namespace_processes(group):
    """Map PG namespace PIDs to host PIDs without privileged /proc/io reads."""
    result = {}
    for value in (Path(group) / "cgroup.procs").read_text().split():
        pid = int(value)
        try:
            line = next(
                s
                for s in Path(f"/proc/{pid}/status").read_text().splitlines()
                if s.startswith("NSpid:")
            )
            result[int(line.split()[-1])] = process(pid)
        except (OSError, StopIteration):
            continue
    return result


class Psql:
    """One host psql, one read-only autocommit connection, never overlapping reads."""

    def __init__(self, fixture_root, port, output):
        config = dict(
            s.split("=", 1)
            for s in (fixture_root / "postgres.env").read_text().splitlines()
        )
        env = {
            k: v
            for k, v in os.environ.items()
            if not k.startswith(("PG", "DOCKER_"))
            and k not in {"DATABASE_URL", "SQL_DSN"}
        }
        env.update(
            PGHOST="127.0.0.1",
            PGPORT=str(port),
            PGDATABASE="postgres",
            PGUSER="a09_admin",
            PGPASSWORD=config["POSTGRES_PASSWORD"],
            PGCONNECT_TIMEOUT="2",
            PGAPPNAME="lmm_resource_external_observer",
            PGOPTIONS="-c default_transaction_read_only=on -c statement_timeout=50ms -c lock_timeout=50ms",
        )
        self.child = subprocess.Popen(
            [
                "/usr/bin/psql",
                "-XqAt",
                "--no-password",
                "-v",
                "ON_ERROR_STOP=0",
                "-v",
                "VERBOSITY=terse",
            ],
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        self.owner = identity(self.child.pid)
        self.poller = selectors.DefaultSelector()
        for stream in [self.child.stdout, self.child.stderr]:
            os.set_blocking(stream.fileno(), False)
            self.poller.register(stream, selectors.EVENT_READ)
        self.buffers = {self.child.stdout: b"", self.child.stderr: b""}
        self.pending = None
        self.serial = 0
        self.completed = 0
        self.error_bytes = 0
        self.errors = collections.Counter()
        self.error_buffer = b""
        self.output = output

    def send(self):
        if self.pending is not None:
            return False
        self.serial += 1
        marker = "LMM_OBSERVER_FRAME_" + str(self.serial)
        self.pending = {
            "serial": self.serial,
            "sent_ns": time.monotonic_ns(),
            "sent_utc": now(),
            "marker": marker,
            "rows": [],
        }
        self.child.stdin.write((WAIT_SQL + "\n\\echo " + marker + "\n").encode())
        self.child.stdin.flush()
        return True

    def collect(self, timeout=0.0):
        deadline = time.monotonic() + timeout
        while True:
            for key, _ in self.poller.select(max(0.0, deadline - time.monotonic())):
                stream = key.fileobj
                chunk = os.read(stream.fileno(), 65536)
                if not chunk:
                    raise RuntimeError("observer psql exited before its bounded stop")
                if stream is self.child.stderr:
                    self.error_bytes += len(
                        chunk
                    )  # Never retain SQL, parameters or error text.
                    self.error_buffer = (self.error_buffer + chunk)[-8192:]
                    while b"\n" in self.error_buffer:
                        line, self.error_buffer = self.error_buffer.split(b"\n", 1)
                        if b"statement timeout" in line:
                            self.errors["statement_timeout"] += 1
                        elif b"ERROR:" in line or b"FATAL:" in line:
                            self.errors["other_error"] += 1
                    continue
                self.buffers[stream] += chunk
                if len(self.buffers[stream]) > 131072:
                    raise RuntimeError("observer result exceeded bounded frame")
                while b"\n" in self.buffers[stream]:
                    line, self.buffers[stream] = self.buffers[stream].split(b"\n", 1)
                    if not self.pending:
                        raise RuntimeError("unexpected unbound observer output")
                    if line.decode() == self.pending["marker"]:
                        pending, self.pending = self.pending, None
                        self.completed += 1
                        elapsed = (time.monotonic_ns() - pending["sent_ns"]) / 1e6
                        rows = pending["rows"]
                        return {
                            "serial": pending["serial"],
                            "client_collect_ms_upper_bound": elapsed,
                            "sent_utc": pending["sent_utc"],
                            "collected_utc": now(),
                            "collection_limited_to_tick": timeout == 0,
                            "result": json.loads(rows[0]) if len(rows) == 1 else None,
                            "result_missing_or_error": len(rows) != 1,
                            "stderr_bytes_cumulative": self.error_bytes,
                            "errors_cumulative": dict(self.errors),
                        }
                    self.pending["rows"].append(line.decode())
            if time.monotonic() >= deadline:
                return None

    def close(self):
        """Only the exact child created here is eligible for escalation."""
        try:
            self.child.stdin.write(b"\\q\n")
            self.child.stdin.flush()
            self.child.wait(timeout=2)
        except (BrokenPipeError, subprocess.TimeoutExpired):
            if self.child.poll() is None:
                if identity(self.child.pid) != self.owner:
                    raise RuntimeError("observer child identity changed; no signal")
                self.child.terminate()
                self.child.wait(timeout=2)
        finally:
            self.poller.close()
            for stream in [self.child.stdin, self.child.stdout, self.child.stderr]:
                stream.close()
        if Path(f"/proc/{self.owner['pid']}").exists():
            raise RuntimeError("observer psql child remained")


class Segments:
    def __init__(self, root):
        self.root, self.number, self.rows, self.stream = root, -1, 64, None

    def write(self, value):
        if self.rows == 64:
            if self.stream:
                self.stream.close()
            self.number += 1
            self.rows = 0
            self.stream = (self.root / f"samples-{self.number:04}.jsonl").open("x")
        self.stream.write(json.dumps(value, separators=(",", ":")) + "\n")
        self.stream.flush()
        self.rows += 1

    def close(self):
        if self.stream:
            self.stream.close()


def core_group(config):
    for path in sorted(
        Path(config["process_root"]).glob("*/resource-identity-*.json"),
        key=lambda p: p.stat().st_mtime_ns,
        reverse=True,
    ):
        try:
            saved = load(path)
        except (json.JSONDecodeError, FileNotFoundError):
            continue  # A create-new receipt may not yet have completed its write.
        pid = saved["actual_pid"]
        if not Path(f"/proc/{pid}").exists():
            continue
        if identity(pid)["start_ticks"] != int(saved["start_ticks"]) or own_cgroup(
            pid
        ) != Path(saved["cgroup"]):
            raise RuntimeError("Core identity/cgroup drift")
        if saved["executable"]["executed_sha256"] != CORE_SHA:
            raise RuntimeError("Core executable binding drift")
        current = Path(f"/proc/{pid}/exe").stat()
        if (current.st_dev, current.st_ino) != (
            saved["executable"]["device"],
            saved["executable"]["inode"],
        ):
            raise RuntimeError("Core executable inode changed")
        return cgroup(saved["cgroup"], CORE_LIMITS)
    return {"status": "not_created_or_already_exited"}


def observe(config):
    output = private(config["output"])
    group = own_cgroup(os.getpid())
    own = identity(os.getpid())
    before = cgroup(group, OBS_LIMITS)
    pg_group = Path(config["pg_cgroup"])
    if group == pg_group:
        raise ValueError("observer belongs to measured PG cgroup")
    psql = Psql(Path(config["fixture_root"]), config["pg_port"], output)
    writer, started = Segments(output), time.monotonic()
    next_tick, index, skipped_ticks = started, 0, 0
    previous_work_ms = None
    cpu_start = usage.getrusage(usage.RUSAGE_SELF)
    try:
        psql.send()
        until, handshake = time.monotonic() + 3, None
        while handshake is None and time.monotonic() < until:
            handshake = psql.collect(0.060)
        if not handshake or not handshake["result"]:
            raise RuntimeError("persistent observer PG connection failed its handshake")
        settings = handshake["result"]
        if (
            settings["read_only"],
            settings["statement_timeout"],
            settings["application_name"],
        ) != ("on", "50ms", "lmm_resource_external_observer"):
            raise RuntimeError("observer PG connection settings differ")
        save(
            output / "ready.json",
            {
                "identity": own,
                "psql_identity": psql.owner,
                "cgroup": str(group),
                "actual_budget": before,
                "external_to_pg": True,
                "handshake": handshake,
                "query_sha256": hashlib.sha256(WAIT_SQL.encode()).hexdigest(),
            },
        )
        while (
            time.monotonic() - started < MAX_SECONDS and not (output / "stop").exists()
        ):
            tick = time.monotonic()
            pg_samples = []
            previous = psql.collect(0)
            if previous is not None:
                pg_samples.append(previous)
            sent = psql.send()
            immediate = psql.collect(0.060)
            if immediate is not None:
                pg_samples.append(immediate)
            if any(
                row["result"]
                and row["result"]["observer_backend_pid"]
                != settings["observer_backend_pid"]
                for row in pg_samples
            ):
                raise RuntimeError("observer PostgreSQL backend changed")
            core = core_group(config)
            gen_path = Path(config["generator_readback"])
            try:
                generator = load(gen_path)
                gen_group = Path(generator["cgroup"])
                if gen_group == group or (
                    core.get("path") and Path(core["path"]) == group
                ):
                    raise ValueError("observer belongs to a measured cgroup")
                gen = cgroup(gen_group, GEN_LIMITS)
            except (FileNotFoundError, json.JSONDecodeError):
                gen = {"status": "not_created_yet"}
            sample = {
                "utc": now(),
                "monotonic_ns": time.monotonic_ns(),
                "index": index,
                "elapsed_ms": (tick - started) * 1000,
                "tick_lag_ms": max(0, (tick - next_tick) * 1000),
                "scheduled_elapsed_ms": (next_tick - started) * 1000,
                "previous_work_ms_including_write": previous_work_ms,
                "skipped_ticks_cumulative": skipped_ticks,
                "Core": core,
                "PG": cgroup(pg_group, PG_LIMITS),
                "generator": gen,
                "PG_namespaced_processes": namespace_processes(pg_group),
                "pg_wait_samples": pg_samples,
                "pg_query_skipped_pending": not sent,
                "pg_query_pending_ms": (time.monotonic_ns() - psql.pending["sent_ns"])
                / 1e6
                if psql.pending
                else None,
                "observer_process": process(os.getpid()),
                "psql_process": process(psql.child.pid),
                "tracking_watcher_process": process(config["tracking_watcher"]["pid"]),
                "observer_cgroup": cgroup(group, OBS_LIMITS),
                "host_stat": Path("/proc/stat").read_text(),
                "host_pressure": {
                    n: Path("/proc/pressure", n).read_text()
                    for n in ["cpu", "io", "memory"]
                },
                "limitations": "external sampling; pool acquire not visible; query_start/active ages are lower bounds, not completed query durations",
            }
            writer.write(sample)
            previous_work_ms = (time.monotonic() - tick) * 1000
            index += 1
            next_tick += INTERVAL
            if time.monotonic() > next_tick:
                skipped = int((time.monotonic() - next_tick) / INTERVAL) + 1
                skipped_ticks += skipped
                next_tick += (
                    skipped * INTERVAL
                )  # Fixed grid, skip rather than queue catch-up work.
            time.sleep(max(0, next_tick - time.monotonic()))
    finally:
        psql_before_close = process(psql.child.pid)
        psql.close()
        writer.close()
        cpu_end = usage.getrusage(usage.RUSAGE_SELF)
        save(
            output / "finished.json",
            {
                "utc": now(),
                "samples": index,
                "seconds": time.monotonic() - started,
                "self_cpu_seconds": cpu_end.ru_utime
                + cpu_end.ru_stime
                - cpu_start.ru_utime
                - cpu_start.ru_stime,
                "psql_before_close": psql_before_close,
                "psql_original_pid_absent": True,
                "queries_sent": psql.serial,
                "queries_completed": psql.completed,
                "uncollected_query_at_stop": {
                    key: psql.pending[key] for key in ["serial", "sent_utc"]
                }
                if psql.pending
                else None,
                "query_errors": dict(psql.errors),
                "skipped_ticks": skipped_ticks,
                "last_work_ms_including_write": previous_work_ms,
                "cgroup_after": cgroup(group, OBS_LIMITS),
                "not_acceptance": True,
            },
        )


def baseline_summary(output):
    samples = []
    for path in sorted(output.glob("samples-*.jsonl")):
        for line in path.read_text().splitlines():
            try:
                samples.append(json.loads(line))
            except json.JSONDecodeError:
                break  # The writer may be finishing its last line; it is not a zero sample.
    if len(samples) < 30:
        raise RuntimeError("observer baseline lacks 30 completed samples")
    first, last = samples[0], samples[-1]
    seconds = (last["monotonic_ns"] - first["monotonic_ns"]) / 1e9

    def counter(sample, key):
        return dict(
            line.split() for line in sample[key]["values"]["cpu.stat"].splitlines()
        )

    observer_cpu = (
        int(counter(last, "observer_cgroup")["usage_usec"])
        - int(counter(first, "observer_cgroup")["usage_usec"])
    ) / 1e6
    handshake = load(output / "ready.json")["handshake"]["result"]
    backend = str(handshake["observer_backend_pid"])
    begin = first["PG_namespaced_processes"].get(backend)
    end = last["PG_namespaced_processes"].get(backend)
    if (
        not begin
        or not end
        or "cpu_ticks" not in begin
        or "cpu_ticks" not in end
        or begin["identity"] != end["identity"]
    ):
        raise RuntimeError("PG observer-backend baseline CPU identity is unavailable")
    pg_cpu = (end["cpu_ticks"] - begin["cpu_ticks"]) / os.sysconf("SC_CLK_TCK")
    frames = [row for sample in samples for row in sample["pg_wait_samples"]]
    results = [row["result"] for row in frames if row["result"]]
    if len(results) < 30 or any(row["result_missing_or_error"] for row in frames):
        raise RuntimeError("PG observer baseline had missing/error query frames")
    durations = sorted(row["server_wall_ms"] for row in results)
    p95 = durations[(len(durations) * 95 + 99) // 100 - 1]
    result = {
        "utc": now(),
        "samples": len(samples),
        "seconds": seconds,
        "observer_and_psql_cpu_seconds": observer_cpu,
        "observer_and_psql_cores": observer_cpu / seconds,
        "pg_observer_backend_cpu_seconds": pg_cpu,
        "pg_observer_backend_cores": pg_cpu / seconds,
        "pg_query_count": len(results),
        "pg_server_wall_ms_p95": p95,
        "max_sample_gap_ms": max(
            (samples[i + 1]["monotonic_ns"] - samples[i]["monotonic_ns"]) / 1e6
            for i in range(len(samples) - 1)
        ),
        "skipped_ticks": last["skipped_ticks_cumulative"],
        "limits": {
            "observer_and_psql_max_cores": 0.05,
            "pg_observer_backend_max_cores": 0.05,
            "pg_query_p95_max_ms": 20,
        },
        "scope": "empty fresh fixture baseline; PG query backend is inside PG budget; not traffic acceptance",
    }
    watch_start, watch_end = (
        first["tracking_watcher_process"],
        last["tracking_watcher_process"],
    )
    result["tracking_watcher_cores"] = (
        (watch_end["cpu_ticks"] - watch_start["cpu_ticks"])
        / os.sysconf("SC_CLK_TCK")
        / seconds
        if "cpu_ticks" in watch_start
        and "cpu_ticks" in watch_end
        and watch_start["identity"] == watch_end["identity"]
        else None
    )
    result["tracking_watcher_caveat"] = (
        "whole owned watcher process CPU; not writer attribution or physical IO allocation"
    )
    result["within_observation_budget"] = (
        observer_cpu / seconds <= 0.05 and pg_cpu / seconds <= 0.05 and p95 <= 20
    )
    return result


def helpers(reference):
    reference = private(reference)
    source = reference / "source"
    if sha(reference / "source-binding.json") != SOURCE_BINDING_SHA:
        raise ValueError(
            "source manifest differs from independently frozen467 reference"
        )
    binding = load(reference / "source-binding.json")
    if binding["revision"] != REVISION or len(binding["files"]) != 249:
        raise ValueError("unexpected frozen reference")
    if any(sha(source / name) != digest for name, digest in binding["files"].items()):
        raise ValueError("frozen reference changed")
    sys.path.insert(0, str(source / "scripts"))
    import verify_model_resources as frozen

    prepared = reference / "prepared-v2"
    if sha(prepared / "prepared.json") != PREPARED_SHA:
        raise ValueError("prepared reference changed")
    data = load(prepared / "prepared.json")
    frozen.check(data["inputs"])
    frozen.artifacts_check(prepared, data["artifacts"])
    frozen.reviewed_tests(data["test_list"])
    if data["artifacts"] != {
        "artifacts/lmm-core": CORE_SHA,
        "artifacts/model_process": MODEL_SHA,
    }:
        raise ValueError("unexpected release/test binding")
    original = reference.parent
    product_binding = original / "product-source-binding.json"
    build = load(original / "release-build-receipt.json")
    if (
        build["exit_code"] != 0
        or build["source_drift"] != []
        or build["binary_sha256"] != CORE_SHA
        or build["source_binding_sha256"] != sha(product_binding)
        or load(product_binding)["files"] != data["inputs"]["product"]
        or build["revision"] != "09964012b5d80e9ca4627a6c650bf1fa76c3b23c"
        or sha(original / "release/lmm-core") != CORE_SHA
    ):
        raise ValueError("original099 release/build/product receipt changed")
    return frozen, prepared, data


def generation(config):
    frozen, prepared, data = helpers(Path(config["reference"]))
    output = private(config["output"])
    group = own_cgroup(os.getpid())
    save(
        output / "readback.json",
        {
            "identity": identity(os.getpid()),
            "cgroup": str(group),
            "budget_before": cgroup(group, GEN_LIMITS),
        },
    )
    environment = frozen.env()
    pg = dict(
        s.split("=", 1)
        for s in (Path(config["fixture_root"]) / "postgres.env")
        .read_text()
        .splitlines()
    )
    environment.update(
        XDG_RUNTIME_DIR=f"/run/user/{os.getuid()}",
        DBUS_SESSION_BUS_ADDRESS=f"unix:path=/run/user/{os.getuid()}/bus",
        DATABASE_URL=f"postgresql://a09_admin:{pg['POSTGRES_PASSWORD']}@127.0.0.1:{config['pg_port']}/postgres",
        TMPDIR=config["process_root"],
        LMM_MODEL_PROCESS_ROOT=config["process_root"],
        LMM_MODEL_PROCESS_CORE_BIN=str(prepared / "artifacts/lmm-core"),
        LMM_MODEL_PROCESS_CORE_SHA256=CORE_SHA,
        LMM_RESOURCE_ACCEPTANCE="1",
    )
    result = frozen.execute_test(
        frozen.selected_command(prepared / "artifacts/model_process", "sustained"),
        environment,
        output / "resource-test.log",
        1000,
    )
    save(
        output / "result.json",
        {
            "test": result,
            "cgroup_after": cgroup(group, GEN_LIMITS),
            "prepared_sha256": sha(prepared / "prepared.json"),
            "not_acceptance": True,
            "original_product_binding_sha256": sha(
                Path(config["reference"]).parent / "product-source-binding.json"
            ),
            "original_release_build_receipt_sha256": sha(
                Path(config["reference"]).parent / "release-build-receipt.json"
            ),
        },
    )
    return result["exit_code"]


def bus_env():
    return dict(
        os.environ,
        XDG_RUNTIME_DIR=f"/run/user/{os.getuid()}",
        DBUS_SESSION_BUS_ADDRESS=f"unix:path=/run/user/{os.getuid()}/bus",
    )


def start_scope(kind, output, config, limits):
    nonce = secrets.token_hex(12)
    unit = f"lmm-diag-{kind}-{nonce}.scope"
    description = f"Owned local Core diagnostic {kind} {nonce}"
    save(
        output / "allocation.json",
        {"unit": unit, "description": description, "owner": identity(os.getpid())},
    )
    command = [
        "systemd-run",
        "--user",
        "--scope",
        "--quiet",
        "--collect",
        "--unit=" + unit,
        "--property=Description=" + description,
        "--property=CPUQuotaPeriodSec=100ms",
        "--property=CPUQuota=" + ("25%" if kind == "observe" else "100%"),
        "--property=MemoryMax=" + limits["memory.max"],
        "--property=MemorySwapMax=0",
        "--property=TasksMax=" + limits["pids.max"],
        "--property=RuntimeMaxSec=1150",
        "--",
        sys.executable,
        "-B",
        str(Path(__file__).resolve()),
        kind,
        "--config",
        str(config),
    ]
    save(output / "command.json", {"command": command})
    out, err = (
        (output / "scope.stdout.log").open("xb"),
        (output / "scope.stderr.log").open("xb"),
    )
    child = subprocess.Popen(
        command, env=bus_env(), stdout=out, stderr=err, start_new_session=True
    )
    out.close()
    err.close()
    return child


def stop_scope(output):
    saved = load(output / "allocation.json")
    result = subprocess.run(
        [
            "systemctl",
            "--user",
            "show",
            saved["unit"],
            "--property=Description",
            "--property=ActiveState",
            "--property=ControlGroup",
        ],
        env=bus_env(),
        capture_output=True,
        text=True,
        timeout=10,
    )
    values = dict(
        line.split("=", 1) for line in result.stdout.splitlines() if "=" in line
    )
    if values.get("ActiveState") in {"active", "activating", "deactivating"}:
        if values.get("Description") != saved["description"]:
            raise RuntimeError("own scope description changed; no stop")
        subprocess.run(
            ["systemctl", "--user", "stop", saved["unit"]],
            env=bus_env(),
            capture_output=True,
            check=True,
            timeout=15,
        )


def scope_final(output, readback_name):
    allocation = load(output / "allocation.json")
    query = subprocess.run(
        [
            "systemctl",
            "--user",
            "show",
            allocation["unit"],
            "--property=ActiveState",
            "--property=ControlGroup",
            "--property=LoadState",
        ],
        env=bus_env(),
        capture_output=True,
        text=True,
        timeout=10,
    )
    values = dict(
        line.split("=", 1) for line in query.stdout.splitlines() if "=" in line
    )
    if values.get("ActiveState") not in {"inactive", "failed"} or values.get(
        "ControlGroup"
    ):
        raise RuntimeError("owned diagnostic scope did not finish")
    result = {"unit": allocation["unit"], "properties": values}
    if (output / readback_name).exists():
        readback = load(output / readback_name)
        result["cgroup_absent"] = not Path(readback["cgroup"]).exists()
        result["original_pid_absent"] = not Path(
            f"/proc/{readback['identity']['pid']}"
        ).exists()
        if not result["cgroup_absent"] or not result["original_pid_absent"]:
            raise RuntimeError("owned diagnostic PID/cgroup remained")
    save(output / "cleanup.json", result)


def run(args):
    output, process_root = private(args.output), private(args.process_root)
    if (
        any(output.iterdir())
        or any(process_root.iterdir())
        or len(os.fsencode(process_root)) > 35
    ):
        raise ValueError(
            "diagnostic output/process roots must be fresh; process root <=35 bytes"
        )
    if sha(__file__) != args.expected_script_sha256:
        raise ValueError("diagnostic control script differs from reviewed bytes")
    frozen, prepared, data = helpers(args.reference)
    lease = Path(os.environ["LMM_TEST_LEASE"])
    acquired = load(lease / "acquired.json")
    if (
        not acquired["heavy"]
        or acquired["lock"]["path"] != "/tmp/lmm-delegate/heavy-build.lock"
    ):
        raise ValueError("actual global heavy lease is required")
    fd = os.open(acquired["lock"]["path"], os.O_RDONLY | os.O_NOFOLLOW)
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            pass
        else:
            raise RuntimeError("global heavy lock is not actually held")
    finally:
        os.close(fd)
    # Git objects are immutable: never read moving workspace product files.
    paths = [*frozen.v.PRODUCT_TREES, *frozen.v.PRODUCT_FILES]
    archive = subprocess.check_output(
        ["git", "archive", REVISION, *paths], cwd=args.repository
    )
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        git_product = {
            member.name: hashlib.sha256(tar.extractfile(member).read()).hexdigest()
            for member in tar.getmembers()
            if member.isfile()
        }
    if len(git_product) != 156 or git_product != data["inputs"]["product"]:
        raise ValueError(
            "467 product156 differs from original099 release product binding"
        )
    save(
        output / "inputs.json",
        {
            "utc": now(),
            "source_revision": REVISION,
            "product156_equal": True,
            "release_original_revision": "09964012b5d80e9ca4627a6c650bf1fa76c3b23c",
            "core_sha256": CORE_SHA,
            "model_process_sha256": MODEL_SHA,
            "prepared_sha256": PREPARED_SHA,
            "control_sha256": sha(__file__),
            "reference": str(args.reference),
            "global_lease": str(lease),
            "interval_seconds": INTERVAL,
            "log_options": LOG_OPTIONS,
            "not_acceptance": True,
            "original_product_binding_sha256": sha(
                args.reference.parent / "product-source-binding.json"
            ),
            "original_release_build_receipt_sha256": sha(
                args.reference.parent / "release-build-receipt.json"
            ),
        },
    )
    tracking_ready = load(args.tracking_ready)
    tracking_watcher = identity(tracking_ready["pid"])
    compilers = []
    for path in Path("/proc").iterdir():
        if path.name.isdigit():
            try:
                name = (path / "comm").read_text().strip()
                if name in {
                    "cargo",
                    "rustc",
                    "rustdoc",
                    "clippy-driver",
                    "go",
                    "compile",
                    "link",
                }:
                    compilers.append({"pid": int(path.name), "command": name})
            except (FileNotFoundError, PermissionError):
                pass
    save(
        output / "compiler-quiescence.json",
        {
            "utc": now(),
            "observed_compilers": compilers,
            "scope": "process-name readback, not proof about unrelated host activity",
        },
    )
    if compilers:
        raise RuntimeError("compilers remain; no fixture or diagnostic timing started")

    class DiagnosticFixture(frozen.Fixture):
        def docker(self, *argv, **kwargs):
            if argv and argv[0] == "create":
                argv = (
                    *argv,
                    *(
                        item
                        for key, value in LOG_OPTIONS.items()
                        for item in ["-c", key + "=" + value]
                    ),
                )
            return super().docker(*argv, **kwargs)

    fixture = DiagnosticFixture(output)
    observer = generator = None
    test, error, units, removed = None, None, None, False
    cleanup_errors = []
    obs_dir, gen_dir = output / "observer", output / "generator"
    for directory in [obs_dir, gen_dir]:
        directory.mkdir(mode=0o700)

    def interrupted(signum, _frame):
        raise InterruptedError("owned diagnostic interrupted by signal " + str(signum))

    handlers = {
        sig: signal.signal(sig, interrupted) for sig in [signal.SIGINT, signal.SIGTERM]
    }
    try:
        fixture.start()
        fixture.verify()
        pg_group = own_cgroup(fixture.binding["pid"])
        logging = json.loads(
            fixture.sql(
                "SELECT json_object_agg(name,setting) FROM pg_settings WHERE name IN ("
                + ",".join("'" + name + "'" for name in LOG_OPTIONS)
                + ");"
            )
        )
        if logging != LOG_OPTIONS:
            raise RuntimeError("actual PG logging settings differ; no traffic started")
        save(
            output / "pg-budget.json",
            {
                "binding": fixture.binding,
                "physical": fixture.identity,
                "budget": cgroup(pg_group, PG_LIMITS),
                "actual_log_options": logging,
            },
        )
        common = {
            "reference": str(args.reference),
            "process_root": str(process_root),
            "fixture_root": str(fixture.root),
            "pg_port": fixture.binding["port"],
            "pg_cgroup": str(pg_group),
            "generator_readback": str(gen_dir / "readback.json"),
            "tracking_watcher": tracking_watcher,
        }
        save(obs_dir / "config.json", dict(common, output=str(obs_dir)))
        save(gen_dir / "config.json", dict(common, output=str(gen_dir)))
        observer = start_scope("observe", obs_dir, obs_dir / "config.json", OBS_LIMITS)
        until = time.monotonic() + 5
        while True:
            try:
                ready = load(obs_dir / "ready.json")
                if (
                    identity(ready["identity"]["pid"]) != ready["identity"]
                    or not ready["handshake"]["result"]
                ):
                    raise RuntimeError(
                        "external observer identity/PG handshake changed"
                    )
                break
            except (FileNotFoundError, json.JSONDecodeError):
                pass
            if observer.poll() is not None or time.monotonic() >= until:
                raise RuntimeError("external observer did not become ready")
            time.sleep(0.05)
        # Ten seconds of identical sampling before offered traffic. This is
        # recorded overhead, not an adjustment to the original traffic phases.
        time.sleep(10)
        if observer.poll() is not None:
            raise RuntimeError(
                "external observer failed during pre-load overhead baseline"
            )
        baseline = baseline_summary(obs_dir)
        save(output / "baseline-ended.json", baseline)
        if not baseline["within_observation_budget"]:
            raise RuntimeError(
                "external observer exceeded its pre-load observation budget; no traffic started"
            )
        generator = start_scope(
            "generator", gen_dir, gen_dir / "config.json", GEN_LIMITS
        )
        deadline = time.monotonic() + 1020
        while generator.poll() is None:
            if observer.poll() is not None or time.monotonic() >= deadline:
                stop_scope(gen_dir)
                raise RuntimeError(
                    "observer failed or diagnostic deadline expired; retain fixture"
                )
            time.sleep(0.25)
        test = load(gen_dir / "result.json")["test"]
        frozen.v.require_success(test)
        frozen.require_selected_success(
            (gen_dir / "resource-test.log").read_text(), data["test_list"]
        )
    except BaseException as caught:
        error = {
            "type": type(caught).__name__,
            "safe_message": str(caught)
            if isinstance(caught, (ValueError, RuntimeError, frozen.FixtureError))
            else "diagnostic control failed; inspect private receipts",
        }
    finally:

        def attempted(label, action):
            try:
                return action()
            except BaseException as failure:
                cleanup_errors.append(
                    {
                        "step": label,
                        "type": type(failure).__name__,
                        "message": "cleanup not confirmed; preserve owned resources and receipts",
                    }
                )
                return None

        def stop_generator():
            if generator is not None:
                if generator.poll() is None:
                    stop_scope(gen_dir)
                    generator.wait(timeout=15)
                scope_final(gen_dir, "readback.json")

        attempted("generator stop/readback", stop_generator)

        def stop_observer():
            if observer is not None:
                if not (obs_dir / "stop").exists():
                    (obs_dir / "stop").touch(exist_ok=False)
                try:
                    observer.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    stop_scope(obs_dir)
                    observer.wait(timeout=15)
                    cleanup_errors.append(
                        {
                            "step": "observer stop",
                            "type": "ObserverStopTimeout",
                            "message": "exact owned observer scope needed stop",
                        }
                    )
                scope_final(obs_dir, "ready.json")
                if observer.returncode:
                    cleanup_errors.append(
                        {
                            "step": "observer exit",
                            "type": "ObserverFailed",
                            "message": "observer failed; diagnostic coverage incomplete",
                        }
                    )

        attempted("observer stop/readback", stop_observer)
        if test is None and (gen_dir / "result.json").exists():
            test = attempted(
                "generator result readback",
                lambda: load(gen_dir / "result.json")["test"],
            )
        units = attempted(
            "owned Core units cleanup",
            lambda: frozen.cleanup_units(
                process_root, bus_env(), CORE_SHA, test["child"] if test else {}
            ),
        )

        def finish_fixture():
            nonlocal removed
            if fixture.container:
                fixture.verify()
                with (
                    (output / "postgres-original.stdout.log").open("xb") as out,
                    (output / "postgres-original.stderr.log").open("xb") as err,
                ):
                    subprocess.run(
                        [
                            "docker",
                            "--host",
                            "unix:///var/run/docker.sock",
                            "logs",
                            "--timestamps",
                            fixture.container,
                        ],
                        env=fixture.env,
                        stdout=out,
                        stderr=err,
                        check=True,
                        timeout=20,
                    )
                if error is None and not cleanup_errors:
                    fixture.cleanup()
                    removed = True

        attempted("frozen input final integrity", lambda: helpers(args.reference))
        attempted("owned PG final identity/logs/cleanup", finish_fixture)
        save(
            output / "result.json",
            {
                "utc": now(),
                "diagnostic_test_result": "FAIL" if error or cleanup_errors else "PASS",
                "error": error,
                "cleanup_errors": cleanup_errors,
                "test": test,
                "unit_cleanup": units,
                "fixture_root": str(fixture.root),
                "fixture_binding": fixture.binding,
                "cluster_removed": removed,
                "failed_fixture_retained": bool(
                    (error or cleanup_errors) and fixture.container
                ),
                "not_acceptance": True,
                "scope": "original sustained gate with extra external observation/logging; no product/threshold changes",
            },
        )
        for sig, handler in handlers.items():
            signal.signal(sig, handler)
    return 1 if error or cleanup_errors else 0


def selftest(args):
    output = private(args.output)
    if any(output.iterdir()):
        raise ValueError("selftest output must be fresh")
    # No PG, test binary, provider or product process. Exercise the OS sampler
    # at its real cadence; repeated aliases are a cost benchmark, not 3 budgets.
    group = own_cgroup(os.getpid())
    before = identity(os.getpid())
    assert identity(os.getpid()) == before
    assert "'query',query" not in WAIT_SQL and "'stage',CASE" in WAIT_SQL
    assert cgroup(Path("/sys/fs/cgroup/nonexistent-lmm-selftest"))["status"] == "absent"
    rejected = False
    try:
        cgroup(group, {"memory.max": "1"})
    except ValueError:
        rejected = True
    assert rejected
    # Exercise framing/bounds only. These are observer unit checks, not mocked
    # PG, billing, resource traffic or a substitute for the later real fixture.
    stdout_read, stdout_write = os.pipe()
    stderr_read, stderr_write = os.pipe()
    stdout = os.fdopen(stdout_read, "rb", buffering=0)
    stderr = os.fdopen(stderr_read, "rb", buffering=0)
    framed = object.__new__(Psql)
    framed.child = SimpleNamespace(stdout=stdout, stderr=stderr, stdin=io.BytesIO())
    framed.poller = selectors.DefaultSelector()
    framed.buffers = {stdout: b"", stderr: b""}
    framed.pending, framed.serial, framed.completed = None, 0, 0
    framed.error_bytes, framed.error_buffer, framed.errors = (
        0,
        b"",
        collections.Counter(),
    )
    for stream in [stdout, stderr]:
        os.set_blocking(stream.fileno(), False)
        framed.poller.register(stream, selectors.EVENT_READ)
    try:
        assert framed.send() and not framed.send() and framed.serial == 1
        os.write(stdout_write, b'{"safe":true}')
        assert framed.collect(0.002) is None and framed.pending is not None
        os.write(stdout_write, b"\nLMM_OBSERVER_FRAME_1\n")
        frame = framed.collect(0.020)
        assert (
            frame["serial"] == 1
            and frame["result"] == {"safe": True}
            and framed.completed == 1
        )
        assert framed.send()
        framed.buffers[stdout] = b"x" * 131072
        os.write(stdout_write, b"extra")
        try:
            framed.collect(0.020)
        except RuntimeError:
            oversized_rejected = True
        else:
            oversized_rejected = False
        assert oversized_rejected
    finally:
        framed.poller.close()
        stdout.close()
        stderr.close()
        os.close(stdout_write)
        os.close(stderr_write)
    checks = [
        "identity stable",
        "absent cgroup is unknown not zero",
        "wrong hard budget rejected",
        "query output excludes SQL text",
        "only one outstanding read",
        "split frame retained",
        "serial completed exactly once",
        "oversized frame rejected",
    ]
    if args.static_only:
        save(
            output / "result.json",
            {
                "static_checks": "PASS",
                "checks": checks,
                "not_measured": ["CPU overhead", "PG query cost"],
                "not_acceptance": True,
            },
        )
        return 0
    samples, started = [], time.monotonic()
    writer = Segments(output)
    cpu_start = usage.getrusage(usage.RUSAGE_SELF)
    for index in range(32):
        time.sleep(max(0, started + INTERVAL * index - time.monotonic()))
        sample = {
            "utc": now(),
            "elapsed_ms": (time.monotonic() - started) * 1000,
            "groups": [cgroup(group) for _ in range(3)],
            "process": process(os.getpid()),
            "host_stat": Path("/proc/stat").read_text(),
        }
        writer.write(sample)
        samples.append(sample)
    writer.close()
    time.sleep(max(0, started + 32 * INTERVAL - time.monotonic()))
    cpu_end = usage.getrusage(usage.RUSAGE_SELF)
    seconds = time.monotonic() - started
    cpu = cpu_end.ru_utime + cpu_end.ru_stime - cpu_start.ru_utime - cpu_start.ru_stime
    save(output / "samples.json", samples)
    save(
        output / "result.json",
        {
            "utc": now(),
            "static_checks": "PASS",
            "checks": checks,
            "samples": len(samples),
            "seconds": seconds,
            "cpu_seconds": cpu,
            "cpu_core_equivalents": cpu / seconds,
            "max_sample_gap_ms": max(
                samples[i + 1]["elapsed_ms"] - samples[i]["elapsed_ms"]
                for i in range(31)
            ),
            "psql_pg_overhead": "NOT_MEASURED; persistent PG query baseline only after run review",
            "not_acceptance": True,
        },
    )
    return 0


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="mode", required=True)
    check = sub.add_parser("selftest")
    check.add_argument("--output", type=Path, required=True)
    check.add_argument("--static-only", action="store_true")
    execute = sub.add_parser("run")
    for name in ["reference", "output", "process-root", "repository"]:
        execute.add_argument("--" + name, type=Path, required=True)
    execute.add_argument("--expected-script-sha256", required=True)
    execute.add_argument("--tracking-ready", type=Path, required=True)
    for mode in ["observe", "generator"]:
        sub.add_parser(mode).add_argument("--config", type=Path, required=True)
    args = parser.parse_args()
    if args.mode == "selftest":
        return selftest(args)
    if args.mode == "observe":
        observe(load(args.config))
        return 0
    if args.mode == "generator":
        return generation(load(args.config))
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
