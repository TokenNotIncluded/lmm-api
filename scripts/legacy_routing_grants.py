#!/usr/bin/env python3
"""Prepare source-bound routing-group grants for a stopped, isolated Core target.

This emits private psql scripts and a read-only outcome probe; it never connects or applies.
It copies only each user's original group and source enabled/deleted state. It
does not activate users, routes, credentials, providers or financial capabilities.
Requires both frozen identity and financial exports. Plan hashes are integrity
checks, not signatures; the supplied exports remain the independently preserved
source inputs. No source query or new export is performed.

Preparation now requires --source-financial-export pointing to the same frozen
source-financial-export.private.jsonl used by the routing planner, plus its
--source-receipt and --export-proof. The independently preserved receipt and
archive are the existing trust anchor, not cryptographic signatures; replacing
all trusted source inputs is outside this offline integrity contract. Catalog,
baseline and archive dependencies are frozen and checked again on resolution.
Existing materials without those inputs cannot be resolved by this version. Preserve them;
replan from the frozen inputs with this planner and prepare fresh material and
SQL against a fresh target snapshot, then review those new artifacts. Do not
edit an old material or its tool hashes to make it pass.
"""
from __future__ import annotations

import argparse
import io
import json
from pathlib import Path
import re
import sys

import financial_import as FIN
import legacy_credential_import as PRIVATE
import legacy_routing_migration as ROUTING

IDENTITY = FIN.IDENTITY
require = IDENTITY.require
Error = IDENTITY.MappingError
SCOPE = "legacy-user-routing-group-import-v1"
TABLE = "core_identity.user_routing_groups"
INPUTS = ("routing_plan", "identity_manifest", "identity_map", "source_identity_export", "source_financial_export",
          "source_receipt", "export_proof")


def financial_source_files(files, plan):
    """Reuse the planner's receipt/catalog/baseline contract and freeze its dependencies."""
    require(type(files) is dict and set(files) == set(INPUTS), "routing_source_files_incomplete")
    data = {name: PRIVATE.private_bytes(Path(item["path"])) for name, item in files.items()}
    require(all(FIN.sha(data[name]) == files[name]["sha256"] for name in INPUTS), "routing_source_file_changed")
    receipt = FIN.PLANNER.parse_json(data["source_receipt"].decode("utf-8"))
    require(type(receipt) is dict and type(receipt.get("archive")) is dict
            and receipt["archive"].get("sha256") == plan["bindings"]["source_archive_sha256"],
            "routing_financial_receipt_binding_mismatch")
    proof_path = Path(files["export_proof"]["path"])
    raw_path = proof_path.with_name("source-financial-export.private.jsonl")
    require(raw_path.absolute() == Path(files["source_financial_export"]["path"]).absolute(),
            "routing_financial_export_path_mismatch")
    hashes = {Path(item["path"]): item["sha256"] for item in files.values()}
    try:
        _, proof, _ = ROUTING.load_routing_export(receipt, proof_path, hashes, retained_tables=("users",))
    except ROUTING.MappingError as exc:
        raise Error(str(exc)) from None
    require(proof["source_export_sha256"] == plan["bindings"]["source_export_sha256"],
            "routing_financial_export_hash_mismatch")
    # The loader records the archive, catalog and baseline in addition to the
    # explicit inputs. Freeze them too so later reads cannot change the receipt's
    # dependencies while updating an adjacent, unsealed proof file.
    artifacts = {str(path.absolute()): digest for path, digest in hashes.items()}
    ROUTING.verify_files(hashes)
    return artifacts


def source_hashes():
    names = ("legacy_routing_grants.py", "legacy_routing_migration.py", "legacy_credential_import.py",
             "financial_import.py", "migrate-legacy-financial.py", "migrate-legacy-snapshot.py")
    return {name: IDENTITY.sha256_file(Path(__file__).with_name(name)) for name in names}


