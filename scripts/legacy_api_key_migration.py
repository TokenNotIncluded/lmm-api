#!/usr/bin/env python3
"""Read-only legacy API-key proposals; never connect, import, or activate.

Inputs are explicit private projections of a frozen source and observed target.
The three-field ID-map contract matches migrate-legacy-snapshot.py. A proposal
retains unsupported semantics as blockers, rather than silently dropping them.
Raw key material is used only to calculate SHA256 and check for accidental echo.
"""

from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime
import hashlib
import importlib.util
import ipaddress
import json
import os
from pathlib import Path
import re
import stat
import sys
from typing import Any


LEGACY_REVISION = "cc52aa195c6f2a01d703a1a407be73c31b89867d"
MAX_I64 = 9223372036854775807
NEVER = 253402300799
HEX = re.compile(r"[0-9a-f]{64}\Z")
SECRET = re.compile(r"[A-Za-z0-9_-]{32,256}\Z")
SOURCE_FIELDS = {"format_version", "source_revision", "source_archive_sha256",
                 "source_schema_sha256", "source_export_sha256", "source_projection_sha256",
                 "snapshot_at", "tokens", "users", "policy"}
TOKEN_FIELDS = {"id", "user_id", "key", "status", "name", "created_time",
                "accessed_time", "expired_time", "remain_quota", "unlimited_quota",
                "one_time_reveal", "account_balance_read", "model_limits_enabled",
                "model_limits", "allow_ips", "used_quota", "group", "cross_group_retry",
                "auto_groups", "oauth_managed", "creation_source", "deleted_at"}
USER_FIELDS = {"id", "status", "deleted_at", "auth_version", "group", "billing_preference"}
TARGET_FIELDS = {"format_version", "source_archive_sha256", "source_target_id_map",
                 "source_target_id_map_sha256", "users"}
TARGET_USER_FIELDS = {"user_id", "personal_account_id", "auth_version", "active",
                      "developer_access_granted", "routing_groups"}
MAP_FIELDS = {"source_user_id", "target_user_id", "target_personal_account_id"}
POLICY_FIELDS = {"auto_groups", "max_token_auto_groups", "user_model_rate_limit_enabled"}
CREATION_SOURCES = {None, "", "manual", "system", "drawing_mcp", "assistant",
                    "assistant_runtime", "red_packet_cover"}
PREFERENCES = {"subscription_first", "wallet_first", "subscription_only", "wallet_only"}


class ProposalError(ValueError):
    """Only fixed error codes may leave the private input boundary."""


def require(condition: bool, code: str) -> None:
    if not condition:
        raise ProposalError(code)


def canonical(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True,
                      separators=(",", ":"), allow_nan=False)


def sha256(value: Any) -> str:
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def fields(value: Any, expected: set[str], code: str) -> None:
    require(type(value) is dict and set(value) == expected, code)


def integer(value: Any, code: str, minimum: int = 0, maximum: int = MAX_I64) -> int:
    require(type(value) is int and minimum <= value <= maximum, code)
    return value


def boolean(value: Any, code: str) -> bool:
    require(type(value) is bool, code)
    return value


def utf8(value: Any, code: str) -> bytes:
    require(type(value) is str, code)
    try:
        return value.encode("utf-8")
    except UnicodeError:
        raise ProposalError(code) from None


def text(value: Any, code: str, maximum: int, *, empty: bool = True) -> str:
    require(len(utf8(value, code)) <= maximum
            and (empty or bool(value)) and not any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in value), code)
    return value


def deleted(value: Any) -> bool:
    if value in (None, ""):
        return False
    require(type(value) is str, "deleted_at_invalid")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        raise ProposalError("deleted_at_invalid") from None
    require(parsed.tzinfo is not None, "deleted_at_timezone_missing")
    return True


def groups(value: Any, code: str, maximum: int) -> list[str]:
    require(type(value) is list and len(value) <= maximum, code)
    result = [text(item, code, 64, empty=False) for item in value]
    require(len(set(result)) == len(result) and "auto" not in result, code)
    return result


def model_limits(raw: Any) -> list[str]:
    # Go's token model map splits only on commas, without trimming entries.
    text(raw, "model_limits_invalid", 65536)
    return [] if raw == "" else raw.split(",")


