#!/usr/bin/env python3
"""Validate a sanitized Core/Extensions cutover record without touching a host.

The record deliberately contains booleans, enum values and SHA-256 bindings only.
It must never contain a token, password, provider payload, database URL or secret
file contents. A blocked result is a normal outcome and exits with status 1.
"""

import argparse
import datetime as dt
import json
import re
import sys
from pathlib import Path

PROTOCOL = "core-cutover-preflight.v1"
SHA = re.compile(r"[0-9a-f]{64}\Z")
CONTRACTS = (
    "model_forwarding",
    "payments",
    "commerce",
    "assistant",
    "assistant_tools",
    "tool_market",
)
KNOWN_CAPABILITIES = {
    "identity", "legacy_web_auth", "console", "humangrant", "models",
    "models_streaming", "events", "payments", "commerce", "abuse_admin",
    "admin_users_read", "images", "assistant", "assistant_tools",
    "tool_market", "web_identity", "oauth2", "market_oauth", "wallet",
    "bounties",
}
CAPABILITIES = {
    "model_forwarding": ("models", "models_streaming"),
    "payments": ("payments",),
    "commerce": ("commerce",),
    "assistant": ("assistant",),
    "assistant_tools": ("assistant_tools",),
    "tool_market": ("tool_market",),
}
ENUMS = {
    "config": {"absent", "reviewed"},
    "credentials": {"absent", "reviewed"},
    "contract": {"absent", "reviewed"},
    "probe": {"not_run", "passed"},
    "extension": {"absent", "reviewed"},
    "provider": {"none", "stripe-test", "external"},
    "migration_status": {"blocked", "completed"},
    "reconciliation": {"not_run", "passed"},
    "edge_mode": {"legacy", "candidate"},
}


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON field")
        result[key] = value
    return result


def object(value, required, optional=()):
    if not isinstance(value, dict):
        raise ValueError("object required")
    allowed = set(required) | set(optional)
    if not set(required) <= set(value) or not set(value) <= allowed:
        raise ValueError("unexpected or missing JSON field")
    return value


def boolean(value):
    if not isinstance(value, bool):
        raise ValueError("boolean required")
    return value


def enum(name, value):
    if value not in ENUMS[name]:
        raise ValueError("invalid " + name)
    return value


def sha(value, nullable=False):
    if nullable and value is None:
        return value
    if not isinstance(value, str) or not SHA.fullmatch(value):
        raise ValueError("sha256 required")
    return value


def nonnegative(value):
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise ValueError("nonnegative integer required")
    return value


def parse_contract(name, value):
    value = object(
        value,
        ("config", "credentials", "contract", "probe", "contract_sha256"),
        ("extension", "provider"),
    )
    for field in ("config", "credentials", "contract", "probe"):
        enum(field, value[field])
    if "extension" in value:
        enum("extension", value["extension"])
    else:
        value["extension"] = "absent"
    if "provider" in value:
        enum("provider", value["provider"])
    else:
        value["provider"] = "none"
    sha(value["contract_sha256"], nullable=True)
    if value["contract"] == "reviewed" and value["contract_sha256"] is None:
        raise ValueError(name + " reviewed contract needs a SHA-256 binding")
    if value["contract"] == "absent" and value["contract_sha256"] is not None:
        raise ValueError(name + " absent contract cannot have a SHA-256 binding")
    return value