def source_records(plan, manifest, mapping_bytes, export_bytes, financial_bytes):
    """Rebuild both source-row hashes from frozen exports, never from plan claims."""
    require(type(plan) is dict and type(plan.get("format_version")) is int and plan["format_version"] == 1
            and plan.get("scope") == "legacy-routing-offline-plan", "routing_plan_contract_invalid")
    require(plan.get("plan_sha256") == FIN.digest({k: v for k, v in plan.items() if k != "plan_sha256"}),
            "routing_plan_digest_mismatch")
    bindings = plan["bindings"]
    require(type(bindings) is dict and set(bindings) == {"source_archive_sha256", "source_export_sha256",
            "identity_map_file_sha256"} and type(manifest) is dict, "routing_source_bindings_invalid")
    for value in bindings.values():
        FIN.checked_hash(value)
    proof = FIN.PLANNER.bind_identity_provenance(manifest, map_file_sha256=FIN.sha(mapping_bytes),
        source_archive_sha256=bindings["source_archive_sha256"])
    require(bindings["identity_map_file_sha256"] == proof["identity_map_file_sha256"]
            and manifest.get("source_export_sha256") == FIN.sha(export_bytes), "routing_identity_source_changed")
    parse_lines = lambda raw: [FIN.PLANNER.parse_json(line) for line in raw.decode("utf-8").splitlines() if line]
    require(FIN.sha(financial_bytes) == bindings["source_export_sha256"],
            "routing_financial_export_hash_mismatch")
    financial_users, financial_framing = {}, {"identity": 0, "schema": 0}
    for line in io.BytesIO(financial_bytes):
        if not line.strip():
            continue
        tagged = FIN.PLANNER.parse_json(line.decode("utf-8"))
        require(type(tagged) is dict, "routing_financial_export_shape_invalid")
        kind, row = tagged.get("kind"), tagged.get("row")
        require(type(row) is dict, "routing_financial_export_shape_invalid")
        if kind in financial_framing:
            require(set(tagged) == {"kind", "row"}, "routing_financial_export_shape_invalid")
            financial_framing[kind] += 1
        else:
            require(kind == "row" and set(tagged) == {"kind", "table", "row"}
                    and type(tagged["table"]) is str, "routing_financial_export_shape_invalid")
            if tagged["table"] == "users":
                uid = FIN.PLANNER.integer(row.get("id"), "routing_financial_user_invalid", minimum=1)
                require(uid not in financial_users, "routing_financial_user_duplicate")
                financial_users[uid] = row
    require(financial_framing == {"identity": 1, "schema": 1}, "routing_financial_export_framing_invalid")
    mapped = parse_lines(mapping_bytes)
    require(all(type(row) is dict and set(row) == {"source_user_id", "target_user_id", "target_personal_account_id"}
                for row in mapped), "routing_identity_map_invalid")
    identities = FIN.PLANNER.identity_index(mapped)
    users, user_hashes, counts, framing = {}, {}, [], {"identity": 0, "schema_identity": 0}
    for tagged in parse_lines(export_bytes):
        require(type(tagged) is dict and set(tagged) == {"kind", "row"}, "routing_identity_export_shape_invalid")
        kind, row = tagged["kind"], tagged["row"]
        if kind == "user":
            require(type(row) is dict, "routing_source_user_invalid")
            uid = FIN.PLANNER.integer(row.get("id"), "routing_source_user_invalid", minimum=1)
            require(uid not in users, "routing_source_user_duplicate")
            require(ROUTING.label(row.get("group")) and type(row.get("status")) is int
                    and row["status"] in (1, 2) and "deleted_at" in row, "routing_source_user_state_invalid")
            if row["deleted_at"] is not None:
                require(type(row["deleted_at"]) is str, "routing_source_user_state_invalid")
                deleted = ROUTING.datetime.fromisoformat(row["deleted_at"].replace("Z", "+00:00"))
                require(deleted.tzinfo is not None, "routing_source_user_state_invalid")
            require(uid in financial_users, "routing_financial_user_coverage_mismatch")
            financial = financial_users[uid]
            financial_utc = ROUTING.user_binding_hash(financial)
            require(financial_utc == ROUTING.user_binding_hash(row),
                    "routing_financial_identity_user_disagrees")
            user_hashes[uid] = (FIN.digest(financial), financial_utc, FIN.digest(row))
            users[uid] = {"user_id": uid, "group_name": row["group"],
                "enabled": row["status"] == 1 and row["deleted_at"] is None,
                "source_reference": "legacy.users/" + str(uid)}
        elif kind == "user_count":
            require(type(row) is int and row > 0, "routing_source_count_invalid")
            counts.append(row)
        elif kind in framing:
            require(type(row) is dict, "routing_identity_export_shape_invalid")
            framing[kind] += 1
        else:
            require(kind in {"second_factor", "password_policy"} and type(row) is dict,
                    "routing_identity_export_shape_invalid")
    require(framing == {"identity": 1, "schema_identity": 1} and counts == [len(users)]
            and users and set(users) == set(identities) == set(financial_users), "routing_identity_coverage_mismatch")
    planned = plan.get("user_group_plan")
    require(type(planned) is list and len(planned) == len(users), "routing_user_group_coverage_mismatch")
    wanted = [users[uid] for uid in sorted(users)]
    require(all(type(row) is dict and set(row) in (set(wanted[0]) | {"source_row_sha256"},
                set(wanted[0]) | {"source_row_sha256", "source_user_utc_sha256"}) for row in planned),
            "routing_user_group_plan_shape_invalid")
    require(FIN.digest([{k: v for k, v in row.items() if k not in {"source_row_sha256", "source_user_utc_sha256"}} for row in planned]) == FIN.digest(wanted),
            "routing_user_group_source_disagrees")
    for row in planned:
        FIN.checked_hash(row["source_row_sha256"])
        raw_hash, utc_hash, identity_hash = user_hashes[row["user_id"]]
        require(row["source_row_sha256"] == raw_hash, "routing_financial_user_hash_disagrees")
        if "source_user_utc_sha256" in row:
            FIN.checked_hash(row["source_user_utc_sha256"])
            require(row["source_user_utc_sha256"] == utc_hash,
                    "routing_user_group_source_hash_disagrees")
        else:
            # Legacy plans contain only the raw financial-export hash. Never
            # guess its timezone: replan from frozen inputs when bytes differ.
            require(row["source_row_sha256"] == identity_hash,
                    "routing_legacy_plan_requires_frozen_replan")
    return wanted, identities, proof


