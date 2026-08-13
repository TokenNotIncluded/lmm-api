#!/usr/bin/env bash
set -Eeuo pipefail

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
: "${TMPDIR:?set TMPDIR to a marker-owned build workspace}"
work=$(mktemp -d "$TMPDIR/lmm-web-activation.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/bin" "$work/source"

cat >"$work/bin/nginx" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
cat >"$work/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$MOCK_SYSTEMCTL_LOG"
exit 0
EOF
cat >"$work/bin/curl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$MOCK_CURL_LOG"
[[ ${MOCK_CURL_FAIL:-0} != 1 ]]
EOF
chmod +x "$work/bin/"*

export PATH="$work/bin:$PATH"
export MOCK_SYSTEMCTL_LOG="$work/systemctl.log"
export MOCK_CURL_LOG="$work/curl.log"
export LMM_API_WEB_ROOT="$work/root"
export LMM_API_WEB_SOURCE="$work/source"
export LMM_API_WEB_PUBLISHER="$HERE/../../deploy/frontend-release.sh"
export LMM_API_WEB_REVISION_FILE="$work/REVISION"
export LMM_API_WEB_PROBE_URL='https://example.invalid/'
printf '%040d\n' 0 >"$work/REVISION"

printf '<!doctype html>first\n' >"$work/source/index.html"
"$HERE/lmm-api-web-bin/lmm-api-web-activate" 1.0.0-1
[[ $(readlink "$work/root/current") == releases/1.0.0-1.g000000000000 ]]

printf '<!doctype html>second\n' >"$work/source/index.html"
"$HERE/lmm-api-web-bin/lmm-api-web-activate" 1.0.1-1
[[ $(readlink "$work/root/current") == releases/1.0.1-1.g000000000000 ]]

printf '<!doctype html>bad\n' >"$work/source/index.html"
if MOCK_CURL_FAIL=1 "$HERE/lmm-api-web-bin/lmm-api-web-activate" 1.0.2-1; then
  printf '%s\n' 'failed web probe unexpectedly succeeded' >&2
  exit 1
fi
[[ $(readlink "$work/root/current") == releases/1.0.1-1.g000000000000 ]]
grep -Fqx 'reload nginx.service' "$work/systemctl.log"
grep -Fq -- '--resolve api.lmm.best:443:127.0.0.1' "$work/curl.log"
if grep -Eq 'restart.*lmm-api|reload.*lmm-api' "$work/systemctl.log"; then
  printf '%s\n' 'frontend activation touched the backend service' >&2
  exit 1
fi

printf '%s\n' 'independent web activation and rollback verified'
