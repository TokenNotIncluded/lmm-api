#!/usr/bin/env bash
set -Eeuo pipefail

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
readonly HERE
ROOT=$(git -C "$HERE" rev-parse --show-toplevel)
readonly ROOT
readonly SHARED="$HERE/../common/lmm-api"
readonly PACKAGES=(
  lmm-api-go
  lmm-api-go-bin
  lmm-api-go-git
  lmm-api-rs-bin
  lmm-api-rs-git
  lmm-api-web-bin
)

die() { printf 'test-aur-matrix: %s\n' "$*" >&2; exit 1; }

contains_srcinfo() {
  local package=$1 expected=$2
  grep -Fqx "$expected" "$HERE/$package/.SRCINFO" ||
    die "$package .SRCINFO is missing: $expected"
}

contains_srcinfo_prefix() {
  local package=$1 expected=$2
  grep -Fq "$expected" "$HERE/$package/.SRCINFO" ||
    die "$package .SRCINFO is missing: $expected"
}

for removed in lmm-api-bin lmm-api-git; do
  [[ ! -e $HERE/$removed/PKGBUILD ]] || die "removed core package still has a PKGBUILD: $removed"
done
for removed in backend.conf lmm-api.install lmm-api-go.service lmm-api.env; do
  [[ ! -e $SHARED/$removed ]] || die "removed launcher/provider asset remains: $removed"
done

for package in "${PACKAGES[@]}"; do
  pkgbuild="$HERE/$package/PKGBUILD"
  [[ -f $pkgbuild ]] || die "missing $pkgbuild"
  bash -n "$pkgbuild"
  if command -v shellcheck >/dev/null 2>&1; then
    shellcheck -s bash -e SC2034,SC2154 "$pkgbuild"
  fi
  srcinfo=$(cd -- "$HERE/$package" && makepkg --printsrcinfo)
  cmp -s <(printf '%s\n' "$srcinfo") "$HERE/$package/.SRCINFO" ||
    die "$package .SRCINFO is stale"
  contains_srcinfo "$package" $'pkgbase = '"$package"
  if grep -Fq $'\tdepends = lmm-api' "$HERE/$package/.SRCINFO"; then
    die "$package retains the removed shared launcher dependency"
  fi
done

for package in lmm-api-go-bin lmm-api-go-git; do
  contains_srcinfo_prefix "$package" $'\tprovides = lmm-api-go'
  contains_srcinfo "$package" $'\tbackup = etc/lmm-api-go/lmm-api-go.env'
done
contains_srcinfo lmm-api-go-bin $'\tconflicts = lmm-api-go-git'
contains_srcinfo lmm-api-go-git $'\tconflicts = lmm-api-go-bin'
for variant in lmm-api-go-bin lmm-api-go-git; do
  contains_srcinfo lmm-api-go $'\tconflicts = '"$variant"
done
contains_srcinfo lmm-api-go $'\tbackup = etc/lmm-api-go/lmm-api-go.env'

for package in lmm-api-rs-bin lmm-api-rs-git; do
  contains_srcinfo_prefix "$package" $'\tprovides = lmm-api-rs'
done
contains_srcinfo lmm-api-rs-bin $'\tconflicts = lmm-api-rs-git'
contains_srcinfo lmm-api-rs-git $'\tconflicts = lmm-api-rs-bin'

for package in lmm-api-go lmm-api-go-bin lmm-api-go-git lmm-api-rs-bin lmm-api-rs-git; do
  for removed_core in lmm-api lmm-api-bin lmm-api-git; do
    contains_srcinfo "$package" $'\tconflicts = '"$removed_core"
  done
done

