"""Source-only contract for unresolved legacy payment operations.

This module deliberately does not write PostgreSQL, create ledger entries, or
turn a pending provider operation into a wallet reservation.  It produces a
hash-bound private preview that a future Core-owned liability writer can
consume after its schema and permission contract is installed.
"""
from __future__ import annotations

import copy
from collections import Counter
import json

FORMAT_VERSION = 1
SCOPE = "legacy-pending-liabilities-preview-v1"
IMPORT_SCOPE = "legacy-pending-liabilities-import-request-v1"
DISPOSITION = "deferred-source-only-no-target-write"
SETTLEMENT_STATE = "unknown"
TABLE_KINDS = {"subscription_orders": "subscription_payment", "top_ups": "topup_payment"}
AMOUNT_FIELDS = {
    "subscription_orders": ("money", "expected_amount_micros", "refunded_amount_micros", "refunded_quota", "charged_quota"),
    "top_ups": ("amount", "money", "expected_amount_micros", "settled_amount_micros", "credited_quota", "refunded_amount_micros", "refunded_quota", "platform_amount_micros"),
}
CURRENCY_FIELDS = {
    "subscription_orders": ("plan_currency", "settlement_currency"),
    "top_ups": ("settlement_currency", "discount_qualifying_unit"),
}
REFERENCE_FIELDS = {
    "subscription_orders": ("trade_no", "provider_subscription_id", "provider_product_id", "provider_store_id"),
    "top_ups": ("trade_no", "provider_transaction_id", "provider_event_id", "provider_product_id", "provider_store_id"),
}
RECORD_FIELDS = frozenset({
    "record_id", "source_table", "source_row_id", "source_row_ordinal", "source_row_sha256",
    "source_user_id", "target_user_id", "target_account_id", "operation_kind", "legacy_status", "provider",
    "references", "raw_amount", "raw_currency", "raw_row", "settlement_state", "spendable",
    "spendable_units", "disposition",
})


def _require_hash(F, value, code):
    F.require(type(value) is str and F.IDENTITY.HEX.fullmatch(value) is not None, code)
    return value


def _row_id(F, value, code):
    return F.integer(value, code, minimum=1)


def _string_or_empty(value):
    return "" if value is None else value if type(value) is str else str(value)


def _source_row(F, table, ordinal, row, identities, target_users, migration_id):
    F.require(type(row) is dict, "pending_liability_source_row_invalid")
    source_id = _row_id(F, row.get("id"), "pending_liability_source_id_invalid")
    user_id = _row_id(F, row.get("user_id"), "pending_liability_user_id_invalid")
    F.require(row.get("status") == "pending", "pending_liability_status_not_pending")
    target_user_id = target_users.get(user_id)
    account_id = identities.get(user_id)
    if target_user_id is not None:
        target_user_id = _row_id(F, target_user_id, "pending_liability_target_user_invalid")
    if account_id is not None:
        account_id = _row_id(F, account_id, "pending_liability_target_account_invalid")
    raw_row = copy.deepcopy(row)
    row_hash = F.digest(raw_row)
    record = {
        "record_id": F.digest([migration_id, table, source_id, row_hash]),
        "source_table": table, "source_row_id": source_id,
        "source_row_ordinal": F.integer(ordinal, "pending_liability_source_ordinal_invalid", minimum=0),
        "source_row_sha256": row_hash, "source_user_id": user_id, "target_user_id": target_user_id,
        "target_account_id": account_id,
        "operation_kind": TABLE_KINDS[table], "legacy_status": "pending",
        "provider": _string_or_empty(raw_row.get("payment_provider")),
        "references": {field: raw_row.get(field) for field in REFERENCE_FIELDS[table] if field in raw_row},
        "raw_amount": {field: raw_row.get(field) for field in AMOUNT_FIELDS[table] if field in raw_row},
        "raw_currency": {field: raw_row.get(field) for field in CURRENCY_FIELDS[table] if field in raw_row},
        "raw_row": raw_row, "settlement_state": SETTLEMENT_STATE, "spendable": False,
        "spendable_units": 0, "disposition": DISPOSITION,
    }
    F.require(set(record) == RECORD_FIELDS, "pending_liability_record_shape_invalid")
    return record