def ip_limits(raw: Any) -> list[str]:
    if raw is None:
        return []
    require(len(utf8(raw, "allow_ips_invalid")) <= 65536, "allow_ips_invalid")
    values = [part.strip().replace(",", "") for part in raw.replace(" ", "").split("\n")]
    result = []
    for value in filter(None, values):
        try:
            # PostgreSQL INET retains host bits; do not widen to its network.
            parsed = ipaddress.ip_interface(value)
        except ValueError:
            raise ProposalError("allow_ips_invalid") from None
        result.append(str(parsed))
    require(len(result) <= 64 and len(set(result)) == len(result), "allow_ips_native_limit")
    return result


def id_map_bytes(rows: list[dict]) -> str:
    return "".join(canonical(row) + "\n" for row in rows)


def target_mapping(target: dict, source_hash: str) -> tuple[dict, dict]:
    fields(target, TARGET_FIELDS, "target_fields_unknown_or_missing")
    require(type(target["format_version"]) is int and target["format_version"] == 1,
            "target_version_unsupported")
    require(target["source_archive_sha256"] == source_hash, "target_source_hash_mismatch")
    rows = target["source_target_id_map"]
    require(type(rows) is list and type(target["users"]) is list, "target_rows_invalid")
    mappings, users, accounts = {}, {}, set()
    for row in rows:
        fields(row, MAP_FIELDS, "id_map_fields_unknown_or_missing")
        for value in row.values():
            integer(value, "id_map_id_invalid", 1)
        source_id = row["source_user_id"]
        require(source_id not in mappings, "id_map_source_duplicate")
        mappings[source_id] = row
    require(list(mappings) == sorted(mappings), "id_map_order_invalid")
    expected_hash = hashlib.sha256(id_map_bytes(rows).encode()).hexdigest()
    require(target["source_target_id_map_sha256"] == expected_hash, "id_map_hash_mismatch")
    for row in target["users"]:
        fields(row, TARGET_USER_FIELDS, "target_user_fields_unknown_or_missing")
        uid = integer(row["user_id"], "target_user_id_invalid", 1)
        account = integer(row["personal_account_id"], "target_account_id_invalid", 1)
        integer(row["auth_version"], "target_auth_version_invalid", 1)
        boolean(row["active"], "target_active_invalid")
        boolean(row["developer_access_granted"], "target_developer_access_invalid")
        groups(row["routing_groups"], "target_groups_invalid", 64)
        require(uid not in users and account not in accounts, "target_user_or_account_duplicate")
        users[uid] = row
        accounts.add(account)
    require(len({row["target_user_id"] for row in rows}) == len(rows), "id_map_target_duplicate")
    require(len({row["target_personal_account_id"] for row in rows}) == len(rows), "id_map_account_duplicate")
    for row in rows:
        observed = users.get(row["target_user_id"])
        require(observed is not None and observed["personal_account_id"] == row["target_personal_account_id"],
                "id_map_target_observation_mismatch")
    return mappings, users