for package in lmm-api-go-bin lmm-api-rs-bin; do
  pkgbuild="$HERE/$package/PKGBUILD"
  grep -Fq 'cosign verify-blob' "$pkgbuild" || die "$package lacks Sigstore verification"
  grep -Fq 'sha256sum' "$pkgbuild" || die "$package lacks SHA-256 verification"
  grep -Fq 'noextract=(' "$pkgbuild" || die "$package extracts before verification"
  if grep -Eq '(^|[[:space:]])(go|bun|cargo)([[:space:]]|$)' "$pkgbuild"; then
    die "$package invokes a project compiler"
  fi
done
grep -Fq '_release_tag="go-v${pkgver}"' "$HERE/lmm-api-go-bin/PKGBUILD" ||
  die 'Go binary package does not use the independent Go release tag'
grep -Fq '.github/workflows/release-go.yml@refs/tags/${_release_tag}' \
  "$HERE/lmm-api-go-bin/PKGBUILD" ||
  die 'Go binary package does not verify the independent Go release identity'
for package in lmm-api-web-bin; do
  pkgbuild="$HERE/$package/PKGBUILD"
  grep -Fq 'cosign verify-blob' "$pkgbuild" || die "$package lacks Sigstore verification"
  grep -Fq 'sha256sum' "$pkgbuild" || die "$package lacks SHA-256 verification"
  grep -Fq 'noextract=(' "$pkgbuild" || die "$package extracts before verification"
  contains_srcinfo_prefix "$package" $'\tprovides = lmm-api-web'
done
grep -Fq 'systemctl reload nginx.service' "$HERE/lmm-api-web-bin/lmm-api-web-activate" ||
  die 'web package activation does not reload nginx'
if grep -Eq '(^|[[:space:]])curl([[:space:]]|$)' "$HERE/lmm-api-web-bin/lmm-api-web-activate"; then
  die 'web package activation performs a network probe inside the package transaction'
fi
if grep -Eq 'systemctl (restart|reload) lmm-api' "$HERE/lmm-api-web-bin/lmm-api-web-activate"; then
  die 'web package activation controls the backend service'
fi
contains_srcinfo lmm-api-go-git $'\tmakedepends = bun'
contains_srcinfo lmm-api-go-git $'\tmakedepends = go>=1.25.1'
contains_srcinfo lmm-api-go $'\tmakedepends = bun'
contains_srcinfo lmm-api-go $'\tmakedepends = git'
contains_srcinfo lmm-api-go $'\tmakedepends = go>=1.25.1'
go_release_commit=11217412480e81b58f96f2b9889bd317120ff8f0
readonly go_release_commit
grep -Fqx "_commit=$go_release_commit" "$HERE/lmm-api-go/PKGBUILD" ||
  die 'canonical Go package is not pinned to the reviewed direct-package revision'
# Pull requests are checked out shallowly without tag refs. Keep the reviewed
# package version as the deterministic fallback, while still validating the
# derived value whenever the local checkout has the tag history available.
readonly reviewed_go_release_pkgver=0.1.1.r490.g112174124
if go_release_description=$(git -C "$ROOT" describe --long --tags --exclude='web-v*' --abbrev=9 "$go_release_commit" 2>/dev/null); then
  go_release_pkgver=$(printf '%s\n' "$go_release_description" | \
    sed -E 's/^v//; s/([^-]*-g)/r\1/; s/-/./g')
else
  go_release_pkgver=$reviewed_go_release_pkgver
fi
grep -Fqx "pkgver=$go_release_pkgver" "$HERE/lmm-api-go/PKGBUILD" ||
  die "canonical Go package version does not match pinned revision: $go_release_pkgver"
contains_srcinfo lmm-api-rs-git $'\tmakedepends = cargo'

grep -Fqx 'ExecStart=/usr/bin/lmm-api serve' "$SHARED/lmm-api.service" ||
  die 'Go systemd service does not execute the backend directly'
grep -Fqx 'Environment=LMM_API_FRONTEND_DIR=/usr/share/lmm-api-go/frontend-dist' \
  "$SHARED/lmm-api.service" || die 'Go service does not bind the packaged frontend'
