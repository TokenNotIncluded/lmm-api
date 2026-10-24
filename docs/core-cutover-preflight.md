# Core cutover preflight

`scripts/core-cutover-preflight.py` is a read-only gate for a production cutover
record. It does not connect to PostgreSQL, SSH, systemd, Nginx or a provider, and
it never reads a credential file. The input is a sanitized contract record: it
contains capability booleans, enum values and SHA-256 bindings only. Duplicate
JSON fields, unknown fields, literal secret fields and an unbound contract are
rejected before a gate result is produced.

A blocked result exits with status `1`; malformed or stale evidence exits with
status `2`. The observation timestamp must be timezone-aware and no more than
15 minutes old. Only `READY` with `safe_to_cutover=true` is a pass. A healthy
Core process by itself cannot produce this result.

## Current ArchDmit evidence

The latest read-only rehearsal record (2026-10-24) is still blocked:

- Core and Extensions are loopback-ready, but the public edge remains on the old
  `lmm-api` service. No public route or payment callback was switched.
- Core's configured capabilities are identity, Web identity and events. `models`,
  `models_streaming`, `payments`, `commerce`, `assistant`, `assistant_tools` and
  `tool_market` are all `false`; `/health/ready` therefore describes configured
  components only.
- Authenticated Extensions module discovery returned only `identity` and
  `remote-control`; no assistant or tool-market module was selected.
- The source snapshot retains 45 pending subscription liabilities and 37 pending
  top-ups (82 unknown, non-spendable rows). There is no final writer fence or
  financial reconciliation receipt.

These facts are recorded in
`.local-tests/deploy-next-20261024/DEPLOY-STAGING-STATUS.md` (the RPC/Web
follow-up at lines 43-55) and the redacted runtime contract at
`.local-tests/deploy-next-20261024/core-check-db-rpc-web.private.json`. The
`CANARY-CUTOVER-REVIEW.md` file contains an earlier pre-install observation; do
not combine it with the later installed-runtime record. Re-run the host checks
and bind all evidence to one source revision before a future cutover.

## Missing contracts and credentials

The validator reports the following as separate gates. A schema catalog or
`/health/ready: 200` does not satisfy any of them.

| Gate | Required evidence before enablement |
| --- | --- |
| Model forwarding | Core model file with non-empty routes, enabled upstream/target bindings, exact credential-reference set and provider-context contract; private upstream credentials; a non-billable forwarding probe; `/health/model-ready: 200`. The source checks these bindings in `services/core/src/config.rs` around lines 2231-2327. |
| Payments | Core payment config/token pair, private webhook secret and explicit RPC permissions; an independent Go host/payment credential; a reviewed external provider contract, durable proof/ledger acceptance, refund/recovery probe and financial reconciliation. The current Core config only accepts `provider='stripe-test'` (`services/core/src/config.rs` around lines 1619-1762), so a test channel cannot pass this production gate. |
| Commerce | Core commerce config/token pair, explicit pay/settle/refund/read/delivery/reconcile permissions, Go store database and a Core funds adapter with independent credentials, plus delivery/recovery and financial reconciliation evidence. |
| Assistant | Core assistant config/token, an enabled model gateway, Web identity, durable assistant schema and an assistant credential distinct from identity, provider and financial tokens; Go must select `assistant` and supply its private cursor key. |
| Assistant tools | All assistant prerequisites plus durable agent/tool schema, business-facts read authority, browser confirmation and tool-market authority. |
| Tool market | Go `toolmarket` selection, dedicated storage, callback URL, socket/token, exactly 32-byte encryption key and independent host/witness/builtin credentials; Core Ready and credit-authority (`LegacyReady`) proofs. |

The source-level checks that enforce these boundaries are in
`services/core/src/config.rs` and
`services/extensions/internal/composition/{runtime.go,assistant.go,toolmarket.go}`.
No production credential value belongs in a preflight record or repository file.

## Sanitized record and command

The record must use this top-level shape:

```json
{
  "protocol": "core-cutover-preflight.v1",
  "scope": "production-cutover",
  "host": "ArchDmit",
  "observed_at": "<ISO-8601 timestamp with timezone>",
  "source_revision": "<64 lowercase hex>",
  "check_db_evidence_sha256": "<64 lowercase hex>",
  "core": {
    "ready": {
      "http_status": 200,
      "database": true,
      "model_ready_status": 503,
      "capabilities": {
        "models": false,
        "models_streaming": false,
        "payments": false,
        "commerce": false,
        "assistant": false,
        "assistant_tools": false,
        "tool_market": false
      }
    },
    "public_edge": {"mode": "legacy", "candidate_selected": false},
    "contracts": {
      "model_forwarding": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null},
      "payments": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null},
      "commerce": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null},
      "assistant": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null},
      "assistant_tools": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null},
      "tool_market": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null}
    }
  },
  "extensions": {"ready_http_status": 200, "modules": [], "host_credential": "absent"},
  "migration": {"status":"blocked","unknown_liabilities":82,"reconciliation":"not_run","writer_fenced":false,"active_writers":1,"legacy_workers_stopped":false}
}
```

Run it from a reviewed evidence directory:

```sh
python3 -B scripts/core-cutover-preflight.py \
  --evidence /absolute/private/core-cutover-preflight.json \
  --output /absolute/private/core-cutover-preflight-result.json
```

The output includes each gate's `missing` list, `unsafe_enablement` for a
capability that is claimed enabled without a complete contract, and the bound
source revision. The output's `secret_values_checked="schema_only"` means that
schema validation prevented secret-shaped fields; it is not provider or
credential verification. Independent host, database, provider and browser
acceptance remain required.

Local synthetic checks (no host access) are:

```sh
python3 -B scripts/test_core_cutover_preflight.py
```
