# Rust Backend Blue-Green Deployment

This Arch Linux native deployment framework separates `lmm-api-rs` process rollout from database migration and currently handles only internal probes.

For production ownership, read the real-time result from
`apps/api-rust/tests/fixtures/routes/migration-gate.tsv`.

As of 2026-08-09, the working tree has 352 routes and all are still marked as Go owner. Entries that are only candidate mounts, unverified diffs, or blocked rows are not production approvals. Legacy status mismatch in gate checks also remains. Therefore, Rust candidates should never be described as having taken traffic or replaced Go.

On 2026-08-01 in ArchDmit read-only verification, production traffic still came from Go using PostgreSQL and dedicated Valkey `127.0.0.1:6380`; Rust slot was not running. Nginx Rust upstream represented only internal probe routing.

A running slot, `active-slot` symlink, `/readyz` pass, or previous rehearsal logs do not prove production ownership by Rust.

## Boundaries and Invariants

- Blue listens on `127.0.0.1:3100`; green listens on `127.0.0.1:3101`.
- Artifacts are installed to `/opt/lmm-api-rs/releases/<revision>/`; slot `current` symlinks are atomically swapped via same-filesystem `rename(2)`.
- Deployment and nginx assets are installed under `/usr/lib/lmm-api-rs/deploy/`; `/usr/local/sbin` only contains shims. Transient units do not depend on caller working directory.
- PostgreSQL migrator is a separate transaction and must never be in `ExecStartPre`, app startup, or blue-green switch scripts.
- `/readyz` must validate PostgreSQL, schema contract, and mounted route access/permissions against real objects and must run checks in parallel.
- If API-token is mounted, schema gate includes token table shape plus `EXPLAIN INSERT` (ID sequence default), UPDATE, and DELETE capability. `EXPLAIN` does not write business rows.
- With global fail-closed API rate limiting enabled, Valkey is required and failures return HTTP 503.
- With global fail-closed disabled, Valkey is cache acceleration and failure returns HTTP 200 with `degraded`.
- `/livez` reports process liveness only.
- Nginx upstream include is written to `.next`, then atomically moved with `mv -T`, followed by `nginx -t` and reload. On failure, old include is restored from audit directory.
- Nginx does not retry non-idempotent requests.
- Current route ownership include only exposes three loopback-only internal GET/HEAD probe locations.
- Every deployment transaction is serialized with `flock`.
- Deployment logs are stored in `/var/log/lmm-api-rs/deployments/<UTC>-<revision>/` and must include non-connection hash, probe results, previous and next upstream state.
- Switch write flow writes `PREPARED` with old/new revision and hash before reload; on successful reload writes `COMMITTED`.
- If process dies during critical window and is SIGKILLed, next startup uses real TLS build canary to detect running worker.
  - If new worker is healthy, commit and stop old slot.
  - Otherwise restore old worker/upstream and stop unused new slot.
- Reconcile journal always preserves original PREPARED artifact revision.
- Only internal GET/HEAD probes are currently managed after switch.
- After switch, send SIGTERM to old slot directly.
- Rust readiness enters draining state, rejects new requests, then exits with bounded draining controlled by `LMM_DRAIN_TIMEOUT_SECONDS`.
- Drain timeout max is 40 seconds and must stay below `TimeoutStopSec=45s` so systemd does not preempt requests.
- Before business traffic cutover, Rust must provide HTTP/SSE/WebSocket lifecycle metrics; shell output-based checks cannot substitute for true drain behavior.

## Initial Installation

Create a non-login system user:

```bash
sudo useradd --system --home-dir /var/lib/lmm-api-rs --shell /usr/bin/nologin lmm-api-rs
sudo deploy/nginx/install-nginx-split.sh install
sudo deploy/backend-rust/install-lmm-api-rs-blue-green.sh
sudo install-nginx-rust-routing
```

Copy `common.env.example` to `/etc/lmm-api-rs/common.env` and fill real PostgreSQL/Valkey URLs.

Keep permissions `0600 root:root`. Do not place sensitive values in repo, command lines, unit files, or audit logs.

Repository-managed `deploy/nginx/new-api.conf` already includes
`/etc/nginx/snippets/lmm-api-rs-probe-locations.conf` in the TLS server block; upstream include files live in nginx `http` context.