def native_users_sql():
    return """SELECT coalesce(jsonb_agg(jsonb_build_object('id',u.id,
 'personal_account_id',u.personal_account_id,'active',u.active,'auth_version',u.auth_version,
 'platform_role',u.platform_role,'account_active',a.active,'account_kind',a.kind) ORDER BY u.id),'[]'::jsonb)
 FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id"""


def snapshot_sql(inventory, migration_id):
    contract = IDENTITY.current_contract()
    require(type(migration_id) is str and IDENTITY.MIGRATION_ID.fullmatch(migration_id), "routing_migration_id_invalid")
    return "\\set ON_ERROR_STOP on\nBEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC';\nSELECT jsonb_build_object(" + \
        "'instance',(" + FIN.instance_sql() + "),'catalog_fingerprint_sha256',(" + IDENTITY.catalog_hash_sql(contract) + ")," + \
        "'ownership_sha256',(" + FIN.ownership_sql() + "),'component_fingerprints'," + \
        "(SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract)," + \
        "'trust_policy_seed',(" + IDENTITY.trust_policy_seed_query() + "),'state',(" + FIN.state_sql(inventory, migration_id) + ")," + \
        "'native_users',(" + native_users_sql() + "),'identity_import'," + \
        "(SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE manifest->>'scope'='identity-only')); COMMIT;\n"


