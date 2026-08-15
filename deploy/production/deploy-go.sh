#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd -P)
readonly SCRIPT_DIR REPO_ROOT
readonly HOST=${LMM_PRODUCTION_HOST:-ArchDmit}
readonly OFFHOST=${LMM_PRODUCTION_OFFHOST:-archczy}
readonly EXPECTED_HOST=arch-dmit
readonly EXPECTED_OFFHOST=archczy

die() { printf 'deploy-go: %s\n' "$*" >&2; exit 2; }
is_sha256() { [[ $1 =~ ^[0-9a-f]{64}$ ]]; }

[[ ${CONFIRM_PRODUCTION:-} == api.lmm.best ]] || die 'CONFIRM_PRODUCTION must equal api.lmm.best'
[[ $HOST == ArchDmit && $OFFHOST == archczy ]] || die 'production and off-host aliases must remain ArchDmit and archczy'
WORKSPACE=${LMM_API_DEPLOY_WORKSPACE:-}
AGE_RECIPIENT_FILE=${LMM_BACKUP_AGE_RECIPIENT_FILE:-}
AGE_IDENTITY_FILE=${LMM_BACKUP_AGE_IDENTITY_FILE:-}
AGE_BINARY=${LMM_BACKUP_AGE_BINARY:-age}
OBSERVATION_SECONDS=${LMM_PRODUCTION_OBSERVATION_SECONDS:-180}
[[ $WORKSPACE == /* && -d $WORKSPACE && ! -L $WORKSPACE ]] || die 'LMM_API_DEPLOY_WORKSPACE must be an absolute real directory'
[[ -f $WORKSPACE/.lmm-deploy-workspace && ! -L $WORKSPACE/.lmm-deploy-workspace ]] || die 'deployment workspace marker is missing'
[[ $(realpath -e -- "$WORKSPACE") == "$WORKSPACE" ]] || die 'deployment workspace must be canonical'
case $WORKSPACE in /tmp|/tmp/*|/var/tmp|/var/tmp/*|"$REPO_ROOT"|"$REPO_ROOT"/*) die 'deployment workspace is in a forbidden location' ;; esac
[[ -f $AGE_RECIPIENT_FILE && ! -L $AGE_RECIPIENT_FILE ]] || die 'LMM_BACKUP_AGE_RECIPIENT_FILE must name a safe age/SSH recipient file'
[[ -f $AGE_IDENTITY_FILE && ! -L $AGE_IDENTITY_FILE && $(stat -c '%u' "$AGE_IDENTITY_FILE") == "$EUID" ]] || \
  die 'LMM_BACKUP_AGE_IDENTITY_FILE must name an owner-controlled private identity file'
identity_mode=$(stat -c '%a' "$AGE_IDENTITY_FILE")
(( (8#$identity_mode & 8#077) == 0 )) || die 'backup age identity must not grant group or other access'
if [[ $AGE_BINARY == */* ]]; then
  [[ $AGE_BINARY == /* && -x $AGE_BINARY && -f $AGE_BINARY && ! -L $AGE_BINARY ]] || \
    die 'LMM_BACKUP_AGE_BINARY must name a safe executable regular file'
else
  AGE_BINARY=$(command -v "$AGE_BINARY") || die 'age binary is unavailable on the controller'
fi
[[ $OBSERVATION_SECONDS =~ ^[0-9]+$ && $OBSERVATION_SECONDS -ge 120 && $OBSERVATION_SECONDS -le 360 ]] || \
  die 'observation window must be 120-360 seconds'
for command in bsdtar bun file git jq makepkg pacman pg_restore realpath scp sha256sum ssh tar; do
  command -v "$command" >/dev/null 2>&1 || die "required controller command is unavailable: $command"
done

[[ $(ssh -o BatchMode=yes "$HOST" hostnamectl --static) == "$EXPECTED_HOST" ]] || die 'production host identity mismatch'
[[ $(ssh -o BatchMode=yes "$OFFHOST" hostnamectl --static) == "$EXPECTED_OFFHOST" ]] || die 'off-host identity mismatch'
git -C "$REPO_ROOT" diff --quiet || die 'tracked worktree changes must be committed first'
git -C "$REPO_ROOT" diff --cached --quiet || die 'staged changes must be committed first'
[[ -z $(git -C "$REPO_ROOT" ls-files --others --exclude-standard) ]] || die 'untracked worktree files must be resolved first'
revision=$(git -C "$REPO_ROOT" rev-parse HEAD)
remote_revision=$(git -C "$REPO_ROOT" ls-remote origin refs/heads/main | awk 'NR == 1 { print $1 }')
[[ $revision == "$remote_revision" ]] || die 'checked-out HEAD must equal origin/main'
base_version=$(git -C "$REPO_ROOT" show "$revision:VERSION")
[[ $base_version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die 'VERSION is not semantic'
revision_count=$(git -C "$REPO_ROOT" rev-list --count "$revision")
short_revision=$(git -C "$REPO_ROOT" rev-parse --short=9 "$revision")
release_version="$base_version.r$revision_count.g$short_revision"
deployment_id="go-$short_revision-$(date -u +%Y%m%dT%H%M%SZ)"
[[ $deployment_id =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$ ]] || die 'generated deployment ID is invalid'

controller_transaction_lock_owned=0
release_controller_owned_transaction_lock() {
  ssh -o BatchMode=yes "$HOST" bash -s -- "$deployment_id" <<'REMOTE'
set -Eeuo pipefail
deployment_id=$1
lock=/var/lib/lmm-api-go-deploy/transaction.lock
marker="$lock/deployment.env"
[[ -d $lock && ! -L $lock && -f $marker && ! -L $marker ]]
[[ $(stat -c '%U:%G:%a' "$lock") == root:root:700 ]]
[[ $(stat -c '%U:%G:%a' "$marker") == root:root:600 ]]
grep -Fqx 'format=1' "$marker"
grep -Fqx "deployment_id=$deployment_id" "$marker"
grep -Fqx 'status=ACTIVE' "$marker"
rm -f -- "$marker"
rmdir -- "$lock"
REMOTE
}
cleanup_controller_transaction_lock() {
  local rc=$?
  trap - EXIT
  if ((rc != 0 && controller_transaction_lock_owned != 0)); then
    set +e
    release_controller_owned_transaction_lock
  fi
  exit "$rc"
}
trap cleanup_controller_transaction_lock EXIT

artifacts=$WORKSPACE/artifacts/$deployment_id
new_dir=$artifacts/new
rollback_dir=$artifacts/rollback
capture_dir=$artifacts/capture
manifest_dir=$WORKSPACE/manifests/$deployment_id
controller_backup=$WORKSPACE/backups/controller/$deployment_id
offhost_backup="/home/arch/.local/state/lmm-api-production-backups/$deployment_id"
for output in "$artifacts" "$manifest_dir" "$controller_backup"; do
  [[ ! -e $output && ! -L $output ]] || die "deployment output already exists: $output"
done
install -d -m0700 "$new_dir" "$rollback_dir" "$capture_dir" "$manifest_dir" \
  "$WORKSPACE/tmp" "$WORKSPACE/staging" "${controller_backup%/*}"

(
  cd -- "$REPO_ROOT"
  VITE_REACT_APP_VERSION=$release_version bun run build:web
  bun run --filter @lmm/web bundle:check
)
if find "$REPO_ROOT/apps/web/dist" \( -type l -o \( ! -type f ! -type d \) \) -print -quit | grep -q .; then
  die 'frontend dist contains an unsupported entry'
fi
frontend_index_sha256=$(sha256sum "$REPO_ROOT/apps/web/dist/index.html" | awk '{print $1}')

TMPDIR=$WORKSPACE/tmp LMM_API_GO_OUT_DIR=$new_dir \
  "$SCRIPT_DIR/build-go-binary.sh" --source-ref "$revision" --version "$release_version"
binary=$new_dir/lmm-api-go
binary_sha256=$(sha256sum "$binary" | awk '{print $1}')
[[ $("$binary" version) == "$release_version" ]] || die 'candidate binary version mismatch'
TMPDIR=$WORKSPACE/tmp "$SCRIPT_DIR/build-go-package.sh" \
  --workspace "$WORKSPACE" --binary "$binary" --frontend "$REPO_ROOT/apps/web/dist" --output-dir "$new_dir"
candidate_packages=("$new_dir/lmm-api-go-$release_version-1-x86_64.pkg.tar."*)
candidate=''
for possible in "${candidate_packages[@]}"; do
  [[ $possible != *.sha256 && -f $possible ]] || continue
  [[ -z $candidate ]] || die 'candidate package was produced more than once'
  candidate=$possible
done
[[ -n $candidate ]] || die 'candidate package is missing'
candidate_sha256=$(sha256sum "$candidate" | awk '{print $1}')
[[ $(pacman -Qp "$candidate") == "lmm-api-go $release_version-1" ]] || die 'candidate package record mismatch'

remote_workspace="/var/lib/lmm-api-go-deploy/work/$deployment_id"
remote_stage="$remote_workspace/staging"
ssh -o BatchMode=yes "$HOST" bash -s -- "$deployment_id" <<'REMOTE'
set -Eeuo pipefail
deployment_id=$1
workspace="/var/lib/lmm-api-go-deploy/work/$deployment_id"
[[ $(hostnamectl --static) == arch-dmit ]]
[[ ! -e $workspace && ! -L $workspace ]]
install -d -o root -g root -m0700 /var/lib/lmm-api-go-deploy /var/lib/lmm-api-go-deploy/work
install -d -m0700 "$workspace/staging"
printf 'format=1\ndeployment_id=%s\nrole=target\n' "$deployment_id" >"$workspace/.lmm-deploy-workspace"
chmod 0600 "$workspace/.lmm-deploy-workspace"
REMOTE
scp -q "$SCRIPT_DIR/capture-precutover-payload.sh" "$binary" "$HOST:$remote_stage/"
ssh -o BatchMode=yes "$HOST" chmod 0700 \
  "$remote_stage/capture-precutover-payload.sh" "$remote_stage/lmm-api-go"
ssh -o BatchMode=yes "$HOST" "$remote_stage/capture-precutover-payload.sh" \
  --workspace "$remote_workspace" --output "$remote_stage/precutover-payload.tar" >/dev/null
ssh -o BatchMode=yes "$HOST" "$remote_stage/lmm-api-go" status \
  --base-url http://127.0.0.1:3000 --timeout 8s \
  --output "$remote_stage/precutover-status.json" --status-file "$remote_stage/precutover-status.code"
old_version=$(ssh -o BatchMode=yes "$HOST" cat -- "$remote_stage/precutover-status.json" |
  jq -er 'select(.success == true and .ready == true and (.data.version | type == "string")) | .data.version')
[[ $old_version =~ ^[0-9][0-9A-Za-z._+]*$ ]] || die 'pre-cutover listener returned an invalid version'
comparison=$(ssh -o BatchMode=yes "$HOST" vercmp "$old_version" "$release_version")
((comparison < 0)) || die "candidate is not an upgrade: $old_version -> $release_version"
scp -q "$HOST:$remote_stage/precutover-payload.tar" "$capture_dir/precutover-payload.tar"
remote_payload_sha256=$(ssh -o BatchMode=yes "$HOST" sha256sum "$remote_stage/precutover-payload.tar" | awk '{print $1}')
local_payload_sha256=$(sha256sum "$capture_dir/precutover-payload.tar" | awk '{print $1}')
[[ $remote_payload_sha256 == "$local_payload_sha256" ]] || die 'captured payload changed in transit'
rollback_layout=$(bsdtar -xOf "$capture_dir/precutover-payload.tar" ./metadata/layout)
case $rollback_layout in split|direct) ;; *) die 'captured rollback layout is invalid' ;; esac

TMPDIR=$WORKSPACE/tmp "$SCRIPT_DIR/build-precutover-packages.sh" \
  --workspace "$WORKSPACE" --payload "$capture_dir/precutover-payload.tar" --output-dir "$rollback_dir"
rollback_go=$(find "$rollback_dir" -maxdepth 1 -type f -name 'lmm-api-go-*.pkg.tar.*' ! -name '*.sha256' -print -quit)
if [[ $rollback_layout == split ]]; then
  rollback_core=$(find "$rollback_dir" -maxdepth 1 -type f -name 'lmm-api-*.pkg.tar.*' ! -name 'lmm-api-go-*' ! -name '*.sha256' -print -quit)
else
  rollback_core=$rollback_dir/rollback-layout.direct
fi
[[ -n $rollback_core && -f $rollback_core && -n $rollback_go ]] || die 'rollback artifacts are incomplete'
if [[ $rollback_layout == direct && $(<"$rollback_core") != direct ]]; then
  die 'direct rollback marker is invalid'
fi
rollback_core_sha256=$(sha256sum "$rollback_core" | awk '{print $1}')
rollback_go_sha256=$(sha256sum "$rollback_go" | awk '{print $1}')
if ! is_sha256 "$rollback_core_sha256" || ! is_sha256 "$rollback_go_sha256"; then
  die 'rollback package checksum is invalid'
fi

"$SCRIPT_DIR/promote-production-backups.sh" \
  --target-host "$HOST" --jump-host "$OFFHOST" \
  --deployment-id "$deployment_id" --controller-workspace "$WORKSPACE" \
  --controller-output "$controller_backup" --offhost-output "$offhost_backup" \
  --age-recipient-file "$AGE_RECIPIENT_FILE" --release-id "$release_version" \
  --artifact-sha256 "$candidate_sha256" --core-sha256 "$rollback_core_sha256" \
  --backend-sha256 "$rollback_go_sha256" --git-revision "$revision" \
  --prepare-script "$SCRIPT_DIR/prepare-production-backup.sh" \
  --copy-script "$SCRIPT_DIR/create-backup-copy.sh" \
  --verify-script "$REPO_ROOT/.agents/skills/lmm-deploy-safely/scripts/verify-backup-set.sh" \
  --precutover-payload "$capture_dir/precutover-payload.tar" \
  --rollback-core-package "$rollback_core" --rollback-go-package "$rollback_go" \
  --rollback-layout "$rollback_layout" \
  >"$manifest_dir/backup-locations.txt"
controller_transaction_lock_owned=1
target_mirror="$WORKSPACE/staging/backup-target-$deployment_id"
[[ -f $target_mirror/database.archive && ! -L $target_mirror/database.archive ]] || die 'plain target database backup mirror is missing'
pg_restore --list "$target_mirror/database.archive" >/dev/null
for archive in "$target_mirror/application.archive" "$target_mirror/frontend.archive" "$target_mirror/configuration.archive"; do
  tar -tf "$archive" >/dev/null
done
target_configuration_sha256=$(sha256sum "$target_mirror/configuration.archive" | awk '{print $1}')
target_database_sha256=$(sha256sum "$target_mirror/database.archive" | awk '{print $1}')
for encrypted_copy in "$controller_backup" "$WORKSPACE/staging/backup-off-host-$deployment_id"; do
  [[ -f $encrypted_copy/configuration.age && ! -L $encrypted_copy/configuration.age ]] || \
    die 'encrypted configuration backup is missing'
  [[ -f $encrypted_copy/database.age && ! -L $encrypted_copy/database.age ]] || \
    die 'encrypted database backup is missing'
  # The target archives above are structurally validated. Consume each decrypted
  # stream completely before comparing its digest: archive listing commands can
  # stop before EOF and make age fail with SIGPIPE under pipefail.
  decrypted_configuration_sha256=$("$AGE_BINARY" --decrypt --identity "$AGE_IDENTITY_FILE" \
    "$encrypted_copy/configuration.age" | sha256sum | awk '{print $1}')
  decrypted_database_sha256=$("$AGE_BINARY" --decrypt --identity "$AGE_IDENTITY_FILE" \
    "$encrypted_copy/database.age" | sha256sum | awk '{print $1}')
  [[ $decrypted_configuration_sha256 == "$target_configuration_sha256" ]] || \
    die 'decrypted configuration backup does not match the target copy'
  [[ $decrypted_database_sha256 == "$target_database_sha256" ]] || \
    die 'decrypted database backup does not match the target copy'
done

scp -q "$SCRIPT_DIR/activate-go-release.sh" "$SCRIPT_DIR/../frontend-release.sh" \
  "$candidate" "$HOST:$remote_stage/"
ssh -o BatchMode=yes "$HOST" chmod 0700 \
  "$remote_stage/activate-go-release.sh" "$remote_stage/frontend-release.sh"
remote_candidate="$remote_stage/${candidate##*/}"
remote_core="$remote_stage/${rollback_core##*/}"
remote_go="$remote_stage/${rollback_go##*/}"
target_backup="/var/lib/lmm-api-go-deploy/backups/$deployment_id"
deploy_unit="lmm-api-go-deploy-$deployment_id"
if ! ssh -o BatchMode=yes "$HOST" systemd-run \
  --unit="$deploy_unit" --collect --property=Type=oneshot --property=TimeoutStartSec=9min \
  "$remote_stage/activate-go-release.sh" activate \
  --workspace "$remote_workspace" \
  --package "$remote_candidate" --package-sha256 "$candidate_sha256" \
  --rollback-core-package "$remote_core" --rollback-core-sha256 "$rollback_core_sha256" \
  --rollback-go-package "$remote_go" --rollback-go-sha256 "$rollback_go_sha256" \
  --rollback-layout "$rollback_layout" \
  --probe-binary "$remote_stage/lmm-api-go" --probe-binary-sha256 "$binary_sha256" \
  --expected-version "$release_version" --old-version "$old_version" \
  --frontend-index-sha256 "$frontend_index_sha256" \
  --frontend-release-script "$remote_stage/frontend-release.sh" \
  --backup-dir "$target_backup" --rollback-seconds 600 >/dev/null; then
  # The remote dispatch result is ambiguous on transport failure. Retain the
  # exact transaction lock so a running activation cannot lose its guard.
  controller_transaction_lock_owned=0
  die 'activation dispatch failed; transaction lock retained for audit'
fi
controller_transaction_lock_owned=0

status_file="$remote_workspace/state/status"
for _ in {1..160}; do
  deployment_status=$(ssh -o BatchMode=yes "$HOST" cat -- "$status_file" 2>/dev/null || true)
  case $deployment_status in
    AWAITING_CONFIRMATION\ *) break ;;
    ROLLED_BACK\ *|ROLLBACK_FAILED\ *) die "$deployment_status" ;;
  esac
  if [[ -z $deployment_status ]] && ssh -o BatchMode=yes "$HOST" systemctl is-failed --quiet "$deploy_unit"; then
    die 'activation unit failed before reaching a rollback-protected state'
  fi
  sleep 3
