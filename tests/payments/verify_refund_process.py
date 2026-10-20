#!/usr/bin/env python3
"""Independent synthetic refund OS recovery; consumes sealed binaries, never builds.

The Go worker binary executes production module/storage/RPC code with a counted
loopback Stripe transport. It is explicitly not lmm-extensions main acceptance.
"""
from __future__ import annotations
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "tests/architecture_completion"))
from pg_fixture import Fixture, FixtureError, clean_environment, write_private
from build_lease import private_directory
CASE = "real_refund_dispatch_response_loss_restarts_core_and_go_worker"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources(root=REPO, mode="git"):
    if mode == "tree":
        entries = sorted(p for p in root.rglob("*") if p.is_file() and ".git" not in p.relative_to(root).parts)
        if any(p.is_symlink() for p in entries):
            raise FixtureError("frozen source tree contains a symbolic link")
        return {str(p.relative_to(root)): digest(p) for p in entries}
    command = ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", "services/core", "services/extensions", "contracts", "deployment", "scripts", "tests/payments", "tests/architecture_completion"]
    paths = subprocess.check_output(command, cwd=root).split(b"\0")
    return {os.fsdecode(p): digest(root / os.fsdecode(p)) for p in sorted(set(paths)) if p and (root / os.fsdecode(p)).is_file()}


