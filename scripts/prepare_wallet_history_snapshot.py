#!/usr/bin/env python3
"""Prepare an offline, immutable top-up history import; never connect to a DB."""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from decimal import Decimal, InvalidOperation, localcontext
import hashlib
import json
from pathlib import Path
import re
import sys

SCOPE = "wallet-topup-history.v1"
NAMESPACE = "lmm_api.lmm_prod_20260802.top_ups"
MAP_FIELDS = {"source_user_id", "target_user_id", "target_personal_account_id"}
INTEGER_FIELDS = ("id", "user_id", "amount", "create_time", "complete_time",
                  "credited_quota", "platform_amount_micros")
REQUIRED_FIELDS = set(INTEGER_FIELDS) | {
    "money_text", "trade_no", "payment_method", "status", "settlement_currency"}
I64_MAX = 2**63 - 1


def require(condition, code):
    if not condition:
        raise ValueError(code)


def canonical(value):
    return json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":"))


def digest(value):
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def file_digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def integer(value, *, positive=False):
    return type(value) is int and (-I64_MAX - 1 <= value <= I64_MAX) and (
        not positive or value > 0)


def decimal_text(value):
    require(isinstance(value, str) and len(value) <= 128 and
            re.fullmatch(r"-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?", value),
            "legacy_money_text_invalid")
    try:
        number = Decimal(value)
    except InvalidOperation as error:
        raise ValueError("legacy_money_text_invalid") from error
    require(number.is_finite() and -1000 <= number.adjusted() <= 1000,
            "legacy_money_out_of_range")
    return number


def prepare(export, mapping, *, expected_rows, archive_sha256, export_sha256):
    # @context-fold begin wallet-history-source-validation
    require(type(export) is dict and export.get("source_namespace") == NAMESPACE,
            "source_namespace_mismatch")
    require(integer(expected_rows) and expected_rows >= 0, "expected_count_invalid")
    require(integer(export.get("observed_at"), positive=True), "source_snapshot_time_invalid")
    rows, users = export.get("rows"), export.get("source_user_ids")
    require(type(rows) is list and type(users) is list, "source_export_shape_invalid")
    require(integer(export.get("source_row_count")) and export["source_row_count"] == expected_rows,
            "source_expected_count_mismatch")
    require(len(rows) == expected_rows, "export_row_count_mismatch")
    require(all(integer(uid, positive=True) for uid in users) and len(users) == len(set(users)),
            "source_users_invalid")
    require(type(mapping) is list, "identity_map_invalid")
    seen = {field: set() for field in MAP_FIELDS}
    for entry in mapping:
        require(type(entry) is dict and set(entry) == MAP_FIELDS, "identity_map_shape_invalid")
        for field in MAP_FIELDS:
            require(integer(entry[field], positive=True) and entry[field] not in seen[field],
                    "identity_map_duplicate_or_invalid")
            seen[field].add(entry[field])
    by_source = {entry["source_user_id"]: entry for entry in mapping}
    missing_users = sorted(set(users) - set(by_source))
    require(not missing_users, "identity_map_missing_source_users:" + ",".join(map(str, missing_users)))
    selected = sorted((by_source[uid] for uid in users), key=lambda x: x["source_user_id"])
    counts = {str(uid): 0 for uid in users}
    ids, records, statuses = set(), [], Counter()
    money_by_currency = defaultdict(Decimal)
    for row in rows:
        require(type(row) is dict and set(row) == REQUIRED_FIELDS, "source_order_shape_invalid")
        require(all(integer(row[field]) for field in INTEGER_FIELDS), "source_order_integer_invalid")
        require(row["id"] > 0 and row["id"] not in ids, "source_order_id_duplicate_or_invalid")
        ids.add(row["id"])
        require(row["user_id"] in by_source and row["user_id"] in users,
                "source_order_user_unmapped")
        require(row["status"] in ("pending", "success", "failed", "expired"),
                "source_order_status_unknown")
        require(all(isinstance(row[field], str) for field in ("trade_no", "payment_method")),
                "source_order_text_invalid")
        require(row["settlement_currency"] is None or isinstance(row["settlement_currency"], str),
                "source_order_currency_invalid")
        require(0 <= row["create_time"] <= export["observed_at"] and row["complete_time"] >= 0,
                "source_order_time_invalid")
        require(row["complete_time"] <= export["observed_at"], "source_order_after_snapshot")
        money = decimal_text(row["money_text"])
        owner = by_source[row["user_id"]]
        record = {field: row[field] for field in INTEGER_FIELDS}
        record["user_id"] = owner["target_user_id"]
        record.update(trade_no=row["trade_no"], payment_method=row["payment_method"],
                      status=row["status"], amount_unit="CREDIT" if row["payment_method"] == "creem" else "LEGACY")
        # Empty/unknown currencies stay empty/unknown; no USD conversion or inference.
        if row["settlement_currency"] is not None:
            record["settlement_currency"] = row["settlement_currency"]
            if row["settlement_currency"]:
                record["currency"] = row["settlement_currency"]
        records.append({**owner, "source_row_sha256": digest(row),
                        "money_text": row["money_text"], "record": record})
        counts[str(row["user_id"])] += 1
        statuses[row["status"]] += 1
        currency = row["settlement_currency"] or "<unknown>"
        with localcontext() as context:
            context.prec = 4096
            money_by_currency[currency] = money_by_currency[currency] + money
    manifest = {
        "scope": SCOPE, "source_namespace": NAMESPACE,
        "observed_at": export["observed_at"], "source_row_count": expected_rows,
        "source_archive_sha256": archive_sha256, "source_export_sha256": export_sha256,
        "mapping_sha256": digest(selected), "complete": True, "identity_map": selected,
        "per_source_user_counts": counts, "counts_by_status": dict(statuses),
        "legacy_money_sums_by_currency": {key: str(value) for key, value in sorted(money_by_currency.items())},
        "rows": sorted(records, key=lambda row: row["record"]["id"]),
    }
    manifest["migration_id"] = "wallet-topup-history-v1-" + hashlib.sha256(NAMESPACE.encode()).hexdigest()[:32]
    return manifest
    # @context-fold end wallet-history-source-validation


