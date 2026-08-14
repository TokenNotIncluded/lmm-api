# LMM deployment safety contract

## Authorization and identity

- Default to `local`.
- Require explicit current-turn authorization for `test` or `production`.
- Verify the expected SSH alias, static hostname, role marker, service name,
  installed package identity, current backend artifact, frontend symlink, and
  installed CLI protocol/service entry point.
- Treat host or role disagreement as a stop condition.
- Preserve the repository's one-branch, one-worktree, one-diff rules. A deploy
  request does not authorize Git repair, branch switching, commit, or push.

## Workspace and build contract

- Create one deployment ID matching `[A-Za-z0-9][A-Za-z0-9._-]{0,127}`.
- Use marker-owned persistent storage, never `/tmp` or `/var/tmp`.
- Place `TMPDIR`, Go build cache, Go module cache, Cargo target output, Bun
  cache, package staging, manifests, and logs under that deployment directory.
- The controller workspace is
  `${XDG_STATE_HOME:-$HOME/.local/state}/lmm-api/deploy-work/<deployment-id>`;
  do not leave build or package bytes in the repository or in `/tmp`.
- Build once. Record and verify the same artifact SHA-256 at every promotion
  boundary.
- Never overwrite an existing immutable release with different bytes.

### Production resource gates

The production root filesystem is 20 GiB and the service cgroup is bounded by
`MemoryHigh=320M`, `MemoryMax=384M`, and `MemorySwapMax=256M`. Before a
mutation, during every build/backup transfer, and throughout the watchdog
window, record `df -h /`, `df -i /`, `free -h`, `vmstat 1 5`, the service
`NRestarts`/memory counters, PostgreSQL readiness, Valkey readiness, and native
CLI `/api/status` plus `/api/livez` probes.

- Green: root and inode use `<70%`, `MemAvailable >=30%`, swap `<10%`, CPU
  `<70%` for 5 minutes, and at least 4 GiB free before a production package.
- Warning: root/inode use `70-80%`, `MemAvailable 20-30%`, swap `10-25%`, or
  CPU `70-85%`; serialize heavy work and prune only measured terminal work.
- Stop: root/inode `>=80%`, root free space below the known package plus three
  backup copies and 1 GiB headroom, `MemAvailable <20%`, swap `>25%` with
  churn, CPU `>85%` for 5 minutes, repeated restart/OOM, or a required probe
  timeout. At `>=90%` storage use, treat the host as an emergency and clean
  storage before touching application state.

Do not clear swap, journals, caches, or the database to make a metric green.
Do not kill unrelated processes. A failed resource or health check is an
incident signal and must be recorded before any retry.

## Database detection

- Inspect configuration without sourcing it and without printing values.
- Classify only SQLite, PostgreSQL, or MySQL from recognized configuration
  keys and safe value prefixes.
- Fail when multiple engines are present, when the deployer and live engine
  disagree, or when the engine is unknown for a database-changing release.
- Never put a DSN in command output, a manifest, `SWAP.md`, or a process title.
- Treat the running service environment and active listeners as current
  evidence; historical prose or a previous cutover result is not enough.
- If the live process uses PostgreSQL but the current `PG_WRITE_BOUNDARY`,
  cutover journal, or post-cutover verification is missing or failed, stop and
  reconcile through the coordinator before migration, backend selection, or
  rollback. Do not silently classify that state as a fresh SQLite migration.

## Legacy CLI safety

- Before invoking `/usr/bin/lmm-api-go` on a target, verify both installed
  entries have the same package owner and bytes, the operator supports the
  `deploy production` transaction, and systemd uses `/usr/bin/lmm-api serve`.
- A legacy binary may start the backend for an unknown command. `status`,
  `deploy`, `--help`, and no-argument calls are not read-only until the
  protocol is proven.
