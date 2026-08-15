#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

readonly EXPECTED_HOST=arch-dmit
readonly ROOT=/var/lib/lmm-api-go-deploy/work
ENV_FILE=''

die() { printf 'prepare-production-backup: %s\n' "$*" >&2; exit 2; }

DEPLOYMENT_ID=''
PRECUTOVER_PAYLOAD=''
ROLLBACK_CORE=''
ROLLBACK_GO=''
ROLLBACK_LAYOUT='split'
CHECK_ENV_ONLY=0
while (($#)); do
  case $1 in
    --deployment-id) DEPLOYMENT_ID=${2:?}; shift 2 ;;
    --env-file) ENV_FILE=${2:?}; shift 2 ;;
    --precutover-payload) PRECUTOVER_PAYLOAD=${2:?}; shift 2 ;;
    --rollback-core-package) ROLLBACK_CORE=${2:?}; shift 2 ;;
    --rollback-go-package) ROLLBACK_GO=${2:?}; shift 2 ;;
    --rollback-layout) ROLLBACK_LAYOUT=${2:?}; shift 2 ;;
    --check-env-only) CHECK_ENV_ONLY=1; shift ;;
    *) die "unknown argument: $1" ;;
  esac
done

case $ROLLBACK_LAYOUT in split|direct) ;; *) die 'rollback layout must be split or direct' ;; esac
if [[ -z $ENV_FILE ]]; then
  if [[ $ROLLBACK_LAYOUT == split ]]; then
    ENV_FILE=/etc/lmm-api/lmm-api.env
  else
    ENV_FILE=/etc/lmm-api-go/lmm-api-go.env
  fi
fi

