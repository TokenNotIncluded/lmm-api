#!/usr/bin/env bash

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
readonly script_dir
readonly cleanup=${script_dir%/scripts/tests}/scripts/cleanup-owned-workspace.sh
test_base=${XDG_STATE_HOME:-$HOME/.local/state}/lmm-api/skill-tests
mkdir -p "$test_base"
test_root=$(mktemp -d -p "$test_base")
root=$test_root/workspaces
mkdir -p "$root"
trap 'rm -rf -- "$test_root"' EXIT

deployment_id='cleanup-contract-01'
readonly deployment_id
workspace=$root/$deployment_id
mkdir -p "$workspace"/{state,staging,tmp,cache}
printf 'payload\n' >"$workspace/staging/package"
printf 'payload\n' >"$workspace/tmp/file"
printf 'payload\n' >"$workspace/cache/file"
cat >"$workspace/.lmm-deploy-workspace" <<EOF
format=1
deployment_id=$deployment_id
role=target
created_at_utc=2026-08-14T00:00:00Z
EOF
printf 'ROLLED_BACK reason=test\n' >"$workspace/state/status"

output=$("$cleanup" --role target --deployment-id "$deployment_id" --root "$root")
grep -Fq 'final_state=ROLLED_BACK' <<<"$output"
output=$("$cleanup" --role target --deployment-id "$deployment_id" --root "$root" --execute)
grep -Fq 'removed=staging,tmp,cache' <<<"$output"
[[ -d $workspace && -f $workspace/.lmm-deploy-workspace && -f $workspace/state/status ]]
[[ ! -e $workspace/staging && ! -e $workspace/tmp && ! -e $workspace/cache ]]

printf 'cleanup retains terminal audit state and removes only disposable children\n'
