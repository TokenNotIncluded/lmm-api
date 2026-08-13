#!/usr/bin/env bash
set -Eeuo pipefail

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
readonly HERE
readonly SHARED="$HERE/../common/lmm-api"

die() { printf 'test-bin-makepkg: %s\n' "$*" >&2; exit 1; }

: "${TMPDIR:?set TMPDIR to a marker-owned build workspace}"
tmp=$(mktemp -d "$TMPDIR/lmm-bin-makepkg.XXXXXXXX")
cleanup() { rm -rf -- "$tmp"; }
trap cleanup EXIT
mkdir -p "$tmp/bin"
install -Dm0755 /usr/bin/true "$tmp/bin/cosign"

add_metadata() {
  local bundle=$1 file
  for file in LICENSE NOTICE THIRD-PARTY-LICENSES.md; do
    printf 'fixture\n' > "$bundle/$file"
  done
  printf '%040d\n' 0 > "$bundle/REVISION"
}

create_archive() {
  local work=$1 artifact=$2
  tar -czf "$work/${artifact}.tar.gz" -C "$work/stage" "$artifact"
  (cd "$work" && sha256sum "${artifact}.tar.gz" > "${artifact}.tar.gz.sha256")
  printf '{}\n' > "$work/${artifact}.tar.gz.sigstore.json"
}

build_package() {
  local package=$1
  shift
  local work="$tmp/$package" archive expected
  PATH="$tmp/bin:$PATH" BUILDDIR="$work/build" PKGDEST="$work/packages" \
    makepkg --dir "$work" --force --nodeps --noconfirm --cleanbuild
  archive=$(printf '%s\n' "$work/packages"/*.pkg.tar.*)
  [[ -f $archive ]] || die "$package did not produce a package archive"
  for expected in "$@"; do
    bsdtar -tf "$archive" | grep -Fqx "$expected" ||
      die "$package archive is missing $expected"
  done
}

go_work="$tmp/lmm-api-go-bin"
go_pkgver=$(awk -F= '$1 == "pkgver" { print $2; exit }' "$HERE/lmm-api-go-bin/PKGBUILD")
[[ $go_pkgver =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "invalid Go binary pkgver: $go_pkgver"
go_artifact="lmm-api-go-${go_pkgver}-linux-amd64"
go_bundle="$go_work/stage/$go_artifact"
mkdir -p "$go_bundle/frontend-dist"
cp "$HERE/lmm-api-go-bin/PKGBUILD" "$go_work/"
printf '#!/bin/sh\nexit 0\n' > "$go_bundle/lmm-api-go"
chmod 0755 "$go_bundle/lmm-api-go"
printf '<!doctype html>\n' > "$go_bundle/frontend-dist/index.html"
cp "$SHARED/lmm-api.service" "$SHARED/lmm-api-go.env" "$go_bundle/"
add_metadata "$go_bundle"
create_archive "$go_work" "$go_artifact"
build_package lmm-api-go-bin \
  usr/bin/lmm-api-go \
  usr/bin/lmm-api \
  usr/lib/systemd/system/lmm-api.service \
  etc/lmm-api-go/lmm-api-go.env \
  usr/share/lmm-api-go/frontend-dist/index.html

rs_work="$tmp/lmm-api-rs-bin"
rs_pkgver=$(awk -F= '$1 == "pkgver" { print $2; exit }' "$HERE/lmm-api-rs-bin/PKGBUILD")
[[ $rs_pkgver =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "invalid Rust binary pkgver: $rs_pkgver"
rs_artifact="lmm-api-rs-${rs_pkgver}-linux-amd64"
rs_bundle="$rs_work/stage/$rs_artifact"
mkdir -p "$rs_bundle"
cp "$HERE/lmm-api-rs-bin/PKGBUILD" "$rs_work/"
for binary in lmm-api-rs lmm-db-migrate; do
  printf '#!/bin/sh\nexit 0\n' > "$rs_bundle/$binary"
  chmod 0755 "$rs_bundle/$binary"
done
add_metadata "$rs_bundle"
create_archive "$rs_work" "$rs_artifact"
build_package lmm-api-rs-bin usr/bin/lmm-api-rs usr/bin/lmm-db-migrate

printf '%s\n' 'prebuilt direct-backend AUR packages built with makepkg'