def plan_api_keys(source: dict, target: dict) -> dict:
    """Return a deterministic proposal, never a ready/apply/activation receipt."""
    fields(source, SOURCE_FIELDS, "source_fields_unknown_or_missing")
    require(type(source["format_version"]) is int and source["format_version"] == 1,
            "source_version_unsupported")
    require(source["source_revision"] == LEGACY_REVISION, "source_revision_not_reviewed")
    for name in ("source_archive_sha256", "source_schema_sha256", "source_export_sha256", "source_projection_sha256"):
        require(type(source[name]) is str and HEX.fullmatch(source[name]) is not None, "source_hash_invalid")
    now = integer(source["snapshot_at"], "snapshot_time_invalid", 0, NEVER - 1)
    require(type(source["tokens"]) is list and type(source["users"]) is list, "source_rows_invalid")
    policy = source["policy"]
    fields(policy, POLICY_FIELDS, "policy_fields_unknown_or_missing")
    groups(policy["auto_groups"], "policy_auto_groups_invalid", 64)
    integer(policy["max_token_auto_groups"], "policy_max_auto_groups_invalid", 1, 64)
    boolean(policy["user_model_rate_limit_enabled"], "policy_rate_limit_invalid")
    mappings, observed_users = target_mapping(target, source["source_archive_sha256"])
    users = {}
    for row in source["users"]:
        fields(row, USER_FIELDS, "user_fields_unknown_or_missing")
        uid = integer(row["id"], "source_user_id_invalid", 1)
        require(uid not in users, "source_user_duplicate")
        require(type(row["status"]) is int and row["status"] in (1, 2), "source_user_status_unknown")
        deleted(row["deleted_at"])
        integer(row["auth_version"], "source_auth_version_invalid", 1)
        text(row["group"], "source_user_group_invalid", 64, empty=False)
        require(row["billing_preference"] is None or (type(row["billing_preference"]) is str
                and row["billing_preference"] in PREFERENCES | {""}),
                "billing_preference_unknown")
        users[uid] = row
    records, reasons, seen_ids, seen_digests, secrets = [], Counter(), set(), set(), []
    for token in source["tokens"]:
        fields(token, TOKEN_FIELDS, "token_fields_unknown_or_missing")
        sid = integer(token["id"], "source_token_id_invalid", 1)
        require(sid not in seen_ids, "source_token_id_duplicate")
        seen_ids.add(sid)
        uid = integer(token["user_id"], "token_user_id_invalid", 1)
        require(uid in users and uid in mappings, "token_owner_unmapped")
        user, mapping = users[uid], mappings[uid]
        observed = observed_users[mapping["target_user_id"]]
        raw = token["key"]
        secret_bytes = utf8(raw, "token_secret_invalid")
        require(1 <= len(secret_bytes) <= 256 and "\0" not in raw, "token_secret_invalid")
        digest = hashlib.sha256(secret_bytes).hexdigest()
        require(digest not in seen_digests, "token_digest_duplicate")
        seen_digests.add(digest)
        secrets.append(raw)
        status = token["status"]
        require(type(status) is int and status in (1, 2, 3, 4), "token_status_unknown")
        name = text(token["name"], "token_name_invalid", 128, empty=False)
        created = integer(token["created_time"], "token_created_time_invalid", 0, now)
        accessed = integer(token["accessed_time"], "token_accessed_time_invalid", 0, now)
        expiry = integer(token["expired_time"], "token_expiry_invalid", -1, NEVER - 1)
        remaining = integer(token["remain_quota"], "token_remaining_invalid", -MAX_I64 - 1)
        used = integer(token["used_quota"], "token_used_invalid")
        basis = integer(used + remaining, "token_quota_basis_overflow", -MAX_I64 - 1)
        for field in ("unlimited_quota", "one_time_reveal", "account_balance_read",
                      "model_limits_enabled", "cross_group_retry", "oauth_managed"):
            boolean(token[field], "token_boolean_invalid")
        require(token["creation_source"] is None or (type(token["creation_source"]) is str
                and token["creation_source"] in CREATION_SOURCES), "token_creation_source_unknown")
        is_deleted = deleted(token["deleted_at"])
        models = model_limits(token["model_limits"])
        ips = ip_limits(token["allow_ips"])
        selected = text(token["group"], "token_group_invalid", 64)
        auto_raw = token["auto_groups"]
        # The nullable SQL column is read into Go's zero-value string. Retain
        # that source fact while applying its known empty-string semantics.
        auto_sql_null = auto_raw is None
        if auto_sql_null:
            auto_raw = ""
        require(len(utf8(auto_raw, "token_auto_groups_invalid")) <= 65536, "token_auto_groups_invalid")
        blocked = []
        if not name.strip():
            blocked.append("token_name_native_shape_unsupported")
        if SECRET.fullmatch(raw) is None or "-" in raw:
            blocked.append("legacy_secret_requires_wire_adapter")
        ordinary = not (token["oauth_managed"] or token["creation_source"] == "assistant_runtime"
                        or raw.startswith("oauth_managed_"))
        if not ordinary:
            blocked.append("managed_credential_requires_separate_migration")
        if token["creation_source"] not in (None, "", "manual") or "的初始令牌" in name:
            blocked.append("creation_source_metadata_requires_adapter")
        if not token["one_time_reveal"]:
            blocked.append("recoverable_secret_encryption_required")
        if not observed["active"]:
            blocked.append("target_owner_inactive")
        source_enabled = user["status"] == 1 and not deleted(user["deleted_at"])
        if not source_enabled and observed["active"]:
            blocked.append("source_owner_disabled_or_deleted_target_active")
        if observed["auth_version"] != user["auth_version"]:
            blocked.append("owner_auth_version_changed")
        if not observed["developer_access_granted"]:
            blocked.append("target_developer_grant_missing")
        if policy["user_model_rate_limit_enabled"]:
            blocked.append("user_aggregate_rate_limit_requires_adapter")
        if len(models) > 200 or len(set(models)) != len(models) or any(not m or len(m.encode()) > 200 for m in models):
            blocked.append("model_limits_native_shape_unsupported")
        if token["model_limits_enabled"]:
            blocked.append("model_name_matching_requires_adapter")
        if selected == "":
            blocked.append("dynamic_user_group_requires_adapter")
        try:
            auto = [] if auto_raw == "" else json.loads(auto_raw)
        except (ValueError, RecursionError):
            auto = "invalid_json"
        if auto == "invalid_json":
            auto_mode, candidates = "deny_all_invalid_json", []
            blocked.append("invalid_auto_groups_deny_all_requires_adapter")
        else:
            # encoding/json accepts null into []string as nil, so Go falls
            # through to dynamic global Auto just as it does for an empty list.
            if auto is None:
                auto = []
            candidates = groups(auto, "token_auto_groups_shape_invalid", 64)
            auto_mode = "dynamic_global" if not candidates else "ordered_snapshot"
        effective_group = selected or user["group"]
        if effective_group == "auto":
            if auto_mode == "dynamic_global":
                blocked.append("dynamic_global_auto_groups_requires_adapter")
            if len(candidates) > 16 or len(candidates) > policy["max_token_auto_groups"]:
                blocked.append("auto_groups_filtering_requires_adapter")
            if len(candidates) > 1:
                blocked.append("cross_group_retry_semantics_require_verification")
            required_groups = candidates
        else:
            required_groups = [effective_group]
            if candidates or token["cross_group_retry"]:
                blocked.append("inactive_auto_settings_require_metadata_adapter")
        if any(group not in observed["routing_groups"] for group in required_groups):
            blocked.append("target_routing_grant_missing")
        usable = ordinary and status == 1 and not is_deleted and source_enabled and (expiry == -1 or expiry >= now) and (token["unlimited_quota"] or remaining > 0)
        preference = user["billing_preference"] or "subscription_first"
        record = {"source_token_id": sid, "source_user_id": uid,
                  "target_user_id": mapping["target_user_id"],
                  "target_personal_account_id": mapping["target_personal_account_id"],
                  "target_auth_version": observed["auth_version"], "digest_sha256": digest,
                  "name": name, "created_time": created, "accessed_time": accessed,
                  "source_status": status, "source_deleted": is_deleted,
                  "source_ordinary_api_key": ordinary,
                  "source_stored_key_roundtrips_relay_parser": "-" not in raw,
                  "source_owner_enabled": source_enabled, "source_usable_at_snapshot": bool(usable),
                  "enabled": status == 1, "revoked": is_deleted,
                  "expiry": {"kind": "never" if expiry == -1 else "inclusive_epoch_second",
                             "source_epoch": expiry, "native_exclusive_epoch": NEVER if expiry == -1 else expiry + 1},
                  "quota": {"remaining": remaining, "opening_consumed": used,
                            "display_limit_credits": basis, "limit_credits": 0 if token["unlimited_quota"] else basis,
                            "unlimited": token["unlimited_quota"],
                            "source_unit": "source_quota_integer", "native_unit": "credit_500k_usd",
                            "unit_conversion_approved": False},
                  "funding": {"personal_billing_preference": preference, "account_order": [mapping["target_personal_account_id"]]},
                  "one_time_reveal": token["one_time_reveal"], "account_balance_read": token["account_balance_read"],
                  "model_limits_enabled": token["model_limits_enabled"], "model_limits": models,
                  "allow_ips": ips, "group": {"mode": "explicit" if selected else "dynamic_user", "value": selected},
                  "auto_groups": {"mode": auto_mode, "values": candidates},
                  "source_auto_groups_sql_null": auto_sql_null,
                  "cross_group_retry": token["cross_group_retry"], "creation_source": token["creation_source"],
                  "blocked_reasons": sorted(set(blocked))}
        records.append(record)
        reasons.update(record["blocked_reasons"])
    records.sort(key=lambda row: row["source_token_id"])
    output = {"format_version": 1, "scope": "legacy-api-key-proposal", "status": "blocked" if reasons else "proposal",
              "apply_supported": False, "activation_approved": False,
              "source_revision": source["source_revision"], "source_archive_sha256": source["source_archive_sha256"],
              "source_schema_sha256": source["source_schema_sha256"], "source_input_sha256": sha256(source),
              "source_export_sha256": source["source_export_sha256"],
              "source_projection_sha256": source["source_projection_sha256"],
              "target_input_sha256": sha256(target), "source_target_id_map_sha256": target["source_target_id_map_sha256"],
              "snapshot_at": now, "policy_sha256": sha256(policy), "records": records,
              "required_before_apply": ["reviewed_wire_compatibility", "frozen_source_or_verified_delta",
                                        "independent_real_database_acceptance", "funding_and_routing_reconciliation",
                                        "credit_unit_and_basis_review", "source_group_authority_reconciliation"],
              "summary": {"source_token_count": len(records), "dropped_token_count": 0,
                          "source_usable_count": sum(row["source_usable_at_snapshot"] for row in records),
                          "source_usable_scope": "credential_state_only_not_model_admission",
                          "blocked_token_count": sum(bool(row["blocked_reasons"]) for row in records),
                          "blocked_reason_counts": dict(sorted(reasons.items())), "rows_written": 0}}
    output["proposal_sha256"] = sha256(output)
    rendered = canonical(output)
    require(not any(secret in rendered for secret in secrets), "secret_echo_in_metadata")
    return output


