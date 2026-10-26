#!/usr/bin/env python3
"""Offline legacy credential producer; credentials and quota become visible atomically.

This consumes the frozen source projection, never connects to its source, and
reuses the existing offline installer's guards, state hashes and private I/O.
Only a verified committed transaction produces the a02 mapping receipt. Managed
keys and legacy policy semantics without a native equivalent reject the batch.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct
import sys
import uuid

import financial_import as FIN
import legacy_api_key_migration as PROPOSAL
import legacy_key_quota_opening as QUOTA

IDENTITY = FIN.IDENTITY
Error = IDENTITY.MappingError
require = IDENTITY.require
canonical = FIN.canonical
digest = FIN.digest
literal = FIN.literal
SCOPE = "legacy-credential-and-quota-import-v1"
RECEIPT_SCOPE = "a02-credential-mapping-receipt"
CHANGED_TABLES = ("core_identity.credentials", "core_identity.api_key_settings",
                  "core_identity.key_funding_rules", "core_charging.budgets",
                  "core_charging.budget_opening_consumption")
CHANGED_SEQUENCES = ("core_identity.credentials_id_seq", "core_charging.budgets_id_seq")
FILES = ("source", "target_projection", "proposal", "review")
RESERVED_PREFIXES = ("lmmk_", "lmms_", "lmmr_", "lmmi_", "lmmf_", "lmmt_", "lmmo_")
MAX_ALIAS_KEYS = 10000
MAPPING_FILE = "credential-mapping-receipt.private.json"
ALIAS_FILE = "legacy-api-key-wire-aliases.private.json"
ARTIFACT_AUDIT_FILE = "credential-artifacts-audit.private.json"


class PublicationError(Error):
    def __init__(self, cleanup: dict):
        super().__init__("credential_artifact_publication_failed")
        self.cleanup = cleanup


def ordinary_secret(raw) -> bool:
    """Exact Core legacy_keys.rs ordinary() contract, including ASCII case folding."""
    if type(raw) is not str or re.fullmatch(r"[A-Za-z0-9_]{32,256}", raw) is None:
        return False
    lower = raw.lower()
    return "lmm_" not in lower and "oauth_managed_" not in lower and not lower.startswith(RESERVED_PREFIXES)


def alias_digest(value, code="credential_alias_digest_invalid") -> str:
    FIN.checked_hash(value, code)
    require(value != "0" * 64, code)
    return value


def validate_wire_source(source: dict) -> None:
    tokens = source.get("tokens")
    require(type(tokens) is list and 1 <= len(tokens) <= MAX_ALIAS_KEYS, "credential_alias_key_count_invalid")
    alias_digest(source.get("source_archive_sha256"), "credential_alias_source_digest_invalid")
    require(all(type(token) is dict and ordinary_secret(token.get("key")) for token in tokens),
            "credential_ordinary_wire_contract_unsupported")


def alias_target(instance: dict) -> dict:
    oid = instance["database_oid"]
    # PostgreSQL's OID JSON representation is text in the frozen instance
    # receipt. Normalize only the Core output; the SQL guard keeps that receipt.
    if type(oid) is str and re.fullmatch(r"[0-9]{1,19}", oid) is not None:
        oid = int(oid)
    target = {"database": instance["database"], "database_oid": oid,
              "instance_nonce": instance["server_instance_nonce"]}
    require(type(target["database"]) is str and 1 <= len(target["database"].encode("utf-8")) <= 63
            and not any(ord(char) < 32 or 127 <= ord(char) <= 159 for char in target["database"])
            and type(target["database_oid"]) is int and 0 < target["database_oid"] <= QUOTA.BIGINT_MAX,
            "credential_alias_target_invalid")
    alias_digest(target["instance_nonce"], "credential_alias_target_nonce_invalid")
    return target


def source_hashes() -> dict:
    names = ("legacy_credential_import.py", "legacy_api_key_migration.py",
             "legacy_key_quota_opening.py", "financial_import.py", "migrate-legacy-financial.py",
             "migrate-legacy-snapshot.py", "legacy_financial_preflight.py")
    result = {name: IDENTITY.sha256_file(Path(__file__).with_name(name)) for name in names}
    root = Path(__file__).resolve().parents[1]
    for name in ("services/core/src/identity/compat.rs", "services/core/src/identity/mod.rs"):
        result[name] = IDENTITY.sha256_file(root / name)
    return result


def private_bytes(path: Path) -> bytes:
    """The installer checks private ownership and every parent; no ambient key lookup."""
    checked = IDENTITY.private_backup_path(path)
    flags = os.O_RDONLY | os.O_NOFOLLOW
    fd = os.open(checked, flags)
    try:
        before = checked.stat()
        opened = os.fstat(fd)
        require((before.st_dev, before.st_ino) == (opened.st_dev, opened.st_ino), "input_replaced")
        require(stat.S_ISREG(opened.st_mode) and opened.st_uid == os.getuid() and opened.st_mode & 0o077 == 0,
                "input_not_private_owned_file")
        with os.fdopen(fd, "rb", closefd=False) as stream:
            result = stream.read()
        after = os.fstat(fd)
        require((opened.st_size, opened.st_mtime_ns, opened.st_ctime_ns) ==
                (after.st_size, after.st_mtime_ns, after.st_ctime_ns), "input_changed_during_read")
        return result
    finally:
        os.close(fd)


def load_json(path: Path) -> dict:
    try:
        value = json.loads(private_bytes(path), object_pairs_hook=PROPOSAL.unique_object)
    except (ValueError, UnicodeError, RecursionError):
        raise Error("private_json_invalid") from None
    require(type(value) is dict, "private_json_object_required")
    return value


def _cipher(master_key: bytes):
    require(type(master_key) is bytes and len(master_key) == 32, "recovery_key_must_be_32_raw_bytes")
    try:
        from cryptography.hazmat.primitives import hashes
        from cryptography.hazmat.primitives.kdf.hkdf import HKDF
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    except ImportError:
        raise Error("cryptography_dependency_unavailable") from None
    # Exact Core identity/compat.rs format; cryptography supplies both primitives.
    derived = HKDF(algorithm=hashes.SHA256(), length=32, salt=b"lmm-api-key-secret-v1",
                   info=b"aes-256-gcm").derive(master_key)
    return AESGCM(derived)


def encrypt_secret(master_key: bytes, credential_id: int, user_id: int, secret: str) -> str:
    aad = struct.pack(">Qqq", 1, credential_id, user_id)
    nonce = os.urandom(12)
    encrypted = nonce + _cipher(master_key).encrypt(nonce, secret.encode("utf-8"), aad)
    require(44 <= len(encrypted) <= 300, "recovery_ciphertext_native_shape_invalid")
    return encrypted.hex()


def verify_encrypted_secret(master_key: bytes, row: dict, raw: str) -> None:
    try:
        encrypted = bytes.fromhex(row["encrypted_secret_hex"])
        recovered = _cipher(master_key).decrypt(encrypted[:12], encrypted[12:],
            struct.pack(">Qqq", 1, row["target_credential_id"], row["target_user_id"]))
    except Error:
        raise
    except Exception:
        raise Error("recovery_ciphertext_verification_failed") from None
    require(recovered == raw.encode("utf-8") and hashlib.sha256(recovered).hexdigest() == row["digest_sha256"],
            "recovery_secret_digest_mismatch")


def mapping_receipt(source_hash: str, rows: list[dict]) -> dict:
    """Candidate binding used inside the transaction; not emitted before commit."""
    return {"scope": RECEIPT_SCOPE, "source_archive_sha256": source_hash,
            "records": [{"source_key_id": row["source_token_id"],
                         "target_credential_id": row["target_credential_id"],
                         "target_user_id": row["target_user_id"],
                         "target_account_id": row["target_personal_account_id"],
                         "source_secret_sha256": row["digest_sha256"]} for row in rows]}


def validate_review(review: dict, source: dict, target: dict, proposal: dict) -> None:
    require(type(review) is dict and set(review) == {"format_version", "scope", "decision",
        "source_input_sha256", "target_input_sha256", "proposal_sha256", "source_frozen",
        "source_unit", "native_unit", "numerator", "denominator", "evidence_sha256",
        "economic_review_sha256"}, "credential_review_shape_invalid")
    require(type(review["format_version"]) is int and review["format_version"] == 1
        and review["scope"] == SCOPE and review["decision"] == "approve-offline-native-equivalent-credentials"
        and review["source_frozen"] is True, "credential_review_not_approved")
    require(review["source_input_sha256"] == PROPOSAL.sha256(source)
        and review["target_input_sha256"] == PROPOSAL.sha256(target)
        and review["proposal_sha256"] == proposal["proposal_sha256"], "credential_review_input_mismatch")
    require(review["source_unit"] == "source_quota_integer" and review["native_unit"] == "credit_500k_usd"
        and type(review["numerator"]) is int and review["numerator"] == 1
        and type(review["denominator"]) is int and review["denominator"] == 1,
        "credential_quota_unit_conversion_unsupported")
    FIN.checked_hash(review["evidence_sha256"])
    FIN.checked_hash(review["economic_review_sha256"])


def _next_id(state: dict, sequence: str, count: int) -> int:
    seq = state["sequences"][sequence]
    require(type(seq.get("increment")) is int and seq["increment"] == 1
            and type(seq.get("cache")) is int and seq["cache"] == 1 and seq.get("cycle") is False
            and type(seq.get("is_called")) is bool, "credential_sequence_not_canonical")
    first = QUOTA.integer(seq.get("last_value"), "credential_sequence_value_invalid", minimum=1) + int(seq["is_called"])
    maximum = QUOTA.integer(seq.get("max"), "credential_sequence_max_invalid", minimum=1)
    require(first + count <= maximum, "credential_sequence_exhausted")
    return first


def _supported_rows(proposal: dict, target: dict, first_id: int) -> list[dict]:
    observed = {row["user_id"]: row for row in target["users"]}
    rows = []
    for offset, original in enumerate(proposal["records"]):
        blocked = set(original["blocked_reasons"]) - {"recoverable_secret_encryption_required"}
        # An inactive owner stays inactive. This imports historical authority,
        # without activating the user, changing auth_version or granting access.
        if not original["source_owner_enabled"] and not observed[original["target_user_id"]]["active"]:
            blocked.discard("target_owner_inactive")
        require(not blocked, "credential_native_semantics_unsupported")
        row = json.loads(canonical(original))
        row["target_credential_id"] = first_id + offset
        row["encrypted_secret_hex"] = None
        status, expiry = row["source_status"], row["expiry"]
        expired = expiry["kind"] != "never" and expiry["source_epoch"] < proposal["snapshot_at"]
        exhausted = not row["quota"]["unlimited"] and row["quota"]["remaining"] <= 0
        # Native list views derive states 3/4 from expiry/budget, rather than
        # a stored status. Never turn an expired/exhausted key into disabled 2.
        require(status not in (3, 4) or (status == 3 and expired) or
                (status == 4 and not expired and exhausted), "credential_status_native_mismatch")
        row["enabled"] = status != 2
        rows.append(row)
    require(bool(rows), "credential_batch_empty")
    return rows


def prepare_material(source: dict, target_projection: dict, proposal: dict, review: dict, *,
                     migration_id: str, snapshot: dict, runtime_roles: list[str], source_files: dict,
                     encryption_key: bytes | None = None) -> dict:
    validate_wire_source(source)
    alias_target(snapshot["instance"])
    require(PROPOSAL.plan_api_keys(source, target_projection) == proposal, "credential_proposal_rebuild_mismatch")
    validate_review(review, source, target_projection, proposal)
    require(type(migration_id) is str and IDENTITY.MIGRATION_ID.fullmatch(migration_id), "credential_migration_id_invalid")
    require(snapshot["native_users"] == target_projection["users"], "credential_target_projection_stale")
    state = snapshot["state"]
    first = _next_id(state, CHANGED_SEQUENCES[0], len(proposal["records"]))
    rows = _supported_rows(proposal, target_projection, first)
    raw = {row["id"]: row for row in source["tokens"]}
    for row in rows:
        token = raw[row["source_token_id"]]
        row["source_row_sha256"] = digest(token)
        row["source_deleted_at"] = token["deleted_at"]
        if not row["one_time_reveal"]:
            require(encryption_key is not None, "credential_recovery_key_required")
            row["encrypted_secret_hex"] = encrypt_secret(encryption_key, row["target_credential_id"], row["target_user_id"], token["key"])
            verify_encrypted_secret(encryption_key, row, token["key"])
    receipt = mapping_receipt(source["source_archive_sha256"], rows)
    user_map = {row["source_user_id"]: {"user_id": row["target_user_id"], "account_id": row["target_personal_account_id"]}
                for row in target_projection["source_target_id_map"]}
    quota = QUOTA.build_plan_from_tokens(source["tokens"], user_map, receipt,
        migration_id=migration_id, source_archive_sha256=source["source_archive_sha256"])
    contract = IDENTITY.current_contract()
    material = {"format_version": 1, "scope": SCOPE, "migration_id": migration_id,
        "source_archive_sha256": source["source_archive_sha256"], "identity_map_sha256": target_projection["source_target_id_map_sha256"],
        "proposal": proposal, "source_review": review, "target_projection": target_projection,
        "records": rows, "credential_receipt": receipt, "quota_plan": quota,
        "source_files": source_files, "runtime_roles": runtime_roles,
        "before_state": state, "financial_import": snapshot["financial_import"],
        "identity_import": snapshot["identity_import"],
        "target": {**{key: snapshot[key] for key in ("instance", "catalog_fingerprint_sha256", "ownership_sha256",
                    "component_fingerprints", "trust_policy_seed")},
                   "source_contract": {key: contract[key] for key in FIN.CONTRACT_FIELDS}},
        "tool_source_hashes": source_hashes()}
    material["mapping_sha256"] = digest(rows)
    validate_material(material)
    require(not any(token["key"] in canonical(material) for token in source["tokens"]), "credential_secret_echo")
    return material


def validate_material(material: dict) -> None:
    require(type(material) is dict and type(material.get("format_version")) is int
            and material["format_version"] == 1 and material.get("scope") == SCOPE,
            "credential_material_contract_invalid")
    require(type(material.get("migration_id")) is str and IDENTITY.MIGRATION_ID.fullmatch(material["migration_id"]),
            "credential_migration_id_invalid")
    require(material.get("tool_source_hashes") == source_hashes(), "credential_tool_source_changed")
    for field in ("source_archive_sha256", "identity_map_sha256", "mapping_sha256"):
        FIN.checked_hash(material.get(field))
    rows = material["records"]
    require(type(rows) is list and 1 <= len(rows) <= MAX_ALIAS_KEYS, "credential_alias_key_count_invalid")
    alias_digest(material["source_archive_sha256"], "credential_alias_source_digest_invalid")
    require(material["mapping_sha256"] == digest(rows), "credential_mapping_hash_mismatch")
    state = material["before_state"]
    FIN.validate_inventory(state["inventory"])
    require(set(CHANGED_TABLES) <= set(state["inventory"]["tables"])
        and set(CHANGED_SEQUENCES) <= set(state["inventory"]["sequences"]), "credential_inventory_incomplete")
    require(set(state["tables"]) == set(state["inventory"]["tables"])
        and set(state["sequences"]) == set(state["inventory"]["sequences"]), "credential_snapshot_inventory_mismatch")
    for row in state["tables"].values():
        QUOTA.integer(row.get("rows"), "credential_snapshot_rows_invalid", minimum=0)
        FIN.checked_hash(row.get("sha256"))
    require(all(state["tables"][name] == {"rows": 0, "sha256": FIN.sha(b"")} for name in CHANGED_TABLES),
            "credential_target_not_empty")
    first = _next_id(state, CHANGED_SEQUENCES[0], len(rows))
    _next_id(state, CHANGED_SEQUENCES[1], len(rows))
    expected = _supported_rows(material["proposal"], material["target_projection"], first)
    require(len(rows) == len(expected) and all(
        {k: v for k, v in actual.items() if k not in {"encrypted_secret_hex", "source_row_sha256", "source_deleted_at"}} ==
        {k: v for k, v in wanted.items() if k != "encrypted_secret_hex"} for actual, wanted in zip(rows, expected)),
        "credential_record_proposal_mismatch")
    for row in rows:
        alias_digest(row["digest_sha256"])
        FIN.checked_hash(row.get("source_row_sha256"))
        require(PROPOSAL.deleted(row.get("source_deleted_at")) == row["revoked"], "credential_deleted_state_mismatch")
        encrypted = row.get("encrypted_secret_hex")
        require((row["one_time_reveal"] and encrypted is None) or
                (not row["one_time_reveal"] and type(encrypted) is str and
                 re.fullmatch(r"[0-9a-f]{88,600}", encrypted) and len(encrypted) % 2 == 0),
                "credential_recovery_ciphertext_invalid")
    require(material["credential_receipt"] == mapping_receipt(material["source_archive_sha256"], rows),
            "credential_receipt_mismatch")
    QUOTA.validate_key_quota_plan(material["quota_plan"])
    require(material["quota_plan"]["source_archive_sha256"] == material["source_archive_sha256"]
        and material["quota_plan"]["migration_id"] == material["migration_id"]
        and len(material["quota_plan"]["key_records"]) == len(rows), "credential_quota_binding_mismatch")
    for row, quota in zip(rows, material["quota_plan"]["key_records"]):
        require(quota == {"source_key_id": row["source_token_id"], "target_credential_id": row["target_credential_id"],
            "target_user_id": row["target_user_id"], "target_account_id": row["target_personal_account_id"],
            "source_secret_sha256": row["digest_sha256"], "used_units": row["quota"]["opening_consumed"],
            "remaining_units": row["quota"]["remaining"], "unlimited": row["quota"]["unlimited"],
            "source_status": row["source_status"], "source_row_sha256": row["source_row_sha256"]}, "credential_quota_row_mismatch")
    financial = material["financial_import"]
    identity = material["identity_import"]
    require(type(identity) is dict and identity.get("source_archive_sha256") == material["source_archive_sha256"]
        and identity.get("manifest", {}).get("scope") == "identity-only"
        and identity["manifest"].get("source_archive_sha256") == material["source_archive_sha256"]
        and identity["manifest"].get("source_target_id_map_sha256") == material["identity_map_sha256"]
        == material["target_projection"]["source_target_id_map_sha256"], "credential_identity_import_binding_mismatch")
    require(type(financial) is dict and financial.get("source_archive_sha256") == material["source_archive_sha256"]
        and financial.get("manifest", {}).get("scope") in {FIN.SCOPE, FIN.ESCROW_SCOPE}
        and financial["manifest"].get("material", {}).get("identity_import") == identity,
        "credential_financial_opening_required")
    economic = financial["manifest"]["material"]
    economic_review = economic.get("source_review")
    FIN.validate_source_review(economic_review, economic, candidate=economic.get("escrow_candidate"),
        unit_evidence=economic.get("escrow_unit_evidence"), inflight=economic.get("inflight_evidence"))
    require(economic.get("source_archive_sha256") == material["source_archive_sha256"]
        and economic.get("source_files", {}).get("source_export", {}).get("sha256") == material["proposal"]["source_export_sha256"]
        and material["source_review"].get("economic_review_sha256") == digest(economic_review)
        and material["source_review"].get("evidence_sha256") == economic_review["evidence_sha256"],
        "credential_economic_review_binding_mismatch")
    files = material["source_files"]
    required = set(FILES) | ({"encryption_key"} if any(not r["one_time_reveal"] for r in rows) else set())
    require(type(files) is dict and set(files) == required, "credential_source_files_incomplete")
    for item in files.values():
        require(type(item) is dict and set(item) == {"path", "sha256"} and Path(item["path"]).is_absolute(), "credential_source_file_invalid")
        FIN.checked_hash(item["sha256"])
    target = material["target"]
    require(target["source_contract"] == {key: IDENTITY.current_contract()[key] for key in FIN.CONTRACT_FIELDS}
        and target["component_fingerprints"] == IDENTITY.current_contract()["component_fingerprints"], "credential_source_contract_changed")
    for field in ("catalog_fingerprint_sha256", "ownership_sha256"):
        FIN.checked_hash(target.get(field))
    IDENTITY.validate_trust_policy_seed(target.get("trust_policy_seed"))
    expected = target["instance"]
    alias_target(expected)
    require(expected.get("database_owner") == expected.get("role") != expected.get("session_user")
        and expected.get("effective_role_can_login") is False and expected.get("tcp_connection") is False
        and expected.get("listen_addresses") == "", "credential_target_not_offline_owner")
    roles = material["runtime_roles"]
    require(type(roles) is list and roles and roles == sorted(set(roles))
        and all(type(r) is str and IDENTITY.IDENTIFIER.fullmatch(r) for r in roles)
        and not {expected["role"], expected["session_user"]}.intersection(roles), "credential_runtime_roles_invalid")


def verify_source_files(material: dict) -> None:
    files = material["source_files"]
    for item in files.values():
        require(FIN.sha(private_bytes(Path(item["path"]))) == item["sha256"], "credential_source_file_changed")
    source, target, proposal, review = (load_json(Path(files[name]["path"])) for name in FILES)
    validate_wire_source(source)
    require(PROPOSAL.plan_api_keys(source, target) == proposal == material["proposal"]
        and target == material["target_projection"] and review == material["source_review"], "credential_frozen_inputs_mismatch")
    validate_review(review, source, target, proposal)
    raw = {r["id"]: r for r in source["tokens"]}
    key = private_bytes(Path(files["encryption_key"]["path"])) if "encryption_key" in files else None
    for row in material["records"]:
        token = raw[row["source_token_id"]]
        require(digest(token) == row["source_row_sha256"] and token["deleted_at"] == row["source_deleted_at"],
                "credential_source_row_mismatch")
        if not row["one_time_reveal"]:
            verify_encrypted_secret(key, row, token["key"])


def native_users_sql() -> str:
    return """SELECT coalesce(jsonb_agg(jsonb_build_object('user_id',u.id,'personal_account_id',u.personal_account_id,
 'auth_version',u.auth_version,'active',u.active AND a.active,
 'developer_access_granted',coalesce(d.granted,false),'routing_groups',coalesce((SELECT jsonb_agg(ug.group_name ORDER BY ug.group_name)
 FROM core_identity.user_routing_groups ug JOIN public.routing_active_release r ON r.singleton
 JOIN public.routing_groups g ON g.version=r.config_version AND g.name=ug.group_name
 WHERE ug.user_id=u.id AND ug.enabled AND g.enabled),'[]'::jsonb)) ORDER BY u.id),'[]'::jsonb)
 FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id AND a.kind='personal'
 LEFT JOIN core_identity.developer_access d ON d.user_id=u.id"""


def snapshot_sql(inventory: dict, migration_id: str) -> str:
    contract = IDENTITY.current_contract()
    return "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC';\nSELECT jsonb_build_object(" + \
        "'instance',(" + FIN.instance_sql() + "),'catalog_fingerprint_sha256',(" + IDENTITY.catalog_hash_sql(contract) + ")," + \
        "'ownership_sha256',(" + FIN.ownership_sql() + "),'component_fingerprints'," + \
        "(SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract)," + \
        "'trust_policy_seed',(" + IDENTITY.trust_policy_seed_query() + "),'state',(" + FIN.state_sql(inventory, migration_id) + ")," + \
        "'native_users',(" + native_users_sql() + "),'identity_import'," + \
        "(SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE manifest->>'scope'='identity-only'),'financial_import'," + \
        "(SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE manifest->>'scope' IN (" + \
        literal(FIN.SCOPE) + "," + literal(FIN.ESCROW_SCOPE) + "))); COMMIT;\n"


def _sql_body(material: dict) -> str:
    first_budget = _next_id(material["before_state"], CHANGED_SEQUENCES[1], len(material["records"]))
    inserts = []
    for offset, row in enumerate(material["records"]):
        cid, uid, aid = row["target_credential_id"], row["target_user_id"], row["target_personal_account_id"]
        quota = row["quota"]
        expiry = row["expiry"]["native_exclusive_epoch"]
        revoked = literal(row["source_deleted_at"]) + "::timestamptz" if row["revoked"] else "NULL"
        encrypted = "NULL" if row["one_time_reveal"] else "decode(" + literal(row["encrypted_secret_hex"]) + ",'hex')"
        array = lambda values: "ARRAY[" + ",".join(literal(v) for v in values) + "]::text[]"
        enabled = "TRUE" if row["enabled"] else "FALSE"
        once = "TRUE" if row["one_time_reveal"] else "FALSE"
        consumed, basis = quota["opening_consumed"], quota["display_limit_credits"]
        display = str(basis) if quota["unlimited"] else "NULL"
        unlimited = "TRUE" if quota["unlimited"] else "FALSE"
        access = f"to_timestamp({row['accessed_time']})" if row["accessed_time"] else "NULL"
        inserts.append(f"""
 INSERT INTO core_identity.credentials(id,digest,kind,user_id,user_version,owner_account_id,personal_billing_preference,
 expires_at,revoked_at,created_at,session_started_at) OVERRIDING SYSTEM VALUE VALUES
 ({cid},decode({literal(row['digest_sha256'])},'hex'),'api_key',{uid},{row['target_auth_version']},{aid},
 {literal(row['funding']['personal_billing_preference'])},to_timestamp({expiry}),{revoked},
 to_timestamp({row['created_time']}),to_timestamp({row['created_time']}));
 INSERT INTO core_identity.api_key_settings(credential_id,name,enabled,one_time_reveal,encrypted_secret,
 model_limits_enabled,model_limits,allow_ips,group_name,auto_groups,cross_group_retry,account_balance_read,accessed_at)
 VALUES({cid},{literal(row['name'])},FALSE,{once},{encrypted},FALSE,{array(row['model_limits'])},
 {array(row['allow_ips'])}::inet[],{literal(row['group']['value'])},{array(row['auto_groups']['values'])},
 {'TRUE' if row['cross_group_retry'] else 'FALSE'},{'TRUE' if row['account_balance_read'] else 'FALSE'},{access});
 INSERT INTO core_identity.key_funding_rules(credential_id,position,payer_account_id) VALUES({cid},0,{aid});
 INSERT INTO core_charging.budgets(id,scope,key_id,issuer_user_id,period,limit_credits,unlimited,display_limit_credits)
 OVERRIDING SYSTEM VALUE VALUES({first_budget + offset},'key',{cid},{uid},'lifetime',{quota['limit_credits']},{unlimited},{display});
 INSERT INTO core_charging.budget_opening_consumption(scope,key_id,period,start_at,end_at,consumed_credits,import_reference)
 VALUES('key',{cid},'lifetime',{QUOTA.LIFETIME_START},{QUOTA.LIFETIME_END},{consumed},
 {literal('key-quota:' + material['migration_id'] + ':' + str(cid))});
 UPDATE core_identity.api_key_settings SET enabled={enabled} WHERE credential_id={cid};
