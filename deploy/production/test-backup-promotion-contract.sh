#!/usr/bin/env bash
set -Eeuo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repo=$(cd -- "$here/../.." && pwd -P)
promoter="$here/promote-production-backups.sh"
verifier="$repo/.agents/skills/lmm-deploy-safely/scripts/verify-backup-set.sh"
test_root=${TMPDIR:-$HOME/.local/state/lmm-api/deploy-work/backup-promotion-contract}
mkdir -p -- "$test_root"
chmod 0700 -- "$test_root"
tmp=$(mktemp -d "$test_root/backup-promotion-test.XXXXXXXX")
trap 'rm -rf -- "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/controller-work/staging"

fail() { printf 'backup-promotion-contract: %s\n' "$*" >&2; exit 1; }
expect_fail() {
  if "$@" >"$tmp/out" 2>"$tmp/err"; then
    fail "expected failure: $*"
  fi
}

cat >"$tmp/bin/ssh" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
printf '%s\n' "$*" >>"$FAKE_SSH_LOG"
case " $* " in
  *' readlink -- /srv/lmm-api-frontend/current '*) printf 'releases/old-release\n' ;;
esac
if [[ $* == *' bash -s -- '* ]]; then
  script=$(cat)
  if [[ $script == *'backup transaction lock ownership changed'* ]]; then
    printf 'failed-backup-lock-release %s\n' "$*" >>"$FAKE_SSH_LOG"
  fi
  if [[ $script == *'required backup command is unavailable:'* ]]; then
    [[ $script == *pg_dump* && $script == *pg_restore* ]] || exit 94
    if [[ ${FAKE_MISSING_TARGET_COMMAND:-} == pg_restore ]]; then
      printf 'required backup command is unavailable: pg_restore\n' >&2
      exit 2
    fi
  fi
  [[ -z ${FAKE_PREEXISTING_MATCH:-} || $* != *"$FAKE_PREEXISTING_MATCH"* ]] || exit 3
fi
if [[ $* == *' test ! -e '* && -n ${FAKE_PREEXISTING_MATCH:-} && $* == *"$FAKE_PREEXISTING_MATCH"* ]]; then
  exit 1
fi
exit 0
EOF

cat >"$tmp/bin/scp" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
printf '%s\n' "$*" >>"$FAKE_SCP_LOG"
args=("$@")
destination=${args[${#args[@]}-1]}
role=''
case $destination in
  */backup-target-*/) role=target; destination=${destination%/} ;;
  "$FAKE_CONTROLLER_OUTPUT") role=controller ;;
  */backup-off-host-*/) role=off-host; destination=${destination%/} ;;
esac
if [[ -n $role ]]; then
  mkdir -p -m0700 -- "$destination"
  encrypted=false
  [[ $role == controller || $role == off-host ]] && encrypted=true
  for kind in application frontend configuration database; do
    suffix=archive
    [[ $encrypted == true && ( $kind == configuration || $kind == database ) ]] && suffix=age
    printf '%s-%s\n' "$role" "$kind" >"$destination/$kind.$suffix"
    chmod 0600 "$destination/$kind.$suffix"
  done
  created=$(date -u +%FT%TZ)
  {
    printf 'format=1\ncreated_at_utc=%s\ndeployment_id=%s\ncopy_role=%s\n' "$created" "$FAKE_DEPLOYMENT_ID" "$role"
    printf 'deployment_role=production\nverified_host=arch-dmit\nrelease_id=%s\n' "$FAKE_DEPLOYMENT_ID"
    printf 'artifact_sha256=%s\ngit_revision=%s\ndatabase_engine=postgres\n' "$FAKE_ARTIFACT_SHA" "$FAKE_REVISION"
    printf 'core_sha256=%s\nbackend_sha256=%s\n' "$FAKE_CORE_SHA" "$FAKE_BACKEND_SHA"
    printf 'service_state=active\nfrontend_release=old-release\n'
    for kind in application frontend configuration database; do
      suffix=archive
      [[ $encrypted == true && ( $kind == configuration || $kind == database ) ]] && suffix=age
      file="$destination/$kind.$suffix"
      printf '%s_file=%s.%s\n%s_size=%s\n%s_mode=600\n%s_mtime_utc=%s\n' \
        "$kind" "$kind" "$suffix" "$kind" "$(stat -c %s "$file")" "$kind" "$kind" \
        "$(date -u -d "@$(stat -c %Y "$file")" +%FT%TZ)"
      [[ $kind == configuration || $kind == database ]] && printf '%s_encrypted=%s\n' "$kind" "$encrypted"
    done
  } >"$destination/manifest.env"
  sha256sum "$destination"/{application.archive,frontend.archive} \
    "$destination/configuration.$([[ $encrypted == true ]] && printf age || printf archive)" \
    "$destination/database.$([[ $encrypted == true ]] && printf age || printf archive)" | \
    sed "s|$destination/||" >"$destination/SHA256SUMS"
