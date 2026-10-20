# Native staging and reviewed cutover

**ArchDmit's selected path is AUR/pacman:** `lmm-api-core-bin` and
`lmm-api-extensions-bin`, package-owned `/usr/bin/lmm-core` and
`/usr/bin/lmm-extensions`, and their packaged service units. Installing them must
not automatically start the new services or stop the old service. Use the
Nginx maintenance/switch tools here with that path. The `/opt` units, staging
store and `activate-component.sh` below are an **optional alternative**, not this
deployment's service entry points. Never mix `/usr/bin` package updates with a
service still executing an `/opt` binary. Read back `/proc/<MainPID>/exe` after
the explicitly authorized service start.

These are operator tools for a separately authorized site change. They do not
constitute a signed release, financial migration implementation, or production
qualification. **The current financial dry-run and identity-only migration do
not satisfy the cutover gate.** Do not manufacture completion receipts from them.

Use existing PostgreSQL, Nginx and systemd on the host. Only Core, optional Go
extensions and static Web are selected. No Kubernetes, build toolchain or new
resident deployment service belongs on the 1 GiB machine. Build off-host.

See [distribution identity](../../docs/release-architecture.md),
[native packages](../../packaging/linux/README.md),
[runtime paths](../../docs/filesystem-layout.md) and
[offline database roles](../postgres/README.md). This site-specific version store
is an alternative to installing package-owned `/usr/bin` files, not a change to
the native package layout. Do not mix package and version-store activation.

## Files and ownership

| Path | Purpose |
| --- | --- |
| `/opt/lmm/releases/<release.json SHA256>/core/` | Immutable verified Core archive contents |
| `/opt/lmm/releases/<release.json SHA256>/extensions/` | Optional independently selected Go contents |
| `/opt/lmm/releases/<release.json SHA256>/web/dist/` | Static Web, never uploads or secrets |
| `/opt/lmm/current/{core,extensions,web}` | Independent component links; explicit activation only |
| `/etc/nginx/lmm/site-mode.conf` | Server-scope symlink to reviewed active or maintenance snippet |
| `/etc/nginx/lmm/modes/` | Root-owned immutable snippets; existing TLS/server retained |
| `/etc/lmm.conf.d/` | Fixed operator TOML, preserved across software upgrades |
| `/var/lib/lmm/{core,extensions}/` | Separate persistent service state, never replaced by an archive |

All version-store/tool/review directories are root-owned and not group/other
writable. Service users can read their program but cannot modify it. Create the
static `lmm-core` and `lmm-extensions` users using the existing sysusers package
definitions; provision an IPC-only `lmm-rpc` group. The Core template uses this
primary group, extensions gain supplementary membership. Set Core
`rpc.shared_group_id` to its **actual numeric GID**, directory0750/socket0660;
private lock files and each service's separate secret files remain0600. No shared
group access to state or secrets. Without RPC, use the package's standalone unit.

The two templates reuse package sandboxing and graceful stop behavior. MemoryMax
is a ceiling, not measured usage: reserve memory for PG, Nginx and the OS, verify
actual total usage, and select feature admission before launch. No forced kill
or short stop timeout is an acceptable substitute for draining a funds operation.

Core and Go get different runtime users, database roles, private directories and
purpose-specific RPC credentials. Identity, commerce, payments, model proxy and
fixed-witness credentials are not interchangeable. Only enable configured
purposes with the independently reviewed runtime grants. Go receives extension
schema credentials, **never Core owner/runtime credentials**. TOML references
absolute secret files readable only by the intended service (0400/0600).
Do not echo secret contents or place URLs/tokens in process arguments.

## Staging, before downtime

1. Fix each actual component's source SHA, platform, archive SHA, version and
   provenance from a trusted build record. Preserve original source identities;
   do not relabel an old binary with today's HEAD. Portable archives remain
   `preview: true`; an independent operator source-build review can approve an
   exact private candidate without claiming a publisher signature or Release.
2. Write a local `lmm-native-artifact-plan-v1` JSON with `platform` and a
   `components` object keyed by selected `core`, `extensions`, `web`. Each item
   contains `archive` (local file path), `sha256`, `source_revision` (full SHA).
   Hash and independently review this plan. No deployment secrets in it.