""")
    return "\n".join(inserts)


def _guard(material: dict) -> str:
    return FIN._guard(material) + f"""
 IF ({native_users_sql()}) IS DISTINCT FROM m->'target_projection'->'users'
 OR (SELECT count(*) FROM core_migration.import_runs WHERE manifest->>'scope'='identity-only')<>1
 OR NOT EXISTS(SELECT 1 FROM core_migration.import_runs r WHERE to_jsonb(r)=m->'identity_import')
 OR NOT EXISTS(SELECT 1 FROM core_migration.import_runs r WHERE to_jsonb(r)=m->'financial_import')
 OR EXISTS(SELECT 1 FROM jsonb_array_elements(m->'target_projection'->'users') u
  WHERE NOT EXISTS(SELECT 1 FROM core_billing.opening_imports o
   WHERE o.account_id=(u->>'personal_account_id')::bigint AND o.migration_id=m->'financial_import'->>'migration_id'
    AND o.request->>'source_archive_sha256'=m->>'source_archive_sha256'))
 THEN RAISE EXCEPTION 'credential_authority_or_financial_opening_changed'; END IF;
 IF {IDENTITY.trust_policy_seed_guard_sql(material['target'])}
 THEN RAISE EXCEPTION 'credential_trust_policy_changed'; END IF;