def parse(record):
    record = object(record, ("protocol", "scope", "host", "observed_at", "source_revision", "check_db_evidence_sha256", "core", "extensions", "migration"))
    if record["protocol"] != PROTOCOL or record["scope"] != "production-cutover":
        raise ValueError("unsupported preflight protocol or scope")
    if not isinstance(record["observed_at"], str):
        raise ValueError("observed_at required")
    try:
        observed = dt.datetime.fromisoformat(record["observed_at"].replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("observed_at must be ISO-8601") from error
    if observed.tzinfo is None or observed > dt.datetime.now(dt.timezone.utc) + dt.timedelta(minutes=5):
        raise ValueError("observed_at must include a non-future timezone")
    if not isinstance(record["host"], str) or not re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", record["host"]):
        raise ValueError("invalid host")
    sha(record["source_revision"])
    sha(record["check_db_evidence_sha256"])

    core = object(record["core"], ("ready", "public_edge", "contracts"))
    ready = object(core["ready"], ("http_status", "database", "model_ready_status", "capabilities"))
    if not isinstance(ready["http_status"], int) or isinstance(ready["http_status"], bool):
        raise ValueError("invalid core ready status")
    if not isinstance(ready["model_ready_status"], int) or isinstance(ready["model_ready_status"], bool):
        raise ValueError("invalid model ready status")
    boolean(ready["database"])
    capabilities = ready["capabilities"]
    if not isinstance(capabilities, dict) or set(capabilities) - KNOWN_CAPABILITIES or any(capability not in capabilities for values in CAPABILITIES.values() for capability in values):
        raise ValueError("core capabilities are incomplete or unknown")
    for value in capabilities.values():
        boolean(value)
    edge = object(core["public_edge"], ("mode", "candidate_selected"))
    enum("edge_mode", edge["mode"])
    boolean(edge["candidate_selected"])
    contracts = core["contracts"]
    if not isinstance(contracts, dict) or set(contracts) != set(CONTRACTS):
        raise ValueError("all component contracts are required")
    core["contracts"] = {name: parse_contract(name, contracts[name]) for name in CONTRACTS}

    extensions = object(record["extensions"], ("ready_http_status", "modules", "host_credential"))
    if not isinstance(extensions["ready_http_status"], int) or isinstance(extensions["ready_http_status"], bool):
        raise ValueError("invalid extensions ready status")
    if not isinstance(extensions["modules"], list) or any(not isinstance(name, str) for name in extensions["modules"]):
        raise ValueError("invalid extension module listing")
    if len(set(extensions["modules"])) != len(extensions["modules"]):
        raise ValueError("duplicate extension module")
    enum("credentials", extensions["host_credential"])

    migration = object(record["migration"], ("status", "unknown_liabilities", "reconciliation", "writer_fenced", "active_writers", "legacy_workers_stopped"))
    enum("migration_status", migration["status"])
    enum("reconciliation", migration["reconciliation"])
    nonnegative(migration["unknown_liabilities"])
    nonnegative(migration["active_writers"])
    for field in ("writer_fenced", "legacy_workers_stopped"):
        boolean(migration[field])
    return record


def evaluate(record):
    core = record["core"]
    ready = core["ready"]
    caps = ready["capabilities"]
    contracts = core["contracts"]
    extension_modules = set(record["extensions"]["modules"])
    gates = []
    for name in CONTRACTS:
        missing = []
        for capability in CAPABILITIES[name]:
            if not caps[capability]:
                missing.append("core.capabilities." + capability)
        if ready["http_status"] != 200 or not ready["database"]:
            missing.append("core health/database")
        contract = contracts[name]
        if contract["config"] != "reviewed":
            missing.append(name + ".config")
        if contract["credentials"] != "reviewed":
            missing.append(name + ".credentials")
        if contract["contract"] != "reviewed":
            missing.append(name + ".contract")
        if contract["probe"] != "passed":
            missing.append(name + ".probe")
        if name == "model_forwarding" and ready["model_ready_status"] != 200:
            missing.append("core.health.model_ready")
        if name == "payments" and contract["provider"] in {"none", "stripe-test"}:
            missing.append("payments.external_provider_contract")
        if name == "assistant" and "assistant" not in extension_modules:
            missing.append("extensions.modules.assistant")
        if name == "assistant_tools":
            if "assistant" not in extension_modules:
                missing.append("extensions.modules.assistant")
            if "toolmarket" not in extension_modules:
                missing.append("extensions.modules.toolmarket")
            if not caps["assistant"]:
                missing.append("core.capabilities.assistant")
        if name == "tool_market" and "toolmarket" not in extension_modules:
            missing.append("extensions.modules.toolmarket")
        if contract["extension"] != "reviewed":
            missing.append(name + ".extension_contract")
        gates.append({"name": name, "status": "READY" if not missing else "BLOCKED", "missing": sorted(set(missing))})

    migration_missing = []
    migration = record["migration"]
    if migration["status"] != "completed":
        migration_missing.append("migration.status")
    if migration["unknown_liabilities"] != 0:
        migration_missing.append("migration.unknown_liabilities")
    if migration["reconciliation"] != "passed":
        migration_missing.append("migration.reconciliation")
    if not migration["writer_fenced"]:
        migration_missing.append("migration.writer_fenced")
    if migration["active_writers"] != 0:
        migration_missing.append("migration.active_writers")
    if not migration["legacy_workers_stopped"]:
        migration_missing.append("migration.legacy_workers_stopped")
    if core["public_edge"]["mode"] != "candidate" or not core["public_edge"]["candidate_selected"]:
        migration_missing.append("core.public_edge.candidate_selected")
    if record["extensions"]["ready_http_status"] != 200:
        migration_missing.append("extensions.health")
    gates.append({"name": "cutover_boundary", "status": "READY" if not migration_missing else "BLOCKED", "missing": sorted(set(migration_missing))})
    unsafe = [gate["name"] for gate in gates[:-1] if any(caps.get(cap, False) for cap in CAPABILITIES[gate["name"]]) and gate["status"] != "READY"]
    status = "READY" if all(gate["status"] == "READY" for gate in gates) else "BLOCKED"
    return {
        "protocol": PROTOCOL,
        "status": status,
        "safe_to_cutover": status == "READY",
        "host": record["host"],
        "source_revision": record["source_revision"],
        "gates": gates,
        "unsafe_enablement": sorted(set(unsafe)),
        "secret_values_checked": "schema_only",
    }


def run(path):
    if path.stat().st_size > 1024 * 1024:
        raise ValueError("preflight record too large")
    with path.open("r", encoding="utf-8") as stream:
        record = json.load(stream, object_pairs_hook=unique)
    return evaluate(parse(record))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        result = run(args.evidence)
        rendered = json.dumps(result, indent=2, sort_keys=True) + "\n"
        if args.output:
            if args.output.exists():
                raise ValueError("output already exists")
            args.output.write_text(rendered, encoding="utf-8")
        else:
            sys.stdout.write(rendered)
        return 0 if result["status"] == "READY" else 1
    except (OSError, ValueError, json.JSONDecodeError):
        parser.exit(2, "core cutover preflight refused; inspect the sanitized contract format\n")


if __name__ == "__main__":
    raise SystemExit(main())