def private_json_with_digest(path: Path) -> tuple[dict, str]:
    require(not path.is_symlink(), "input_symlink_rejected")
    with path.open("rb") as stream:
        info = os.fstat(stream.fileno())
        require(stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid() and info.st_mode & 0o077 == 0,
                "input_not_private_owned_file")
        data = stream.read()
    try:
        parsed = json.loads(data, object_pairs_hook=unique_object)
    except (ValueError, UnicodeError, RecursionError):
        raise ProposalError("input_json_invalid") from None
    return parsed, hashlib.sha256(data).hexdigest()


def private_json(path: Path) -> dict:
    return private_json_with_digest(path)[0]


def unique_object(pairs: list[tuple[str, Any]]) -> dict:
    result = {}
    for key, value in pairs:
        require(key not in result, "json_duplicate_field")
        result[key] = value
    return result


def source_projection_from_export(tables: dict, proof: dict, *, snapshot_at: int,
                                  runtime_observation: dict, provenance: dict) -> tuple[dict, dict]:
    """Project only an export already checked by reuse_frozen_export()."""
    require(runtime_observation.get("source_revision") == LEGACY_REVISION
            and runtime_observation.get("buildinfo_matches_live_binary") is True
            and runtime_observation.get("source_archive_sha256") == proof["source_archive_sha256"],
            "legacy_runtime_source_binding_invalid")
    require(set(provenance) == {"source_receipt_sha256", "source_export_proof_sha256", "legacy_runtime_observation_sha256"},
            "projection_provenance_invalid")
    require(all(type(value) is str and HEX.fullmatch(value) for value in provenance.values()),
            "projection_provenance_invalid")
    require({"tokens", "users", "options"}.issubset(tables), "source_export_tables_missing")
    schemas = {name: proof["source_column_schema"][name] for name in ("tokens", "users", "options")}
    token_columns = {column["column_name"] for column in schemas["tokens"]}
    require(token_columns == TOKEN_FIELDS, "source_token_column_coverage_mismatch")
    options = {}
    for option in tables["options"]:
        fields(option, {"key", "value"}, "source_option_fields_unknown_or_missing")
        require(type(option["key"]) is str and option["key"] not in options, "source_option_duplicate")
        options[option["key"]] = option["value"]
    try:
        auto = json.loads(options.get("AutoGroups", '["default"]'), object_pairs_hook=unique_object)
        maximum = int(options.get("MaxTokenAutoGroups", "5"))
    except (ValueError, TypeError, RecursionError):
        raise ProposalError("source_auto_group_policy_unsupported") from None
    defaults = [name for name in ("AutoGroups", "MaxTokenAutoGroups", "ModelRequestRateLimitEnabled") if name not in options]
    users = []
    for row in tables["users"]:
        require(USER_FIELDS - {"billing_preference"} <= set(row) and "setting" in row,
                "source_user_projection_columns_missing")
        try:
            setting = json.loads(row["setting"] or "{}", object_pairs_hook=unique_object)
        except (ValueError, TypeError, RecursionError):
            raise ProposalError("source_user_setting_unsupported") from None
        require(type(setting) is dict, "source_user_setting_unsupported")
        users.append({**{name: row[name] for name in sorted(USER_FIELDS - {"billing_preference"})},
                      "billing_preference": setting.get("billing_preference")})
    source = {"format_version": 1, "source_revision": LEGACY_REVISION,
              "source_archive_sha256": proof["source_archive_sha256"], "source_schema_sha256": sha256(schemas),
              "source_export_sha256": proof["source_export_sha256"], "snapshot_at": snapshot_at,
              "tokens": tables["tokens"], "users": users,
              "policy": {"auto_groups": [] if auto is None else auto, "max_token_auto_groups": maximum,
                         "user_model_rate_limit_enabled": options.get("ModelRequestRateLimitEnabled", "false") == "true"}}
    receipt = {"format_version": 1, "scope": "legacy-api-key-source-projection", **provenance,
               "snapshot_at": snapshot_at,
               "source_revision": LEGACY_REVISION, "source_archive_sha256": proof["source_archive_sha256"],
               "source_export_sha256": proof["source_export_sha256"], "source_schema_sha256": source["source_schema_sha256"],
               "source_instance_sha256": sha256(proof["source_instance"]), "metadata_hashes": proof["metadata_hashes"],
               "source_token_columns": sorted(token_columns), "source_user_projection_fields": sorted(USER_FIELDS),
               "policy_compiled_defaults_used": defaults, "source_queries_this_run": 0,
               "source_row_counts": {name: len(tables[name]) for name in ("tokens", "users", "options")}}
    source["source_projection_sha256"] = sha256(receipt)
    return source, receipt