done
[[ $deployment_status == AWAITING_CONFIRMATION\ * ]] || die 'activation did not reach the observation gate before its rollback deadline'

observation_epoch=$(ssh -o BatchMode=yes "$HOST" date +%s)
observation_started=$(date +%s)
while (( $(date +%s) - observation_started < OBSERVATION_SECONDS )); do
  if ! ssh -o BatchMode=yes "$HOST" bash -s -- \
    "$remote_workspace" "$release_version" "$frontend_index_sha256" "$deployment_id" "$observation_epoch" <<'REMOTE'
set -Eeuo pipefail
workspace=$1
expected_version=$2
frontend_sha=$3
deployment_id=$4
observation_epoch=$5
cli="$workspace/staging/lmm-api-go"
state="$workspace/state"
token="$state/probe-token"
timer="lmm-api-go-rollback-$deployment_id.timer"
nginx_observation_is_clean() {
  local log_line
  while IFS= read -r log_line; do
    case $log_line in
      '') continue ;;
      # Public scanners probe unrelated static paths continuously. The release
      # assets are validated by native CLI probes and package integrity, so
      # only these explicit file-miss lines are observation noise.
      *'open() "'*' failed (2: No such file or directory)'*'request: "'*' /static/'*) continue ;;
      *) printf 'actionable nginx error: %s\n' "$log_line" >&2; return 1 ;;
    esac
  done < <(journalctl --quiet -u nginx.service --since "@$observation_epoch" \
    --priority=err --no-pager --output=cat)
}
systemctl is-active --quiet lmm-api.service
systemctl is-active --quiet "$timer"
[[ $(systemctl show lmm-api.service -p NRestarts --value) == 0 ]]
[[ $(pacman -Q lmm-api-go) == "lmm-api-go $expected_version-1" ]]
! pacman -Qq lmm-api >/dev/null 2>&1
[[ -L /usr/bin/lmm-api && $(readlink -- /usr/bin/lmm-api) == lmm-api-go ]]
for removed in /usr/bin/lmm-api-select /usr/lib/lmm-api /usr/lib/systemd/system/lmm-api-go.service; do
  [[ ! -e $removed && ! -L $removed ]]
