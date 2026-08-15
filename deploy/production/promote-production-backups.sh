#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

die() { printf 'promote-production-backups: %s\n' "$*" >&2; exit 2; }
is_safe_path() { [[ $1 == /* && $1 != / && $1 != /tmp* && $1 != /var/tmp* && $1 != *..* ]]; }

TARGET_HOST=''
JUMP_HOST=''
DEPLOYMENT_ID=''
CONTROLLER_WORKSPACE=''
CONTROLLER_OUTPUT=''
OFFHOST_OUTPUT=''
AGE_RECIPIENT_FILE=''
RELEASE_ID=''
ARTIFACT_SHA256=''
CORE_SHA256=''
BACKEND_SHA256=''
GIT_REVISION=''
SSH_CONFIG=${LMM_API_DEPLOY_SSH_CONFIG:-$HOME/.ssh/config}
PREPARE_SCRIPT=''
COPY_SCRIPT=''
VERIFY_SCRIPT=''
PRECUTOVER_PAYLOAD=''
ROLLBACK_CORE_PACKAGE=''
ROLLBACK_GO_PACKAGE=''
ROLLBACK_LAYOUT='split'
while (($#)); do
  case $1 in
    --target-host) TARGET_HOST=${2:?}; shift 2 ;;
    --jump-host) JUMP_HOST=${2:?}; shift 2 ;;
    --deployment-id) DEPLOYMENT_ID=${2:?}; shift 2 ;;
    --controller-workspace) CONTROLLER_WORKSPACE=${2:?}; shift 2 ;;
    --controller-output) CONTROLLER_OUTPUT=${2:?}; shift 2 ;;
    --offhost-output) OFFHOST_OUTPUT=${2:?}; shift 2 ;;
    --age-recipient-file) AGE_RECIPIENT_FILE=${2:?}; shift 2 ;;
    --release-id) RELEASE_ID=${2:?}; shift 2 ;;
    --artifact-sha256) ARTIFACT_SHA256=${2:?}; shift 2 ;;
    --core-sha256) CORE_SHA256=${2:?}; shift 2 ;;
    --backend-sha256) BACKEND_SHA256=${2:?}; shift 2 ;;
    --git-revision) GIT_REVISION=${2:?}; shift 2 ;;
    --ssh-config) SSH_CONFIG=${2:?}; shift 2 ;;
    --prepare-script) PREPARE_SCRIPT=${2:?}; shift 2 ;;
    --copy-script) COPY_SCRIPT=${2:?}; shift 2 ;;
    --verify-script) VERIFY_SCRIPT=${2:?}; shift 2 ;;
    --precutover-payload) PRECUTOVER_PAYLOAD=${2:?}; shift 2 ;;
    --rollback-core-package) ROLLBACK_CORE_PACKAGE=${2:?}; shift 2 ;;
    --rollback-go-package) ROLLBACK_GO_PACKAGE=${2:?}; shift 2 ;;
    --rollback-layout) ROLLBACK_LAYOUT=${2:?}; shift 2 ;;
    *) die "unknown argument: $1" ;;
  esac
done

case $ROLLBACK_LAYOUT in split|direct) ;; *) die 'rollback layout must be split or direct' ;; esac

[[ $TARGET_HOST == ArchDmit ]] || die 'target host must be ArchDmit'
[[ $JUMP_HOST == archczy ]] || die 'jump/off-host must be archczy'
[[ $DEPLOYMENT_ID =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ ]] || die 'invalid deployment ID'
for path in "$CONTROLLER_WORKSPACE" "$CONTROLLER_OUTPUT" "$OFFHOST_OUTPUT"; do is_safe_path "$path" || die 'unsafe output path'; done
[[ -d $CONTROLLER_WORKSPACE && ! -L $CONTROLLER_WORKSPACE ]] || die 'controller workspace is missing'
[[ ! -e $CONTROLLER_OUTPUT && ! -L $CONTROLLER_OUTPUT ]] || die 'controller backup output exists'
[[ -f $AGE_RECIPIENT_FILE && ! -L $AGE_RECIPIENT_FILE ]] || die 'age recipient file is missing'
[[ $ARTIFACT_SHA256 =~ ^[0-9a-f]{64}$ && $CORE_SHA256 =~ ^[0-9a-f]{64}$ && \
  $BACKEND_SHA256 =~ ^[0-9a-f]{64}$ && $GIT_REVISION =~ ^[0-9a-f]{7,64}$ ]] || die 'invalid release identity'
for script in "$PREPARE_SCRIPT" "$COPY_SCRIPT" "$VERIFY_SCRIPT"; do [[ -x $script && ! -L $script ]] || die 'required backup helper is missing'; done
for input in "$PRECUTOVER_PAYLOAD" "$ROLLBACK_CORE_PACKAGE" "$ROLLBACK_GO_PACKAGE"; do
  [[ $input == /* && -s $input && -f $input && ! -L $input ]] || die 'rollback backup input is missing or unsafe'
done
[[ -f $SSH_CONFIG && ! -L $SSH_CONFIG && $(stat -c '%u' "$SSH_CONFIG") == "$EUID" ]] || die 'SSH config is missing, unsafe, or not owner-controlled'
ssh_mode=$(stat -c '%a' "$SSH_CONFIG")
(( (8#$ssh_mode & 8#022) == 0 )) || die 'SSH config is group/other writable'
grep -Eiq '^[[:space:]]*Host[[:space:]]+([^#]*[[:space:]])?archczy([[:space:]]|$)' "$SSH_CONFIG" || die 'SSH config does not define archczy'

ssh_bin=${LMM_DEPLOY_SSH_BIN:-ssh}
scp_bin=${LMM_DEPLOY_SCP_BIN:-scp}
target_endpoint=root@45.59.187.63
target_port=222
[[ $ssh_bin =~ ^[A-Za-z0-9_./-]+$ && $SSH_CONFIG =~ ^[A-Za-z0-9_./-]+$ ]] || \
  die 'SSH executable and configuration paths must be shell-safe'
control_root=/run/user/$EUID
[[ -d $control_root && ! -L $control_root && -w $control_root && $(stat -c '%u' "$control_root") == "$EUID" ]] || \
  die 'controller runtime directory is missing or unsafe'
control_digest=$(printf '%s' "$DEPLOYMENT_ID" | sha256sum)
control_digest=${control_digest%% *}
control_tag=${control_digest:0:16}
target_control="$control_root/lmm-api-$control_tag-target-%C"
jump_control="$control_root/lmm-api-$control_tag-jump-%C"
# ProxyCommand is expanded once by the target SSH process. Escape %C so the
# jump SSH process receives it and computes its own deployment-private socket.
proxy_command="exec $ssh_bin -F $SSH_CONFIG -o BatchMode=yes -o ControlMaster=auto -o ControlPersist=60 -o ControlPath=$control_root/lmm-api-$control_tag-jump-%%C -W %h:%p $JUMP_HOST"
remote_workspace="/var/lib/lmm-api-go-deploy/work/$DEPLOYMENT_ID"
remote_stage="$remote_workspace/staging"
target_output="/var/lib/lmm-api-go-deploy/backups/$DEPLOYMENT_ID"
controller_remote="$remote_stage/controller-copy"
offhost_remote="$remote_stage/offhost-copy"
target_mirror="$CONTROLLER_WORKSPACE/staging/backup-target-$DEPLOYMENT_ID"
offhost_mirror="$CONTROLLER_WORKSPACE/staging/backup-off-host-$DEPLOYMENT_ID"
target_owned=0
controller_remote_owned=0
offhost_remote_owned=0
controller_owned=0
offhost_owned=0
target_mirror_owned=0
offhost_mirror_owned=0
transaction_lock_claimed=0
target_ssh=("$ssh_bin" -F "$SSH_CONFIG" -o BatchMode=yes -o ControlMaster=auto \
  -o ControlPersist=60 -o "ControlPath=$target_control" -o "ProxyCommand=$proxy_command" \
  -p "$target_port" "$target_endpoint")
offhost_ssh=("$ssh_bin" -F "$SSH_CONFIG" -o BatchMode=yes -o ControlMaster=auto \
  -o ControlPersist=60 -o "ControlPath=$jump_control" "$JUMP_HOST")
target_scp=("$scp_bin" -F "$SSH_CONFIG" -o BatchMode=yes -o ControlMaster=auto \
  -o ControlPersist=60 -o "ControlPath=$target_control" -o "ProxyCommand=$proxy_command" \
  -P "$target_port")

release_failed_backup_transaction_lock() {
  "${target_ssh[@]}" bash -s -- "$DEPLOYMENT_ID" <<'EOF'
set -Eeuo pipefail
deployment_id=$1
lock=/var/lib/lmm-api-go-deploy/transaction.lock
marker="$lock/deployment.env"
[[ -e $lock || -L $lock ]] || exit 0
[[ -d $lock && ! -L $lock && -f $marker && ! -L $marker ]] || {
  printf 'backup transaction lock is unsafe\n' >&2
  exit 3
}
grep -Fqx 'format=1' "$marker"
grep -Fqx "deployment_id=$deployment_id" "$marker" || {
  printf 'backup transaction lock ownership changed\n' >&2
  exit 3
}
grep -Fqx 'status=ACTIVE' "$marker"
rm -f -- "$marker"
rmdir -- "$lock"
EOF
}

cleanup_partial() {
  set +e
  ((controller_remote_owned == 0)) || "${target_ssh[@]}" rm -rf -- "$controller_remote"
  ((offhost_remote_owned == 0)) || "${target_ssh[@]}" rm -rf -- "$offhost_remote"
  ((target_owned == 0)) || "${target_ssh[@]}" rm -rf -- "$target_output"
  ((offhost_owned == 0)) || "${offhost_ssh[@]}" rm -rf -- "$OFFHOST_OUTPUT"
  ((controller_owned == 0)) || rm -rf -- "$CONTROLLER_OUTPUT"
  ((offhost_mirror_owned == 0)) || rm -rf -- "$offhost_mirror"
  ((target_mirror_owned == 0)) || rm -rf -- "$target_mirror"
  ((transaction_lock_claimed == 0)) || release_failed_backup_transaction_lock
}
on_error() {
  local rc=$?
  cleanup_partial
  exit "$rc"
}
trap on_error ERR

transaction_lock_claimed=1
"${target_ssh[@]}" bash -s -- "$DEPLOYMENT_ID" <<'EOF'
set -Eeuo pipefail
deployment_id=$1
workspace="/var/lib/lmm-api-go-deploy/work/$deployment_id"
lock=/var/lib/lmm-api-go-deploy/transaction.lock
lock_marker="$lock/deployment.env"
for command in age bash chmod install pg_dump pg_restore readlink sha256sum tar; do
  command -v "$command" >/dev/null 2>&1 || { echo "required backup command is unavailable: $command" >&2; exit 2; }
done
if mkdir -m0700 -- "$lock" 2>/dev/null; then
  printf 'format=1\ndeployment_id=%s\nstatus=ACTIVE\n' "$deployment_id" >"$lock_marker"
  chmod 0600 "$lock_marker"
else
  [[ -d $lock && ! -L $lock && -f $lock_marker && ! -L $lock_marker ]]
  grep -Fqx "deployment_id=$deployment_id" "$lock_marker"
  grep -Fqx 'status=ACTIVE' "$lock_marker"
fi
install -d -o root -g root -m0700 /var/lib/lmm-api-go-deploy /var/lib/lmm-api-go-deploy/work \
  /var/lib/lmm-api-go-deploy/backups
install -d -m0700 "$workspace" "$workspace/staging"
printf 'format=1\ndeployment_id=%s\nrole=target\nworkspace=%s\ncreated_at_utc=%s\n' \
  "$deployment_id" "$workspace" "$(date -u +%FT%TZ)" >"$workspace/.lmm-deploy-workspace"
chmod 0600 "$workspace/.lmm-deploy-workspace"
EOF
"${target_ssh[@]}" bash -s -- "$target_output" "$controller_remote" "$offhost_remote" <<'EOF'
set -Eeuo pipefail
for output in "$@"; do [[ ! -e $output && ! -L $output ]] || exit 3; done
EOF
"${offhost_ssh[@]}" test ! -e "$OFFHOST_OUTPUT"
[[ ! -e $target_mirror && ! -L $target_mirror && ! -e $offhost_mirror && ! -L $offhost_mirror ]] || \
  die 'controller backup mirror already exists'
target_owned=1
controller_remote_owned=1
offhost_remote_owned=1
"${target_scp[@]}" "$PREPARE_SCRIPT" "$COPY_SCRIPT" \
  "$AGE_RECIPIENT_FILE" "$PRECUTOVER_PAYLOAD" "$ROLLBACK_CORE_PACKAGE" \
  "$ROLLBACK_GO_PACKAGE" "$target_endpoint:$remote_stage/"
"${target_ssh[@]}" chmod 0700 \
  "$remote_stage/${PREPARE_SCRIPT##*/}" "$remote_stage/${COPY_SCRIPT##*/}"
"${target_ssh[@]}" \
  "$remote_stage/${PREPARE_SCRIPT##*/}" --deployment-id "$DEPLOYMENT_ID" \
  --precutover-payload "$remote_stage/${PRECUTOVER_PAYLOAD##*/}" \
  --rollback-core-package "$remote_stage/${ROLLBACK_CORE_PACKAGE##*/}" \
  --rollback-go-package "$remote_stage/${ROLLBACK_GO_PACKAGE##*/}" \
  --rollback-layout "$ROLLBACK_LAYOUT" >/dev/null

frontend_release=$("${target_ssh[@]}" \
  readlink -- /srv/lmm-api-frontend/current)
frontend_release=${frontend_release#releases/}
inputs="$remote_stage/backup-inputs"
common_args=(--deployment-id "$DEPLOYMENT_ID" --verified-host arch-dmit --release-id "$RELEASE_ID" \
  --artifact-sha256 "$ARTIFACT_SHA256" --core-sha256 "$CORE_SHA256" --backend-sha256 "$BACKEND_SHA256" \
  --git-revision "$GIT_REVISION" --service-state active \
  --frontend-release "$frontend_release" --application "$inputs/application.tar" --frontend "$inputs/frontend.tar" \
  --configuration "$inputs/configuration.tar" --database "$inputs/postgresql.dump")
"${target_ssh[@]}" "$remote_stage/${COPY_SCRIPT##*/}" \
  --copy-role target --output "$target_output" "${common_args[@]}" >/dev/null
for role in controller off-host; do
  output=$controller_remote
  [[ $role == controller ]] || output=$offhost_remote
  "${target_ssh[@]}" "$remote_stage/${COPY_SCRIPT##*/}" \
    --copy-role "$role" --output "$output" --age-recipient-file "$remote_stage/${AGE_RECIPIENT_FILE##*/}" \
    "${common_args[@]}" >/dev/null
done

mkdir -m0700 -- "$target_mirror" "$offhost_mirror"
target_mirror_owned=1
offhost_mirror_owned=1
"${target_scp[@]}" -p -r "$target_endpoint:$target_output/." "$target_mirror/"
controller_owned=1
"${target_scp[@]}" -p -r "$target_endpoint:$controller_remote" "$CONTROLLER_OUTPUT"
"${target_scp[@]}" -p -r "$target_endpoint:$offhost_remote/." "$offhost_mirror/"

"${offhost_ssh[@]}" install -d -m0700 "${OFFHOST_OUTPUT%/*}"
offhost_owned=1
"$scp_bin" -F "$SSH_CONFIG" -p -r "$offhost_mirror" "$JUMP_HOST:$OFFHOST_OUTPUT"
"${offhost_ssh[@]}" bash -s -- "$OFFHOST_OUTPUT" <<'EOF'
set -Eeuo pipefail
directory=$1
[[ -d $directory && ! -L $directory ]]
(cd "$directory" && sha256sum -c SHA256SUMS >/dev/null)
EOF

"$VERIFY_SCRIPT" --role production --deployment-id "$DEPLOYMENT_ID" \
  --copy "target=$target_mirror" --copy "controller=$CONTROLLER_OUTPUT" \
  --copy "off-host=$offhost_mirror" >/dev/null
trap - ERR
"${target_ssh[@]}" rm -rf -- "$controller_remote" "$offhost_remote" "$inputs"
printf 'target=%s\ncontroller=%s\noffhost=%s\n' "$target_output" "$CONTROLLER_OUTPUT" "$OFFHOST_OUTPUT"
