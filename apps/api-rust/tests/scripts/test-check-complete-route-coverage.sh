#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd -P)
checker="$repo_root/apps/api-rust/tests/scripts/check-complete-route-coverage.sh"
runtime=$(mktemp -d "${TMPDIR:-/tmp}/lmm-complete-route-coverage-test.XXXXXX")
trap 'rm -rf -- "$runtime"' EXIT
fail() { printf 'complete route coverage self-test: %s\n' "$*" >&2; exit 1; }
expect_fail() { "$@" >"$runtime/out" 2>"$runtime/err" && fail "expected failure: $*" || true; }

printf '%s\n' \
  $'GET\t/candidate\tgo.candidate' \
  $'GET\t/stub\tgo.stub' \
  $'POST\t/blocked\tgo.blocked' \
  $'GET\t/static\tgo.static' >"$runtime/manifest.tsv"
cp "$runtime/manifest.tsv" "$runtime/frozen.tsv"
printf '%s\n' '# method	path	Rust handler' $'GET\t/candidate\trust.candidate' >"$runtime/implemented.tsv"
printf '%s\n' \
  $'method\tpath\trust_source\tlegacy_handler\tfrozen_ledger\tbehavior_test\trationale' \
  $'GET\t/stub\trust.stub\tgo.stub\tfrozen\ttest\texplicit 501' >"$runtime/stubs.tsv"
printf '%s\n' \
  $'method\tpath\tadapter/boundary\tcurrent fail-closed behavior\tfrozen Go capability\treusable real implementation if any\tpriority\trequired safe test configuration\tsource evidence\tnotes' \
  $'POST\t/blocked\tDenyProvider\tfixed fail-closed\tfrozen\tfixture\tP1\tisolation\tsource\tnotes' >"$runtime/blockers.tsv"
printf '%s\n' '# method	path	source evidence' $'GET\t/candidate\trust.mount' >"$runtime/normal.tsv"
: >"$runtime/mcp.tsv"
mkdir "$runtime/results"
printf '%s\n' '{"method":"GET","path":"/candidate","differential_verified":true,"differences":null,"mismatch_names":[]}' >"$runtime/results/candidate.jsonl"

run() {
  COMPLETE_GO_MANIFEST="$runtime/manifest.tsv" \
  COMPLETE_FROZEN_LEDGER="$runtime/frozen.tsv" \
  COMPLETE_RUST_IMPLEMENTED_LEDGER="$runtime/implemented.tsv" \
  COMPLETE_LEGACY_STUB_LEDGER="$runtime/stubs.tsv" \
  COMPLETE_RUNTIME_BLOCKERS_LEDGER="$runtime/blockers.tsv" \
  COMPLETE_NORMAL_MOUNTS_LEDGER="$runtime/normal.tsv" \
  COMPLETE_MCP_PATHS="$runtime/mcp.tsv" \
  COMPLETE_DIFFERENTIAL_RESULTS_DIR="$runtime/results" \
  bash "$checker"
}

bash -n "$checker" "$0"
run >"$runtime/report.jsonl"
[[ $(wc -l <"$runtime/report.jsonl" | tr -d ' ') == 5 ]] || fail 'expected four route rows and one summary row'
for class in differential-candidate legacy-501 provider-blocked static-only; do
  grep -Fq "\"class\":\"$class\"" "$runtime/report.jsonl" || fail "missing $class"
done
grep -Fq '"differential_verified":true' "$runtime/report.jsonl" || fail 'differential result was not carried through'
grep -Fq '"coverage_complete":true' "$runtime/report.jsonl" || fail 'summary lacks coverage_complete'
grep -Fq '"parity_claimed":false' "$runtime/report.jsonl" || fail 'summary must not claim parity'

printf '%s\n' '{"method":"GET","path":"/missing","differential_verified":false}' >"$runtime/results/missing.json"
expect_fail run
rm "$runtime/results/missing.json"

printf '%s\n' $'GET\t/candidate\tgo.candidate' >>"$runtime/manifest.tsv"
expect_fail run
sed -i '$d' "$runtime/manifest.tsv"
printf '%s\n' $'GET\t/unclassified\tgo.unclassified' >>"$runtime/manifest.tsv"
expect_fail env COMPLETE_REQUIRE_EXPLICIT_CLASSIFICATION=1 \
  COMPLETE_GO_MANIFEST="$runtime/manifest.tsv" \
  COMPLETE_FROZEN_LEDGER="$runtime/frozen.tsv" \
  COMPLETE_RUST_IMPLEMENTED_LEDGER="$runtime/implemented.tsv" \
  COMPLETE_LEGACY_STUB_LEDGER="$runtime/stubs.tsv" \
  COMPLETE_RUNTIME_BLOCKERS_LEDGER="$runtime/blockers.tsv" \
  COMPLETE_NORMAL_MOUNTS_LEDGER="$runtime/normal.tsv" \
  COMPLETE_MCP_PATHS="$runtime/mcp.tsv" bash "$checker"
sed -i '$d' "$runtime/manifest.tsv"

# Current repository ledgers are consumable with a safe manifest override; no
# backend build or listener starts during this self-test.
COMPLETE_GO_MANIFEST="$repo_root/apps/api-rust/tests/fixtures/routes/legacy-go-routes.tsv" \
COMPLETE_MCP_PATHS="$runtime/mcp.tsv" bash "$checker" >"$runtime/current-ledger-report.jsonl"
grep -Fq '"total":352' "$runtime/current-ledger-report.jsonl" || fail 'current repository ledgers could not be checked'
echo 'complete route coverage self-test: passed'
