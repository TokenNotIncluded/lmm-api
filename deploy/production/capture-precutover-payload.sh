#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

readonly EXPECTED_HOST=arch-dmit
if [[ ${LMM_DEPLOY_TEST_MODE:-0} == 1 ]]; then
  WORK_ROOT=${LMM_DEPLOY_TEST_WORK_ROOT:?}
  FILESYSTEM_ROOT=${LMM_DEPLOY_TEST_FILESYSTEM_ROOT:?}
  [[ $WORK_ROOT == /* && -d $WORK_ROOT && ! -L $WORK_ROOT ]] || { printf 'capture-precutover-payload: unsafe test work root\n' >&2; exit 2; }
  [[ $FILESYSTEM_ROOT == /* && -d $FILESYSTEM_ROOT && ! -L $FILESYSTEM_ROOT ]] || \
    { printf 'capture-precutover-payload: unsafe test filesystem root\n' >&2; exit 2; }
else
  WORK_ROOT=/var/lib/lmm-api-go-deploy/work
  FILESYSTEM_ROOT=''
fi
readonly WORK_ROOT FILESYSTEM_ROOT

die() { printf 'capture-precutover-payload: %s\n' "$*" >&2; exit 2; }

WORKSPACE=''
OUTPUT=''
while (($#)); do
  case $1 in
    --workspace) (($# >= 2)) || die '--workspace requires a value'; WORKSPACE=$2; shift 2 ;;
    --output) (($# >= 2)) || die '--output requires a value'; OUTPUT=$2; shift 2 ;;
    -h|--help)
      printf '%s\n' 'Usage: capture-precutover-payload.sh --workspace PATH --output PATH'
      exit 0
      ;;
    *) die "unknown argument: $1" ;;
  esac
done

[[ $EUID -eq 0 || ${LMM_DEPLOY_TEST_MODE:-0} == 1 ]] || die 'must run as root'
observed_host=${LMM_DEPLOY_OBSERVED_HOST:-$(hostnamectl --static)}
[[ $observed_host == "$EXPECTED_HOST" ]] || die 'production host identity mismatch'
[[ $WORKSPACE == "$WORK_ROOT"/* && -d $WORKSPACE && ! -L $WORKSPACE ]] || die 'unsafe workspace'
[[ -f $WORKSPACE/.lmm-deploy-workspace && ! -L $WORKSPACE/.lmm-deploy-workspace ]] || die 'workspace marker is missing'
[[ $OUTPUT == "$WORKSPACE"/staging/* && ! -e $OUTPUT && ! -L $OUTPUT ]] || die 'unsafe or pre-existing output'
for command in chmod cp find install pacman realpath sha256sum stat systemctl tar; do
  command -v "$command" >/dev/null 2>&1 || die "required command is unavailable: $command"
done
[[ $(pacman -Qq lmm-api-go) == lmm-api-go ]] || die 'pre-cutover Go package is not installed'
if [[ $(pacman -Qq lmm-api 2>/dev/null || true) == lmm-api ]]; then
  rollback_layout='split'
else
  rollback_layout='direct'
fi

capture_root=$(mktemp -d "$WORKSPACE/staging/precutover-capture.XXXXXXXX")
cleanup() { rm -rf -- "$capture_root"; rm -f -- "$OUTPUT.new"; }
trap cleanup EXIT
mkdir -p "$capture_root/metadata" "$capture_root/core-root" "$capture_root/go-root"

copy_package_payload() {
  local package=$1 destination=$2 source logical_source physical_source relative
  while IFS= read -r source; do
    [[ $source == /* && $source != *$'\n'* ]] || die 'package contains an unsafe path'
    logical_source=$source
    physical_source=$FILESYSTEM_ROOT$logical_source
    case "$rollback_layout:$package:$logical_source" in
      split:lmm-api:/etc/lmm-api/lmm-api.env|direct:lmm-api-go:/etc/lmm-api-go/lmm-api-go.env)
        if [[ $rollback_layout == direct ]]; then
          install -Dm0600 /dev/null "$destination/etc/lmm-api-go/lmm-api-go.env"
        else
          install -Dm0600 /dev/null "$destination/etc/lmm-api/lmm-api.env"
        fi
        continue
        ;;
      split:lmm-api:/etc/|split:lmm-api:/etc/lmm-api/|split:lmm-api:/etc/lmm-api/*|\
      split:lmm-api:/usr/|split:lmm-api:/usr/bin/|split:lmm-api:/usr/bin/lmm-api|split:lmm-api:/usr/bin/lmm-api-select|\
      split:lmm-api:/usr/lib/|split:lmm-api:/usr/lib/systemd/|split:lmm-api:/usr/lib/systemd/system/|\
      split:lmm-api:/usr/lib/systemd/system/lmm-api.service|split:lmm-api:/usr/share/doc/lmm-api/*|\
      split:lmm-api:/usr/share/|split:lmm-api:/usr/share/doc/|split:lmm-api:/usr/share/licenses/|\
      split:lmm-api:/usr/share/licenses/lmm-api/*|split:lmm-api-go:/usr/|split:lmm-api-go:/usr/lib/|\
      split:lmm-api-go:/usr/lib/lmm-api/*|\
      direct:lmm-api-go:/etc/|direct:lmm-api-go:/etc/lmm-api-go/|\
      direct:lmm-api-go:/usr/|direct:lmm-api-go:/usr/bin/|direct:lmm-api-go:/usr/bin/lmm-api-go|\
      direct:lmm-api-go:/usr/lib/|direct:lmm-api-go:/usr/lib/systemd/|direct:lmm-api-go:/usr/lib/systemd/system/|\
      direct:lmm-api-go:/usr/lib/systemd/system/geoip2-country-update.service|\
      direct:lmm-api-go:/usr/lib/systemd/system/geoip2-country-update.timer|\
      direct:lmm-api-go:/usr/lib/systemd/system/lmm-api-go.service|direct:lmm-api-go:/usr/share/|\
      direct:lmm-api-go:/usr/share/doc/|direct:lmm-api-go:/usr/share/doc/lmm-api-go/|\
      direct:lmm-api-go:/usr/share/doc/lmm-api-go/*|direct:lmm-api-go:/usr/share/licenses/|\
      direct:lmm-api-go:/usr/share/licenses/lmm-api-go/|direct:lmm-api-go:/usr/share/licenses/lmm-api-go/*|\
      direct:lmm-api-go:/usr/share/lmm-api-go/|direct:lmm-api-go:/usr/share/lmm-api-go/edge-policy/|\
      direct:lmm-api-go:/usr/share/lmm-api-go/edge-policy/*|\
      direct:lmm-api-go:/usr/share/lmm-api-go/frontend-dist/|\
      direct:lmm-api-go:/usr/share/lmm-api-go/frontend-dist/*) ;;
      *) die "package exposes an unexpected path: $package:$logical_source" ;;
    esac
    relative=${logical_source#/}
    if [[ -d $physical_source && ! -L $physical_source ]]; then
      install -d -m0755 "$destination/$relative"
    elif [[ -f $physical_source && ! -L $physical_source ]]; then
      install -d -m0755 "$(dirname -- "$destination/$relative")"
      cp -a -- "$physical_source" "$destination/$relative"
    else
      die "package path is missing or unsupported: $logical_source"
    fi
  done < <(pacman -Qlq "$package")
}

printf '%s\n' "$rollback_layout" >"$capture_root/metadata/layout"
if [[ $rollback_layout == split ]]; then
  copy_package_payload lmm-api "$capture_root/core-root"
  copy_package_payload lmm-api-go "$capture_root/go-root"
  chmod 0700 "$capture_root/core-root/etc/lmm-api"
  chmod 0600 "$capture_root/core-root/etc/lmm-api/lmm-api.env"
  chmod 0644 "$capture_root/core-root/etc/lmm-api/backend.conf" \
    "$capture_root/core-root/usr/lib/systemd/system/lmm-api.service"
  chmod 0755 "$capture_root/core-root/usr/bin/lmm-api" \
    "$capture_root/core-root/usr/bin/lmm-api-select" \
    "$capture_root/go-root/usr/lib/lmm-api/backends/go/lmm-api"
  public_roots=("$capture_root/core-root/usr/share/doc" "$capture_root/core-root/usr/share/licenses")
  secret_stub=$capture_root/core-root/etc/lmm-api/lmm-api.env
  checksum_target=$capture_root/go-root/usr/lib/lmm-api/backends/go/lmm-api
  packages=(lmm-api lmm-api-go)
  observed_service=lmm-api.service
else
  copy_package_payload lmm-api-go "$capture_root/go-root"
  chmod 0700 "$capture_root/go-root/etc/lmm-api-go"
  chmod 0600 "$capture_root/go-root/etc/lmm-api-go/lmm-api-go.env"
  chmod 0755 "$capture_root/go-root/usr/bin/lmm-api-go"
  chmod 0644 "$capture_root/go-root/usr/lib/systemd/system/lmm-api-go.service"
  public_roots=(
    "$capture_root/go-root/usr/share/doc"
    "$capture_root/go-root/usr/share/licenses"
    "$capture_root/go-root/usr/share/lmm-api-go/frontend-dist"
  )
  secret_stub=$capture_root/go-root/etc/lmm-api-go/lmm-api-go.env
  checksum_target=$capture_root/go-root/usr/bin/lmm-api-go
  packages=(lmm-api-go)
  observed_service=lmm-api-go.service
fi
for public_root in "${public_roots[@]}"; do
  [[ ! -d $public_root ]] || find "$public_root" -type d -exec chmod 0755 {} +
  [[ ! -d $public_root ]] || find "$public_root" -type f -exec chmod 0644 {} +
done
[[ ! -s $secret_stub ]] || die 'secret environment data entered the payload'
if find "$capture_root/core-root" "$capture_root/go-root" -type l -print -quit | grep -q .; then
  die 'captured package payload contains a symlink'
fi

for package in "${packages[@]}"; do
  read -r name version < <(pacman -Q "$package")
  [[ $name == "$package" && $version =~ ^[0-9][0-9A-Za-z._+:-]*-[1-9][0-9.]*$ ]] || \
    die "invalid installed package identity: $package"
  printf '%s\t%s\n' "$name" "$version" >>"$capture_root/metadata/packages.tsv"
done
sha256sum "$checksum_target" >"$capture_root/metadata/actual-go.sha256"
printf 'service_active=%s\nservice_enabled=%s\n' \
  "$(systemctl is-active "$observed_service")" "$(systemctl is-enabled "$observed_service")" \
  >"$capture_root/metadata/service.env"

tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
  -C "$capture_root" -cf "$OUTPUT.new" .
chmod 0600 "$OUTPUT.new"
mv -T -- "$OUTPUT.new" "$OUTPUT"
sha256sum "$OUTPUT" >"$OUTPUT.sha256"
printf 'precutover_payload=%s\n' "$OUTPUT"