"""


def _readback_sql(material: dict) -> str:
    checks = []
    array = lambda values: "ARRAY[" + ",".join(literal(v) for v in values) + "]::text[]"
    first_budget = _next_id(material["before_state"], CHANGED_SEQUENCES[1], len(material["records"]))
    for offset, row in enumerate(material["records"]):
        cid, uid, aid = row["target_credential_id"], row["target_user_id"], row["target_personal_account_id"]
        q = row["quota"]
        encrypted = "NULL" if row["one_time_reveal"] else "decode(" + literal(row["encrypted_secret_hex"]) + ",'hex')"
        revoked = literal(row["source_deleted_at"]) + "::timestamptz" if row["revoked"] else "NULL"
        accessed = f"to_timestamp({row['accessed_time']})" if row["accessed_time"] else "NULL"
        checks.append(f"""
 IF NOT EXISTS(SELECT 1 FROM core_identity.credentials c JOIN core_identity.api_key_settings s ON s.credential_id=c.id
 WHERE c.id={cid} AND c.digest=decode({literal(row['digest_sha256'])},'hex') AND c.kind='api_key'
 AND c.user_id={uid} AND c.user_version={row['target_auth_version']} AND c.owner_account_id={aid}
 AND c.personal_billing_preference={literal(row['funding']['personal_billing_preference'])}
 AND c.expires_at=to_timestamp({row['expiry']['native_exclusive_epoch']}) AND c.revoked_at IS NOT DISTINCT FROM {revoked}
 AND c.created_at=to_timestamp({row['created_time']}) AND c.session_started_at=to_timestamp({row['created_time']})
 AND s.name={literal(row['name'])} AND s.enabled={'TRUE' if row['enabled'] else 'FALSE'}
 AND s.one_time_reveal={'TRUE' if row['one_time_reveal'] else 'FALSE'} AND s.encrypted_secret IS NOT DISTINCT FROM {encrypted}
 AND NOT s.model_limits_enabled AND s.model_limits={array(row['model_limits'])} AND s.allow_ips={array(row['allow_ips'])}::inet[]
 AND s.group_name={literal(row['group']['value'])} AND s.auto_groups={array(row['auto_groups']['values'])}
 AND s.cross_group_retry={'TRUE' if row['cross_group_retry'] else 'FALSE'}
 AND s.account_balance_read={'TRUE' if row['account_balance_read'] else 'FALSE'} AND s.accessed_at IS NOT DISTINCT FROM {accessed})
 OR (SELECT count(*) FROM core_identity.key_funding_rules WHERE credential_id={cid})<>1
 OR NOT EXISTS(SELECT 1 FROM core_identity.key_funding_rules WHERE credential_id={cid} AND position=0 AND payer_account_id={aid})
 OR (SELECT count(*) FROM core_charging.budgets WHERE key_id={cid})<>1
 OR NOT EXISTS(SELECT 1 FROM core_charging.budgets WHERE id={first_budget + offset} AND scope='key' AND key_id={cid}
 AND account_id IS NULL AND user_id IS NULL AND issuer_user_id={uid} AND period='lifetime' AND timezone='UTC'
 AND anchor=0 AND seconds=0 AND revision=1 AND limit_credits={q['limit_credits']}
 AND unlimited={'TRUE' if q['unlimited'] else 'FALSE'} AND display_limit_credits IS NOT DISTINCT FROM
 {str(q['display_limit_credits']) if q['unlimited'] else 'NULL'}::bigint)
 OR (SELECT count(*) FROM core_charging.budget_opening_consumption WHERE key_id={cid})<>1
 OR NOT EXISTS(SELECT 1 FROM core_charging.budget_opening_consumption WHERE scope='key' AND key_id={cid}
 AND account_id IS NULL AND user_id IS NULL AND period='lifetime' AND timezone='UTC' AND anchor=0 AND seconds=0
 AND start_at={QUOTA.LIFETIME_START} AND end_at={QUOTA.LIFETIME_END} AND consumed_credits={q['opening_consumed']}
 AND import_reference={literal('key-quota:' + material['migration_id'] + ':' + str(cid))})
 THEN RAISE EXCEPTION 'credential_exact_readback_failed'; END IF;