`install-nginx-rust-routing` does not require any Rust slot running. It performs atomic installation of port-9 disabled upstream and loopback-only GET/HEAD ownership, runs `nginx -t` and reload, records `is-active`, and sets state to `none`. First deployment then chooses blue.

Failure during write/test/reload/health checks restores both previous files.
Installer and deployer share one lock.

`deploy.conf.example` has the TLS canary example:

```bash
curl --resolve api.lmm.best:443:127.0.0.1
```

This keeps production SNI/Host and explicit CA bundle so checks pass through real TLS server, route ownership, and active upstream. Probe allows GET/HEAD and must remain loopback-only.

Do not configure POST retries in nginx.

## Build and Upgrades

Inject immutable revision before build and compute artifact hash:

```bash
cd rust
LMM_BUILD_REVISION="$(git rev-parse HEAD)" cargo build --release --locked -p lmm-api-rs
sha256sum target/release/lmm-api-rs
```

Run read-only plan first (no instance start, no traffic switch). It validates config, hash, and production route disablement gates and reads current slot from nginx upstream only.

It must not fix state/journal, start/stop slots, reload nginx, or mutate runtime state:

```bash
sudo deploy-lmm-api-rs --artifact /absolute/path/lmm-api-rs \
  --sha256 <sha256> --revision <git-sha> --dry-run
```

Go still owns production service routes today; there is no default production cutover command.

A one-time, internal-probes-only cutover may occur only with explicit written approval:

```bash
revision=<git-sha>
export LMM_RS_CUTOVER_APPROVAL=GO_FREEZE_OVERRIDE_INTERNAL_PROBES
sudo --preserve-env=LMM_RS_CUTOVER_APPROVAL deploy-lmm-api-rs \
  --artifact /absolute/path/lmm-api-rs --sha256 <sha256> --revision "$revision" --systemd-run \
  --approve-cutover --cutover-target internal-probes --cutover-revision "$revision"
unset LMM_RS_CUTOVER_APPROVAL
```

This path supports only fixed target/revision and cannot set business routes. If
`/etc/lmm-api-rs/production-routing.enabled` exists, the deployer still blocks the run.

It is intentional that this command returns immediately. Do not treat transient unit completion (`systemctl show`) as proof. Use:

- `/var/log/lmm-api-rs/deployments/*/result`
- `/opt/lmm-api-rs/active-slot`
- TLS build identity

Failure of transient unit journaling is diagnostic only and is not durable proof.

The deployer installs inactive slot, starts it, checks `/livez`, `/readyz`, revision, pre-heats mounted read-only status/public-content paths, then writes PREPARED journal and switches internal probe upstream atomically.

Then it reloads nginx and verifies via real TLS `/readyz` and `/build` that nginx selects target revision.

Nginx reload is async; old workers may still serve briefly. Canary rechecks are bounded.

Convergence is confirmed only when readiness, revision, and slot all match.

Rollback order:

1. start old slot and ensure direct-ready
2. atomically restore old upstream
3. `nginx -t` / reload / is-active
4. TLS canary confirms old revision
5. stop new slot

Any failure writes `NEEDS_ATTENTION` and never reports success.

Old release is never deleted in the same transaction.

## Fault Injection and Validation

Repository tests cover error hash, immutable release, concurrent lock, accidental business route enablement, loopback GET/HEAD ownership, disallowing non-idempotent retry, installer rollback, and SIGKILL replay:

```bash
bash deploy/backend-rust/test-blue-green.sh
bash -n deploy/backend-rust/*.sh
shellcheck deploy/backend-rust/*.sh
```

In staging you may set:

```bash
LMM_DEPLOY_FAIL_AT=install|ready|kill-before-reload|nginx-test|switch|kill-after-reload
```

to validate failure points.

The two `kill` modes SIGKILL the transaction and must only be used in isolated rehearsal.

On next transaction, PREPARED journal and TLS canary reconcile state and auto-fix active slot/upstream as needed.

Do not keep fail variables in production steady state.

## Production Enablement Gate

There is currently no switch to turn on business traffic ownership.

Creating `/etc/lmm-api-rs/production-routing.enabled` blocks deployer execution.