done
"$cli" status --base-url http://127.0.0.1:3000 --timeout 8s --output "$state/observe-local.json"
jq -e --arg version "$expected_version" '.success == true and .ready == true and .data.version == $version' "$state/observe-local.json" >/dev/null
"$cli" status --base-url https://api.lmm.best --timeout 8s --output "$state/observe-public.json"
jq -e --arg version "$expected_version" '.success == true and .ready == true and .data.version == $version' "$state/observe-public.json" >/dev/null
"$cli" doctor --base-url http://127.0.0.1:3000 --timeout 8s --output "$state/observe-live.json"
jq -e '.success == true and .live == true' "$state/observe-live.json" >/dev/null
"$cli" request --base-url https://api.lmm.best --path / --timeout 8s --fail --output "$state/observe-index.html"
[[ $(sha256sum "$state/observe-index.html" | awk '{print $1}') == "$frontend_sha" ]]
"$cli" request --base-url https://api.lmm.best --path /v1/models --timeout 8s --fail \
  --token-file "$token" --output "$state/observe-models.json"
jq -e '.data | type == "array"' "$state/observe-models.json" >/dev/null
[[ -z $(journalctl --quiet -u lmm-api.service --since "@$observation_epoch" --priority=err --no-pager --output=cat) ]]
nginx_observation_is_clean
REMOTE
  then
    die 'production observation detected an anomaly; rollback timer remains armed'
  fi
  sleep 15
