# LMM deployment path map

The installed package has one public operator entry point: `/usr/bin/lmm-api`.
Use `lmm-api deploy ...` for deployment phases and `lmm-api serve` for the
systemd service. Do not document or invoke a source-tree deployment helper or
a second public CLI.

## Controller and package inputs

| Purpose | Current path or entry point |
| --- | --- |
| Go artifact | `apps/api-go/out/lmm-api` |
| Rust artifacts | `apps/api-rust/target/release/lmm-api-rs`, `lmm-db-migrate` |
| Frontend build | `apps/web/dist` |
| Local split-package builder | `packaging/local/lmm-api-split/build-local-package.sh` |
| Local package output | `packaging/local/lmm-api-split/out` by default |
| Persistent controller work | `${XDG_STATE_HOME:-$HOME/.local/state}/lmm-api/deploy-work/<deployment-id>` |
| Durable controller backups | `$HOME/backup/lmm-api/<verified-host>/<deployment-id>` |
| Read-only production pressure report | `.agents/skills/lmm-deploy-safely/scripts/resource-pressure-report.sh` |

The local split package installs frontend files at
`/usr/share/lmm-api/frontend-dist`. Deployment publishes immutable frontend
releases through the installed CLI transaction.

## Historical parity oracle input

Historical Go/Rust differential scripts accept an optional external immutable
Go source tree through `LMM_GO_ORACLE_ROOT`. Set it to the absolute path of the
exact revision-named tree, for example:

```bash
LMM_GO_ORACLE_ROOT=/absolute/path/to/5418ce6b6d45ed69167b0aad53f2f595e5bc8de9
```

Keep this input outside the repository, require the consumer's existing
revision and checksum checks to pass, and treat it only as parity evidence.
Never treat the current dirty `apps/api-go` working tree as a frozen oracle or
substitute it when `LMM_GO_ORACLE_ROOT` is absent. This contract does not alter
deployment backup, rollback, or retention requirements.

## Shared installed package layout

| Purpose | Current path |
| --- | --- |
| Launcher and public CLI | `/usr/bin/lmm-api` |
| Go backend | `/usr/lib/lmm-api/backends/go/lmm-api` |
| Rust backend and migrator | `/usr/lib/lmm-api/backends/rs/` |
| Backend selection | `/etc/lmm-api/backend.conf` |
| Application environment | `/etc/lmm-api/lmm-api.env` |
| systemd unit | `/usr/lib/systemd/system/lmm-api.service` |
| Runtime state | `/var/lib/lmm-api` via `StateDirectory=lmm-api` |
| Service port | `3000` |

The AUR matrix consists of one core package (`lmm-api-bin` or `lmm-api-git`)
and a Go and/or Rust provider package. Package installation does not
authorize starting, restarting, enabling, or switching a service. The service
unit invokes exactly `/usr/bin/lmm-api serve`.

Before using the transaction on an existing target, verify that the installed
core really provides this launcher protocol. A legacy `/usr/bin/lmm-api` may be
the provider binary itself and may start the backend when given an unknown
subcommand. Such a target is pre-transaction: inspect it with systemd, package,
process, and sanitized HTTP probes, then upgrade the core package through the
guarded path before calling `deploy` phases.

## Production transaction

| Purpose | Current path or entry point |
| --- | --- |
| Controller entry point | `/usr/bin/lmm-api deploy production ...` |
| Target activator | Immutable payload under the marker-owned deployment workspace |
| Default SSH alias | `ArchDmit` |
| Required static hostname | `arch-dmit` |
| Target work root | `/var/lib/lmm-api-go/deploy-work/<deployment-id>` (the service-managed path resolves under `/var/lib/private/lmm-api-go`) |
| Target backup root | `/var/lib/lmm-api-go/deploy-backups/<deployment-id>` (the service-managed path resolves under `/var/lib/private/lmm-api-go`) |
| Off-host backup root | `/home/arch/.local/state/lmm-api-production-backups/<deployment-id>` on the ArchCzy host through the case-sensitive SSH alias `archczy` |
| Frontend release root | `/srv/lmm-api-frontend` |
| Frontend releases | `/srv/lmm-api-frontend/releases/<version>` |
| Active frontend | `/srv/lmm-api-frontend/current` |
| Backend service | `lmm-api.service` |

The supported phases are `preflight`, `inspect`, `build`, `package`,
`backup`, `watchdog`, `switch`, `confirm`, `rollback`, and `cleanup`. The
default is read-only preflight. Remote mutation requires explicit execution,
verified role/host identity, and current-turn authorization.