def _validate_record(F, record, *, migration_id=None):
    F.require(type(record) is dict and set(record) == RECORD_FIELDS, "pending_liability_record_shape_invalid")
    _require_hash(F, record["record_id"], "pending_liability_record_id_invalid")
    F.require(record["source_table"] in TABLE_KINDS, "pending_liability_source_table_invalid")
    _row_id(F, record["source_row_id"], "pending_liability_source_id_invalid")
    F.integer(record["source_row_ordinal"], "pending_liability_source_ordinal_invalid", minimum=0)
    _require_hash(F, record["source_row_sha256"], "pending_liability_source_row_hash_invalid")
    _row_id(F, record["source_user_id"], "pending_liability_user_id_invalid")
    if record["target_user_id"] is not None:
        _row_id(F, record["target_user_id"], "pending_liability_target_user_invalid")
    if record["target_account_id"] is not None:
        _row_id(F, record["target_account_id"], "pending_liability_target_account_invalid")
    F.require(record["operation_kind"] == TABLE_KINDS[record["source_table"]], "pending_liability_kind_mismatch")
    F.require(record["legacy_status"] == "pending", "pending_liability_status_invalid")
    F.require(type(record["provider"]) is str and type(record["references"]) is dict
              and type(record["raw_amount"]) is dict and type(record["raw_currency"]) is dict
              and type(record["raw_row"]) is dict, "pending_liability_raw_fact_shape_invalid")
    F.require(record["settlement_state"] == SETTLEMENT_STATE, "pending_liability_settlement_state_invalid")
    F.require(record["spendable"] is False and type(record["spendable_units"]) is int
              and record["spendable_units"] == 0, "pending_liability_spendable_contract_invalid")
    F.require(record["disposition"] == DISPOSITION, "pending_liability_disposition_invalid")
    if migration_id is not None:
        expected = F.digest([migration_id, record["source_table"], record["source_row_id"], record["source_row_sha256"]])
        F.require(record["record_id"] == expected, "pending_liability_record_binding_mismatch")
    # The persisted JSON artifact uses the planner's typed display tags for
    # Decimal values.  Recomputing a source-row hash after a plain JSON load
    # would silently turn Decimal into binary float or a literal tag object.
    # Source-row hash rederivation therefore belongs to the frozen-export
    # verifier, while this function verifies the preview hash and contract.


def _summary(F, records, *, source_archive_sha256, mapping_sha256, source_export_sha256, migration_id):
    by_table = Counter(row["source_table"] for row in records)
    by_provider = Counter(row["provider"] or "unknown" for row in records)
    return {
        "status": "PENDING_LIABILITIES_PRESERVED_SOURCE_ONLY", "record_count": len(records),
        "counts_by_table": dict(sorted(by_table.items())), "counts_by_provider": dict(sorted(by_provider.items())),
        "unmapped_owner_count": sum(row["target_user_id"] is None or row["target_account_id"] is None for row in records),
        "spendable_units_written": 0, "ledger_writes": 0, "target_writes": 0, "source_only": True,
        "settlement_state": SETTLEMENT_STATE, "disposition": DISPOSITION, "migration_id": migration_id,
        "source_archive_sha256": source_archive_sha256, "mapping_sha256": mapping_sha256,
        "source_export_sha256": source_export_sha256,
    }


