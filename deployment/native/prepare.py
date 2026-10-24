#!/usr/bin/env python3
"""Offline native staging and evidence checks. Never connects, starts or migrates."""
import argparse
import datetime as dt
import email.utils
import hashlib
import html
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[2]
COMPONENTS = {"core", "extensions", "web"}
SHA = re.compile(r"[0-9a-f]{64}")
PREFLIGHT_GATES = {
    "release_provenance", "model_forwarding", "payments", "commerce",
    "assistant", "assistant_tools", "tool_market", "cutover_boundary", "identity",
}
IDENTITY_INVENTORY_FIELDS = (
    "sessions", "api_keys", "passkeys", "oauth_bindings",
    "oauth_server_tokens", "mfa_factors", "mfa_backup_codes",
)
IDENTITY_HASH_FIELDS = (
    "source_archive_sha256", "source_export_sha256",
    "source_baseline_proof_sha256", "mapping_sha256", "mapping_file_sha256",
    "source_target_id_map_sha256", "identity_contract_sha256",
    "target_import_sql_sha256", "target_receipt_sha256",
)
IDENTITY_ACTION_FIELDS = ("reauth_required", "rekey_required", "reenroll_required")
IDENTITY_ACTIONS = {
    "sessions": "reauth_required", "api_keys": "rekey_required",
    "passkeys": "reenroll_required", "oauth_bindings": "reauth_required",
    "oauth_server_tokens": "reauth_required", "mfa_factors": "reenroll_required",
    "mfa_backup_codes": "reenroll_required",
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON field")
        result[key] = value
    return result


def regular(path):
    require(path.is_file() and not path.is_symlink(), "expected regular file")
    # Parent symlinks must not redirect a reviewed evidence/artifact tree.
    require(all(not p.is_symlink() for p in path.parents), "symlink parent rejected")


def pinned_json(path, expected):
    regular(path)
    require(path.stat().st_mode & 0o022 == 0, "writable evidence rejected")
    require(SHA.fullmatch(expected) and digest(path) == expected, "pinned evidence digest mismatch")
    require(path.stat().st_size <= 8 * 1024 * 1024, "evidence too large")
    return json.loads(path.read_text(), object_pairs_hook=unique)


def inventory(directory):
    result = {}
    require(not directory.is_symlink(), "symlink directory rejected")
    for path in sorted(directory.rglob("*")):
        require(not path.is_symlink(), "symlink payload rejected")
        require(path.stat().st_mode & 0o022 == 0, "writable payload rejected")
        if path.is_file():
            result[str(path.relative_to(directory))] = {
                "sha256": digest(path), "mode": path.stat().st_mode & 0o777,
                "size": path.stat().st_size}
        else:
            require(path.is_dir(), "special payload rejected")
    return result


def stage(plan_path, expected, output):
    spec = importlib.util.spec_from_file_location("distribution", ROOT / "scripts/build-distribution.py")
    distribution = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(distribution)
    plan = pinned_json(plan_path, expected)
    require(plan.get("kind") == "lmm-native-artifact-plan-v1", "artifact plan kind invalid")
    items = plan.get("components", {})
    require(items and set(items) <= COMPONENTS, "component set invalid")
    require(not output.exists(), "output already exists")
    output.parent.mkdir(parents=True, exist_ok=True)
    # Copy first: verify and extract the same private bytes, never mutable source paths.
    with tempfile.TemporaryDirectory(prefix=".lmm-stage-", dir=output.parent) as temporary:
        temp = Path(temporary)
        result = {"kind": "lmm-native-stage-v1", "formal_release": False,
                  "artifact_plan_sha256": expected, "components": {}}
        for component, item in items.items():
            archive = Path(item["archive"])
            regular(archive)
            copy = temp / (component + ".tar.gz")
            shutil.copyfile(archive, copy)
            require(SHA.fullmatch(item["sha256"]) and digest(copy) == item["sha256"], "archive digest mismatch")
            manifest = distribution.verify(copy, item["source_revision"])
            require(manifest["component"] == component, "archive component mismatch")
            require(manifest["platform"] == ("any" if component == "web" else plan["platform"]), "archive platform mismatch")
            require(plan["platform"] in {"linux-amd64", "linux-arm64"}, "native platform invalid")
            dest = temp / "payload" / component
            dest.mkdir(parents=True)
            with tarfile.open(copy, "r:gz") as tar:
                for entry in tar:
                    require(entry.isfile(), "nonregular archive entry")
                    name = distribution.safe_path(entry.name)
                    target = dest / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with tar.extractfile(entry) as source, target.open("xb") as target_stream:
                        shutil.copyfileobj(source, target_stream)
                    target.chmod(entry.mode)
            result["components"][component] = {
                "version": manifest["version"], "source_revision": manifest["revision"],
                "platform": manifest["platform"], "preview": manifest["preview"],
                "archive_sha256": item["sha256"], "files": inventory(dest)}
        payload = temp / "payload"
        (payload / "release.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
        # Exclusive destination; no replacement of any earlier version.
        output.mkdir()
        for path in payload.iterdir():
            shutil.move(str(path), output / path.name)
    return digest(output / "release.json")


def release(directory, expected):
    record = pinned_json(directory / "release.json", expected)
    require(record.get("kind") == "lmm-native-stage-v1" and record.get("formal_release") is False,
            "unsupported release identity")
    require(record.get("components") and set(record["components"]) <= COMPONENTS, "component set invalid")
    require({p.name for p in directory.iterdir()} == set(record["components"]) | {"release.json"}, "unexpected release files")
    for component, item in record["components"].items():
        require(inventory(directory / component) == item["files"], "staged payload changed")
    return record


def verify_preflight_result(preflight):
    """Validate the sanitized result emitted by core-cutover-preflight.py.

    A pinned JSON file is provenance, not a schema check. Keep this boundary
    strict so a hand-written READY summary cannot bypass the identity or route
    gates that produced the result.
    """
    require(preflight.get("status") == "READY" and preflight.get("safe_to_cutover") is True,
            "cutover preflight must be READY")
    require(preflight.get("public_edge") == {"mode": "candidate", "candidate_selected": True},
            "cutover preflight edge selection is not candidate")
    gates = preflight.get("gates")
    require(isinstance(gates, list) and len(gates) == len(PREFLIGHT_GATES),
            "cutover preflight gate list is incomplete")
    names = []
    for gate in gates:
        require(isinstance(gate, dict) and set(gate) == {"name", "status", "missing"},
                "cutover preflight gate shape invalid")
        require(gate["name"] in PREFLIGHT_GATES and gate["name"] not in names,
                "cutover preflight gate name invalid")
        require(gate["status"] == "READY" and gate["missing"] == [],
                "cutover preflight contains a blocked gate")
        names.append(gate["name"])
    require(set(names) == PREFLIGHT_GATES, "cutover preflight gates are incomplete")
    require(preflight.get("unsafe_enablement") == [],
            "cutover preflight has unsafe enablement")
    require(preflight.get("secret_values_checked") == "schema_only",
            "cutover preflight secret boundary invalid")

    identity = preflight.get("identity")
    required_identity = {
        "migration_status", "mapping_status", "target_comparison_status",
        "legacy_inventory", *IDENTITY_HASH_FIELDS, *IDENTITY_ACTION_FIELDS, "actions",
    }
    require(isinstance(identity, dict) and set(identity) == required_identity,
            "cutover preflight identity shape invalid")
    require(identity["migration_status"] in {"identity_only", "completed"},
            "cutover preflight identity migration status invalid")
    require(identity["mapping_status"] == "passed"
            and identity["target_comparison_status"] == "passed",
            "cutover preflight identity comparison is incomplete")
    inventory_values = identity["legacy_inventory"]
    require(isinstance(inventory_values, dict)
            and set(inventory_values) == set(IDENTITY_INVENTORY_FIELDS),
            "cutover preflight identity inventory shape invalid")
    for value in inventory_values.values():
        require(type(value) is int and value >= 0,
                "cutover preflight identity inventory count invalid")
    for field in IDENTITY_HASH_FIELDS:
        require(isinstance(identity[field], str) and SHA.fullmatch(identity[field]),
                "cutover preflight identity hash invalid: " + field)
    for field in IDENTITY_ACTION_FIELDS:
        value = identity[field]
        require(value is True or (type(value) is int and value >= 0),
                "cutover preflight identity action count invalid: " + field)
    actions = identity["actions"]
    require(isinstance(actions, dict) and set(actions) == set(IDENTITY_INVENTORY_FIELDS),
            "cutover preflight identity actions shape invalid")
    for kind, action in IDENTITY_ACTIONS.items():
        require(actions[kind] == action,
                "cutover preflight identity action classification invalid: " + kind)


def instant(value):
    result = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    require(result.tzinfo is not None, "timestamp requires timezone")
    return result


def version(value):
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:-(alpha|beta|preview|rc)(?:\.(\d+))?)?", value)
    require(match is not None, "unsupported version: explicit version review required")
    major, minor, patch, label, number = match.groups()
    return (int(major), int(minor), int(patch), {"alpha": 0, "beta": 1, "preview": 2, "rc": 3, None: 4}[label], int(number or 0))


def verify_cutover_preflight(current, evidence, expected_source_commit=None):
    preflight_ref = evidence.get("cutover_preflight", {})
    require(isinstance(preflight_ref, dict), "cutover preflight reference required")
    path = preflight_ref.get("path")
    expected_sha = preflight_ref.get("sha256", "")
    require(isinstance(path, str) and path, "cutover preflight path required")
    path = Path(path)
    require(path.is_absolute(), "cutover preflight path must be absolute")
    require(path.stat().st_mode & 0o022 == 0, "cutover preflight writable evidence rejected")
    preflight = pinned_json(path, expected_sha)
    require(preflight.get("protocol") == "core-cutover-preflight.v1", "cutover preflight protocol invalid")
    verify_preflight_result(preflight)
    release = preflight.get("release", {})
    require(isinstance(release, dict), "cutover release provenance missing")
    source_commit = preflight.get("source_commit")
    require(isinstance(source_commit, str) and re.fullmatch(r"[0-9a-f]{40}", source_commit),
            "cutover source commit invalid")
    if expected_source_commit is not None:
        require(re.fullmatch(r"[0-9a-f]{40}", expected_source_commit),
                "expected cutover source commit invalid")
        require(source_commit == expected_source_commit,
                "cutover source commit does not match expected candidate")
    require(source_commit == release.get("source_commit") == release.get("runtime_source_commit"),
            "cutover source commit provenance mismatch")
    for field in (
        "core_artifact_sha256", "extensions_artifact_sha256", "sbom_sha256",
        "schema_fingerprint_sha256", "provider_matrix_sha256",
        "identity_evidence_sha256", "financial_evidence_sha256",
        "edge_config_sha256", "runtime_release_identity_sha256",
        "runtime_core_artifact_sha256", "runtime_extensions_artifact_sha256",
    ):
        require(isinstance(release.get(field), str) and SHA.fullmatch(release[field]),
                "cutover release provenance field invalid: " + field)
    require(type(release.get("writer_epoch")) is int
            and type(release.get("previous_writer_epoch")) is int
            and release["writer_epoch"] > release["previous_writer_epoch"],
            "cutover writer epoch is not monotonic")
    for component, artifact_field in (
        ("core", "core_artifact_sha256"),
        ("extensions", "extensions_artifact_sha256"),
    ):
        if component not in current["components"]:
            continue
        item = current["components"][component]
        require(item.get("source_revision") == preflight.get("source_commit"),
                component + " source commit does not match preflight")
        require(release.get(artifact_field) == item.get("archive_sha256"),
                component + " artifact does not match preflight")
        require(release.get("runtime_" + artifact_field) == item.get("archive_sha256"),
                component + " runtime artifact does not match preflight")
    manifest_ref = evidence.get("cutover_manifest", {})
    require(isinstance(manifest_ref, dict), "cutover release manifest reference required")
    manifest_path = manifest_ref.get("path")
    manifest_sha = manifest_ref.get("sha256", "")
    require(isinstance(manifest_path, str) and manifest_path and Path(manifest_path).is_absolute(),
            "absolute cutover release manifest path required")
    manifest = pinned_json(Path(manifest_path), manifest_sha)
    require(manifest.get("protocol") == "release-manifest.v1", "cutover release manifest protocol invalid")
    manifest_fields = (
        "source_commit", "core_artifact_sha256", "extensions_artifact_sha256",
        "sbom_sha256", "schema_fingerprint_sha256", "provider_matrix_sha256",
        "identity_evidence_sha256", "financial_evidence_sha256", "edge_config_sha256",
        "runtime_release_identity_sha256", "runtime_source_commit",
        "runtime_core_artifact_sha256", "runtime_extensions_artifact_sha256",
        "previous_writer_epoch", "writer_epoch",
    )
    for field in manifest_fields:
        require(manifest.get(field) == release.get(field),
                "cutover manifest binding mismatch: " + field)
    return expected_sha, manifest_sha


def gate(args):
    current = release(args.release_dir, args.release_sha256)
    evidence = pinned_json(args.review, args.review_sha256)
    require(evidence.get("kind") == "lmm-native-activation-v1", "activation record kind invalid")
    for key, value in {"status": "ready", "mode": args.mode, "host": args.host,
                       "target_database_identity_sha256": args.database_identity_sha256,
                       "release_sha256": args.release_sha256}.items():
        require(evidence.get(key) == value, "activation binding mismatch: " + key)
    require(SHA.fullmatch(args.database_identity_sha256), "database identity digest invalid")
    require(evidence.get("target_runtime_check_passed") is True,
            "actual restricted target check required; timeout is incomplete")
    require(evidence.get("all_valid_writes_preserved") is True, "valid writes retention unproven")
    configs = evidence.get("configuration_sha256", {})
    expected = {"/etc/lmm.conf.d/" + c + ".toml" for c in current["components"] if c != "web"}
    require(set(configs) == expected, "runtime configuration inventory mismatch")
    for path, expected_sha in configs.items():
        regular(Path(path))
        require(SHA.fullmatch(expected_sha) and digest(Path(path)) == expected_sha, "runtime configuration changed")
    if args.mode == "cutover":
        preflight_sha256 = None
        manifest_sha256 = None
        if any(component in current["components"] for component in ("core", "extensions")):
            expected_source_commit = getattr(args, "expected_source_commit", None)
            require(expected_source_commit is not None,
                    "expected source commit required for native cutover")
            preflight_sha256, manifest_sha256 = verify_cutover_preflight(
                current, evidence, expected_source_commit)
        backup = evidence.get("backup", {})
        require(SHA.fullmatch(backup.get("sha256", "")) and backup.get("restore_verified") is True,
                "backup restoration unverified")
        migration = evidence.get("migration", {})
        require(migration.get("status") == "completed" and migration.get("unresolved_count") == 0
                and migration.get("financial_reconciliation_passed") is True,
                "migration incomplete; dry-run is not completion")
        fence = evidence.get("final_fence", {})
        require(fence.get("writes_fenced") is True and fence.get("active_writers") == 0
                and fence.get("legacy_workers_stopped") is True, "live source not fenced")
        boundary = fence.get("source_final_boundary_sha256", "")
        require(SHA.fullmatch(boundary) and migration.get("source_final_boundary_sha256") == boundary,
                "final migrated write boundary mismatch")
        age = (dt.datetime.now(dt.timezone.utc) - instant(fence["observed_at"])).total_seconds()
        require(0 <= age <= 900, "final fence stale or future")
    else:
        require(args.previous_dir is not None and args.previous_sha256, "previous release required")
        previous = release(args.previous_dir, args.previous_sha256)
        require(evidence.get("previous_release_sha256") == args.previous_sha256, "previous identity mismatch")
        require(set(current["components"]) == set(previous["components"]), "component set change requires explicit review")
        for component, item in current["components"].items():
            old = previous["components"][component]
            if item != old:
                require(version(item["version"]) > version(old["version"]), "software may only upgrade")
        schema = evidence.get("schema", {})
        require(SHA.fullmatch(schema.get("before_sha256", ""))
                and schema.get("before_sha256") == schema.get("after_sha256")
                and schema.get("ordinary_startup_only") is True, "software update cannot migrate schema")
    return {"status": "activation-record-and-files-verified", "formal_release": False,
            "release_sha256": args.release_sha256, "evidence_sha256": args.review_sha256,
            "cutover_preflight_sha256": preflight_sha256 if args.mode == "cutover" else None,
            "cutover_manifest_sha256": manifest_sha256 if args.mode == "cutover" else None,
            "warning": "Hashes bind actual operator evidence, not publisher signatures or proof of migration truth."}


def maintenance(eta, output):
    until = instant(eta)
    now = dt.datetime.now(dt.timezone.utc)
    require(0 < (until - now).total_seconds() <= 86400, "ETA must be within next 24 hours")
    text = "服务维护中。预计恢复时间：" + html.escape(until.isoformat()) + "。请稍后重试。"
    body = "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><title>服务维护</title><p>" + text + "</p></html>"
    # JSON quoting is valid for this restricted Nginx literal (no dollar or backslash input).
    quoted = json.dumps(body, ensure_ascii=False)
    content = ('# Include at server scope, before existing location blocks.\n'
               'add_header Cache-Control "no-store" always;\n'
               'add_header Retry-After "' + email.utils.format_datetime(until.astimezone(dt.timezone.utc), usegmt=True) + '" always;\n'
               'default_type text/html;\nreturn 503 ' + quoted + ';\n')
    with output.open("x") as stream:
        stream.write(content)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    item = sub.add_parser("stage")
    item.add_argument("--plan", type=Path, required=True)
    item.add_argument("--plan-sha256", required=True)
    item.add_argument("--output", type=Path, required=True)
    item = sub.add_parser("maintenance")
    item.add_argument("--eta", required=True)
    item.add_argument("--output", type=Path, required=True)
    item = sub.add_parser("gate")
    item.add_argument("--mode", choices=["cutover", "update"], required=True)
    item.add_argument("--evidence", dest="review", type=Path, required=True)
    item.add_argument("--evidence-sha256", dest="review_sha256", required=True)
    for option in ["release-dir", "previous-dir"]:
        item.add_argument("--" + option, type=Path, required=option != "previous-dir")
    for option in ["release-sha256", "host", "database-identity-sha256"]:
        item.add_argument("--" + option, required=True)
    item.add_argument("--previous-sha256")
    item.add_argument("--expected-source-commit")
    args = parser.parse_args()
    try:
        if args.command == "stage":
            print(stage(args.plan, args.plan_sha256, args.output))
        elif args.command == "maintenance":
            maintenance(args.eta, args.output)
        else:
            print(json.dumps(gate(args), indent=2))
    except (ValueError, KeyError, OSError, TypeError) as error:
        # Do not echo artifact/evidence contents or credentials.
        parser.exit(2, "native preparation refused (" + type(error).__name__ + "). See reviewed contracts.\n")


if __name__ == "__main__":
    main()