""")
    checks.append(" IF " + " OR ".join(f"(SELECT count(*) FROM {FIN.relation(n)})<>{len(material['records'])}"
                                         for n in CHANGED_TABLES) +
                  " THEN RAISE EXCEPTION 'credential_row_coverage_mismatch'; END IF;")
    return "\n".join(checks)


def _seal_sql(sql: str) -> str:
    tag = "$lmm_credential_" + FIN.sha(sql.encode())[:24] + "$"
    require(tag not in sql, "credential_sql_delimiter_collision")
    prefix, separator, body = sql.partition("END $lmm_credential$;")
    require(bool(separator), "credential_sql_template_invalid")
    prefix = prefix.replace("DO $lmm_credential$", "DO " + tag, 1)
    body = body.replace("DO $lmm_credential$", "DO " + tag, 1)
    middle, separator, suffix = body.rpartition("END $lmm_credential$;")
    require(bool(separator), "credential_sql_template_invalid")
    return prefix + "END " + tag + ";" + middle + "END " + tag + ";" + suffix


def _start(*, readonly: bool) -> str:
    # Reuse the shared session fence, taken before choosing the snapshot.
    return FIN._start({}, readonly=readonly).replace("$lmm_financial$", "$lmm_credential$")


def build_apply_sql(material: dict) -> str:
    validate_material(material)
    inventory = material["before_state"]["inventory"]
    state = FIN.state_sql(inventory, material["migration_id"])
    changed = "ARRAY[" + ",".join(literal(n) for n in CHANGED_TABLES) + "]::text[]"
    sequences = "ARRAY[" + ",".join(literal(n) for n in CHANGED_SEQUENCES) + "]::text[]"
    restarts = "\n".join(f"ALTER SEQUENCE {FIN.relation(n)} RESTART WITH {_next_id(material['before_state'], n, len(material['records'])) + len(material['records'])};"
                            for n in CHANGED_SEQUENCES)
    sql = _start(readonly=False) + f"""