- For a legacy target, inspect with `systemctl show`, `readlink`, sanitized
  process-environment scheme classification, and explicit health probes only.
  Upgrade the core package through a guarded transaction before using deploy
  phases. Preserve and report any artifact created by an unsafe probe; remove
  it only after exact ownership and scope are confirmed.

## Local acceptance preview override

For this repository's local acceptance preview, SQLite is unconditionally
forbidden, including fallback, autodetection, and default behavior. Require a
fresh marker-owned local PostgreSQL database and role plus a fresh isolated
marker-owned Valkey instance, both initialized with zero production or
business data. Do not use production DSNs, snapshots, or cache dumps. Bind
both to loopback or Unix sockets only; do not reuse an existing database,
schema, role, cache namespace, or port. Verify database and cache identities
before startup. Missing or ambiguous configuration, an unexpected engine, a
nonempty target, or inability to verify identity is an immediate `STOP`, never
a fallback. This override applies only to local acceptance previews; generic
engine detection above remains applicable to other deployment roles.

## Optional backup copies

Backups are not a deployment prerequisite. Do not create, transfer, verify, or
prune them unless the user explicitly requests backups in the current turn.
When the opt-in backup path is selected, require:

| Role | Required verified copies |
| --- | --- |
| local | controller |
| test | target, controller |
| production | target, controller, off-host |

The production controller copy lives at
`$HOME/backup/lmm-api/<verified-host>/<deployment-id>`. The off-host copy lives
on the ArchCzy host, reached through the case-sensitive SSH alias `archczy`,
under a fixed root or in explicitly configured object storage. Verify that
`ssh archczy` resolves to static hostname `archczy`; do not assume the display
name `ArchCzy` is a configured SSH alias.

Each copy contains `manifest.env`, `SHA256SUMS`, a nonempty application archive,
frontend archive, configuration archive, and database backup. The manifest
records:

- format and deployment ID;
- copy role, deployment role, and verified host;
- release ID, artifact SHA-256, and Git revision;
- database engine;
- service state and frontend release identity;
- archive paths, sizes, modes, and UTC timestamps.

The target may retain a root-only plaintext configuration snapshot. Controller
and off-host configuration or database archives must be encrypted before
transfer. Checksums cover the encrypted bytes that were actually transferred.

Default retention is five target, ten controller, and thirty off-host copies.
Prune only after confirmation or verified rollback. Never remove the active
release, an unconfirmed deployment, the latest known-good backup, or a copy
whose remaining peers have not been verified.

Canonical roots are fixed:

- controller workspace: `${XDG_STATE_HOME:-$HOME/.local/state}/lmm-api/deploy-work/<deployment-id>`;
- durable controller copy: `$HOME/backup/lmm-api/<verified-host>/<deployment-id>`;
- production target workspace: `/var/lib/lmm-api-go/deploy-work/<deployment-id>`
  (resolved private state is below `/var/lib/private/lmm-api-go`);
- production target backup: `/var/lib/lmm-api-go/deploy-backups/<deployment-id>`;
- off-host copy: `/home/arch/.local/state/lmm-api-production-backups/<deployment-id>`
  on the ArchCzy host through SSH alias `archczy`.

Only the exact workspace's `staging`, `tmp`, and cache children are disposable.
After terminal state, retain its marker/status audit record and any explicitly
requested durable copies; remove staging by exact path. Never recursively clean a backup
root, release root, home/root, `/tmp`, `/var/tmp`, an unresolved variable, or a
glob. A terminal workspace older than 24 hours may be pruned oldest first only
after checksum/decryption verification and only while the storage gate remains
green.

## Rollback watchdog

- Install a persistent systemd watchdog before the first switch.
- Store a root-only, checksum-verified guard containing the exact deployment,
  old/new backend identities, old/new frontend identities, rollback artifacts,
  status, and deadline.
- Arm the watchdog before switching application or frontend state.
- After successful identity and health probes, set a ten-minute deadline and
  return `AWAITING_CONFIRMATION`.