def prepare_material(plan, manifest, mapping_bytes, export_bytes, financial_bytes, *, migration_id, snapshot, runtime_roles, source_files):
    records, identities, proof = source_records(plan, manifest, mapping_bytes, export_bytes, financial_bytes)
    require(type(snapshot) is dict and all(type(snapshot.get(name)) is dict for name in
            ("instance", "state", "identity_import", "component_fingerprints", "trust_policy_seed"))
            and type(snapshot.get("native_users")) is list, "routing_snapshot_shape_invalid")
    contract = IDENTITY.current_contract()
    material = {"format_version": 1, "scope": SCOPE, "migration_id": migration_id,
        "source_archive_sha256": proof["source_archive_sha256"], "identity_manifest": manifest,
        "identity_map_file_sha256": proof["identity_map_file_sha256"],
        "identity_accounts": {str(uid): aid for uid, aid in identities.items()},
        "routing_plan_sha256": plan["plan_sha256"], "records": records, "mapping_sha256": FIN.digest(records),
        "native_users": snapshot["native_users"], "identity_import": snapshot["identity_import"],
        "before_state": snapshot["state"], "runtime_roles": runtime_roles, "source_files": source_files,
        "target": {**{k: snapshot[k] for k in ("instance", "catalog_fingerprint_sha256", "ownership_sha256",
            "component_fingerprints", "trust_policy_seed")},
            "source_contract": {k: contract[k] for k in FIN.CONTRACT_FIELDS}},
        "tool_source_hashes": source_hashes(), "source_artifacts": financial_source_files(source_files, plan)}
    validate_material(material)
    return material


def validate_material(material):
    require(type(material) is dict and type(material.get("format_version")) is int
            and material["format_version"] == 1 and material.get("scope") == SCOPE, "routing_material_contract_invalid")
    require(type(material.get("migration_id")) is str and IDENTITY.MIGRATION_ID.fullmatch(material["migration_id"]),
            "routing_migration_id_invalid")
    require(material.get("tool_source_hashes") == source_hashes(), "routing_tool_source_changed")
    for key in ("source_archive_sha256", "identity_map_file_sha256", "routing_plan_sha256", "mapping_sha256"):
        FIN.checked_hash(material.get(key))
    manifest = material["identity_manifest"]
    require(type(manifest) is dict, "routing_identity_manifest_invalid")
    FIN.PLANNER.bind_identity_provenance(manifest, map_file_sha256=material["identity_map_file_sha256"],
        source_archive_sha256=material["source_archive_sha256"])
    imported = material["identity_import"]
    require(type(imported) is dict and imported.get("source_archive_sha256") == material["source_archive_sha256"]
            and imported.get("mapping_sha256") == manifest["mapping_sha256"] and imported.get("manifest") == manifest,
            "routing_identity_import_mismatch")
    identities = material["identity_accounts"]
    # Keep text keys in the sealed material so its typed digest survives JSON I/O.
    require(type(identities) is dict and all(type(k) is str and k.isascii() and k.isdigit()
            and str(int(k)) == k for k in identities),
            "routing_identity_map_invalid")
    identities = {FIN.PLANNER.integer(int(k), "routing_identity_map_invalid", minimum=1): FIN.PLANNER.integer(v, "routing_identity_map_invalid", minimum=1)
                  for k, v in identities.items()}
    require(identities and len(set(identities.values())) == len(identities), "routing_identity_map_invalid")
    native = material["native_users"]
    require(type(native) is list and all(type(row) is dict for row in native) and len(native) == len(identities)
            and [r.get("id") for r in native] == sorted(identities), "routing_target_user_coverage_mismatch")
    require(all(type(row) is dict and set(row) == {"id", "personal_account_id", "active", "auth_version", "platform_role",
                    "account_active", "account_kind"} and type(row["id"]) is int and type(row["personal_account_id"]) is int
                and identities[row["id"]] == row["personal_account_id"]
                and type(row["active"]) is bool and type(row["account_active"]) is bool and row["account_kind"] == "personal"
                and type(row["auth_version"]) is int and row["auth_version"] > 0
                and row["platform_role"] in {"user", "admin", "superadmin"} for row in native),
            "routing_target_identity_changed")
    rows = material["records"]
    require(type(rows) is list and all(type(row) is dict for row in rows) and [r.get("user_id") for r in rows] == sorted(identities)
            and all(type(row) is dict and set(row) == {"user_id", "group_name", "enabled", "source_reference"}
                and type(row["user_id"]) is int and ROUTING.label(row["group_name"]) and type(row["enabled"]) is bool
                and row["source_reference"] == "legacy.users/" + str(row["user_id"]) for row in rows)
            and material["mapping_sha256"] == FIN.digest(rows), "routing_grant_rows_invalid")
    state = material["before_state"]
    require(type(state) is dict and set(state) == {"inventory", "tables", "sequences"}
            and type(state["tables"]) is dict and type(state["sequences"]) is dict,
            "routing_snapshot_inventory_mismatch")
    FIN.validate_inventory(state["inventory"])
    require(TABLE in state["inventory"]["tables"] and set(state["tables"]) == set(state["inventory"]["tables"])
            and set(state["sequences"]) == set(state["inventory"]["sequences"]), "routing_snapshot_inventory_mismatch")
    for entry in state["tables"].values():
        require(type(entry) is dict, "routing_snapshot_count_invalid")
        FIN.PLANNER.integer(entry.get("rows"), "routing_snapshot_count_invalid", minimum=0)
        FIN.checked_hash(entry.get("sha256"))
    require(state["tables"][TABLE] == {"rows": 0, "sha256": FIN.sha(b"")}, "routing_target_grants_not_empty")
    target, contract = material["target"], IDENTITY.current_contract()
    require(type(target) is dict and type(target.get("instance")) is dict, "routing_target_instance_invalid")
    expected = target["instance"]
    require(expected.get("database_owner") == expected.get("role") != expected.get("session_user")
            and expected.get("effective_role_can_login") is False and expected.get("tcp_connection") is False
            and expected.get("listen_addresses") == "", "routing_target_not_offline_owner")
    for field in ("catalog_fingerprint_sha256", "ownership_sha256"):
        FIN.checked_hash(target.get(field))
    require(target.get("source_contract") == {k: contract[k] for k in FIN.CONTRACT_FIELDS}
            and target.get("component_fingerprints") == contract["component_fingerprints"], "routing_source_contract_changed")
    IDENTITY.validate_trust_policy_seed(target.get("trust_policy_seed"))
    roles = material["runtime_roles"]
    require(type(roles) is list and roles and all(type(r) is str and IDENTITY.IDENTIFIER.fullmatch(r) for r in roles)
            and roles == sorted(set(roles))
            and not {expected["role"], expected["session_user"]}.intersection(roles), "routing_runtime_roles_invalid")
    artifacts = material.get("source_artifacts")
    require(type(artifacts) is dict and artifacts and all(type(path) is str and Path(path).is_absolute()
            for path in artifacts), "routing_source_artifacts_invalid")
    for digest in artifacts.values():
        FIN.checked_hash(digest)
    files = material["source_files"]
    require(type(files) is dict and set(files) == set(INPUTS), "routing_source_files_incomplete")
    for item in files.values():
        require(type(item) is dict and set(item) == {"path", "sha256"} and type(item["path"]) is str
                and Path(item["path"]).is_absolute(),
                "routing_source_file_invalid")
        FIN.checked_hash(item["sha256"])