DO $lmm_credential$
DECLARE m jsonb := {literal(material)}::jsonb; previous core_migration.import_runs%ROWTYPE;
 before_state jsonb; after_state jsonb;
BEGIN
 {_guard(material)}
 LOCK TABLE {','.join(FIN.relation(n) for n in inventory['tables'])} IN ACCESS EXCLUSIVE MODE;
 {_guard(material)}
 SELECT ({state}) INTO before_state;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256<>m->>'source_archive_sha256' OR previous.mapping_sha256<>m->>'mapping_sha256'
   OR previous.manifest->>'scope' IS DISTINCT FROM {literal(SCOPE)}
   OR previous.manifest->'material' IS DISTINCT FROM m OR previous.manifest->'post_state' IS DISTINCT FROM before_state
  THEN RAISE EXCEPTION 'credential_replay_conflict'; END IF;
 ELSE
  IF before_state IS DISTINCT FROM m->'before_state' THEN RAISE EXCEPTION 'credential_target_state_changed'; END IF;
  IF EXISTS(SELECT 1 FROM core_migration.import_runs WHERE manifest->>'scope'={literal(SCOPE)})
   THEN RAISE EXCEPTION 'credential_other_import_conflict'; END IF;
  {_sql_body(material)}
  {restarts}
  SET CONSTRAINTS ALL IMMEDIATE;
  SELECT ({state}) INTO after_state;
  IF (before_state->'tables') - {changed} IS DISTINCT FROM (after_state->'tables') - {changed}
   OR (before_state->'sequences') - {sequences} IS DISTINCT FROM (after_state->'sequences') - {sequences}
  THEN RAISE EXCEPTION 'credential_unrelated_state_changed'; END IF;
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
  VALUES(m->>'migration_id',m->>'source_archive_sha256',m->>'mapping_sha256',
   jsonb_build_object('scope',{literal(SCOPE)},'material',m,'post_state',after_state));
 END IF;
 {_readback_sql(material)}
 {_guard(material)}