fi
[[ -z ${FAKE_SCP_FAIL_MATCH:-} || $* != *"$FAKE_SCP_FAIL_MATCH"* ]] || exit 91
exit 0
EOF
chmod 0700 "$tmp/bin/ssh" "$tmp/bin/scp"

ssh_config="$tmp/ssh-config"
cat >"$ssh_config" <<'EOF'
Host archczy
  HostName 127.0.0.1
  User test
EOF
chmod 0600 "$ssh_config"

export LMM_DEPLOY_SSH_BIN="$tmp/bin/ssh"
export LMM_DEPLOY_SCP_BIN="$tmp/bin/scp"
export FAKE_SSH_LOG="$tmp/ssh.log" FAKE_SCP_LOG="$tmp/scp.log"
export FAKE_ARTIFACT_SHA=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
export FAKE_CORE_SHA=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
export FAKE_BACKEND_SHA=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
export FAKE_REVISION=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
printf 'age1testrecipient\n' >"$tmp/recipient"
printf 'captured payload\n' >"$tmp/precutover-payload.tar"
printf 'rollback core\n' >"$tmp/rollback-core.pkg.tar.zst"
printf 'rollback go\n' >"$tmp/rollback-go.pkg.tar.zst"

run_promoter() {
  local deployment_id=$1 controller_output=$2 offhost_output=$3 layout=${4:-split}
  FAKE_DEPLOYMENT_ID=$deployment_id FAKE_CONTROLLER_OUTPUT=$controller_output \
    "$promoter" --target-host ArchDmit --jump-host archczy --ssh-config "$ssh_config" \
    --deployment-id "$deployment_id" --controller-workspace "$tmp/controller-work" \
    --controller-output "$controller_output" --offhost-output "$offhost_output" \
    --age-recipient-file "$tmp/recipient" --release-id "$deployment_id" \
    --artifact-sha256 "$FAKE_ARTIFACT_SHA" --core-sha256 "$FAKE_CORE_SHA" \
    --backend-sha256 "$FAKE_BACKEND_SHA" --git-revision "$FAKE_REVISION" \
    --prepare-script "$here/prepare-production-backup.sh" \
    --copy-script "$here/create-backup-copy.sh" --verify-script "$verifier" \
    --precutover-payload "$tmp/precutover-payload.tar" \
    --rollback-core-package "$tmp/rollback-core.pkg.tar.zst" \
    --rollback-go-package "$tmp/rollback-go.pkg.tar.zst" \
    --rollback-layout "$layout"
}

# The remote prerequisite gate must reject a missing pg_restore before it
# creates or claims any local or remote backup output.
: >"$FAKE_SSH_LOG"
: >"$FAKE_SCP_LOG"
missing_restore_controller="$tmp/controller-missing-pg-restore"
missing_restore_offhost=/var/backups/lmm-api/missing-pg-restore
export FAKE_MISSING_TARGET_COMMAND=pg_restore
expect_fail run_promoter missing-pg-restore "$missing_restore_controller" \
  "$missing_restore_offhost"