def verify_source_files(material):
    validate_material(material)
    files = material["source_files"]
    data = {name: PRIVATE.private_bytes(Path(item["path"])) for name, item in files.items()}
    require(all(FIN.sha(data[name]) == files[name]["sha256"] for name in INPUTS), "routing_source_file_changed")
    plan, manifest = (FIN.PLANNER.parse_json(data[name].decode("utf-8")) for name in INPUTS[:2])
    require(all(IDENTITY.sha256_file(IDENTITY.private_backup_path(Path(path))) == digest
                for path, digest in material["source_artifacts"].items()), "routing_source_artifacts_changed")
    require(financial_source_files(files, plan) == material["source_artifacts"], "routing_source_artifacts_changed")
    rows, identities, _ = source_records(plan, manifest, data["identity_map"], data["source_identity_export"],
                                         data["source_financial_export"])
    require(rows == material["records"] and {str(uid): aid for uid, aid in identities.items()} == material["identity_accounts"]
            and manifest == material["identity_manifest"] and plan["plan_sha256"] == material["routing_plan_sha256"],
            "routing_frozen_inputs_mismatch")


def _guard(material):
    return FIN._guard(material) + f"""
 IF ({native_users_sql()}) IS DISTINCT FROM m->'native_users'
 OR (SELECT count(*) FROM core_migration.import_runs WHERE manifest->>'scope'='identity-only')<>1
 OR NOT EXISTS(SELECT 1 FROM core_migration.import_runs r WHERE to_jsonb(r)=m->'identity_import')
 THEN RAISE EXCEPTION 'routing_identity_binding_changed'; END IF;
 IF {IDENTITY.trust_policy_seed_guard_sql(material['target'])}
 THEN RAISE EXCEPTION 'routing_trust_policy_changed'; END IF;
"""


