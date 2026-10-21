#!/usr/bin/env python3
"""Offline Key Quota & Opening Consumption Writer.

Consumes a separately reviewed Key Quota Import Plan and target snapshot.
Executes an atomic PostgreSQL transaction inserting lifetime key budgets
into core_charging.budgets, historical opening usage into
core_charging.budget_opening_consumption, and an immutable receipt into
core_migration.import_runs.

No wallet funds, balances, credit transfers, or model request journals are created.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import uuid

_SCRIPTS_DIR = str(Path(__file__).parent)
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

_SPEC = importlib.util.spec_from_file_location(
    "legacy_financial_planner", Path(__file__).with_name("migrate-legacy-financial.py")
)
PLANNER = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(PLANNER)
IDENTITY = PLANNER.IDENTITY
Error = IDENTITY.MappingError
require = IDENTITY.require
integer = PLANNER.integer

SCOPE = "key-quota-opening-only-v1"
DOMAIN = "lmm.offline-key-quota.v1"
FENCE = 741803121
TABLE_FILTER = "n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'"
CHANGED_TABLES = (
    "core_charging.budgets",
    "core_charging.budget_opening_consumption",
    "core_migration.import_runs",
)
CHANGED_SEQUENCES = (
    "core_charging.budgets_id_seq",
)
LIFETIME_START = -62135596800
LIFETIME_END = 253402300800
BIGINT_MIN = -9223372036854775808
BIGINT_MAX = 9223372036854775807


def canonical(value) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def digest(value) -> str:
    return PLANNER.digest(value)


def literal(value) -> str:
    value = value if type(value) is str else canonical(value)
    require("\0" not in value, "nul_in_sql_material")
    return IDENTITY.sql_literal(value)


def validate_key_quota_plan(plan: dict, *, verify_mapping_digest: bool = True) -> None:
    require(type(plan) is dict, "invalid_plan_shape")
    require(plan.get("format_version") == 1, "invalid_plan_format_version")
    require(plan.get("scope") == SCOPE, "invalid_plan_scope")
    require(type(plan.get("migration_id")) is str and IDENTITY.MIGRATION_ID.fullmatch(plan["migration_id"]),
            "invalid_plan_migration_id")
    require(type(plan.get("source_archive_sha256")) is str and IDENTITY.HEX.fullmatch(plan["source_archive_sha256"]),
            "invalid_source_archive_sha256")
    require(type(plan.get("mapping_sha256")) is str and IDENTITY.HEX.fullmatch(plan["mapping_sha256"]),
            "invalid_mapping_sha256")
    records = plan.get("key_records")
    require(type(records) is list and bool(records), "empty_key_records")
    if verify_mapping_digest:
        require(plan.get("mapping_sha256") == digest(records), "mapping_sha256_mismatch")
    seen_source_keys = set()
    seen_target_credentials = set()
    for row in records:
        require(type(row) is dict, "invalid_key_record_shape")
        for field in ("source_key_id", "target_credential_id", "target_user_id", "target_account_id", "used_units"):
            integer(row.get(field), f"invalid_field_{field}")
        require(row["used_units"] >= 0, "negative_key_used_counter")
        sid = row["source_key_id"]
        require(sid not in seen_source_keys, "duplicate_source_key_id")
        seen_source_keys.add(sid)
        cid = row["target_credential_id"]
        require(cid not in seen_target_credentials, "duplicate_target_credential_id")
        seen_target_credentials.add(cid)
        require(type(row.get("unlimited")) is bool, "invalid_unlimited_flag")
        integer(row.get("remaining_units"), "invalid_remaining_units")
        used = row["used_units"]
        remain = row["remaining_units"]
        basis = used + remain
        require(BIGINT_MIN <= basis <= BIGINT_MAX, "key_quota_basis_out_of_range")
        if not row["unlimited"]:
            require(0 <= basis <= BIGINT_MAX, "key_lifetime_limit_out_of_range")
        secret_hash = row.get("source_secret_sha256")
        require(type(secret_hash) is str and IDENTITY.HEX.fullmatch(secret_hash), "invalid_secret_sha256")
        if "source_row_sha256" in row:
            require(type(row["source_row_sha256"]) is str and IDENTITY.HEX.fullmatch(row["source_row_sha256"]),
                    "invalid_source_row_sha256")


def build_apply_sql(plan: dict, target: dict) -> str:
    """Construct an atomic PostgreSQL script executing the key quota import."""
    validate_key_quota_plan(plan, verify_mapping_digest=False)
    conn = target.get("connection", {})
    owner, login = IDENTITY.connection_roles(conn)
    ident = target.get("instance_identity", {})
    tc = target.get("contract", {})
    migration_id = plan["migration_id"]
    source_archive = plan["source_archive_sha256"]
    mapping_hash = plan["mapping_sha256"]

    manifest_json = canonical({
        "scope": SCOPE,
        "key_count": len(plan["key_records"]),
        "records_sha256": digest(plan["key_records"]),
    })

    # Pre-encode SQL insertion statements and credential integrity assertions
    budget_inserts = []
    opening_inserts = []
    credential_checks = []
    initial_checks = []
    applied_checks = []

    for row in plan["key_records"]:
        cid = row["target_credential_id"]
        uid = row["target_user_id"]
        aid = row["target_account_id"]
        used = row["used_units"]
        remain = row["remaining_units"]
        unlimited = row["unlimited"]
        secret_hash = row["source_secret_sha256"]
        basis = used + remain
        limit_credits = 0 if unlimited else basis
        display_limit_sql = str(basis) if unlimited else "NULL"
        unlimited_sql = "TRUE" if unlimited else "FALSE"
        import_ref = f"key-quota:{migration_id}:{cid}"

        credential_checks.append(f"""
 IF NOT EXISTS(
   SELECT 1 FROM core_identity.credentials
   WHERE id={cid}
     AND kind='api_key'
     AND user_id={uid}
     AND owner_account_id={aid}
     AND digest=decode({literal(secret_hash)}, 'hex')
 ) THEN
   RAISE EXCEPTION 'target credential % invalid, mismatched ownership, or secret digest mismatch', {cid};
 END IF;""")
        initial_checks.append(f"""
 IF EXISTS(SELECT 1 FROM core_charging.budgets WHERE scope='key' AND key_id={cid}) THEN
   RAISE EXCEPTION 'key budget already exists for credential %', {cid};
 END IF;
 IF EXISTS(SELECT 1 FROM core_charging.budget_opening_consumption WHERE scope='key' AND key_id={cid}) THEN
   RAISE EXCEPTION 'key opening already exists for credential %', {cid};
 END IF;""")
        applied_checks.append(f"""
 IF (SELECT count(*) FROM core_charging.budgets WHERE scope='key' AND key_id={cid})<>1
 OR NOT EXISTS(SELECT 1 FROM core_charging.budgets WHERE scope='key' AND key_id={cid}
   AND issuer_user_id={uid} AND account_id IS NULL AND user_id IS NULL
   AND period='lifetime' AND timezone='UTC' AND anchor=0 AND seconds=0 AND revision=1
   AND limit_credits={limit_credits} AND unlimited={unlimited_sql}
   AND display_limit_credits IS NOT DISTINCT FROM {display_limit_sql}::BIGINT)
 OR (SELECT count(*) FROM core_charging.budget_opening_consumption WHERE scope='key' AND key_id={cid})<>1
 OR NOT EXISTS(SELECT 1 FROM core_charging.budget_opening_consumption WHERE scope='key' AND key_id={cid}
   AND account_id IS NULL AND user_id IS NULL AND period='lifetime' AND anchor=0 AND seconds=0
   AND timezone='UTC' AND start_at={LIFETIME_START} AND end_at={LIFETIME_END}
   AND consumed_credits={used} AND import_reference={literal(import_ref)}) THEN
   RAISE EXCEPTION 'key quota applied row mismatch';
 END IF;""")

        budget_inserts.append(
            f"INSERT INTO core_charging.budgets(id,scope,key_id,issuer_user_id,period,limit_credits,unlimited,display_limit_credits) "
            f"OVERRIDING SYSTEM VALUE VALUES (v_next_budget_id,'key',{cid},{uid},'lifetime',{limit_credits},{unlimited_sql},{display_limit_sql});\n"
            f" v_next_budget_id := v_next_budget_id + 1;"
        )
        opening_inserts.append(
            f"INSERT INTO core_charging.budget_opening_consumption"
            f"(scope,key_id,period,start_at,end_at,consumed_credits,import_reference) "
            f"VALUES ('key',{cid},'lifetime',{LIFETIME_START},{LIFETIME_END},{used},{literal(import_ref)});"
        )

    all_cred_checks = "\n".join(credential_checks)
    all_initial_checks = "\n".join(initial_checks)
    all_applied_checks = "\n".join(applied_checks)
    key_ids = ",".join(str(r["target_credential_id"]) for r in plan["key_records"])
    budget_rows = f"(SELECT COALESCE(jsonb_agg(to_jsonb(b) ORDER BY id),'[]'::jsonb) FROM core_charging.budgets b WHERE scope='key' AND key_id IN ({key_ids}))"
    opening_rows = f"(SELECT COALESCE(jsonb_agg(to_jsonb(o) ORDER BY key_id),'[]'::jsonb) FROM core_charging.budget_opening_consumption o WHERE scope='key' AND key_id IN ({key_ids}))"
    all_budget_sql = "\n ".join(budget_inserts)
    all_opening_sql = "\n ".join(opening_inserts)

    catalog_hash_sql = IDENTITY.catalog_hash_sql(tc)
    contract_json_literal = literal(canonical(tc.get("component_fingerprints", {}))) + "::jsonb"

    script = f"""BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL statement_timeout = '120s';