def build(F, *, tables, identities, target_users=None, migration_id, observed_at,
          source_archive_sha256, mapping_sha256, source_export_sha256):
    """Build a deterministic, private preview. It never writes or converts money."""
    F.require(type(tables) is dict and type(identities) is dict, "pending_liability_input_shape_invalid")
    target_users = {} if target_users is None else target_users
    F.require(type(target_users) is dict, "pending_liability_target_users_shape_invalid")
    F.require(type(migration_id) is str and F.IDENTITY.MIGRATION_ID.fullmatch(migration_id) is not None,
              "pending_liability_migration_id_invalid")
    observed_at = F.integer(observed_at, "pending_liability_observation_invalid", minimum=1, maximum=253402300799)
    for name, value in (("source_archive_sha256", source_archive_sha256), ("mapping_sha256", mapping_sha256), ("source_export_sha256", source_export_sha256)):
        _require_hash(F, value, "pending_liability_" + name + "_invalid")
    records = []
    for table in sorted(TABLE_KINDS):
        rows = tables.get(table, [])
        F.require(type(rows) is list, "pending_liability_table_shape_invalid")
        seen = set()
        for ordinal, row in enumerate(rows):
            if row.get("status") != "pending":
                continue
            record = _source_row(F, table, ordinal, row, identities, target_users, migration_id)
            F.require(record["source_row_id"] not in seen, "pending_liability_source_id_duplicate")
            seen.add(record["source_row_id"])
            records.append(record)
    records.sort(key=lambda row: (row["source_table"], row["source_row_id"]))
    material = {
        "format_version": FORMAT_VERSION, "scope": SCOPE, "migration_id": migration_id, "observed_at": observed_at,
        "source_archive_sha256": source_archive_sha256, "mapping_sha256": mapping_sha256,
        "source_export_sha256": source_export_sha256,
        "policy": {"spendable": False, "spendable_units": 0, "settlement_state": SETTLEMENT_STATE,
                    "disposition": DISPOSITION, "ledger_entry_policy": "never"}, "records": records,
    }
    # Persist the same typed JSON artifact that the private file carries. In
    # particular, Decimal values become explicit {$decimal: ...} objects before
    # the preview hash is computed, so plain-JSON readback keeps the same hash.
    material = json.loads(F.canonical(material))
    material["preview_sha256"] = F.digest(material)
    material["public_summary"] = _summary(F, material["records"], source_archive_sha256=source_archive_sha256,
                                           mapping_sha256=mapping_sha256, source_export_sha256=source_export_sha256,
                                           migration_id=migration_id)
    return material


def verify(F, preview):
    """Verify preview integrity and the no-spend contract."""
    F.require(type(preview) is dict and set(preview) == {
        "format_version", "scope", "migration_id", "observed_at", "source_archive_sha256", "mapping_sha256",
        "source_export_sha256", "policy", "records", "preview_sha256", "public_summary",
    }, "pending_liability_preview_shape_invalid")
    F.require(preview["format_version"] == FORMAT_VERSION and preview["scope"] == SCOPE,
              "pending_liability_preview_profile_invalid")
    material = {key: value for key, value in preview.items() if key not in ("preview_sha256", "public_summary")}
    _require_hash(F, preview["preview_sha256"], "pending_liability_preview_hash_invalid")
    F.require(F.digest(material) == preview["preview_sha256"], "pending_liability_preview_hash_mismatch")
    F.require(type(preview["records"]) is list, "pending_liability_records_invalid")
    seen = set()
    for row in preview["records"]:
        _validate_record(F, row, migration_id=preview["migration_id"])
        F.require(row["record_id"] not in seen, "pending_liability_record_duplicate")
        seen.add(row["record_id"])
    F.require(preview["policy"] == {
        "spendable": False, "spendable_units": 0, "settlement_state": SETTLEMENT_STATE,
        "disposition": DISPOSITION, "ledger_entry_policy": "never",
    }, "pending_liability_policy_invalid")
    expected_summary = _summary(F, preview["records"], source_archive_sha256=preview["source_archive_sha256"],
                                mapping_sha256=preview["mapping_sha256"], source_export_sha256=preview["source_export_sha256"],
                                migration_id=preview["migration_id"])
    F.require(preview["public_summary"] == expected_summary, "pending_liability_summary_mismatch")
    return True


def build_import_request(F, preview, *, target_schema_contract_sha256):
    """Create a handoff to a future owner-only writer, never execute it."""
    verify(F, preview)
    _require_hash(F, target_schema_contract_sha256, "pending_liability_target_contract_hash_invalid")
    return {
        "format_version": FORMAT_VERSION, "scope": IMPORT_SCOPE,
        "decision": "pending-target-schema-owner-approval", "migration_id": preview["migration_id"],
        "preview_sha256": preview["preview_sha256"], "target_schema_contract_sha256": target_schema_contract_sha256,
        "records_sha256": F.digest(preview["records"]), "record_count": len(preview["records"]),
        "target_writes": 0, "ledger_writes": 0, "spendable_units": 0,
        "provider_io": "outside-target-transaction", "applied": False,
        "requires_owner_writer": True, "requires_provider_reconciliation": True,
    }


def _sql_literal(F, value):
    return F.IDENTITY.sql_literal(F.canonical(value))