3. Run on the build/review machine:

   ```sh
   python3 deployment/native/prepare.py stage \
     --plan "$ARTIFACT_PLAN" --plan-sha256 "$REVIEWED_PLAN_SHA256" \
     --output "$NEW_LOCAL_STAGE_DIRECTORY"
   ```

   It copies before verifying with the existing distribution verifier, then
   extracts only validated regular files. Existing output is refused. The
   printed digest identifies `release.json`; keep its component source/archive
   identity and file inventory. This is staging, not installation or activation.
4. Transfer that exact directory under its digest to the host version store.
   Re-run `prepare.py gate` below on host bytes; it checks the complete extracted
   file inventory. Provision the reviewed unit/config/account files separately.
   `gate` is standalone Python3.11+; `stage` needs this repository's distribution
   verifier and specification. Neither downloads anything.
5. Inspect and merge the **actual backed-up Nginx server**; do not read or publish
   unrelated backup secrets. Include `site-mode.conf` at server scope, pointing
   initially to a root-owned `active.conf`. Keep all current TLS/certificate
   renewal routes, method/URI semantics, streaming settings, security headers,
   OAuth discovery and Core/Go API routes. `web-location.conf` is only a static
   SPA example, not a replacement site config. Never expose private UDS APIs.
   Nginx must read static files but not service secrets.
6. Explicitly initialize the **new, verified empty** database once using the
   separate offline owner TOML and `lmm-core --config /absolute/owner.toml init-db`.
   Provision exact reviewed runtime grants. Normal start/update never initializes
   or changes schema. Keep owner secrets outside network service configuration.
   Extension schema setup uses its separately reviewed offline installer; never
   connect Go to Core. Do not run init-db against an existing site database.

## Cutover, after migration is qualified

Stage everything first; estimate the maintenance interval from measured migration
work. Streaming backup while the old site writes is preparation, not a final
snapshot. The reviewed migration owner must implement the actual write fence,
worker/callback stop/drain, final boundary export, post-backup delta replay,
complete rights/funds/held balances reconciliation and recovery. These tools do
not supply that missing migration. Preserve every valid write and unknown outcome.

1. At the actual maintenance boundary, provide a future ISO8601 time with timezone:

   ```sh
   python3 deployment/native/prepare.py maintenance \
     --eta "$ACTUAL_RECOVERY_ETA" --output "$NEW_MAINTENANCE_CONF"
   ```

   It emits server-scope **503**, absolute HTTP-date `Retry-After`, `no-store` and
   a visible concrete ETA, never a200 maintenance page. Install the reviewed file
   as root into `/etc/nginx/lmm/modes/`. Update the ETA explicitly if it changes.
   `sh switch-nginx-mode.sh /etc/nginx/lmm/modes/<chosen>.conf` validates current
   config, atomically selects the snippet, validates candidate config, then asks
   for a graceful reload. A rejected config restores only the prior config link;
   a reload error requires readback before retry. Verify actual HTTPS503/header,
   static/API paths and worker state. There is no automatic Nginx restart.
2. Fence all legacy writers and drain in-flight requests/workers by the separately
   reviewed migration procedure. No new business writes may enter the old data
   after the final source boundary. Preserve old data and backup/recovery evidence.
3. Complete migration and reconciliation. Record the **actual completed results**, not self-test counts or this tool's
   schema check. Recheck
   the final source fence immediately before activation (maximum age15min).
4. Run the gate with the actual local activation record SHA (schema below), exact host
   name and a non-secret digest of the actual target database identity:

   ```sh
   python3 deployment/native/prepare.py gate --mode cutover \
     --release-dir "$HOST_RELEASE_DIRECTORY" --release-sha256 "$RELEASE_SHA256" \
     --evidence "$ACTIVATION_RECORD" --evidence-sha256 "$ACTUAL_RECORD_SHA256" \
     --host "$EXACT_HOST" --database-identity-sha256 "$TARGET_DATABASE_IDENTITY_SHA256"
   ```