def verify_result(result):
    if result.get("case") != CASE or result.get("result") != "PASS":
        raise FixtureError("wrong refund case or result")
    required = ("core_os_restart", "go_module_worker_os_restart", "core_ack_before_provider_post", "unknown_original_key_preserved", "duplicate_event_replay_checked", "signed_event_id_payload_conflict_denied", "success_after_release_denied", "journal_balance_checked", "children_reaped")
    if any(result.get(key) is not True for key in required):
        raise FixtureError("missing refund process invariant")
    for key, expected in (("provider_posts", 1), ("refund_releases", 1), ("refund_commits", 0), ("dispatch_bindings", 1)):
        if type(result.get(key)) is not int or result[key] != expected:
            raise FixtureError("wrong exact refund invariant: " + key)
    pids = result.get("product_pids")
    if not isinstance(pids, list) or len(pids) != 4 or len(set(pids)) != 4 or any(type(pid) is not int or pid <= 0 or Path(f"/proc/{pid}").exists() for pid in pids):
        raise FixtureError("four distinct reaped OS process IDs required")
    if result.get("scope") != "real Core main and Go payment module worker; synthetic counted TCP provider; not Go main acceptance":
        raise FixtureError("worker fixture mislabeled as Go main acceptance")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root",type=Path,default=REPO,help="immutable source tree used to compile all sealed binaries")
    parser.add_argument("--source-mode",choices=("git","tree"),default="git",help="tree hashes every frozen copied file without requiring Git metadata")
    parser.add_argument("--lock-file",type=Path,default=REPO / "output/progress-20261020/development/heavy.lock",help="shared original workspace execution lock")
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--process-root", type=Path, required=True)
    parser.add_argument("--compiled-inputs", type=Path, required=True, help="sources() JSON frozen before compiling test and worker artifacts")
    for kind in ("core", "go", "test", "worker"):
        parser.add_argument(f"--{kind}-binary", type=Path, required=True)
        parser.add_argument(f"--{kind}-sha256", required=True)
    args = parser.parse_args()
    for kind in ("core", "go", "test", "worker"):
        binary, sha = getattr(args, kind + "_binary"), getattr(args, kind + "_sha256")
        if not binary.is_absolute() or binary.is_symlink() or not binary.is_file() or not re.fullmatch("[a-f0-9]{64}", sha) or digest(binary) != sha:
            parser.error(kind + " sealed binary mismatch")
    if not args.source_root.is_absolute() or args.source_root.is_symlink() or not (args.source_root / "services/core").is_dir():
        parser.error("source root must be an absolute source tree directory")
    if not args.lock_file.is_absolute() or args.lock_file.is_symlink() or not args.lock_file.parent.is_dir():
        parser.error("lock file must use an existing absolute coordination directory")
    if not args.output_root.is_absolute() or args.output_root.is_symlink():
        parser.error("output root must be an absolute private directory")
    if args.source_mode=="tree" and args.output_root.resolve().is_relative_to(args.source_root.resolve()):
        parser.error("frozen tree output must be outside immutable sources")
    if not args.process_root.is_absolute() or len(os.fsencode(args.process_root)) > 35 or args.process_root.is_symlink():
        parser.error("process root must be private, short and absolute")
    private_directory(args.output_root)
    private_directory(args.process_root)
    if any(args.process_root.iterdir()):
        parser.error("process root must be fresh and empty")
    before = sources(args.source_root,args.source_mode)
    if json.loads(args.compiled_inputs.read_text()) != before:
        parser.error("compiled-input source manifest differs from current tracked and untracked sources")
    fixture = None
    started = time.monotonic()
    with args.lock_file.open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            if sources(args.source_root,args.source_mode) != before:
                raise FixtureError("source drift while acquiring execution lock")
            for kind in ("core", "go", "test", "worker"):
                if digest(getattr(args, kind + "_binary")) != getattr(args, kind + "_sha256"):
                    raise FixtureError("sealed binary changed while waiting for execution lock")
            env = {key: value for key, value in clean_environment().items() if not key.startswith(("LMM_", "SQLX_", "PAYMENT_"))}
            env["TMPDIR"] = str(args.process_root)
            discovery = subprocess.run([str(args.test_binary), "--list"], capture_output=True, text=True, env=env, check=True).stdout
            if discovery.splitlines().count(CASE + ": test") != 1:
                raise FixtureError("selected real refund case not discovered exactly once")
            fixture = Fixture(args.output_root)
            fixture.start()
            password = dict(line.split("=", 1) for line in (fixture.root / "postgres.env").read_text().splitlines())["POSTGRES_PASSWORD"]
            env["DATABASE_URL"] = f"postgres://a09_admin:{password}@127.0.0.1:{fixture.binding['port']}/postgres?sslmode=disable"
            for kind, source in (("core", "core"), ("go", "go"), ("bootstrap", "worker")):
                env[f"LMM_PAYMENT_{kind.upper()}_BIN"] = str(getattr(args, source + "_binary"))
                env[f"LMM_PAYMENT_{kind.upper()}_SHA256"] = getattr(args, source + "_sha256")
            command = [str(args.test_binary), CASE, "--exact", "--nocapture", "--test-threads=1"]
            write_private(fixture.root / "refund-test.log", "")
            with (fixture.root / "refund-test.log").open("a") as log:
                child = subprocess.Popen(command, cwd=args.source_root / "services/core", env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                # This group is created solely for this exact spawned test;
                # never discover/kill processes by binary name or a stale PID.
                write_private(fixture.root / "owned-process-group.json", json.dumps({"leader_pid": child.pid, "group_id": child.pid, "source": "Popen(start_new_session=True) for sealed refund test"}) + "\n")
                try:
                    code = child.wait(timeout=180)
                except (subprocess.TimeoutExpired, KeyboardInterrupt):
                    try:
                        os.killpg(child.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    child.wait(timeout=10)
                    write_private(fixture.root / "owned-process-group-stop.json", json.dumps({"leader_pid": child.pid, "group_id": child.pid, "signal": "SIGKILL", "leader_reaped": True, "exit_code": child.returncode}) + "\n")
                    raise
            if code:
                raise FixtureError("real refund test process failed")
            log = (fixture.root / "refund-test.log").read_text()
            summaries = re.findall(r"^test result: .*?$", log, re.MULTILINE)
            if len(summaries) != 1 or not re.fullmatch(r"test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in [0-9.]+s", summaries[0]):
                raise FixtureError("selected refund test did not run exactly once")
            receipts = list(args.process_root.glob("pay-*/refund-result.json"))
            if len(receipts) != 1:
                raise FixtureError("missing unique real refund receipt")
            result = json.loads(receipts[0].read_text())
            verify_result(result)
            for kind in ("core", "go", "test", "worker"):
                if digest(getattr(args, kind + "_binary")) != getattr(args, kind + "_sha256"):
                    raise FixtureError("sealed binary drift")
            if sources(args.source_root,args.source_mode) != before:
                raise FixtureError("source added, removed or changed during execution")
            fixture.verify()
            fixture.cleanup()
            receipt = {"result": "PASS", "case": CASE, "source_root":str(args.source_root),"source_mode":args.source_mode,"execution_lock":str(args.lock_file),"source_sha256": before, "binary_sha256": {kind: getattr(args, kind + "_sha256") for kind in ("core", "go", "test", "worker")}, "scope": result["scope"], "process_result": str(receipts[0]), "cluster_removed": True, "independent_acceptance": True, "seconds": round(time.monotonic()-started, 3)}
            write_private(fixture.root / "refund-acceptance.json", json.dumps(receipt, sort_keys=True) + "\n")
            print("PASS: scoped refund OS recovery; receipt " + str(fixture.root / "refund-acceptance.json"))
            return 0
        except (Exception, KeyboardInterrupt) as error:
            target = fixture.root if fixture else args.output_root
            write_private(target / "refund-failure.json", json.dumps({"result": "FAIL", "error_type": type(error).__name__, "fixture_retained": fixture is not None, "seconds": round(time.monotonic()-started, 3)}) + "\n")
            print("FAIL: " + type(error).__name__ + "; private evidence retained at " + str(target), file=sys.stderr)
            return 1


if __name__ == "__main__":
    raise SystemExit(main())