def _readback():
    return f"""
 IF (SELECT count(*) FROM {FIN.relation(TABLE)})<>jsonb_array_length(m->'records')
 OR EXISTS(SELECT 1 FROM jsonb_array_elements(m->'records') r LEFT JOIN {FIN.relation(TABLE)} g
  ON g.user_id=(r->>'user_id')::bigint AND g.group_name COLLATE "C"=(r->>'group_name') COLLATE "C"
  WHERE g.user_id IS NULL OR g.enabled IS DISTINCT FROM (r->>'enabled')::boolean
   OR g.source_reference IS DISTINCT FROM r->>'source_reference')
 THEN RAISE EXCEPTION 'routing_exact_grants_readback_failed'; END IF;
"""


def build_apply_sql(material):
    validate_material(material)
    state = FIN.state_sql(material["before_state"]["inventory"], material["migration_id"])
    locks = ",".join(FIN.relation(n) for n in material["before_state"]["inventory"]["tables"])
    sql = FIN._start(material, readonly=False) + f"""
DO $lmm_financial$
DECLARE m jsonb := {FIN.literal(material)}::jsonb; previous core_migration.import_runs%ROWTYPE;
 before_state jsonb; after_state jsonb;
BEGIN
 {_guard(material)}
 LOCK TABLE {locks} IN ACCESS EXCLUSIVE MODE;
 {_guard(material)}
 SELECT ({state}) INTO before_state;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256<>m->>'source_archive_sha256' OR previous.mapping_sha256<>m->>'mapping_sha256'
   OR previous.manifest->>'scope' IS DISTINCT FROM {FIN.literal(SCOPE)}
   OR previous.manifest->'material' IS DISTINCT FROM m OR previous.manifest->'post_state' IS DISTINCT FROM before_state
  THEN RAISE EXCEPTION 'routing_replay_conflict'; END IF;
 ELSE
  IF before_state IS DISTINCT FROM m->'before_state'
   OR EXISTS(SELECT 1 FROM core_migration.import_runs WHERE manifest->>'scope'={FIN.literal(SCOPE)})
  THEN RAISE EXCEPTION 'routing_target_state_changed'; END IF;
  INSERT INTO {FIN.relation(TABLE)}(user_id,group_name,enabled,source_reference)
   SELECT (r->>'user_id')::bigint,r->>'group_name',(r->>'enabled')::boolean,r->>'source_reference'
   FROM jsonb_array_elements(m->'records') r ORDER BY (r->>'user_id')::bigint;
  SET CONSTRAINTS ALL IMMEDIATE;
  {_readback()}
  SELECT ({state}) INTO after_state;
  IF (before_state->'tables')-{FIN.literal(TABLE)} IS DISTINCT FROM (after_state->'tables')-{FIN.literal(TABLE)}
   OR before_state->'sequences' IS DISTINCT FROM after_state->'sequences'
  THEN RAISE EXCEPTION 'routing_unrelated_state_changed'; END IF;
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
   VALUES(m->>'migration_id',m->>'source_archive_sha256',m->>'mapping_sha256',
    jsonb_build_object('scope',{FIN.literal(SCOPE)},'material',m,'post_state',after_state));
 END IF;
 {_readback()}
 {_guard(material)}
END $lmm_financial$;
COMMIT;
SELECT jsonb_build_object('status','COMMITTED','migration_id',{FIN.literal(material['migration_id'])},
 'material_sha256',{FIN.literal(FIN.digest(material))});
"""
    # A standalone `psql -f` must stop on a failed fence or transaction instead
    # of continuing past a rollback to the success response.
    return "\\set ON_ERROR_STOP on\n" + FIN._dollar_quote(sql)