5. `activate-component.sh` takes11 explicit positional parameters:
   `component candidate-directory release-sha activation-record record-sha host database-sha mode previous-directory previous-sha tool-directory`.
   In cutover mode previous values may be `- -`. It re-runs the gate under an
   exclusive host lock, stops only the chosen modern service, waits for actual
   inactive/MainPID0, atomically selects the component, and starts it. Web changes
   only its link. The legacy service fence remains the migration owner's task.
   Keep maintenance selected throughout. No rollback to older software is provided.
6. As the actual service UID run the new Core `check-db` with runtime TOML, inspect
   the schema/authority result and process source/artifact identity. A timeout or
   mere listening socket is not success. Verify distinct-UID RPC with each enabled
   purpose and negative permissions, readiness, identity/setup, real configured
   business reads and non-spending acceptance. Check journals for secret leakage.
   Check Go and static Web separately. Model/payment disabled states remain disabled
   until their own authorized qualification; never make a paid probe as a healthcheck.
7. Only after independent acceptance, select `active.conf` through the same tested
   Nginx mode switch and read back real HTTPS and process versions. Keep final
   fence/migration receipts and exact old/new software identities. Configuration
   reload does not prove business functionality or zero dropped streams.

## Software-only updates

Keep the current database and every valid write. Never restore a backup, rerun
initialization, reset financial state or migrate schema as a software update.
Prepare a new immutable candidate off-host, carrying unchanged components with
their original identities. Changed components require strictly higher supported
semantic versions; same-version artifact replacement and downgrades are rejected.
Retain old immutable files for evidence; failure recovery uses a higher version.

Use `gate --mode update` with the previous release directory/digest and a fresh
activation record. The gate additionally requires equal before/after schema
digests and ordinary-startup-only evidence. If schema changes are required, stop
and obtain a separate qualified migration procedure. Only changed services are
stopped; a Go update must not restart Core or PG. Measure drain time and bound the
maintenance ETA honestly. Keep historical hashed Web assets available to existing
browser sessions using a separately reviewed append-only asset directory/route;
do not purge old assets when switching the SPA index. This example does not yet
implement that host-specific asset union and does not promise uninterrupted tabs.

## One activation record, no extra signature workflow

Staging needs only the real archive/source hashes, versions and configuration
checks. The final activation uses **one operator-written record of actual
results**, not a new signing service or a bundle of mandatory approval files.
A hash binds its bytes; it does not prove a publisher signature or migration truth.
Do not populate ready fields from a dry-run or assumed success.

Use `kind=lmm-native-activation-v1`, `status=ready`, `mode=cutover|update`, `host`,
`target_database_identity_sha256`, `release_sha256`,
`target_runtime_check_passed=true`, `all_valid_writes_preserved=true`, and
`configuration_sha256` mapping the fixed `/etc/lmm.conf.d/core.toml` and/or
`extensions.toml` for selected programs to their current file hashes. Web needs
no runtime configuration entry. The gate re-hashes actual files and full staged
payloads. Keep the actual command/result paths in this same record for audit;
they can be extra fields. Never include credentials.

Cutover additionally records:

- `backup`: actual `sha256` and `restore_verified=true`.
- `migration`: `status=completed`, `unresolved_count=0`,
  `financial_reconciliation_passed=true`, `source_final_boundary_sha256`.
- `final_fence`: `writes_fenced=true`, `active_writers=0`,
  `legacy_workers_stopped=true`, timezone-aware `observed_at` within15min,
  and the **same** `source_final_boundary_sha256`.

Update instead records `previous_release_sha256` and `schema` with equal64hex
`before_sha256`/`after_sha256` and `ordinary_startup_only=true`. Actual process
checks still matter: timeout is never a passed schema/permission observation.

No fabricated ready example is shipped. The gate checks recorded facts and file
identity, not the database itself. Missing real backup, complete migration,
post-backup write preservation, reconciliation or final fence means no cutover.
Independent business acceptance remains required by the project, without creating
an extra manual signing workflow merely to stage files.

## Local checks and remaining host acceptance

`python3 -B deployment/native/test_prepare.py` exercises only synthetic local
files/negative gates. `sh -n` checks both scripts. Validate systemd directives with
`systemd-analyze verify` on the target installation and Nginx with `nginx -t` after
merging the actual existing server. These checks do not operate the production
database, prove migration or independently accept this newly developed tooling.