done

ssh -o BatchMode=yes "$HOST" "$remote_stage/activate-go-release.sh" confirm --workspace "$remote_workspace" \
  >"$manifest_dir/confirmation.txt"
final_status=$(ssh -o BatchMode=yes "$HOST" cat -- "$status_file")
[[ $final_status == "CONFIRMED version=$release_version" ]] || die 'production confirmation state is wrong'
rollback_timer="lmm-api-go-rollback-$deployment_id.timer"
rollback_service="lmm-api-go-rollback-$deployment_id.service"
if ssh -o BatchMode=yes "$HOST" systemctl is-active --quiet "$rollback_timer"; then
  die 'rollback timer is still active after confirmation'
fi
if ssh -o BatchMode=yes "$HOST" systemctl is-active --quiet "$rollback_service"; then
  die 'rollback service is still active after confirmation'
fi

{
  printf 'deployment_id=%s\nrelease_version=%s\ngit_revision=%s\n' "$deployment_id" "$release_version" "$revision"
  printf 'candidate_sha256=%s\nrollback_core_sha256=%s\nrollback_go_sha256=%s\n' \
    "$candidate_sha256" "$rollback_core_sha256" "$rollback_go_sha256"
	printf 'controller_backup=%s\noffhost_backup=%s\ntarget_backup=%s\n' \
		"$controller_backup" "$offhost_backup" "$target_backup"
	printf 'observation_seconds=%s\nfinal_status=%s\nrollback_timer=inactive\n' "$OBSERVATION_SECONDS" "$final_status"
	printf 'database_rollback=old-service-compatible-forward-schema\n'
} >"$manifest_dir/result.env"
chmod 0600 "$manifest_dir/result.env" "$manifest_dir/backup-locations.txt" "$manifest_dir/confirmation.txt"
printf 'deployment_id=%s\nrelease_version=%s\nstatus=%s\ncontroller_backup=%s\noffhost_backup=%s\n' \
  "$deployment_id" "$release_version" "$final_status" "$controller_backup" "$offhost_backup"