unset FAKE_MISSING_TARGET_COMMAND
grep -Fq 'required backup command is unavailable: pg_restore' "$tmp/err" || \
  fail 'missing pg_restore did not fail at the remote prerequisite gate'
[[ ! -e $missing_restore_controller ]] || fail 'missing pg_restore created a controller output'
[[ ! -s $FAKE_SCP_LOG ]] || fail 'missing pg_restore reached an SCP operation'
if grep -Fq 'rm -rf --' "$FAKE_SSH_LOG"; then
  fail 'missing pg_restore triggered cleanup for an unclaimed output'
fi

: >"$FAKE_SSH_LOG"
: >"$FAKE_SCP_LOG"
controller_output="$tmp/controller-copy"
offhost_output=/var/backups/lmm-api/promotion-test
run_promoter promotion-test "$controller_output" "$offhost_output" >/dev/null
[[ -f $controller_output/configuration.age && -f $controller_output/database.age ]] || \
  fail 'controller backup was not promoted'
[[ -f $tmp/controller-work/staging/backup-off-host-promotion-test/configuration.age ]] || fail 'off-host mirror is missing'
[[ -f $tmp/controller-work/staging/backup-target-promotion-test/configuration.archive ]] || fail 'target mirror is missing'
control_prefix="/run/user/$(id -u)/lmm-api-"
grep -Fq -- "-F $ssh_config -o BatchMode=yes -o ControlMaster=auto -o ControlPersist=60 -o ControlPath=$control_prefix" \
  "$FAKE_SSH_LOG" || fail 'target SSH transport does not use an isolated control socket'
grep -Fq -- "ProxyCommand=exec $LMM_DEPLOY_SSH_BIN -F $ssh_config -o BatchMode=yes -o ControlMaster=auto" \
  "$FAKE_SSH_LOG" || fail 'target SSH transport does not use the isolated jump proxy'
grep -Fq -- "-p 222 root@45.59.187.63" "$FAKE_SSH_LOG" || \
  fail 'target SSH transport does not use the controlled port and endpoint'
grep -Fq -- "-F $ssh_config -o BatchMode=yes -o ControlMaster=auto -o ControlPersist=60 -o ControlPath=$control_prefix" \
  "$FAKE_SCP_LOG" || fail 'target SCP transport does not use an isolated control socket'
grep -Fq -- "ProxyCommand=exec $LMM_DEPLOY_SSH_BIN -F $ssh_config -o BatchMode=yes -o ControlMaster=auto" \
  "$FAKE_SCP_LOG" || fail 'target SCP transport does not use the isolated jump proxy'
grep -Fq 'root@45.59.187.63:' "$FAKE_SCP_LOG" || fail 'target SCP endpoint is not explicit'
grep -Fq -- "-p -r root@45.59.187.63:" "$FAKE_SCP_LOG" || \
  fail 'target backup downloads do not preserve manifest timestamps'
grep -Fq -- "-F $ssh_config -p -r $tmp/controller-work/staging/backup-off-host-promotion-test archczy:$offhost_output" \
  "$FAKE_SCP_LOG" || fail 'off-host publication does not preserve manifest timestamps'
if grep -Eq 'StrictHostKeyChecking=no|UserKnownHostsFile=/dev/null|-F /dev/null' "$FAKE_SSH_LOG" "$FAKE_SCP_LOG"; then
  fail 'transport bypassed SSH host-key or user configuration controls'
fi

rm -rf -- "$tmp/controller-work/staging/backup-target-promotion-test" \
  "$tmp/controller-work/staging/backup-off-host-promotion-test" "$controller_output"

