# Frontend and backend upgrades

The installed package exposes one public operator CLI. On the current
production host (ArchDmit) its exact path is `/usr/bin/lmm-api-go`.
Serving, health checks, HTTP requests, and deployment are subcommands of that
CLI; source-tree deployment helpers and a second public deploy command are not
supported.

Use the native CLI for application-level server control:

```bash
ssh ArchDmit /usr/bin/lmm-api-go status
ssh ArchDmit /usr/bin/lmm-api-go doctor
ssh ArchDmit /usr/bin/lmm-api-go request --show-status /api/status
```

SSH is only the transport here. Host-level inspection (systemd, filesystem,
memory, and journal reads) remains separate and read-only unless a guarded
deployment transaction has been explicitly authorized.

## Current production boundary

The 2026-08-14 read-only audit found `api.lmm.best` running the Go backend with
PostgreSQL and dedicated Valkey. The canonical lowercase
`pg-write-boundary` and `cutover-journal` agree on the transaction, schema and
revision; the journal phase is `COMPLETE`; and `post-cutover-verify.json`
attests the PostgreSQL historical migration as verified. Re-run the sanitized
`inspect-state.sh` gate before every mutation instead of treating this dated
observation as permanent evidence. Rust remains internal-probe-only and does
not own production business traffic.

## Frontend: zero-downtime static releases

Build the frontend into an immutable `apps/web/dist` directory in CI or a
marker-owned deployment workspace, then use the installed CLI's production
frontend-only transaction. The exact package and deployment identity are
required by the CLI; a frontend publication does not restart the backend
service or nginx.

```bash
sudo /usr/bin/lmm-api-go deploy production \
  --frontend-only \
  --host ArchDmit \
  --deployment-id <deployment-id> \
  --core-package /path/to/lmm-api-core.pkg.tar.zst \
  --execute-remote switch
```

Use the same CLI transaction phases (`preflight`, `inspect`, `build`,
`package`, `backup`, `watchdog`, `switch`, `confirm`, `rollback`, and
`cleanup`) for preview, preparation, confirmation, rollback, and cleanup.
The default is read-only preflight. Remote mutation requires the explicit
execution option, verified host identity, all required backup copies, and the
appropriate current-turn authorization.

The frontend transaction validates `index.html` and local asset references,
copies into same-filesystem staging, and atomically replaces
`/srv/lmm-api-frontend/current`. Before switching, it copies `/static` files
into the cumulative immutable store at `/srv/lmm-api-frontend/assets`,
rejecting a same-name/different-content collision, then makes the versioned
release read-only. A browser holding the previous `index.html` can therefore
continue lazy-loading its hashed chunks after a switch. `flock` serializes
publishers and retention always includes the current release. Static assets
are not garbage-collected in this phase; any future GC must preserve assets
referenced by every retained release.

Rollback is performed by the same guarded transaction and restores the exact
verified prior frontend release. Do not invoke a source-tree publisher or
construct a parallel public helper command.

The site uses its own `/etc/nginx/lmm-api-mime.types`; it never creates or
overwrites nginx's global `/etc/nginx/mime.types`. Publish all nginx inputs
with the controlled transaction rather than copying them individually. The
installer serializes with `flock`, reserves a unique non-overwritable backup,
and manages `/etc/nginx/lmm-api-mime.types`, the HTTP `map`, server locations,
and `/etc/nginx/conf.d/new-api.conf`. Each candidate is installed as a
root-owned `0644` same-directory temporary file and published atomically. Only
after every file is in place does it run `nginx -t`, reload nginx, and verify
the unit remains active. Failure restores the prior presence or exact content
of every managed file and repeats validation, reload, and service checks.

The server-scoped template explicitly includes `/etc/nginx/lmm-api-mime.types`.
The template sends known backend route families to port 3000, preserves
WebSocket/SSE behavior, serves the shared `/static` asset store with immutable
caching, and makes entry points revalidate. Missing static or root-public
assets return 404 instead of SPA HTML. Production `/terms` and `/privacy`
remain exact aliases to the legal HTML files.