Even with PostgreSQL running, production ownership requires independent verification of schema contracts, forward-only boundaries, and canaries.

Business route should be enabled only after these are all complete and separately reviewed:

1. Re-validated active DB identity, PostgreSQL schema contract, and forward-only boundary; if SQLite residue remains, complete migration and rollback strategy verified.
2. Route, auth, quota, billing, streaming, and error contract differencing completed between Rust and Go.
3. Expand/contract schema strategy; N and N-1 compatibility proven; singleton lease for background jobs.
4. WebSocket/SSE drain and reconnect behavior defined; non-idempotent requests are not retried by proxy.
5. Staging completed readiness, safe GET canary, failover, and rollback rehearsals.

## ArchDmit Internal Probe Rehearsal

On 2026-08-01, a non-production traffic-cutting rehearsal was completed on ArchDmit:

- Dedicated rehearsal PostgreSQL `lmm_api_rs_rehearsal` and dedicated Valkey `127.0.0.1:6380` were used; this is isolated from production DB identity.
- First publication used port 9 bootstrap to blue, then switched to green on the same revision.
- TLS build canary confirmed slot identity on each transition.
- After real nginx reload, first canary hit old worker and returned 502; deployer now retries readiness + revision + slot within a bounded window.
- SIGKILL applied to deploy process post-reload; independent systemd reconcile used TLS worker and PREPARED -> COMMITTED to finalize, retaining original revision.
- `/api/status` stayed 200; `/v1/models` remained Go auth and returned 401; Rust internal probe from public source returned 403.
- Go process did not restart (`NRestarts=0`) during the entire rehearsal.

This shows internal probe blue-green and crash recovery are operational. It does not prove PostgreSQL production migration acceptance, full Rust route parity, or complete backend cutover.

If production already points to PostgreSQL, validate with fresh live checks, boundary, schema, and canary evidence.

Do not treat this historical rehearsal as current acceptance.

## Rust Business Migration State

`apps/api-rust/tests/fixtures/routes/migration-gate.tsv` is the only source of truth for migration progress.

Updating docs or adding candidate code cannot by itself change gate conclusions.

Route count, mount status, differential validation, and approval state change as local migration work progresses. Do not treat history docs as current migration progress.

A new production ownership claim is valid only from TSV + rerun commands.

The owner in production is decided only by TSV values; until explicit production handover authorization, business routes remain with Go.

`migration_routes` candidate mounting under root router is determined by TSV entries.

Candidate source, even local partial mount, does not equal production route ownership and cannot count as completion.

Only TSV rows with independent differential, approval, and owner status may be treated as done.

No frozen local Go oracle is retained in repo; historical differential tests must explicitly use an external immutable revision tree, for example:

```bash
LMM_GO_ORACLE_ROOT=/absolute/path/to/5418ce6b6d45ed69167b0aad53f2f595e5bc8de9
```

Do not use local uncommitted `apps/api-go` state as frozen evidence or rollback baseline.

Target architecture states PostgreSQL 18 as the single authoritative persistence layer. Valkey only carries rebuildable cache, session/revoke propagation, and rate limit state.

Public content in candidate path is cache-aside: cache miss/fail/timeout must fallback to PostgreSQL; cache write failure must not be treated as success.

If global fail-closed rate limiting is enabled, Go and Rust must share the same dedicated Valkey URL and key contract; no business ownership may be moved from Go to Rust otherwise.

Current verification and gate checks must use read-only commands only:

```bash
awk -F '\t' 'NR > 1 { owner[$8]++; mount[$5]++; diff[$6]++ }
  END { for (k in owner) print "owner", k, owner[k];
        for (k in mount) print "mount", k, mount[k];
        for (k in diff) print "differential", k, diff[k] }' \
  apps/api-rust/tests/fixtures/routes/migration-gate.tsv
bash apps/api-rust/tests/scripts/check-migration-plan.sh
bash apps/api-rust/tests/scripts/check-real-integration-gates.sh
```

These commands do not start services, modify upstream, or expose credentials.

If gate validation fails (including inconsistent `legacy-go`/mounted state), fix the gate first.

Never modify candidate rows to force pass.

Only when every route independently completes TCP differential, integration gate, and human review may ownership be updated by separate commits.