The pressure report is a separate read-only observer, not a deployment phase.
Run it on `ArchDmit` with the exact `arch-dmit` hostname check; it does not
install a timer, restart `lmm-api`, clear swap, or remove files. Its report
uses the 20 GiB root / 951 MiB RAM production profile as a visible reference
and includes the actual service cgroup memory and restart counters.

The transaction is marker-owned and persistent. Before a switch it requires
the role-appropriate target/controller/off-host backup set, checksum
verification, encrypted secret-bearing controller and off-host archives, and
a persistent ten-minute watchdog armed before switching. A switch ends in
`AWAITING_CONFIRMATION`; only exact-release identity checks and explicit
confirmation produce `CONFIRMED`. Automatic rollback never restores a
database.

## Existing database and cutover state

`deploy/backup/backup-sqlite-to-archczy.sh` is only for an explicitly verified
SQLite source. It creates an online SQLite backup in a temporary directory and
publishes a checksum pair to `/var/backups/lmm-api/sqlite/<instance>` on
the ArchCzy host via SSH alias `archczy`; it does not create a controller copy.
Do not invoke it when the live
service uses PostgreSQL.

The live service may already use Go with PostgreSQL and dedicated Valkey after a
historical cutover. Inspect the running process environment, active listeners,
`/var/lib/lmm-api-cutover`, and `/var/log/lmm-api-cutover` together. A historical
`SUCCESS_POSTGRES` result is not acceptance when the post-cutover verification
failed or the current `PG_WRITE_BOUNDARY`/journal is absent; stop and reconcile
before any migration, backend selection, or rollback. Never infer Rust business
ownership from PostgreSQL, Valkey, Rust artifacts, or an internal-probe
rehearsal.

For the 2026-08-09 read-only audit, ArchDmit's running service classified as Go
with PostgreSQL and Valkey on port 6380, while Rust slots were inactive and the
business route gate remained Go-owned. This is a time-stamped observation, not
a substitute for the next preflight.

## Rust internal-probe blue/green

| Purpose | Current path |
| --- | --- |
| Immutable releases | `/opt/lmm-api-rs/releases/<revision>` |
| Slot links | `/opt/lmm-api-rs/slots/{blue,green}/current` |
| Configuration | `/etc/lmm-api-rs` |
| Durable incoming artifacts | `/var/lib/lmm-api-rs/artifacts` |
| Deployment audit | `/var/log/lmm-api-rs/deployments/<transaction>` |
| Blue/green ports | `3100`, `3101` |
| Entrypoint | `deploy/backend-rust/deploy-lmm-api-rs.sh` |

This mechanism owns internal probes only, not production business traffic.

## Workspace and backup lifecycle

The controller workspace is a transaction workspace, not durable backup
storage. Keep its marker and terminal status for audit, but remove exact
`staging`, `tmp`, and cache children after `CONFIRMED` or `ROLLED_BACK` and
after the controller, target, and off-host copies have passed checksum and
decryption verification. Production target workspaces follow the same rule;
the target backup root and off-host root are durable and must not be removed
by workspace cleanup. Private directories are `0700`, manifests/status and
encrypted archives are `0600`, and no secret-bearing plaintext may leave the
target. Never use `/tmp`, `/var/tmp`, an unresolved glob, or a broad root as a
deployment or cleanup target.

On the small production root filesystem, stop new builds at 80% used (90% is
an emergency) and keep at least 4 GiB free before a package/backup
transaction. A terminal workspace older than 24 hours may be pruned oldest
first only after its durable copies and checksums are verified; retain the
active release, latest-known-good snapshot, and any unconfirmed transaction.

## Other retained deployment state

| Component | Backup or state root |
| --- | --- |
| nginx split installer | `/var/lib/lmm-api-nginx-deploy/backups` |
| lmm-api-go edge-policy assets | `/usr/share/lmm-api-go/edge-policy` |
| edge-policy transaction restore | `<deployment>/config-restore/nginx-edge` |
| DB-IP country database | `/var/lib/geoip2/DBIP-Country-Lite.mmdb` |
| fallback nginx installer | `/var/lib/lmm-api-rs-fallback-nginx/backups` |
| dedicated Valkey installer | `/var/lib/valkey-lmm-api-deploy/backups` |
| database cutover | `/var/lib/lmm-api-cutover`, `/var/log/lmm-api-cutover` |

These older mechanisms have independent retention. Do not broaden cleanup to
`/tmp`, backup roots, release roots, or another deployment's workspace.

## Temporary-path findings

Future deployment work must redirect task-specific caches, staging, manifests,
and logs into the marker-owned persistent work directory. Never use `/tmp` or
`/var/tmp` for deployment artifacts or cleanup targets, and never perform a
broad temporary-directory deletion.
