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

The preflight also requires a `release` provenance block. It binds the exact
40-character source commit to the Core and Extensions artifact digests, SBOM,
schema fingerprint, provider/identity/financial/edge evidence digests and the
runtime release identity. Runtime source and artifact digests must match the
candidate package, and the writer epoch must advance beyond the previous epoch.
This is still an evidence contract: a correctly shaped hash is not proof that a
provider, database or public edge actually produced it. Independent host and
runtime checks must calculate these values and bind them to the same record.

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
- Identity remains a separate gate: the legacy snapshot has 723 sessions, 815
  API keys, 8 passkeys, 235 OAuth bindings, 1,214 OAuth server tokens, 2 MFA
  factors and 8 backup codes. These rows require reauthentication, key
  recreation or re-enrollment; they are not native Core credentials.

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
  "source_commit": "<40 lowercase hex commit>",
  "source_revision": "<64 lowercase hex>",
  "check_db_evidence_sha256": "<64 lowercase hex>",
  "release": {
    "source_commit": "<same 40 lowercase hex commit>",
    "core_artifact_sha256": "<64 lowercase hex>",
    "extensions_artifact_sha256": "<64 lowercase hex>",
    "sbom_sha256": "<64 lowercase hex>",
    "schema_fingerprint_sha256": "<64 lowercase hex>",
    "provider_matrix_sha256": "<64 lowercase hex>",
    "identity_evidence_sha256": "<64 lowercase hex>",
    "financial_evidence_sha256": "<64 lowercase hex>",
    "edge_config_sha256": "<64 lowercase hex>",
    "runtime_release_identity_sha256": "<64 lowercase hex>",
    "runtime_source_commit": "<same 40 lowercase hex commit>",
    "runtime_core_artifact_sha256": "<same core artifact digest>",
    "runtime_extensions_artifact_sha256": "<same extensions artifact digest>",
    "previous_writer_epoch": 7,
    "writer_epoch": 8
  },
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
      "model_forwarding": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null,"extension":"absent","provider":"none"},
      "payments": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null,"extension":"absent","provider":"none"},
      "commerce": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null,"extension":"absent","provider":"none"},
      "assistant": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null,"extension":"absent","provider":"none"},
      "assistant_tools": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null,"extension":"absent","provider":"none"},
      "tool_market": {"config":"absent","credentials":"absent","contract":"absent","probe":"not_run","contract_sha256":null,"extension":"absent","provider":"none"}
    }
  },
  "extensions": {"ready_http_status": 200, "modules": [], "host_credential": "absent"},
  "identity": {
    "migration_status": "identity_only",
    "mapping_status": "passed",
    "target_comparison_status": "passed",
    "legacy_inventory": {
      "sessions": 723,
      "api_keys": 815,
      "passkeys": 8,
      "oauth_bindings": 235,
      "oauth_server_tokens": 1214,
      "mfa_factors": 2,
      "mfa_backup_codes": 8
    },
    "source_archive_sha256": "<64 lowercase hex>",
    "source_export_sha256": "<64 lowercase hex>",
    "source_baseline_proof_sha256": "<64 lowercase hex>",
    "mapping_sha256": "<64 lowercase hex>",
    "mapping_file_sha256": "<64 lowercase hex>",
    "source_target_id_map_sha256": "<64 lowercase hex>",
    "identity_contract_sha256": "<64 lowercase hex>",
    "target_import_sql_sha256": "<64 lowercase hex>",
    "target_receipt_sha256": "<64 lowercase hex>",
    "reauth_required": 2172,
    "rekey_required": 815,
    "reenroll_required": 18,
    "actions": {
      "sessions": "reauth_required",
      "api_keys": "rekey_required",
      "passkeys": "reenroll_required",
      "oauth_bindings": "reauth_required",
      "oauth_server_tokens": "reauth_required",
      "mfa_factors": "reenroll_required",
      "mfa_backup_codes": "reenroll_required"
    }
  },
  "migration": {
    "status":"blocked",
    "unknown_liabilities":82,
    "reconciliation":"not_run",
    "writer_fenced":false,
    "active_writers":1,
    "legacy_workers_stopped":false,
    "financial_evidence_sha256":"<64 lowercase hex>",
    "pending_preview_evidence_sha256":"<64 lowercase hex>",
    "provider_contract_evidence_sha256":"<64 lowercase hex>",
    "provider_reconciliation_evidence_sha256":"<64 lowercase hex>",
    "provider_io_performed":false,
    "target_writes":0,
    "ledger_writes":0,
    "spendable_writes":0,
    "receipt_writes":0
  }
}
```

The `identity` block is strict. `legacy_inventory` must contain exactly the
seven non-negative counts shown above. `mapping_status` and
`target_comparison_status` must both be `passed`; `migration_status` must be
`identity_only` or `completed`. Each inventory class has a fixed safe action:
sessions and OAuth records require reauthentication, API keys require new-key
creation, and passkeys/MFA records require re-enrollment. The action counts may
be `true` (all rows) or a non-negative integer covering the rows assigned to
that action. Every hash field is required and must be bound to the same frozen
source export, mapping, identity contract, target import SQL and target receipt;
the preflight checks format and presence, while the independent runbook checks
the bytes. Any uncovered, differently classified or unbound legacy row blocks
the identity gate and therefore the complete cutover result.

## Build the identity block from an apply directory

`scripts/build-identity-cutover-evidence.py` reads a private identity apply
directory and emits only the sanitized `identity` block. It verifies the
identity manifest/report, mapping and source proof files, generated SQL,
target-import result, target receipt, and an independent identity PASS result.
It checks their SHA-256 bindings and target instance boundary without printing
any file contents. The normal synthetic contract uses
`independent-pass.private.json`; the existing rehearsal layout is also
recognized when the input is `.../identity-apply`, with the target receipt in
its parent and `independent-identity/identity-independent-result.private.json`
beside it.

The existing rehearsal PASS file does not contain the seven legacy credential
counts or reauthentication plan. The builder therefore returns `not_run` until
the same directory (or its parent) contains a sanitized
`identity-inventory.private.json`; it never infers those counts from the source
dump or empty Core credential tables.

```sh
python3 -B scripts/build-identity-cutover-evidence.py \
  --input /absolute/private/identity-apply \
  --output /absolute/private/identity-evidence.json
```

The command exits `0` only for `PASS`; incomplete evidence is a sanitized
`not_run` or `blocked` result and exits `1`.

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

The migration block also requires four strict evidence SHA-256 bindings: the
financial review, pending-liability preview, provider contract, and provider
reconciliation evidence. It rejects any provider I/O or non-zero target,
ledger, spendable, or receipt write counter. A hand-written
`reconciliation="passed"` therefore cannot pass the cutover boundary without
the bound evidence and a zero-write boundary. The current 82-row preview has
no provider reconciliation evidence and remains blocked.

Local synthetic checks (no host access) are:

```sh
python3 -B scripts/test_core_cutover_preflight.py
```