## Backend service and upgrades

Systemd runs the canonical service entry directly:

```ini
ExecStart=/usr/bin/lmm-api serve
```

Backend selection and status remain launcher subcommands (`lmm-api-go select`
and `lmm-api-go status`); deployment phases are invoked as
`lmm-api-go deploy ...`.
Production backend changes are autonomous, locked transactions with offline
backup verification, health validation, persistent audit output, a ten-minute
rollback watchdog, and explicit confirmation before the transaction is
considered complete. The transaction continues without the initiating shell
or API connection, but restarting the only process creates a bounded
interruption. It is not a zero-downtime or blue/green deployment.

Before invoking a subcommand on an existing target, verify that the installed
core package actually provides this launcher protocol and that systemd uses
`ExecStart=/usr/bin/lmm-api-go serve`. Legacy provider binaries may start the
backend when given an unknown command, so `status`, `deploy`, and `--help` are
not safe inspection commands until the protocol is proven. Use systemd/package
metadata, the running PID, sanitized process-environment scheme checks, and
explicit HTTP probes for a legacy target; upgrade the core package through the
guarded transaction before using deployment phases.

Always read `apps/api-rust/tests/fixtures/routes/migration-gate.tsv` for the
current route ownership and approval state; prose is not an authority for
route counts.

## Rust internal-probe blue/green foundation

The native Rust blue/green deployment foundation uses blue on `127.0.0.1:3100`
and green on `127.0.0.1:3101`. Nginx ownership remains only
loopback-restricted GET/HEAD liveness, readiness, and build probes. Releases,
per-slot symlinks, nginx upstream publication, PREPARED/COMMITTED journals,
crash reconciliation, and bounded SIGTERM drain are independent of the Go
process. Neither a mounted candidate nor a historical internal-probe rehearsal
owns production traffic; see `docs/rust-blue-green.md` and the migration TSV.

This foundation is not a production API cutover. Production business routes
remain on the approved backend until every route passes independent
differential gates and the PostgreSQL cutover is approved.

## PostgreSQL production migration and reconciliation prerequisite

The verified SQLite-to-PostgreSQL copier and autonomous coordinator remain the
only approved path for a new migration or reconciliation. A live target may
already run Go on PostgreSQL and dedicated Valkey after a historical cutover;
that runtime fact does not prove that the current boundary, schema contract,
or authenticated canaries were accepted. When the active process is PostgreSQL
but the current `PG_WRITE_BOUNDARY`/journal is absent or post-cutover
verification failed, stop and reconcile before changing backend ownership.
The coordinator stops the Go writer when a SQLite source is still involved,
backs up and verifies the source, copies into a fresh versioned schema, durably
marks the forward-only boundary before publishing PostgreSQL and Valkey
configuration, and runs public plus authenticated canaries. The strict journal,
immutable candidate hash, `--reconcile-only` path, and systemd boot gate make
process death and reboot idempotently recoverable.

The one-time database cutover is not zero downtime: the SQLite freeze stops
the sole Go process and disconnects active HTTP, SSE, and WebSocket
connections. Production remains prohibited until the isolated rehearsal passes
and an operator explicitly approves the maintenance window.

Before business traffic moves to Rust, route/auth/quota/billing/streaming
parity, expand/contract migrations compatible with N and N-1, singleton
background-job ownership, authenticated canaries, and graceful SSE/WebSocket
draining and reconnection remain mandatory. Nginx must not automatically retry
non-idempotent requests.

The recommended release order is frontend-only publication (only when its API
requests remain Go-compatible), observation and human confirmation, then Rust
internal probes, Rust business canaries, and finally the guarded backend
switch. A mounted Rust slot, a successful `/readyz`, or an old blue/green
rehearsal is never a substitute for the route gate and paired listener evidence.
