#!/usr/bin/env python3
"""Build a sanitized identity cutover block from a private apply directory.

This tool only reads local files.  It never connects to PostgreSQL, SSH or a
provider, and it never prints the contents of a manifest, SQL file, target
receipt or verifier result.  A successful result contains counts and nine
SHA-256 bindings; incomplete evidence is reported as ``not_run`` or
``blocked``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any


PROTOCOL = "identity-cutover-evidence.v1"
SHA = re.compile(r"[0-9a-f]{64}\Z")
INVENTORY_FIELDS = (
    "sessions",
    "api_keys",
    "passkeys",
    "oauth_bindings",
    "oauth_server_tokens",
    "mfa_factors",
    "mfa_backup_codes",
)
ACTION_FIELDS = ("reauth_required", "rekey_required", "reenroll_required")
ACTION_BY_CLASS = {
    "sessions": "reauth_required",
    "api_keys": "rekey_required",
    "passkeys": "reenroll_required",
    "oauth_bindings": "reauth_required",
    "oauth_server_tokens": "reauth_required",
    "mfa_factors": "reenroll_required",
    "mfa_backup_codes": "reenroll_required",
}
IDENTITY_HASH_FIELDS = (
    "source_archive_sha256",
    "source_export_sha256",
    "source_baseline_proof_sha256",
    "mapping_sha256",
    "mapping_file_sha256",
    "source_target_id_map_sha256",
    "identity_contract_sha256",
    "target_import_sql_sha256",
    "target_receipt_sha256",
)
FILES = {
    "manifest": "manifest.private.json",
    "report": "report.private.json",
    "mapping": "identity-mapping.private.json",
    "source_export": "source-identity-export.private.jsonl",
    "source_baseline": "source-baseline-proof.private.json",
    "id_map": "source-target-id-map.private.jsonl",
    "sql": "identity-import.private.sql",
    "target_import": "target-import.private.jsonl",
    "target_receipt": "target-receipt.private.json",
    "independent": "independent-pass.private.json",
}
ACTUAL_INDEPENDENT = "independent-identity/identity-independent-result.private.json"
ACTUAL_STAGE_ACCEPTANCE = "independent-identity/IDENTITY-STAGE-INDEPENDENT-ACCEPTANCE.private.json"
ACTUAL_FROZEN_INPUTS = "independent-identity/frozen-inputs.private.json"


class EvidenceError(ValueError):
    """A fixed, non-sensitive evidence error."""


def unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise EvidenceError("duplicate JSON field")
        result[key] = value
    return result


def object(value: Any, required: tuple[str, ...], optional: tuple[str, ...] = ()) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EvidenceError("object required")
    allowed = set(required) | set(optional)
    if not set(required) <= set(value) or not set(value) <= allowed:
        raise EvidenceError("unexpected or missing JSON field")
    return value


def sha(value: Any) -> str:
    if not isinstance(value, str) or SHA.fullmatch(value) is None:
        raise EvidenceError("sha256 required")
    return value


def nonnegative(value: Any) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise EvidenceError("nonnegative integer required")
    return value


def load_json(path: Path) -> dict[str, Any]:
    try:
        with path.open("r", encoding="utf-8") as stream:
            value = json.load(stream, object_pairs_hook=unique)
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise EvidenceError("JSON evidence unreadable") from error
    if not isinstance(value, dict):
        raise EvidenceError("object required")
    return value


def digest(path: Path) -> str:
    try:
        data = path.read_bytes()
    except (OSError, ValueError) as error:
        raise EvidenceError("evidence file unreadable") from error
    return hashlib.sha256(data).hexdigest()


def require_file(root: Path, key: str) -> Path:
    candidates = [root / FILES[key]]
    if key == "target_receipt":
        candidates.extend((root.parent / FILES[key], root / "target-prep-v2" / FILES[key]))
    elif key == "independent":
        candidates.extend((root.parent / ACTUAL_INDEPENDENT, root.parent / ACTUAL_STAGE_ACCEPTANCE))
    for path in candidates:
        if path.is_file() and not path.is_symlink():
            return path
    raise EvidenceError("required evidence missing")


def action_state(value: Any) -> bool | int:
    if isinstance(value, bool):
        return value
    return nonnegative(value)


def state_covers(state: bool | int, required: int) -> bool:
    return state is True or (type(state) is int and state >= required)


def validate_inventory(independent: dict[str, Any]) -> tuple[dict[str, int], dict[str, str], dict[str, bool | int]]:
    inventory = object(independent["legacy_inventory"], INVENTORY_FIELDS)
    counts = {field: nonnegative(inventory[field]) for field in INVENTORY_FIELDS}
    actions = object(independent["actions"], INVENTORY_FIELDS)
    action_map = {}
    for field in INVENTORY_FIELDS:
        action = actions[field]
        if action not in ACTION_FIELDS:
            raise EvidenceError("identity action invalid")
        action_map[field] = action
        if action != ACTION_BY_CLASS[field]:
            raise EvidenceError("identity action classification invalid")
    states = {field: action_state(independent[field]) for field in ACTION_FIELDS}
    required = {field: 0 for field in ACTION_FIELDS}
    for kind, count in counts.items():
        required[action_map[kind]] += count
    for field, count in required.items():
        if count and not state_covers(states[field], count):
            raise EvidenceError("identity action coverage incomplete")
    return counts, action_map, states


def validate_target_receipt(receipt: dict[str, Any], migration_id: str, source_hash: str) -> None:
    if receipt.get("status") != "initialized-empty-identity-target":
        raise EvidenceError("target receipt status invalid")
    if receipt.get("migration_id") != migration_id or receipt.get("source_archive_sha256") != source_hash:
        raise EvidenceError("target receipt binding mismatch")
    if receipt.get("registry_provisioned") is not True or receipt.get("application_or_worker_started") is not False:
        raise EvidenceError("target receipt runtime state invalid")
    instance = receipt.get("instance_identity")
    if not isinstance(instance, dict):
        raise EvidenceError("target instance identity missing")
    for field in ("database_oid", "postmaster_started_at", "server_version_num", "server_instance_nonce"):
        if field not in instance:
            raise EvidenceError("target instance identity missing")
    database_oid = instance["database_oid"]
    if not (
        (isinstance(database_oid, int) and not isinstance(database_oid, bool) and database_oid > 0)
        or (isinstance(database_oid, str) and database_oid.isdigit() and int(database_oid) > 0)
    ):
        raise EvidenceError("target instance identity invalid")
    if not isinstance(instance["postmaster_started_at"], str) or not instance["postmaster_started_at"]:
        raise EvidenceError("target instance identity invalid")
    if not isinstance(instance["server_version_num"], str) or not instance["server_version_num"].isdigit():
        raise EvidenceError("target instance identity invalid")
    if not isinstance(instance["server_instance_nonce"], str) or SHA.fullmatch(instance["server_instance_nonce"]) is None:
        raise EvidenceError("target instance identity invalid")


def validate_target_import(path: Path, migration_id: str) -> None:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError) as error:
        raise EvidenceError("target import unreadable") from error
    lines = [line for line in lines if line.strip()]
    if len(lines) != 1:
        raise EvidenceError("target import result shape invalid")
    value = load_json_line(lines[0])
    if value.get("status") != "identity-import-verified" or value.get("migration_id") != migration_id:
        raise EvidenceError("target import result not verified")


def frozen_hashes(root: Path) -> dict[str, str]:
    candidates = [root / ACTUAL_FROZEN_INPUTS, root.parent / ACTUAL_FROZEN_INPUTS]
    for path in candidates:
        if not path.is_file() or path.is_symlink():
            continue
        value = load_json(path)
        if not value or any(not isinstance(key, str) or not isinstance(item, str) for key, item in value.items()):
            raise EvidenceError("frozen artifact hash map invalid")
        return {key: sha(item) for key, item in value.items()}
    return {}


def normalize_actual_independent(
    independent: dict[str, Any], root: Path, paths: dict[str, Path], source_hash: str
) -> dict[str, Any]:
    """Adapt the existing local readback receipt without weakening its checks.

    The real rehearsal stores PASS booleans and artifact hashes in sibling
    files, while the normalized synthetic contract carries them in one JSON
    object.  Missing legacy inventory remains ``not_run`` rather than being
    inferred from a dump or from Core's empty credential tables.
    """
    if "legacy_inventory" in independent:
        return independent
    required = {
        "status",
        "scope",
        "archive_and_all_frozen_artifact_hashes_verified",
        "binary_compiled_schema_manifest_verified",
        "exact_SQL_regenerated_equal",
        "no_write_or_SSH",
        "source_mapping_recomputed_equal",
        "source_users_equal_applied_export",
        "target_actual_instance_matches_receipt",
        "password_verifiers_exact",
        "privileged_facts_exact_19_fields",
        "users_exact_fields",
    }
    if set(independent) != required:
        raise EvidenceError("independent identity result shape invalid")
    if independent["status"] != "PASS" or independent["scope"] != "real-snapshot-local-identity-independent-readback":
        raise EvidenceError("independent identity result not PASS")
    boolean_fields = {
        "archive_and_all_frozen_artifact_hashes_verified",
        "binary_compiled_schema_manifest_verified",
        "exact_SQL_regenerated_equal",
        "no_write_or_SSH",
        "source_mapping_recomputed_equal",
        "source_users_equal_applied_export",
        "target_actual_instance_matches_receipt",
    }
    if any(independent[field] is not True for field in boolean_fields):
        raise EvidenceError("independent identity result incomplete")
    for field in ("password_verifiers_exact", "privileged_facts_exact_19_fields", "users_exact_fields"):
        if not isinstance(independent[field], int) or isinstance(independent[field], bool) or independent[field] < 0:
            raise EvidenceError("independent identity result incomplete")
    frozen = frozen_hashes(root)
    if not frozen:
        raise EvidenceError("frozen artifact hash map missing")
    for key in ("manifest", "report", "sql", "target_import", "target_receipt"):
        frozen_hash = frozen.get(str(paths[key].resolve()))
        if frozen_hash != digest(paths[key]):
            raise EvidenceError("frozen artifact hash mismatch")
    source_keys = [key for key in frozen if key.endswith("/source.dump")]
    if source_keys and frozen[source_keys[0]] != source_hash:
        raise EvidenceError("frozen source archive hash mismatch")
    normalized = dict(independent)
    normalized["manifest_sha256"] = frozen.get(str(paths["manifest"].resolve()))
    normalized["report_sha256"] = frozen.get(str(paths["report"].resolve()))
    normalized["identity_import_sql_sha256"] = frozen.get(str(paths["sql"].resolve()))
    normalized["target_import_sha256"] = frozen.get(str(paths["target_import"].resolve()))
    normalized["target_receipt_sha256"] = frozen.get(str(paths["target_receipt"].resolve()))
    normalized["source_archive_sha256"] = source_hash
    # A separate sanitized inventory may be supplied beside the apply result;
    # never derive counts by reading the source dump or target tables.
    inventory_candidates = [root / "identity-inventory.private.json", root.parent / "identity-inventory.private.json"]
    inventory_path = next((path for path in inventory_candidates if path.is_file() and not path.is_symlink()), None)
    if inventory_path is None:
        raise EvidenceError("legacy inventory evidence missing")
    normalized.update(load_json(inventory_path))
    return normalized


def load_json_line(line: str) -> dict[str, Any]:
    try:
        value = json.loads(line, object_pairs_hook=unique)
    except (json.JSONDecodeError, UnicodeError) as error:
        raise EvidenceError("JSON evidence unreadable") from error
    if not isinstance(value, dict):
        raise EvidenceError("object required")
    return value


def build(root: Path) -> dict[str, Any]:
    if not root.is_absolute() or not root.is_dir() or root.is_symlink():
        raise EvidenceError("evidence directory invalid")
    paths = {key: require_file(root, key) for key in FILES}
    manifest = load_json(paths["manifest"])
    report = load_json(paths["report"])
    independent = load_json(paths["independent"])
    target = load_json(paths["target_receipt"])

    if manifest.get("scope") != "identity-only" or manifest.get("format_version") != 2:
        raise EvidenceError("identity manifest scope invalid")
    migration_id = manifest.get("migration_id")
    if not isinstance(migration_id, str) or not migration_id:
        migration_id = report.get("migration_id")
    if not isinstance(migration_id, str) or not migration_id:
        raise EvidenceError("migration id missing")
    source_hash = sha(manifest.get("source_archive_sha256"))
    if report.get("source_archive_sha256") != source_hash or report.get("migration_id") != migration_id:
        raise EvidenceError("report binding mismatch")
    if report.get("status") != "identity-import-verified" or report.get("apply") is not True:
        raise EvidenceError("identity apply not verified")
    summary = manifest.get("summary")
    if not isinstance(summary, dict) or any(report.get(field) != value for field, value in summary.items()):
        raise EvidenceError("manifest report summary mismatch")
    if (
        report.get("mapping_sha256") != manifest.get("mapping_sha256")
        or report.get("mapping_file_sha256") != manifest.get("mapping_file_sha256")
        or report.get("identity_contract") != manifest.get("identity_contract")
    ):
        raise EvidenceError("manifest report artifact binding mismatch")
    validate_target_receipt(target, migration_id, source_hash)
    validate_target_import(paths["target_import"], migration_id)

    manifest_hash = digest(paths["manifest"])
    report_hash = digest(paths["report"])
    sql_hash = digest(paths["sql"])
    target_import_hash = digest(paths["target_import"])
    target_receipt_hash = digest(paths["target_receipt"])
    expected_file_hashes = {
        "source_export_sha256": paths["source_export"],
        "source_baseline_proof_sha256": paths["source_baseline"],
        "source_target_id_map_sha256": paths["id_map"],
        "mapping_file_sha256": paths["mapping"],
    }
    for field, path in expected_file_hashes.items():
        if sha(manifest.get(field)) != digest(path):
            raise EvidenceError("manifest artifact hash mismatch")
    identity_contract = sha(manifest.get("import_contract_sha256"))

    independent = normalize_actual_independent(independent, root, paths, source_hash)
    independent = object(
        independent,
        (
            "status", "scope", "source_frozen", "target_actual_instance_match",
            "mapping_status", "target_comparison_status", "legacy_inventory",
            *ACTION_FIELDS, "actions", "source_archive_sha256", "manifest_sha256",
            "report_sha256", "identity_import_sql_sha256", "target_import_sha256",
            "target_receipt_sha256",
        ),
    )
    if independent["status"] != "PASS" or independent["scope"] != "identity-cutover":
        raise EvidenceError("independent identity result not PASS")
    if independent["source_frozen"] is not True or independent["target_actual_instance_match"] is not True:
        raise EvidenceError("independent identity boundary missing")
    if independent["mapping_status"] != "passed" or independent["target_comparison_status"] != "passed":
        raise EvidenceError("independent identity comparison not PASS")
    if sha(independent["source_archive_sha256"]) != source_hash:
        raise EvidenceError("independent source binding mismatch")
    expected_independent_hashes = {
        "manifest_sha256": manifest_hash,
        "report_sha256": report_hash,
        "identity_import_sql_sha256": sql_hash,
        "target_import_sha256": target_import_hash,
        "target_receipt_sha256": target_receipt_hash,
    }
    for field, expected in expected_independent_hashes.items():
        if sha(independent[field]) != expected:
            raise EvidenceError("independent artifact hash mismatch")
    counts, actions, states = validate_inventory(independent)
    if not isinstance(summary, dict):
        raise EvidenceError("identity summary missing")
    zero_fields = ("sessions_imported", "api_keys_imported", "financial_rows_written")
    if any(type(summary.get(field)) is not int or summary[field] != 0 for field in zero_fields):
        raise EvidenceError("identity-only summary scope invalid")
    gaps = summary.get("functional_gaps")
    if not isinstance(gaps, list) or not {"legacy_oauth_and_passkeys_archived_only", "financial_data_not_migrated"}.issubset(gaps):
        raise EvidenceError("identity-only gaps missing")

    identity = {
        "migration_status": "identity_only",
        "mapping_status": independent["mapping_status"],
        "target_comparison_status": independent["target_comparison_status"],
        "legacy_inventory": counts,
        "source_archive_sha256": source_hash,
        "source_export_sha256": sha(manifest["source_export_sha256"]),
        "source_baseline_proof_sha256": sha(manifest["source_baseline_proof_sha256"]),
        "mapping_sha256": sha(manifest.get("mapping_sha256")),
        "mapping_file_sha256": sha(manifest["mapping_file_sha256"]),
        "source_target_id_map_sha256": sha(manifest["source_target_id_map_sha256"]),
        "identity_contract_sha256": identity_contract,
        "target_import_sql_sha256": sql_hash,
        "target_receipt_sha256": target_receipt_hash,
        **states,
        "actions": actions,
    }
    return {"protocol": PROTOCOL, "status": "PASS", "identity": identity, "blocking_reasons": []}


def blocked_result(status: str, reason: str) -> dict[str, Any]:
    return {"protocol": PROTOCOL, "status": status, "identity": None, "blocking_reasons": [reason]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True, help="Private identity apply directory")
    parser.add_argument("--output", type=Path, help="New sanitized output file")
    args = parser.parse_args()
    try:
        result = build(args.input)
    except EvidenceError as error:
        result = blocked_result("not_run" if "missing" in str(error) or "unreadable" in str(error) else "blocked", str(error))
    rendered = json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    try:
        if args.output:
            if args.output.exists():
                raise EvidenceError("output already exists")
            args.output.write_text(rendered, encoding="utf-8")
        else:
            sys.stdout.write(rendered)
    except (OSError, EvidenceError):
        parser.exit(2, "identity evidence output refused\n")
    return 0 if result["status"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