END $lmm_credential$;
COMMIT;
SELECT jsonb_build_object('status','COMMITTED','migration_id',{literal(material['migration_id'])},
 'material_sha256',{literal(digest(material))});
"""
    return _seal_sql(sql)


def build_verify_sql(material: dict, application_name: str) -> str:
    validate_material(material)
    require(type(application_name) is str and re.fullmatch(r"lmm-credential-[0-9a-f]{32}", application_name),
            "credential_original_attempt_invalid")
    state = FIN.state_sql(material["before_state"]["inventory"], material["migration_id"])
    return _seal_sql(_start(readonly=True) + f"""
DO $lmm_credential$
DECLARE m jsonb := {literal(material)}::jsonb; previous core_migration.import_runs%ROWTYPE; actual jsonb;
BEGIN
 {_guard(material)}
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name={literal(application_name)}
  AND datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid())
 THEN RAISE EXCEPTION 'credential_original_attempt_active'; END IF;
 SELECT ({state}) INTO actual;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256<>m->>'source_archive_sha256' OR previous.mapping_sha256<>m->>'mapping_sha256'
   OR previous.manifest->>'scope' IS DISTINCT FROM {literal(SCOPE)}
   OR previous.manifest->'material' IS DISTINCT FROM m OR previous.manifest->'post_state' IS DISTINCT FROM actual
  THEN RAISE EXCEPTION 'credential_verify_conflict'; END IF;
  {_readback_sql(material)}
 ELSIF actual IS DISTINCT FROM m->'before_state' THEN RAISE EXCEPTION 'credential_absence_not_proven'; END IF;
END $lmm_credential$;
SELECT jsonb_build_object('status',CASE WHEN EXISTS(SELECT 1 FROM core_migration.import_runs
 WHERE migration_id={literal(material['migration_id'])}) THEN 'COMMITTED' ELSE 'NOT_APPLIED' END,
 'migration_id',{literal(material['migration_id'])},'material_sha256',{literal(digest(material))});