SET LOCAL lock_timeout = '30s';
SELECT pg_advisory_xact_lock({FENCE});

DO $$
DECLARE
 table_row RECORD;
 actual_catalog TEXT;
 existing_run RECORD;
 target_count INT;
 v_seq_last_val BIGINT;
 v_seq_is_called BOOLEAN;
 v_next_budget_id BIGINT;
 applied_manifest JSONB;
BEGIN
 -- 1. Target Identity and Role Guard
 IF current_database()<>{literal(ident['database'])} OR current_user<>{literal(owner)} OR session_user<>{literal(login)}
  OR (SELECT rolcanlogin FROM pg_roles WHERE rolname=current_user) IS DISTINCT FROM false
  OR (SELECT rolcanlogin FROM pg_roles WHERE rolname=session_user) IS DISTINCT FROM true
  OR (SELECT rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls FROM pg_roles WHERE rolname=current_user)
  OR (SELECT rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls FROM pg_roles WHERE rolname=session_user)
  OR NOT pg_has_role(session_user,current_user,'SET')
  OR (SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database())<>current_user
  OR (SELECT oid::bigint FROM pg_database WHERE datname=current_database())<>{ident['database_oid']}
  OR (SELECT system_identifier::text FROM pg_control_system())<>{literal(ident['cluster_system_identifier'])}
  OR pg_postmaster_start_time()<>{literal(ident['postmaster_started_at'])}::timestamptz
  OR current_setting('server_version_num')<>{literal(str(ident['server_version_num']))}
  OR current_setting('port')::int<>{ident['port']}
  OR current_setting('lmm.migration_instance_nonce',true) IS DISTINCT FROM {literal(ident['server_instance_nonce'])}
  OR inet_server_addr() IS NOT NULL THEN
   RAISE EXCEPTION 'opening target identity mismatch';
 END IF;

 -- 2. Zero Active External Connections
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid()) THEN
   RAISE EXCEPTION 'opening target has another client';
 END IF;

 -- 3. Lock Relevant Tables
 LOCK TABLE core_charging.budgets IN ACCESS EXCLUSIVE MODE;
 LOCK TABLE core_charging.budget_opening_consumption IN ACCESS EXCLUSIVE MODE;
 LOCK TABLE core_migration.import_runs IN ACCESS EXCLUSIVE MODE;
 LOCK TABLE core_identity.credentials IN SHARE MODE;

 -- 4. Verify Catalog & Contract Hashes
 SELECT ({catalog_hash_sql}) INTO actual_catalog;
 IF actual_catalog<>{literal(target['contract']['catalog_fingerprint_sha256'])}
  OR NOT EXISTS(SELECT 1 FROM core_meta.runtime_contract WHERE singleton AND encode(catalog_fingerprint,'hex')=actual_catalog)
  OR NOT EXISTS(SELECT 1 FROM core_meta.schema_contract WHERE singleton AND version={tc['identity_version']} AND encode(fingerprint,'hex')={literal(tc['identity_fingerprint_sha256'])})
  OR (SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract) IS DISTINCT FROM {contract_json_literal} THEN
   RAISE EXCEPTION 'opening installed contract mismatch';
 END IF;

 -- 5. Check Idempotency & Replay
{all_cred_checks}
 SELECT * INTO existing_run FROM core_migration.import_runs WHERE migration_id={literal(migration_id)};
 IF FOUND THEN
   IF existing_run.source_archive_sha256 = {literal(source_archive)}
      AND existing_run.mapping_sha256 = {literal(mapping_hash)}
      AND existing_run.manifest - ARRAY['applied_budget_rows','applied_opening_rows'] = {literal(manifest_json)}::jsonb
      AND {literal(mapping_hash)} = {literal(digest(plan["key_records"]))} THEN
{all_applied_checks}
     IF existing_run.manifest->'applied_budget_rows' IS DISTINCT FROM {budget_rows}
       OR existing_run.manifest->'applied_opening_rows' IS DISTINCT FROM {opening_rows} THEN
       RAISE EXCEPTION 'key quota applied baseline mismatch';
     END IF;
     -- Exact replay: no-op only after real rows and their original IDs match.
     RETURN;
   ELSE
     RAISE EXCEPTION 'key_quota_import_conflict: migration_id already recorded with different material';
   END IF;
 END IF;

 IF {literal(mapping_hash)} <> {literal(digest(plan["key_records"]))} THEN
   RAISE EXCEPTION 'key_quota_import_conflict: mapping_sha256 mismatch with actual records';
 END IF;

 -- 6. Verify Target Credential Existence, Kind, Ownership, and Secret Digest
{all_initial_checks}

 -- 7. Allocate Transactional Budget IDs and Insert Budgets and Opening Consumptions
 SELECT last_value, is_called INTO v_seq_last_val, v_seq_is_called
 FROM core_charging.budgets_id_seq;

 IF v_seq_is_called THEN
   v_next_budget_id := v_seq_last_val + 1;
 ELSE
   v_next_budget_id := v_seq_last_val;
 END IF;

 {all_budget_sql}
 {all_opening_sql}
{all_applied_checks}
 applied_manifest := {literal(manifest_json)}::jsonb || jsonb_build_object(
   'applied_budget_rows',{budget_rows},'applied_opening_rows',{opening_rows});

 -- 8. Record Immutable Import Run
 INSERT INTO core_migration.import_runs(migration_id, source_archive_sha256, mapping_sha256, manifest)
 VALUES ({literal(migration_id)}, {literal(source_archive)}, {literal(mapping_hash)}, applied_manifest);

 -- 9. Transactionally Advance Sequence to Avoid Allocation Gaps or Collisions
 EXECUTE 'ALTER SEQUENCE core_charging.budgets_id_seq RESTART WITH ' || v_next_budget_id;