def build_resolve_sql(material, original_application_name):
    validate_material(material)
    require(type(original_application_name) is str and re.fullmatch(r"lmm-routing-[0-9a-f]{32}", original_application_name),
            "routing_original_attempt_invalid")
    state = FIN.state_sql(material["before_state"]["inventory"], material["migration_id"])
    return "\\set ON_ERROR_STOP on\n" + FIN._dollar_quote(FIN._start(material, readonly=True) + f"""
DO $lmm_financial$
DECLARE m jsonb := {FIN.literal(material)}::jsonb; previous core_migration.import_runs%ROWTYPE; actual jsonb;
BEGIN
 {_guard(material)}
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name={FIN.literal(original_application_name)}
  AND datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid())
 THEN RAISE EXCEPTION 'routing_original_attempt_active'; END IF;
 SELECT ({state}) INTO actual;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256<>m->>'source_archive_sha256' OR previous.mapping_sha256<>m->>'mapping_sha256'
   OR previous.manifest->>'scope' IS DISTINCT FROM {FIN.literal(SCOPE)}
   OR previous.manifest->'material' IS DISTINCT FROM m OR previous.manifest->'post_state' IS DISTINCT FROM actual
  THEN RAISE EXCEPTION 'routing_resolve_conflict'; END IF;
  {_readback()}
 ELSIF actual IS DISTINCT FROM m->'before_state' THEN RAISE EXCEPTION 'routing_absence_not_proven'; END IF;
END $lmm_financial$;
SELECT jsonb_build_object('status',CASE WHEN EXISTS(SELECT 1 FROM core_migration.import_runs
 WHERE migration_id={FIN.literal(material['migration_id'])}) THEN 'COMMITTED' ELSE 'NOT_APPLIED' END,
 'migration_id',{FIN.literal(material['migration_id'])},'material_sha256',{FIN.literal(FIN.digest(material))});
COMMIT;
""")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--material", type=Path, help="Resolve an existing material instead of preparing one")
    parser.add_argument("--original-application-name", help="Exact original lmm-routing-<32 hex> apply attempt")
    for name in (*INPUTS, "snapshot"):
        parser.add_argument("--" + name.replace("_", "-"), type=Path)
    parser.add_argument("--migration-id")
    parser.add_argument("--runtime-role", action="append")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.material:
            require(args.original_application_name and not any(getattr(args, name) for name in (*INPUTS, "snapshot"))
                    and not args.migration_id and not args.runtime_role, "routing_resolve_arguments_invalid")
            material = PRIVATE.load_json(args.material)
            verify_source_files(material)
            outputs = {"resolve.private.sql": build_resolve_sql(material, args.original_application_name)}
            status = "OUTCOME_PROBE_PREPARED_NOT_RUN"
        else:
            require(not args.original_application_name and all(getattr(args, name) for name in (*INPUTS, "snapshot"))
                    and args.migration_id and args.runtime_role, "routing_prepare_arguments_missing")
            data = {name: PRIVATE.private_bytes(getattr(args, name)) for name in INPUTS}
            plan, manifest = (FIN.PLANNER.parse_json(data[name].decode("utf-8")) for name in INPUTS[:2])
            files = {name: {"path": str(getattr(args, name).absolute()), "sha256": FIN.sha(data[name])} for name in INPUTS}
            material = prepare_material(plan, manifest, data["identity_map"], data["source_identity_export"],
                data["source_financial_export"], migration_id=args.migration_id, snapshot=PRIVATE.load_json(args.snapshot),
                runtime_roles=sorted(args.runtime_role), source_files=files)
            verify_source_files(material)
            outputs = {"material.private.json": FIN.canonical(material) + "\n", "grants.private.sql": build_apply_sql(material)}
            status = "PREPARED_NOT_APPLIED"
        IDENTITY.private_backup_path(args.output.parent, directory=True)
        args.output.mkdir(mode=0o700)
        for name, content in outputs.items():
            FIN.write_private(args.output, name, content)
        verify_source_files(material)
        print(FIN.canonical({"status": status, "rows": len(material["records"]),
            "material_sha256": FIN.digest(material), "target_writes": 0, "serving_approved": False}))
        return 0
    except (Error, OSError, ValueError, KeyError, TypeError, UnicodeError):
        print('{"status":"REFUSED","error_code":"routing_grant_inputs_or_private_output_invalid"}', file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