def source_export_provenance_cli(source_receipt: Path, export_proof: Path,
                                 runtime_observation: Path, *, snapshot_at: int) -> tuple[dict, dict]:
    """Reuse the existing archive/instance/schema/all-row verifier; query no DB."""
    receipt, receipt_hash = private_json_with_digest(source_receipt)
    observed, observation_hash = private_json_with_digest(runtime_observation)
    expected_proof, proof_hash = private_json_with_digest(export_proof)
    # Share the routing loader's bounded-memory reuse of financial-export
    # authority. It checks every exported table; retain only our three tables.
    spec = importlib.util.spec_from_file_location("legacy_credential_frozen_export",
                                                Path(__file__).with_name("legacy_routing_migration.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    hashes = {source_receipt: receipt_hash, export_proof: proof_hash, runtime_observation: observation_hash}
    try:
        tables, proof, _ = module.load_routing_export(receipt, export_proof, hashes,
                                                    retained_tables=("users", "tokens", "options"))
        require(proof == expected_proof, "source_export_proof_changed_during_read")
        _, _, _, _, schemas = module.F.metadata(receipt)
        require(all(module.F.digest(proof["source_column_schema"][table]) == module.F.digest(schemas[table])
                    for table in ("users", "tokens", "options")), "source_column_schema_mismatch")
        module.verify_files(hashes)
    except (module.F.MappingError, ValueError, TypeError, KeyError, RecursionError, OverflowError, AttributeError):
        raise ProposalError("frozen_source_export_verification_failed") from None
    provenance = {"source_receipt_sha256": hashes[source_receipt], "source_export_proof_sha256": hashes[export_proof],
                  "legacy_runtime_observation_sha256": hashes[runtime_observation]}
    projected = source_projection_from_export(tables, proof, snapshot_at=snapshot_at,
                                             runtime_observation=observed, provenance=provenance)
    module.verify_files(hashes)
    return projected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--target", type=Path, required=True)
    parser.add_argument("--source-receipt", type=Path, required=True)
    parser.add_argument("--source-export-proof", type=Path, required=True)
    parser.add_argument("--legacy-runtime-observation", type=Path, required=True)
    arguments = parser.parse_args()
    try:
        source = private_json(arguments.source)
        projected, _ = source_export_provenance_cli(arguments.source_receipt, arguments.source_export_proof,
                                                   arguments.legacy_runtime_observation, snapshot_at=source["snapshot_at"])
        require(source == projected, "source_export_projection_mismatch")
        result = plan_api_keys(source, private_json(arguments.target))
        print(canonical(result))
        return 2 if result["status"] == "blocked" else 0
    except (ValueError, TypeError, KeyError, RecursionError, OverflowError, AttributeError, OSError):
        print('{"status":"blocked","error":"invalid_private_proposal_input"}', file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