def literal(value):
    # Set standard_conforming_strings in the generated SQL before using literals.
    text = canonical(value) if not isinstance(value, str) else value
    return "'" + text.replace("'", "''") + "'"


def verification_sql(manifest):
    mid = literal(manifest["migration_id"])
    return f"""SELECT jsonb_build_object(
 'migration_id',r.migration_id,'source_namespace',r.manifest->>'source_namespace',
 'source_rows',(r.manifest->>'source_row_count')::bigint,
 'imported_rows',jsonb_array_length(r.manifest->'rows'),
 'missing_rows',(r.manifest->>'source_row_count')::bigint-jsonb_array_length(r.manifest->'rows'),
 'counts_by_status',r.manifest->'counts_by_status',
 'legacy_money_sums_by_currency',r.manifest->'legacy_money_sums_by_currency',
 'legacy_credit_log_records',(SELECT count(*) FROM core_history.log_records WHERE log_type=1),
 'native_payment_intents',(SELECT count(*) FROM core_payments.intents),
 'log_counts_are_not_order_counts',true,
 'exact_manifest_match',r.manifest={literal(manifest)}::jsonb)
 FROM core_migration.import_runs r WHERE r.migration_id={mid};"""


def import_sql(manifest):
    # @context-fold begin wallet-history-append-only-import
    return f"""BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL standard_conforming_strings=on;
SET LOCAL lock_timeout='5s';
SET LOCAL statement_timeout='60s';
SELECT pg_advisory_xact_lock(741803120);
DO $wallet_history$
DECLARE m jsonb := {literal(manifest)}::jsonb; previous core_migration.import_runs%ROWTYPE;
BEGIN
 IF current_user<>session_user OR NOT EXISTS(
  SELECT 1 FROM pg_class WHERE oid='core_migration.import_runs'::regclass
  AND pg_get_userbyid(relowner)=current_user)
 THEN RAISE EXCEPTION 'wallet_history_installation_owner_required'; END IF;
 IF jsonb_array_length(m->'rows')<>(m->>'source_row_count')::bigint
 THEN RAISE EXCEPTION 'wallet_history_source_count_mismatch'; END IF;
 IF EXISTS(SELECT 1 FROM jsonb_to_recordset(m->'identity_map')
   AS x(source_user_id bigint,target_user_id bigint,target_personal_account_id bigint)
   LEFT JOIN core_identity.users u ON u.id=x.target_user_id
   LEFT JOIN core_identity.accounts a ON a.id=u.personal_account_id
   WHERE u.id IS NULL OR u.personal_account_id<>x.target_personal_account_id OR a.kind<>'personal')
 THEN RAISE EXCEPTION 'wallet_history_target_identity_mismatch'; END IF;
 IF EXISTS(SELECT 1 FROM core_migration.import_runs
  WHERE manifest->>'scope'={literal(SCOPE)} AND migration_id<>m->>'migration_id')
 THEN RAISE EXCEPTION 'wallet_history_other_source_already_installed'; END IF;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.manifest IS DISTINCT FROM m
   OR previous.source_archive_sha256 IS DISTINCT FROM m->>'source_archive_sha256'
   OR previous.mapping_sha256 IS DISTINCT FROM m->>'mapping_sha256'
  THEN RAISE EXCEPTION 'wallet_history_import_conflict'; END IF;
 ELSE
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
  VALUES(m->>'migration_id',m->>'source_archive_sha256',m->>'mapping_sha256',m);
 END IF;
 IF NOT EXISTS(SELECT 1 FROM core_migration.import_runs
  WHERE migration_id=m->>'migration_id' AND manifest=m)
 THEN RAISE EXCEPTION 'wallet_history_exact_readback_failed'; END IF;
END $wallet_history$;
{verification_sql(manifest)}
COMMIT;
"""
    # @context-fold end wallet-history-append-only-import


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-export", required=True, type=Path)
    parser.add_argument("--identity-map", required=True, type=Path)
    parser.add_argument("--source-archive", required=True, type=Path)
    parser.add_argument("--expected-rows", required=True, type=int)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    # A new private directory prevents stale apply.sql surviving a failed preparation.
    args.output.mkdir(mode=0o700)
    try:
        export = json.loads(args.source_export.read_text())
        mapping = json.loads(args.identity_map.read_text())
        manifest = prepare(export, mapping, expected_rows=args.expected_rows,
                           archive_sha256=file_digest(args.source_archive),
                           export_sha256=file_digest(args.source_export))
        files = {
            "manifest.json": canonical(manifest) + "\n",
            "apply.sql": import_sql(manifest),
            "verify.sql": "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\nSET LOCAL standard_conforming_strings=on;\n"
                          + verification_sql(manifest) + "\nROLLBACK;\n",
            "plan.json": canonical({
                "ready": True, "migration_id": manifest["migration_id"],
                "source_rows": manifest["source_row_count"], "imported_rows": len(manifest["rows"]),
                "missing_rows": 0, "mapped_users": len(manifest["identity_map"]),
                "source_archive_sha256": manifest["source_archive_sha256"],
                "source_export_sha256": manifest["source_export_sha256"],
                "identity_map_file_sha256": file_digest(args.identity_map),
                "mapping_sha256": manifest["mapping_sha256"],
                "counts_by_status": manifest["counts_by_status"],
                "legacy_money_sums_by_currency": manifest["legacy_money_sums_by_currency"],
                "writes": ["core_migration.import_runs"], "money_written": False,
            }) + "\n",
        }
        for name, body in files.items():
            path = args.output / name
            with path.open("x", encoding="utf-8") as output:
                path.chmod(0o600)
                output.write(body)
    except (ValueError, OSError, KeyError, TypeError) as error:
        (args.output / "blocked.json").write_text(canonical({"ready": False, "reason": str(error)}) + "\n")
        print("wallet_history_prepare_blocked; see blocked.json", file=sys.stderr)
        return 2
    print(canonical({"ready": True, "source_rows": len(manifest["rows"]), "money_written": False}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
