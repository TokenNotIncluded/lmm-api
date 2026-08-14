#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"
OUT_DIR="${LMM_API_GO_OUT_DIR:-$REPO_DIR/apps/api-go/out}"
SOURCE_REF='HEAD'
RELEASE_VERSION_OVERRIDE=''

usage() {
  printf 'Usage: %s [--source-ref REF] [--version VERSION]\n' "${0##*/}" >&2
}

while (($# > 0)); do
  case "$1" in
    --source-ref)
      (($# >= 2)) || { usage; exit 2; }
      SOURCE_REF="$2"
      shift 2
      ;;
    --version)
      (($# >= 2)) || { usage; exit 2; }
      RELEASE_VERSION_OVERRIDE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
done

git -C "$REPO_DIR" rev-parse --verify --quiet "${SOURCE_REF}^{commit}" >/dev/null || {
  printf 'source ref is not a commit: %s\n' "$SOURCE_REF" >&2
  exit 1
}
git -C "$REPO_DIR" cat-file -e "${SOURCE_REF}:apps/api-go/go.mod" 2>/dev/null || {
  printf 'source ref does not contain apps/api-go/go.mod: %s\n' "$SOURCE_REF" >&2
  exit 1
}

RELEASE_VERSION=$RELEASE_VERSION_OVERRIDE
if [[ -z $RELEASE_VERSION ]]; then
  RELEASE_VERSION=$(git -C "$REPO_DIR" show "${SOURCE_REF}:VERSION")
fi
[[ -n "$RELEASE_VERSION" ]] || {
  printf '%s\n' 'could not read VERSION from source ref' >&2
  exit 1
}
[[ $RELEASE_VERSION =~ ^[0-9][0-9A-Za-z._+]*$ ]] || {
  printf 'invalid release version: %s\n' "$RELEASE_VERSION" >&2
  exit 1
}
SOURCE_REVISION=$(git -C "$REPO_DIR" rev-parse "${SOURCE_REF}^{commit}")
: "${TMPDIR:?set TMPDIR to a marker-owned build workspace}"
SOURCE_DIR="$(mktemp -d "$TMPDIR/lmm-api-go-build.XXXXXX")"
OUTPUT_TMP=''
cleanup() {
  rm -rf -- "$SOURCE_DIR"
  if [[ -n "$OUTPUT_TMP" ]]; then
    rm -f -- "$OUTPUT_TMP"
  fi
}
trap cleanup EXIT

git -C "$REPO_DIR" archive "${SOURCE_REF}:apps/api-go" | tar -x -C "$SOURCE_DIR"

mkdir -p "$OUT_DIR"
OUTPUT_TMP="$(mktemp "$OUT_DIR/.lmm-api-go.XXXXXX")"
(
  cd "$SOURCE_DIR"
  GOPROXY=off CGO_ENABLED=0 go build -trimpath -buildvcs=false \
    -ldflags "-s -w -extldflags '-static' -X github.com/LIghtJUNction/api.lmm.best/common.Version=$RELEASE_VERSION" \
    -o "$OUTPUT_TMP" .
)
chmod 0755 "$OUTPUT_TMP"
version_output="$("$OUTPUT_TMP" version)"
if [[ "$version_output" != "$RELEASE_VERSION" ]]; then
  printf 'version assertion failed: expected %s, got %s\n' "$RELEASE_VERSION" "$version_output" >&2
  exit 1
fi
file_output="$(file -Lb "$OUTPUT_TMP")"
if [[ "$file_output" != *'statically linked'* ]]; then
  printf 'static file assertion failed: %s\n' "$file_output" >&2
  exit 1
fi
ldd_output="$(ldd "$OUTPUT_TMP" 2>&1 || true)"
if [[ "$ldd_output" != *'not a dynamic executable'* && "$ldd_output" != *'statically linked'* ]]; then
  printf 'static ldd assertion failed: %s\n' "$ldd_output" >&2
  exit 1
fi
mv -f -- "$OUTPUT_TMP" "$OUT_DIR/lmm-api-go"
OUTPUT_TMP=''

printf 'built %s from %s (%s) version=%s\n' \
  "$OUT_DIR/lmm-api-go" "$SOURCE_REF" "$SOURCE_REVISION" "$RELEASE_VERSION"
sha256sum "$OUT_DIR/lmm-api-go"