- Manual confirmation must name the exact deployment and re-verify backend
  checksum/version, frontend symlink, relevant services, and health identity.
- Confirmation writes `CONFIRMED` durably before stopping and disabling the
  timer.
- Expiry restores the prior application package, frontend link, and approved
  configuration snapshot, verifies health, writes `ROLLED_BACK`, and preserves
  evidence for review.
- Automatic rollback never restores a database. Block the release unless its
  migrations are compatible with both N and N-1 during the confirmation
  window.

## Go/Web AUR migration order

- Require signed immutable release assets and pinned AUR hashes before target
  package assembly. `paru` may assemble a `-bin` package; it must not compile or
  replace the signed application artifact.
- Arm the watchdog before stopping Go. Run the candidate binary as a root
  transient unit with the production environment file; do not use the stopped
  service's `DynamicUser` identity.
- Run `migrate --apply` and then `migrate --verify` before `paru -U`. A failed
  verification blocks package installation. Never repair the gate with ad-hoc
  production SQL.
- Start Go before final Web activation. A local Web activation probe avoids a
  DNS-only package-hook failure; public status remains a confirmation gate.
- Observe at least 120 seconds, then confirm exact Go/Web package versions,
  revisions, frontend link, restart count, database/cache readiness, and memory.

## Rust ownership gate

- The Rust blue/green slots own internal GET/HEAD probes only until the route
  gate and independent business differential evidence approve production
  ownership.
- `migration-gate.tsv` must pass source-mode validation before packaging and
  activation-mode validation before a Rust switch. Reject inconsistent
  `legacy-go`/mounted rows, unresolved routes, unverified auth/quota/billing or
  streaming behavior, and any route without independent approval.
- PostgreSQL and dedicated Valkey identity, shared rate-limit/session state,
  background-job singleton behavior, and SSE/WebSocket drain/reconnect are
  required evidence; a mounted slot, active symlink, or `/readyz` response is
  not production proof.

## Cleanup

- Clean only the exact deployment workspace carrying the expected marker and
  matching deployment ID.
- Require a durable `CONFIRMED`, `ROLLED_BACK`, controller-only pre-switch
  `VALIDATED`, or pre-switch `ABORTED` final state. Use `VALIDATED` for
  completed pre-release checks and `ABORTED` for an interrupted attempt, only
  after proving no switch occurred and stopping every workspace-owned process.
- Reject `/`, home roots, workspace roots, `/tmp`, `/var/tmp`, backup roots,
  release roots, unresolved variables, tildes, globs, symlinks, and paths not
  owned by the marker.
- Preview cleanup before execution.
- Never implement “clear tmp” as a broad directory deletion. Identify and
  remove only stale, inactive, marker-owned LMM deployment workspaces.
- Re-run disk/inode/RAM/swap, service state, PostgreSQL/Valkey readiness, and
  native CLI status/livez checks after cleanup; record exact paths removed and
  the remaining free-space margin.

## Minimum validation before production enablement

- Skill validation and shell syntax checks pass.
- Offline tests cover safe IDs and paths, host mismatch, database disagreement,
  optional-backup selection and verification, timer
  arming, exact-release confirmation, expiry rollback, reboot recovery, and
  scoped cleanup.
- Existing production contract tests require watchdog state and
  `AWAITING_CONFIRMATION`; opted-in backup tests require all requested copies.
- A fresh runtime audit reconciles any historical PostgreSQL cutover result and
  proves the active schema, Valkey endpoint, and forward-only boundary.
- Frontend-only publication is proven compatible with the current Go API before
  it is promoted independently of Rust.
- A local deployment of the identical artifact passes application and frontend
  checks before any test or production promotion.
- Authenticated canaries and representative business requests pass in addition
  to generic health checks; browser, SSE, and WebSocket behavior is reviewed
  when affected by the release.
- An independent Reviewer approves the deployment behavior and residual risk.