COMMIT;
""")


def approval_for(material: dict, sql: str, decision="REVIEW_REQUIRED") -> dict:
    return {"format_version": 1, "scope": SCOPE, "decision": decision,
        "material_sha256": digest(material), "sql_sha256": FIN.sha(sql.encode()),
        "source_review_sha256": digest(material["source_review"])}


def build_alias_manifest(material: dict, receipt_bytes: bytes) -> dict:
    """Bind Core's exact alias schema to the canonical committed receipt bytes."""
    require(type(receipt_bytes) is bytes and receipt_bytes ==
            (canonical(material["credential_receipt"]) + "\n").encode("utf-8"),
            "credential_receipt_bytes_changed")
    source_file = material["source_files"]["source"]
    source_bytes = private_bytes(Path(source_file["path"]))
    require(FIN.sha(source_bytes) == source_file["sha256"], "credential_source_file_changed")
    try:
        source = json.loads(source_bytes, object_pairs_hook=PROPOSAL.unique_object)
    except (ValueError, UnicodeError, RecursionError):
        raise Error("private_json_invalid") from None
    validate_wire_source(source)
    alias_digest(material["source_archive_sha256"], "credential_alias_source_digest_invalid")
    tokens = {token["id"]: token for token in source["tokens"]}
    require(len(tokens) == len(source["tokens"]) == len(material["records"]), "credential_alias_source_coverage_mismatch")
    target = alias_target(material["target"]["instance"])
    keys, wire_digests, secrets = [], set(), []
    require(1 <= len(material["records"]) <= MAX_ALIAS_KEYS, "credential_alias_key_count_invalid")
    for row in material["records"]:
        token = tokens.get(row["source_token_id"])
        require(type(token) is dict and digest(token) == row["source_row_sha256"], "credential_source_row_mismatch")
        raw = token["key"]
        require(ordinary_secret(raw), "credential_ordinary_wire_contract_unsupported")
        raw_hash = alias_digest(FIN.sha(raw.encode("utf-8")))
        alias_hash = alias_digest(FIN.sha(("sk-" + raw).encode("utf-8")))
        require(raw_hash == row["digest_sha256"], "credential_alias_secret_digest_mismatch")
        for value in (raw_hash, alias_hash):
            require(value not in wire_digests, "credential_alias_digest_collision")
            wire_digests.add(value)
        keys.append({"source_key_id": row["source_token_id"], "target_credential_id": row["target_credential_id"],
            "target_user_id": row["target_user_id"], "target_account_id": row["target_personal_account_id"],
            "credential_user_version": row["target_auth_version"], "source_secret_sha256": raw_hash,
            "sk_alias_sha256": alias_hash})
        secrets.append(raw)
    manifest = {"version": 1, "scope": "legacy-api-key-wire-aliases-v1",
        "source_archive_sha256": material["source_archive_sha256"],
        "credential_receipt_sha256": alias_digest(FIN.sha(receipt_bytes)), "target": target, "keys": keys}
    rendered = canonical(manifest)
    require(not any(secret in rendered or "sk-" + secret in rendered for secret in secrets), "credential_alias_secret_echo")
    return manifest


