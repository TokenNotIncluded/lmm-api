#!/usr/bin/env python3
"""Observe reviewed local target wallet buckets in one read-only MVCC snapshot.

This produces target facts, never source-derived balances or economic approval.
Incomplete observations are retained but no consumable wallet inventory is emitted.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

_HELPER_DIRECTORY = Path(importlib.util.find_spec("financial_import").origin).resolve().parent
_MODULE_INPUTS = {}
for _path in [Path(__file__).resolve(), *(_HELPER_DIRECTORY / name for name in (
        "financial_import.py", "legacy_financial_preflight.py", "migrate-legacy-financial.py",
        "migrate-legacy-snapshot.py"))]:
    _raw = _path.read_bytes()
    _MODULE_INPUTS[_path.name] = {"path": _path, "raw": _raw, "sha256": hashlib.sha256(_raw).hexdigest()}

import financial_import as F

H = F.IDENTITY
UNIT = "credit_500k_usd"
FIELDS = {"source_user_id", "target_user_id", "target_personal_account_id"}
BUCKETS = ("wallet", "debt", "affiliate")
MAX_ID = 2**63 - 1


def require(condition, code):
    H.require(condition, code)


def exact_integer(value, *, minimum=1, maximum=MAX_ID):
    return type(value) is int and minimum <= value <= maximum


# @mark migration.financial-target-observation role=implementation
def validate_mapping(rows, expected_count):
    require(type(rows) is list and exact_integer(expected_count), "observation_mapping_invalid")
    require(len(rows) == expected_count, "observation_mapping_count_mismatch")
    seen = {field: set() for field in FIELDS}
    for row in rows:
        require(type(row) is dict and set(row) == FIELDS, "observation_mapping_shape_invalid")
        for field in FIELDS:
            value = row[field]
            require(exact_integer(value), "observation_mapping_integer_invalid")
            require(value not in seen[field], "observation_mapping_duplicate")
            seen[field].add(value)
    return sorted(rows, key=lambda row: row["source_user_id"])


# @mark migration.financial-target-observation role=implementation
def build_sql(mapping, manifest, migration_id, *, contract=None):
    """All interpolated values are reviewed JSON/quoted literals, never SQL."""
    contract = H.current_contract() if contract is None else contract
    return f"""BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL timezone='UTC';
SET LOCAL statement_timeout='30s';
SET LOCAL lock_timeout='5s';
WITH mapping AS (
 SELECT * FROM jsonb_to_recordset({F.literal(mapping)}::jsonb)
 AS m(source_user_id bigint,target_user_id bigint,target_personal_account_id bigint)
), observed AS (
 SELECT m.source_user_id,m.target_user_id,m.target_personal_account_id,
 u.personal_account_id AS account_id,ac.kind AS account_kind,
 coalesce(jsonb_agg(jsonb_build_object('ledger_account_id',a.id,'bucket',a.bucket,
 'unit',a.unit,'balance_units',b.balance_units,'revision',b.revision)
 ORDER BY a.bucket,a.id) FILTER (WHERE a.id IS NOT NULL),'[]'::jsonb) AS buckets
 FROM mapping m LEFT JOIN core_identity.users u ON u.id=m.target_user_id
 LEFT JOIN core_identity.accounts ac ON ac.id=u.personal_account_id
 LEFT JOIN core_billing.ledger_accounts a ON a.owner_account_id=u.personal_account_id
 LEFT JOIN core_billing.balance_state b ON b.ledger_account_id=a.id
 GROUP BY m.source_user_id,m.target_user_id,m.target_personal_account_id,u.personal_account_id,ac.kind
)
SELECT jsonb_build_object(
 'instance',({F.instance_sql().removeprefix('SELECT ').rstrip(';')}),
 'read_only',current_setting('transaction_read_only'),
 'isolation',current_setting('transaction_isolation'),
 'snapshot',pg_current_snapshot()::text,'transaction_timestamp',transaction_timestamp(),
 'identity_contract',(SELECT jsonb_build_object('version',version,'fingerprint_sha256',encode(fingerprint,'hex')) FROM core_meta.schema_contract),
 'component_contracts',(SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract),
 'catalog_fingerprint_sha256',({H.catalog_hash_sql(contract)}),
 'identity_import_receipt_count',(SELECT count(*) FROM core_migration.import_runs WHERE migration_id={F.literal(migration_id)}),
 'identity_manifest_matches',(SELECT coalesce(bool_and(manifest={F.literal(manifest)}::jsonb),false) FROM core_migration.import_runs WHERE migration_id={F.literal(migration_id)}),
 'identity_import_receipts',(SELECT coalesce(jsonb_agg(jsonb_build_object('migration_id',migration_id,
 'manifest_sha256',encode(sha256(convert_to(manifest::text,'UTF8')),'hex')) ORDER BY migration_id),'[]'::jsonb)
 FROM core_migration.import_runs),
 'scope','reviewed-identity-map-personal-accounts-only',
 'observation_details',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY source_user_id),'[]'::jsonb) FROM observed o));