def _sql_text_literal(F, value):
    """Quote plain SQL text without JSON string encoding."""
    F.require(type(value) is str, "pending_liability_sql_text_invalid")
    return F.IDENTITY.sql_literal(value)


def _owner_request(F, preview, row):
    F.require(row["target_user_id"] is not None and row["target_account_id"] is not None,
              "pending_liability_owner_mapping_incomplete")
    source_key = "id:" + str(row["source_row_id"])
    payload = {
        "record_id": row["record_id"], "operation_kind": row["operation_kind"],
        "provider": row["provider"], "references": row["references"],
        "legacy_status": row["legacy_status"], "source_row_ordinal": row["source_row_ordinal"],
        "raw_row": row["raw_row"], "disposition": row["disposition"],
    }
    request_meta = {
        "preview_sha256": preview["preview_sha256"], "record_id": row["record_id"],
        "source_row_key": source_key, "disposition": DISPOSITION,
    }
    return {
        "amount_raw_json": row["raw_amount"], "currency_raw_json": row["raw_currency"],
        "mapping_sha256": preview["mapping_sha256"], "migration_id": preview["migration_id"],
        "observed_at": preview["observed_at"], "owner_account_id": row["target_account_id"],
        "owner_user_id": row["target_user_id"], "payload_json": payload,
        "request": request_meta, "result": {"status": "imported", "spendable": False, "spendable_units": 0},
        "settlement_state": SETTLEMENT_STATE, "source_archive_sha256": preview["source_archive_sha256"],
        "source_parent_plan_sha256": None, "source_row_key": source_key,
        "source_row_sha256": row["source_row_sha256"], "source_table": row["source_table"],
        "spendable": False, "spendable_units": 0, "status": "pending",
    }


def build_owner_apply_sql(F, preview):
    """Generate the owner-only, source-liability import transaction.

    The generated SQL calls only the dedicated Core function. It does not call
    opening, escrow, ledger, balance, or provider APIs.  The target schema
    function enforces owner identity and serializable durable commit.
    """
    verify(F, preview)
    requests = [_owner_request(F, preview, row) for row in preview["records"]]
    lines = [
        "BEGIN;",
        "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE;",
        "SET LOCAL synchronous_commit='on';",
        "SELECT pg_advisory_xact_lock(741803122);",
    ]
    lines.extend(
        "SELECT core_billing.import_legacy_pending_liability(" + _sql_literal(F, request) + "::jsonb);"
        for request in requests
    )
    lines.append("COMMIT;")
    return "\n".join(lines) + "\n"


def build_readback_sql(F, preview):
    """Generate a read-only aggregate readback for the dedicated table."""
    verify(F, preview)
    # The request payloads are JSONB and therefore use _sql_literal.  The
    # migration predicate is a PostgreSQL TEXT comparison and must not contain
    # JSON's surrounding double quotes.
    migration = _sql_text_literal(F, preview["migration_id"])
    return f"""BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL timezone='UTC';
SELECT jsonb_build_object(
  'status','READBACK',
  'migration_id',{migration},
  'row_count',(SELECT count(*) FROM core_billing.legacy_pending_liabilities WHERE migration_id={migration}),
  'source_rows_sha256',(SELECT encode(sha256(convert_to(coalesce(string_agg(
      jsonb_build_array(source_table,source_row_key,source_row_sha256,owner_user_id,owner_account_id,status,settlement_state,spendable,spendable_units)::text,
      '' ORDER BY source_table,source_row_key),'') ,'UTF8')),'hex')
      FROM core_billing.legacy_pending_liabilities WHERE migration_id={migration}),
  'spendable_true_count',(SELECT count(*) FROM core_billing.legacy_pending_liabilities WHERE migration_id={migration} AND spendable),
  'spendable_units_sum',(SELECT coalesce(sum(spendable_units),0) FROM core_billing.legacy_pending_liabilities WHERE migration_id={migration}),
  'non_unknown_state_count',(SELECT count(*) FROM core_billing.legacy_pending_liabilities WHERE migration_id={migration} AND settlement_state <> 'unknown')
);
COMMIT;
"""


def replay_status(F, original, candidate):
    """Return a deterministic replay result without touching a target."""
    verify(F, original)
    verify(F, candidate)
    return "replay-noop" if original["preview_sha256"] == candidate["preview_sha256"] else "conflict-material-mismatch"