def cleanup_outputs(output: Path, names: list[str]) -> dict:
    """Keep deleting after individual failures and report observed residual state."""
    report = {"attempted_files": list(names), "absent_files": [], "remaining_files": [],
              "unconfirmed_files": [], "unlink_failed_files": [], "directory_fsynced": False}
    for name in names:
        try:
            (output / name).unlink(missing_ok=True)
        except OSError:
            report["unlink_failed_files"].append(name)
    try:
        descriptor = os.open(output, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(descriptor)
            report["directory_fsynced"] = True
        finally:
            os.close(descriptor)
    except OSError:
        pass
    for name in names:
        try:
            (output / name).lstat()
            report["remaining_files"].append(name)
        except FileNotFoundError:
            report["absent_files"].append(name)
        except OSError:
            report["unconfirmed_files"].append(name)
    return report


def _publication_failure(outcome: dict, cleanup: dict) -> None:
    outcome["status"] = "UNKNOWN"
    outcome["publication_error_code"] = "credential_artifact_publication_failed"
    outcome["publication_cleanup"] = cleanup
    outcome["publication_incomplete"] = bool(cleanup["remaining_files"] or cleanup["unconfirmed_files"]
                                              or not cleanup["directory_fsynced"])


def publish_committed_outputs(material: dict, outcome: dict, output: Path) -> None:
    """Publish both Core inputs only after a verified COMMITTED acknowledgement."""
    require(outcome == {"status": "COMMITTED", "migration_id": material["migration_id"],
                        "material_sha256": digest(material)}, "credential_artifacts_require_commit")
    receipt_text = canonical(material["credential_receipt"]) + "\n"
    receipt_bytes = receipt_text.encode("utf-8")
    manifest = build_alias_manifest(material, receipt_bytes)
    manifest_text = canonical(manifest) + "\n"
    audit = {"format_version": 1, "scope": "legacy-credential-committed-artifacts-v1", **outcome,
        "source_archive_sha256": material["source_archive_sha256"], "target": manifest["target"],
        "credential_count": len(manifest["keys"]), "credential_receipt_sha256": FIN.sha(receipt_bytes),
        "alias_manifest_sha256": FIN.sha(manifest_text.encode("utf-8"))}
    contents = {MAPPING_FILE: receipt_text, ALIAS_FILE: manifest_text, ARTIFACT_AUDIT_FILE: canonical(audit) + "\n"}
    require(not any((output / name).exists() or (output / name).is_symlink() for name in contents),
            "credential_artifact_output_exists")
    created = []
    try:
        for name, value in contents.items():
            created.append(name)
            FIN.write_private(output, name, value)
            require(private_bytes(output / name) == value.encode("utf-8"), "credential_artifact_readback_mismatch")
    except Exception:
        # These names were absent before this exclusive publication attempt.
        # Keep SQL/attempt diagnostics and report any input that could not be removed.
        raise PublicationError(cleanup_outputs(output, created)) from None


def execute(material: dict, *, original_sql: str, approval: dict, connection: dict,
            output: Path, original_attempt: dict | None = None) -> dict:
    validate_material(material)
    require(original_sql == build_apply_sql(material), "credential_sql_changed")
    require(approval == approval_for(material, original_sql, "approve-offline-credential-and-quota-import"),
            "credential_approval_mismatch")
    verify_source_files(material)
    owner, login = IDENTITY.connection_roles(connection)
    instance = material["target"]["instance"]
    require(owner == instance["role"] and login == instance["session_user"]
        and connection.get("database") == instance["database"] and connection.get("port") == instance["port"],
        "credential_connection_binding_mismatch")
    IDENTITY.socket_connection(connection)
    attempt = {"format_version": 1, "migration_id": material["migration_id"], "material_sha256": digest(material),
        "sql_sha256": FIN.sha(original_sql.encode()), "application_name": "lmm-credential-" + uuid.uuid4().hex,
        "operation": "verify" if original_attempt else "apply"}
    sql = original_sql
    if original_attempt is not None:
        require(original_attempt.get("operation") == "apply" and all(original_attempt.get(k) == attempt[k]
                for k in ("migration_id", "material_sha256", "sql_sha256")), "credential_original_attempt_material_mismatch")
        sql = build_verify_sql(material, original_attempt.get("application_name"))
    FIN.write_private(output, "attempt.private.json", canonical(attempt) + "\n")
    FIN.write_private(output, "executed.private.sql", sql)
    outcome = {"status": "UNKNOWN", "migration_id": material["migration_id"], "material_sha256": digest(material)}
    published = False
    try:
        code, stdout = FIN.run_sql(connection, sql, output, attempt["application_name"])
        if code == 0:
            verify_source_files(material)
            require(source_hashes() == material["tool_source_hashes"], "credential_tool_source_changed")
            result = json.loads(stdout)
            if type(result) is dict and set(result) == set(outcome) and result.get("status") in (
                    ("COMMITTED", "NOT_APPLIED") if original_attempt else ("COMMITTED",)) and all(
                    result.get(k) == outcome[k] for k in ("migration_id", "material_sha256")):
                outcome = result
        if outcome["status"] == "COMMITTED":
            publish_committed_outputs(material, outcome, output)
            published = True
    except Exception as error:
        outcome["status"] = "UNKNOWN"
        if isinstance(error, PublicationError):
            _publication_failure(outcome, error.cleanup)
    try:
        FIN.write_private(output, "result.private.json", canonical(outcome) + "\n")
    except Exception:
        outcome["status"] = "UNKNOWN"
        if published:
            # The commit remains resolvable from its original durable attempt.
            _publication_failure(outcome, cleanup_outputs(output, [MAPPING_FILE, ALIAS_FILE, ARTIFACT_AUDIT_FILE]))
        # Do not replace the failed result file or lose the original attempt identity.
        try:
            FIN.write_private(output, "publication-status.private.json", canonical(outcome) + "\n")
        except Exception:
            outcome["publication_status_persisted"] = False
    return outcome


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--prepare", action="store_true")
    mode.add_argument("--apply", action="store_true")
    mode.add_argument("--verify-attempt", type=Path)
    for name in (*FILES, "encryption-key", "target-receipt", "material", "sql", "approval"):
        parser.add_argument("--" + name.replace("_", "-"), type=Path)
    parser.add_argument("--migration-id")
    parser.add_argument("--runtime-role", action="append")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        require(args.target_receipt is not None, "credential_target_receipt_required")
        if args.prepare:
            require(all(getattr(args, name) for name in FILES) and args.migration_id and args.runtime_role,
                    "credential_prepare_arguments_missing")
            require(all(getattr(args, name) is None for name in ("material", "sql", "approval")),
                    "credential_execution_arguments_on_prepare")
        else:
            require(all((args.material, args.sql, args.approval)), "credential_execution_arguments_missing")
            require(all(getattr(args, name) is None for name in (*FILES, "encryption_key", "migration_id", "runtime_role")),
                    "credential_prepare_arguments_on_execution")
        IDENTITY.private_backup_path(args.output.parent, directory=True)
        args.output.mkdir(mode=0o700)
        output = IDENTITY.private_backup_path(args.output, directory=True)
        target = load_json(args.target_receipt)
        require(target.get("application_or_worker_started") is False, "credential_runtime_not_stopped")
        connection = target["connection"]
        if args.prepare:
            inputs = {name: load_json(getattr(args, name)) for name in FILES}
            validate_wire_source(inputs["source"])
            alias_target(target["instance_identity"])
            require(PROPOSAL.plan_api_keys(inputs["source"], inputs["target_projection"]) == inputs["proposal"],
                    "credential_proposal_rebuild_mismatch")
            validate_review(inputs["review"], inputs["source"], inputs["target_projection"], inputs["proposal"])
            _supported_rows(inputs["proposal"], inputs["target_projection"], 1)
            files = {name: {"path": str(IDENTITY.private_backup_path(getattr(args, name))),
                           "sha256": FIN.sha(private_bytes(getattr(args, name)))} for name in FILES}
            key = None
            if args.encryption_key:
                key = private_bytes(args.encryption_key)
                files["encryption_key"] = {"path": str(args.encryption_key.resolve()), "sha256": FIN.sha(key)}
            code, raw = FIN.run_sql(connection, "BEGIN READ ONLY;" + FIN.inventory_sql() + ";COMMIT;",
                                    output, "lmm-credential-preview", readonly=True, label="inventory")
            require(code == 0, "credential_inventory_read_failed")
            code, raw = FIN.run_sql(connection, snapshot_sql(json.loads(raw), args.migration_id), output,
                                    "lmm-credential-preview", readonly=True, label="snapshot")
            require(code == 0, "credential_snapshot_read_failed")
            snapshot = json.loads(raw)
            IDENTITY.verify_instance(snapshot["instance"], connection, target["instance_identity"], source=False)
            material = prepare_material(**inputs, migration_id=args.migration_id, snapshot=snapshot,
                runtime_roles=sorted(args.runtime_role), source_files=files, encryption_key=key)
            verify_source_files(material)
            sql = build_apply_sql(material)
            FIN.write_private(output, "material.private.json", canonical(material) + "\n")
            FIN.write_private(output, "credentials.private.sql", sql)
            FIN.write_private(output, "approval-required.private.json", canonical(approval_for(material, sql)) + "\n")
            result = {"status": "PREPARED_NOT_APPLIED", "material_sha256": digest(material), "target_writes": 0}
        else:
            result = execute(load_json(args.material), original_sql=private_bytes(args.sql).decode("utf-8"),
                approval=load_json(args.approval), connection=connection, output=output,
                original_attempt=load_json(args.verify_attempt) if args.verify_attempt else None)
        print(canonical(result))
        return 2 if result["status"] == "UNKNOWN" else 0
    except (OSError, ValueError, TypeError, KeyError, Error, PROPOSAL.ProposalError) as error:
        code = str(error) if isinstance(error, (Error, PROPOSAL.ProposalError)) else "credential_input_or_validation_failed"
        if not re.fullmatch(r"[a-z][a-z0-9_]{0,127}", code):
            code = "credential_input_or_validation_failed"
        print(canonical({"status": "REFUSED", "error_code": code}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