if grep -R -Eq 'lmm-api-launcher|backends/(go|rs)' \
    "$HERE"/*/PKGBUILD "$SHARED/lmm-api.service"; then
  die 'package layout retains a launcher or provider directory'
fi

: "${TMPDIR:?set TMPDIR to a marker-owned build workspace}"
tmp=$(mktemp -d "$TMPDIR/lmm-aur-matrix.XXXXXXXX")
cleanup() { rm -rf -- "$tmp"; }
trap cleanup EXIT

stage="$tmp/stage"
go_bin_pkgver=$(sed -n 's/^pkgver=//p' "$HERE/lmm-api-go-bin/PKGBUILD")
rs_bin_pkgver=$(sed -n 's/^pkgver=//p' "$HERE/lmm-api-rs-bin/PKGBUILD")
[[ $go_bin_pkgver =~ ^[0-9]+(\.[0-9]+)*$ ]] || die 'Go binary package version is not a fixture-safe release version'
[[ $rs_bin_pkgver =~ ^[0-9]+(\.[0-9]+)*$ ]] || die 'Rust binary package version is not a fixture-safe release version'
go_bundle="$stage/go/lmm-api-go-${go_bin_pkgver}-linux-amd64"
rs_bundle="$stage/rs/lmm-api-rs-${rs_bin_pkgver}-linux-amd64"
mkdir -p "$go_bundle/frontend-dist" "$rs_bundle"
printf '#!/bin/sh\n' > "$go_bundle/lmm-api-go"
printf '#!/bin/sh\n' > "$rs_bundle/lmm-api-rs"
printf '#!/bin/sh\n' > "$rs_bundle/lmm-db-migrate"
chmod 0755 "$go_bundle/lmm-api-go" "$rs_bundle/lmm-api-rs" "$rs_bundle/lmm-db-migrate"
printf '<!doctype html>\n' > "$go_bundle/frontend-dist/index.html"
cp "$SHARED/lmm-api.service" "$SHARED/lmm-api-go.env" "$go_bundle/"
for bundle in "$go_bundle" "$rs_bundle"; do
  for file in LICENSE NOTICE THIRD-PARTY-LICENSES.md; do
    printf 'fixture\n' > "$bundle/$file"
  done
  printf '%040d\n' 0 > "$bundle/REVISION"
done

(
  # shellcheck disable=SC2034 # Read by the sourced PKGBUILD.
  CARCH=x86_64
  srcdir="$stage/go"
  pkgdir="$tmp/pkg-go"
  # shellcheck disable=SC1091
  source "$HERE/lmm-api-go-bin/PKGBUILD"
  package
)
(
  # shellcheck disable=SC2034 # Read by the sourced PKGBUILD.
  srcdir="$stage/rs"
  # shellcheck disable=SC2034 # Read by the sourced PKGBUILD.
  pkgdir="$tmp/pkg-rs"
  # shellcheck disable=SC1091
  source "$HERE/lmm-api-rs-bin/PKGBUILD"
  package
)

for packaged_path in \
  pkg-go/usr/bin/lmm-api-go \
  pkg-go/usr/lib/systemd/system/lmm-api.service \
  pkg-go/etc/lmm-api-go/lmm-api-go.env \
  pkg-go/usr/share/lmm-api-go/frontend-dist/index.html \
  pkg-rs/usr/bin/lmm-api-rs \
  pkg-rs/usr/bin/lmm-db-migrate; do
  [[ -f $tmp/$packaged_path ]] || die "mock package layout is missing $packaged_path"
done
[[ -L $tmp/pkg-go/usr/bin/lmm-api ]] || die 'Go package is missing the canonical provider symlink'
[[ ! -e $tmp/pkg-rs/usr/bin/lmm-api && ! -L $tmp/pkg-rs/usr/bin/lmm-api ]] ||
  die 'Rust package exposes the Go provider command'

printf '%s\n' 'six-package direct-backend and web AUR matrix verified'