: >"$FAKE_SSH_LOG"
: >"$FAKE_SCP_LOG"
direct_controller="$tmp/controller-direct"
direct_offhost=/var/backups/lmm-api/promotion-direct
run_promoter promotion-direct "$direct_controller" "$direct_offhost" direct >/dev/null
grep -Fq -- '--rollback-layout direct' "$FAKE_SSH_LOG" || \
  fail 'direct rollback layout was not forwarded to target backup preparation'
rm -rf -- "$tmp/controller-work/staging/backup-target-promotion-direct" \
  "$tmp/controller-work/staging/backup-off-host-promotion-direct" "$direct_controller"

# Each pre-existing destination must fail before ownership is claimed, so the
# cleanup trap must never issue a removal for that destination.
: >"$FAKE_SSH_LOG"
: >"$FAKE_SCP_LOG"
export FAKE_PREEXISTING_MATCH=/var/lib/lmm-api-go-deploy/backups/preexisting-target
expect_fail run_promoter preexisting-target "$tmp/controller-preexisting-target" \
  /var/backups/lmm-api/preexisting-target
if grep -Fq "rm -rf -- $FAKE_PREEXISTING_MATCH" "$FAKE_SSH_LOG"; then
  fail 'pre-existing target output was removed'
fi

: >"$FAKE_SSH_LOG"
: >"$FAKE_SCP_LOG"
export FAKE_PREEXISTING_MATCH=/var/backups/lmm-api/preexisting-offhost
expect_fail run_promoter preexisting-offhost "$tmp/controller-preexisting-offhost" \
  "$FAKE_PREEXISTING_MATCH"
if grep -Fq "rm -rf -- $FAKE_PREEXISTING_MATCH" "$FAKE_SSH_LOG"; then
  fail 'pre-existing off-host output was removed'
fi

unset FAKE_PREEXISTING_MATCH
preexisting_controller="$tmp/controller-preexisting"
mkdir -m0700 "$preexisting_controller"
printf 'preserve\n' >"$preexisting_controller/owner-marker"
expect_fail run_promoter preexisting-controller "$preexisting_controller" \
  /var/backups/lmm-api/preexisting-controller
[[ $(<"$preexisting_controller/owner-marker") == preserve ]] || fail 'pre-existing controller output was modified'

# Fail after target, controller, and mirror outputs exist. Cleanup must remove
# only those invocation-owned outputs, including the remote off-host path that
# was claimed immediately before the failing publication.
: >"$FAKE_SSH_LOG"
: >"$FAKE_SCP_LOG"
failure_id=promotion-failure
failure_controller="$tmp/controller-failure"
failure_offhost="/var/backups/lmm-api/$failure_id"
export FAKE_SCP_FAIL_MATCH="archczy:$failure_offhost"
expect_fail run_promoter "$failure_id" "$failure_controller" "$failure_offhost"
unset FAKE_SCP_FAIL_MATCH
[[ ! -e $failure_controller ]] || fail 'invocation-owned controller partial was retained'
[[ ! -e $tmp/controller-work/staging/backup-target-$failure_id ]] || fail 'invocation-owned target mirror was retained'
[[ ! -e $tmp/controller-work/staging/backup-off-host-$failure_id ]] || fail 'invocation-owned off-host mirror was retained'
grep -Fq "rm -rf -- /var/lib/lmm-api-go-deploy/backups/$failure_id" "$FAKE_SSH_LOG" || \
  fail 'invocation-owned target partial was not removed'
grep -Fq "rm -rf -- $failure_offhost" "$FAKE_SSH_LOG" || \
  fail 'invocation-owned off-host partial was not removed'
grep -Fq "rm -rf -- /var/lib/lmm-api-go-deploy/work/$failure_id/staging/controller-copy" "$FAKE_SSH_LOG" || \
  fail 'invocation-owned target-side controller partial was not removed'
grep -Fq "failed-backup-lock-release" "$FAKE_SSH_LOG" || \
  fail 'failed backup promotion did not release its transaction lock'

printf 'backup promotion contract verified\n'