ROLLBACK;
"""


# @mark migration.financial-target-observation role=implementation
def classify_observation(observation, mapping):
    """Preserve missing buckets as missing; do not synthesize a zero balance."""
    require(type(observation) is dict, "observation_result_invalid")
    details = observation.get("observation_details")
    require(type(details) is list and len(details) == len(mapping), "observation_row_count_mismatch")
    expected = {row["source_user_id"]: row for row in mapping}
    seen, seen_ledger_ids, records, issues = set(), set(), [], []
    for row in details:
        require(type(row) is dict, "observation_row_invalid")
        source_id = row.get("source_user_id")
        require(exact_integer(source_id) and source_id in expected and source_id not in seen,
                "observation_source_identity_invalid")
        seen.add(source_id)
        mapped = expected[source_id]
        require(all(type(row.get(key)) is int and row[key] == mapped[key] for key in FIELDS),
                "observation_mapping_identity_mismatch")
        owner_ok = (type(row.get("account_id")) is int
                    and row["account_id"] == mapped["target_personal_account_id"]
                    and row.get("account_kind") == "personal")
        if not owner_ok:
            issues.append({"source_user_id": source_id, "code": "target_personal_owner_missing_or_mismatch"})
        buckets = row.get("buckets")
        require(type(buckets) is list, "observation_buckets_invalid")
        by_bucket = {}
        for bucket in buckets:
            require(type(bucket) is dict and set(bucket) == {
                "ledger_account_id", "bucket", "unit", "balance_units", "revision"},
                "observation_bucket_shape_invalid")
            name = bucket["bucket"]
            require(name in (*BUCKETS, "reserved"), "observation_bucket_unknown")
            require(exact_integer(bucket["ledger_account_id"])
                    and bucket["ledger_account_id"] not in seen_ledger_ids,
                    "observation_ledger_identity_duplicate_or_invalid")
            seen_ledger_ids.add(bucket["ledger_account_id"])
            require(name not in by_bucket, "observation_bucket_duplicate")
            by_bucket[name] = bucket
            if (bucket["unit"] != UNIT
                    or not exact_integer(bucket["balance_units"], minimum=-MAX_ID-1)
                    or not exact_integer(bucket["revision"], minimum=0)):
                issues.append({"source_user_id": source_id, "bucket": name,
                               "code": "target_bucket_unit_or_balance_missing_or_invalid"})
        for name in BUCKETS:
            if name not in by_bucket:
                issues.append({"source_user_id": source_id, "bucket": name, "code": "target_bucket_missing"})
        complete = owner_ok and all(name in by_bucket
            and by_bucket[name]["unit"] == UNIT
            and exact_integer(by_bucket[name]["balance_units"], minimum=-MAX_ID-1)
            and exact_integer(by_bucket[name]["revision"], minimum=0) for name in BUCKETS)
        if complete:
            wallet, debt, affiliate = (by_bucket[name]["balance_units"] for name in BUCKETS)
            if wallet < 0 or debt > 0:
                issues.append({"source_user_id": source_id, "code": "target_bucket_sign_invalid"})
            else:
                records.append({"source_user_id": source_id, "account_id": row["account_id"],
                    "wallet_units": wallet, "debt_units_signed": debt, "affiliate_units": affiliate})
    passed = not issues and len(records) == len(mapping)
    return {"status": "COMPLETE_SCOPED_OBSERVATION" if passed else "INCOMPLETE_TARGET_OBSERVATION",
        "target_inventory_provenance_verified": False,
        "target_inventory_scope_completeness_verified": passed,
        "financial_migration_or_economic_approval": False,
        "issues": issues,
        "inventory": {"format_version": 1, "scope": "legacy-wallet-positions", "unit": UNIT,
                      "records": sorted(records, key=lambda row: row["source_user_id"])} if passed else None}


def parse_json(text):
    """Reject duplicate JSON keys rather than silently choosing one value."""
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "observation_json_duplicate_key")
            result[key] = value
        return result
    return json.loads(text, object_pairs_hook=pairs)


def freeze_input(path):
    checked = H.private_backup_path(path)
    raw = checked.read_bytes()
    return {"path": checked, "raw": raw, "sha256": hashlib.sha256(raw).hexdigest()}


def verify_frozen_inputs(inputs, *, private=True):
    for item in inputs.values():
        checked = H.private_backup_path(item["path"]) if private else H.owned_path(item["path"], private=False)
        require(hashlib.sha256(checked.read_bytes()).hexdigest() == item["sha256"],
                "observation_input_changed_during_export")


def export(args):
    verify_frozen_inputs(_MODULE_INPUTS, private=False)
    inputs = {"target_receipt": freeze_input(args.target_receipt),
              "identity_manifest": freeze_input(args.identity_manifest),
              "identity_map": freeze_input(args.identity_map)}
    target = parse_json(inputs["target_receipt"]["raw"])
    manifest = parse_json(inputs["identity_manifest"]["raw"])
    require(type(target) is dict and type(manifest) is dict, "observation_receipt_shape_invalid")
    require(target.get("initialized_by") == "core-init-db", "observation_target_initializer_unverified")
    require(manifest.get("scope") == "identity-only", "observation_identity_manifest_scope_invalid")
    F.checked_hash(manifest.get("source_archive_sha256"), "observation_source_archive_hash_invalid")
    require(manifest.get("source_archive_sha256") == target.get("source_archive_sha256"),
            "observation_source_archive_binding_mismatch")
    require(type(manifest.get("summary")) is dict, "observation_identity_manifest_summary_invalid")
    require(inputs["identity_map"]["sha256"] == manifest.get("source_target_id_map_sha256"),
            "observation_mapping_file_hash_mismatch")
    mapping = validate_mapping([parse_json(line) for line in inputs["identity_map"]["raw"].splitlines() if line.strip()],
                               manifest.get("summary", {}).get("mapped_user_count"))
    contract = H.current_contract()
    schema_sources = {}
    for name, expected in contract["source_file_hashes"].items():
        path = H.owned_path(H.ROOT / name, private=False)
        raw = path.read_bytes()
        require(hashlib.sha256(raw).hexdigest() == expected, "observation_schema_source_changed")
        schema_sources[name] = {"path": path, "raw": raw, "sha256": expected}
    schema = target.get("schema_manifest", {})
    require(type(schema) is dict, "observation_schema_manifest_receipt_invalid")
    inputs["schema_manifest"] = freeze_input(Path(schema.get("path", "")))
    require(inputs["schema_manifest"]["sha256"] == schema.get("sha256"), "observation_schema_manifest_hash_mismatch")
    # Match the public compiled schema-manifest contract without executing a
    # target-supplied binary or importing test code into this exporter.
    expected_manifest = {"format_version": 1, "scope": "compiled-core-schema-contract",
        "components": [{"name": name, "raw_schema_sha256": value}
                       for name, value in contract["component_fingerprints"].items()],
        "identity": {"version": contract["identity_version"],
                     "fingerprint_sha256": contract["identity_fingerprint_sha256"]},
        "installer_contract_sql_sha256": contract["installer_contract_sha256"],
        "catalog": {"query_utf8": contract["catalog_query"], "query_sha256": contract["catalog_query_sha256"],
            "query_hash_domain": "sha256-raw-utf8-v1",
            "fingerprint_hash_domain": "sha256-u64be-byte-length-prefixed-utf8-v1",
            "definition_order": "postgresql-c-collation"}}
    require(parse_json(inputs["schema_manifest"]["raw"]) == expected_manifest, "observation_compiled_schema_contract_mismatch")
    connection = target.get("connection", {})
    require(type(connection) is dict and type(target.get("contract")) is dict,
            "observation_target_receipt_contract_invalid")
    require(type(target.get("migration_id")) is str and 1 <= len(target["migration_id"]) <= 128,
            "observation_migration_id_invalid")
    inputs["pgpass_file"] = freeze_input(Path(connection.get("pgpass_file", "")))
    output = args.output
    require(output.is_absolute() and output.parent.resolve().is_relative_to(H.BACKUP_ROOT.resolve()),
            "observation_output_outside_backups")
    if output.parent.resolve() == H.BACKUP_ROOT.resolve():
        H.owned_path(output.parent, private=True, directory=True)
    else:
        H.private_backup_path(output.parent, directory=True)
    output.mkdir(mode=0o700)
    args._created_output = True
    H.private_backup_path(output, directory=True)
    try:
        H.socket_connection(connection)
    except OSError:
        require(False, "observation_local_target_unavailable")
    H.connection_roles(connection)
    verify_frozen_inputs(inputs)
    verify_frozen_inputs(_MODULE_INPUTS, private=False)
    verify_frozen_inputs(schema_sources, private=False)
    for name, item in inputs.items():
        H.write_private(output, "frozen-"+name+".private.input", item["raw"].decode("utf-8"))
    for name, item in _MODULE_INPUTS.items():
        H.write_private(output, "frozen-module-"+name+".private.input", item["raw"].decode("utf-8"))
    # The client consumes the frozen password file, not a path that could be
    # replaced while the observation is in flight. All copies stay private.
    frozen_connection = {**connection, "pgpass_file": str(output / "frozen-pgpass_file.private.input")}
    sql = build_sql(mapping, manifest, target.get("migration_id"), contract=contract)
    H.write_private(output, "observation-query.private.sql", sql)
    raw = H.psql(frozen_connection, sql, readonly=True, private_dir=output, label="target-wallet-observation")
    H.write_private(output, "observation-raw.private.json", raw)
    observation = parse_json(raw)
    H.verify_instance(observation["instance"], connection, target.get("instance_identity", {}), source=False)
    require(observation.get("read_only") == "on" and observation.get("isolation") == "repeatable read"
            and observation.get("snapshot"), "observation_readonly_snapshot_unverified")
    require(observation.get("identity_contract") == {
        "version": contract["identity_version"], "fingerprint_sha256": contract["identity_fingerprint_sha256"]}
        and observation.get("component_contracts") == contract["component_fingerprints"]
        and observation.get("catalog_fingerprint_sha256") == target.get("contract", {}).get("catalog_fingerprint_sha256"),
            "observation_actual_schema_contract_mismatch")
    require(observation.get("identity_import_receipt_count") == 1
            and observation.get("identity_manifest_matches") is True, "observation_identity_import_receipt_mismatch")
    result = classify_observation(observation, mapping)
    verify_frozen_inputs(inputs)
    verify_frozen_inputs(_MODULE_INPUTS, private=False)
    verify_frozen_inputs(schema_sources, private=False)
    result["target_inventory_provenance_verified"] = True
    result["bindings"] = {**{name+"_sha256": item["sha256"] for name, item in inputs.items()},
        "query_sha256": hashlib.sha256(sql.encode()).hexdigest(),
        "source_archive_sha256": manifest["source_archive_sha256"], "snapshot": observation["snapshot"],
        "exporter_sha256": _MODULE_INPUTS[Path(__file__).name]["sha256"],
        "module_source_hashes": {name: item["sha256"] for name, item in _MODULE_INPUTS.items()},
        "schema_source_hashes": {name: item["sha256"] for name, item in schema_sources.items()}}
    H.write_private(output, "observation.private.json", F.canonical(result)+"\n")
    if result["inventory"] is not None:
        H.write_private(output, "target-wallet-inventory.private.json", F.canonical(result["inventory"])+"\n")
    print(json.dumps({"status": result["status"], "mapped_owners": len(mapping),
        "issues": len(result["issues"]), "inventory_emitted": result["inventory"] is not None}))
    return 0 if result["inventory"] is not None else 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("target-receipt", "identity-manifest", "identity-map", "output"):
        parser.add_argument("--"+option, type=Path, required=True)
    args = parser.parse_args()
    args._created_output = False
    try:
        return export(args)
    except (H.MappingError, OSError, ValueError, KeyError, TypeError) as error:
        # Database diagnostics remain private; do not echo values, SQL or DSNs.
        code = str(error) if isinstance(error, H.MappingError) else "observation_export_failed"
        if args._created_output:
            H.write_private(args.output, "refusal.private.json", F.canonical({
                "status": "REFUSED", "code": code, "inventory_emitted": False,
                "exporter_sha256": _MODULE_INPUTS[Path(__file__).name]["sha256"]})+"\n")
        print(json.dumps({"status": "REFUSED", "code": code}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