END $$;
SELECT jsonb_build_object(
 'status', 'succeeded',
 'migration_id', {literal(migration_id)},
 'keys_imported', {len(plan['key_records'])}
);
COMMIT;
"""
    return script


def build_plan_from_tokens(
    tokens: list[dict],
    user_map: dict[int, dict[str, int]],
    credential_receipt: dict,
    *,
    migration_id: str,
    source_archive_sha256: str,
) -> dict:
    """Build a validated KeyQuotaImportPlan from raw token rows and credential bindings."""
    require(type(user_map) is dict, "invalid_user_map")
    require(type(credential_receipt) is dict and credential_receipt.get("scope") == "a02-credential-mapping-receipt",
            "invalid_credential_receipt")
    require(credential_receipt.get("source_archive_sha256") == source_archive_sha256,
            "receipt_source_archive_mismatch")
    require(type(migration_id) is str and IDENTITY.MIGRATION_ID.fullmatch(migration_id),
            "invalid_migration_id")
    require(type(source_archive_sha256) is str and IDENTITY.HEX.fullmatch(source_archive_sha256),
            "invalid_source_archive_sha256")

    bindings = {}
    for r in credential_receipt.get("records", []):
        require(type(r) is dict, "invalid_receipt_record_shape")
        skid = integer(r.get("source_key_id"), "invalid_receipt_source_key_id", minimum=1)
        require(skid not in bindings, "duplicate_source_key_in_receipt")
        require(type(r.get("source_secret_sha256")) is str and IDENTITY.HEX.fullmatch(r["source_secret_sha256"]),
                "invalid_receipt_secret_sha256")
        bindings[skid] = r

    records = []
    seen_tokens = set()
    for token in tokens:
        tid = integer(token.get("id"), "invalid_token_id", minimum=1)
        require(tid not in seen_tokens, "duplicate_token_id")
        seen_tokens.add(tid)
        uid = integer(token.get("user_id"), "invalid_user_id", minimum=1)
        require(tid in bindings, f"missing_credential_binding_for_key_{tid}")
        binding = bindings[tid]
        require(uid in user_map, f"unmapped_user_{uid}")
        target_info = user_map[uid]
        require(target_info.get("user_id") == binding["target_user_id"], "binding_user_mismatch")
        require(target_info.get("account_id") == binding["target_account_id"], "binding_account_mismatch")

        used = integer(token.get("used_quota"), "invalid_used_quota")
        remain = integer(token.get("remain_quota"), "invalid_remain_quota")
        unlimited = token.get("unlimited_quota")
        require(type(unlimited) is bool, "invalid_unlimited_quota")
        require(used >= 0, "negative_key_used_counter")

        bearer = token.get("key", "")
        secret_hash = hashlib.sha256(bearer.encode("utf-8")).hexdigest()
        require(secret_hash == binding["source_secret_sha256"], "secret_hash_mismatch")

        records.append({
            "source_key_id": tid,
            "target_credential_id": binding["target_credential_id"],
            "target_user_id": binding["target_user_id"],
            "target_account_id": binding["target_account_id"],
            "used_units": used,
            "remaining_units": remain,
            "unlimited": unlimited,
            "source_secret_sha256": secret_hash,
            "source_status": token.get("status", 1),
            "source_row_sha256": digest(token),
        })

    records.sort(key=lambda r: r["source_key_id"])
    mapping_hash = digest(records)

    plan = {
        "format_version": 1,
        "scope": SCOPE,
        "migration_id": migration_id,
        "source_archive_sha256": source_archive_sha256,
        "mapping_sha256": mapping_hash,
        "key_records": records,
    }
    validate_key_quota_plan(plan, verify_mapping_digest=True)
    return plan