parse_database_url() {
  local line value database_url='' database_assignments=0
  [[ -f $ENV_FILE && ! -L $ENV_FILE ]] || die 'application environment file is missing or unsafe'
  while IFS= read -r line || [[ -n $line ]]; do
    line=${line%$'\r'}
    # Match literal shell execution syntax; nothing from this file is evaluated.
    # shellcheck disable=SC2016
    [[ $line != *'$('* && $line != *'`'* ]] || die 'environment file contains executable shell syntax'
    [[ $line =~ ^[[:space:]]*(SQL_DSN|DATABASE_URL)[[:space:]]*=(.*)$ ]] || continue
    ((database_assignments += 1))
    value=${BASH_REMATCH[2]}
    value=${value#"${value%%[![:space:]]*}"}
    value=${value%"${value##*[![:space:]]}"}
    if [[ $value == \'*\' && ${#value} -ge 2 ]]; then
      value=${value:1:${#value}-2}
      [[ $value != *\'* ]] || die 'database URL has unsafe single-quote syntax'
    elif [[ $value == \"*\" && ${#value} -ge 2 ]]; then
      value=${value:1:${#value}-2}
      [[ $value != *\\* && $value != *'$'* && $value != *'"'* ]] || die 'database URL has unsafe double-quote syntax'
    fi
    [[ $value == postgres://* || $value == postgresql://* ]] || die 'production database is not unambiguously PostgreSQL'
    [[ $value != *[[:space:]]* && $value != *'('* && $value != *')'* && $value != *'{'* && \
      $value != *'}'* && $value != *';'* ]] || die 'database URL contains unsafe characters'
    database_url=$value
  done <"$ENV_FILE"
  ((database_assignments == 1)) || die 'environment file must contain exactly one recognized database URL assignment'
  printf '%s' "$database_url"
}

if ((CHECK_ENV_ONLY)); then
  parse_database_url >/dev/null
  printf 'database_engine=postgres\n'
  exit 0
fi

[[ $EUID -eq 0 || ${LMM_DEPLOY_TEST_MODE:-0} == 1 ]] || die 'must run as root'
observed_host=${LMM_DEPLOY_OBSERVED_HOST:-$(hostnamectl --static)}
[[ $observed_host == "$EXPECTED_HOST" ]] || die 'production host identity mismatch'
[[ $DEPLOYMENT_ID =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ ]] || die 'invalid deployment ID'
workspace="$ROOT/$DEPLOYMENT_ID"
marker="$workspace/.lmm-deploy-workspace"
[[ -d $workspace && ! -L $workspace && -f $marker && ! -L $marker ]] || die 'target workspace is not marker-owned'
grep -Fqx "deployment_id=$DEPLOYMENT_ID" "$marker" || die 'target workspace marker mismatch'
for input in "$PRECUTOVER_PAYLOAD" "$ROLLBACK_CORE" "$ROLLBACK_GO"; do
  [[ $input == "$workspace"/staging/* && -s $input && -f $input && ! -L $input ]] || die 'rollback input is missing or unsafe'
done
[[ $(pacman -Qp "$ROLLBACK_GO") == "$(pacman -Q lmm-api-go)" ]] || die 'Go rollback package does not match production'
if [[ $ROLLBACK_LAYOUT == split ]]; then
  [[ $(pacman -Qp "$ROLLBACK_CORE") == "$(pacman -Q lmm-api)" ]] || die 'core rollback package does not match production'
else
  [[ $(<"$ROLLBACK_CORE") == direct ]] || die 'direct rollback marker is invalid'
  ! pacman -Q lmm-api >/dev/null 2>&1 || die 'direct backup unexpectedly found the split core package'
fi

output="$workspace/staging/backup-inputs"
[[ ! -e $output && ! -L $output ]] || die 'backup inputs already exist'
mkdir -m0700 "$output"
application_stage=$(mktemp -d "$workspace/staging/application-backup.XXXXXXXX")
cleanup() { rm -rf -- "$application_stage"; }
trap cleanup EXIT
install -Dm0600 "$PRECUTOVER_PAYLOAD" "$application_stage/precutover-payload.tar"
install -Dm0600 "$ROLLBACK_CORE" "$application_stage/${ROLLBACK_CORE##*/}"
install -Dm0600 "$ROLLBACK_GO" "$application_stage/${ROLLBACK_GO##*/}"
if [[ $ROLLBACK_LAYOUT == split ]]; then
  dropin_roots=(/etc/systemd/system/lmm-api.service.d /etc/systemd/system.control/lmm-api.service.d)
  package_names=(lmm-api lmm-api-go)
  observed_service=lmm-api.service
  configuration_entries=(lmm-api)
else
  dropin_roots=(/etc/systemd/system/lmm-api-go.service.d /etc/systemd/system.control/lmm-api-go.service.d)
  package_names=(lmm-api-go)
  observed_service=lmm-api-go.service
  configuration_entries=(lmm-api-go)
  [[ ! -d /etc/lmm-api || -L /etc/lmm-api ]] || configuration_entries+=(lmm-api)
fi
for dropin_root in "${dropin_roots[@]}"; do
  if [[ -d $dropin_root && ! -L $dropin_root ]]; then
    relative=${dropin_root#/}
    install -d -m0700 "$application_stage/${relative%/*}"
    cp -a -- "$dropin_root" "$application_stage/$relative"
  fi
done
pacman -Qi "${package_names[@]}" >"$application_stage/package-info.txt"
systemctl show "$observed_service" -p LoadState -p ActiveState -p SubState -p UnitFileState \
  >"$application_stage/service-state.txt"
tar --sort=name --numeric-owner --owner=0 --group=0 -C "$application_stage" -cf "$output/application.tar" .

frontend_link=$(readlink -- /srv/lmm-api-frontend/current)
[[ $frontend_link =~ ^releases/([A-Za-z0-9][A-Za-z0-9._-]{0,127})$ ]] || die 'frontend identity is unsafe'
frontend_release=${BASH_REMATCH[1]}
frontend_dir="/srv/lmm-api-frontend/$frontend_link"
[[ -d $frontend_dir && ! -L $frontend_dir ]] || die 'frontend release directory is unsafe'
tar --sort=name --numeric-owner --owner=0 --group=0 -C "$frontend_dir" -cf "$output/frontend.tar" .
tar --sort=name --numeric-owner --owner=0 --group=0 -C /etc -cf "$output/configuration.tar" \
  "${configuration_entries[@]}"

database_url=$(parse_database_url)
pg_dump --dbname="$database_url" --format=custom --file="$output/postgresql.dump.new"
pg_restore --list "$output/postgresql.dump.new" >/dev/null
mv -T "$output/postgresql.dump.new" "$output/postgresql.dump"
chmod 0600 "$output"/*
sha256sum "$output"/* >"$output/SHA256SUMS"
printf 'backup_inputs=%s\nfrontend_release=%s\n' "$output" "$frontend_release"
