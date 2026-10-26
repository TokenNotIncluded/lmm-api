#!/usr/bin/env python3
"""Offline opening writer. It does not activate keys, subscriptions or runtime.

The planner stays SELECT-only. This module consumes a separately reviewed opening
and an exact target snapshot. SQL is private, deterministic and approved by hash.
A failed write process means UNKNOWN, not rollback. Resolve the original attempt
before deciding whether the same material may be submitted again.
"""
from __future__ import annotations

import argparse
from datetime import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import uuid

import legacy_financial_preflight as PREFLIGHT

_SPEC = importlib.util.spec_from_file_location(
    "legacy_financial_planner", Path(__file__).with_name("migrate-legacy-financial.py")
)
PLANNER = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(PLANNER)
IDENTITY = PLANNER.IDENTITY
Error = IDENTITY.MappingError
require = IDENTITY.require
SCOPE = "financial-opening-only-v1"
ESCROW_SCOPE = "financial-opening-with-legacy-escrow-v1"
ESCROW_POLICY = "restore-clearing-to-reserved-without-wallet-debit"
INFLIGHT_POLICY = "independently-reviewed-zero-nonescrow-inflight"
SEED = "identity-bound-fresh26-zero-ledger-v2"
FENCE = 741803121  # Shared with the offline identity importer.
TABLE_FILTER = "n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'"
CHANGED_TABLES = (
    "core_billing.ledger_accounts", "core_billing.balance_state",
    "core_billing.ledger_operations", "core_billing.ledger_journals",
    "core_billing.ledger_entries", "core_billing.opening_import_runs",
    "core_billing.opening_imports", "core_billing.account_opening_counters",
)
ESCROW_TABLES = (
    "core_billing.legacy_escrow_batch_runs", "core_billing.legacy_escrow_projects",
    "core_billing.legacy_escrow_claims", "core_billing.legacy_escrow_disputes",
    "core_billing.legacy_escrow_state",
)
ESCROW_SOURCE_FILES = frozenset({"escrow_candidate", "escrow_unit_evidence", "source_review_evidence",
    "escrow_policy_evidence", "inflight_evidence", "inflight_process_evidence"})
SOURCE_FILES = frozenset({"archive", "source_receipt", "financial_plan", "source_export",
    "source_export_proof", "identity_map", "identity_manifest", "source_review", "reservations"})
FRESH_EMPTY_TABLES = (
    "core_meta.schema_upgrades", "core_meta.component_upgrades", "core_meta.ledger_revisions",
    "core_trust_pricing.user_bases", "core_trust_pricing.legacy_paid_orders",
    "core_telegram_auth.bindings", "core_telegram_auth.flows", "core_telegram_auth.assertions",
)
CONTRACT_FIELDS = (
    "identity_version", "identity_fingerprint_sha256", "component_fingerprints",
    "catalog_query_sha256", "oauth_catalog_query_sha256", "installer_contract_sha256", "source_file_hashes",
)
CHANGED_SEQUENCES = (
    "core_billing.ledger_accounts_id_seq", "core_billing.ledger_journals_id_seq",
)


def canonical(value) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def digest(value) -> str:
    # Reuse the planner's type-tagged digest, including its integer/bool split.
    return PLANNER.digest(value)


def checked_hash(value, code="invalid_digest") -> str:
    require(type(value) is str and IDENTITY.HEX.fullmatch(value) is not None, code)
    return value


def literal(value) -> str:
    value = value if type(value) is str else canonical(value)
    require("\0" not in value, "nul_in_sql_material")
    return IDENTITY.sql_literal(value)


def relation(value: str) -> str:
    require(type(value) is str and re.fullmatch(r"[a-z_][a-z_0-9]*\.[a-z_][a-z_0-9]*", value) is not None,
            "invalid_relation_name")
    return ".".join('"' + part + '"' for part in value.split("."))


def inventory_sql() -> str:
    return f"""SELECT jsonb_build_object(
 'tables',coalesce(jsonb_agg(n.nspname||'.'||c.relname ORDER BY n.nspname,c.relname)
  FILTER(WHERE c.relkind IN ('r','p')),'[]'::jsonb),
 'sequences',coalesce(jsonb_agg(n.nspname||'.'||c.relname ORDER BY n.nspname,c.relname)
  FILTER(WHERE c.relkind='S'),'[]'::jsonb))
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {TABLE_FILTER}"""


def validate_inventory(inventory: dict) -> None:
    require(type(inventory) is dict and set(inventory) == {"tables", "sequences"}, "invalid_inventory")
    for kind, values in inventory.items():
        require(type(values) is list and values == sorted(set(values)), "invalid_inventory")
        for name in values:
            relation(name)
    require(set(CHANGED_TABLES) | set(FRESH_EMPTY_TABLES) | {"core_migration.import_runs", "core_identity.users", "core_trust_pricing.policies"}
            <= set(inventory["tables"]), "incomplete_inventory")
    require(set(CHANGED_SEQUENCES) <= set(inventory["sequences"]), "incomplete_inventory")


def state_sql(inventory: dict, migration_id: str) -> str:
    """Hash all application rows and sequence state, excluding only this receipt.

    Fixed-size row hashes preserve duplicates. Sorting uses UTF-8 bytes, not the
    database collation. Sequence parameters are included, not only last_value.
    """
    validate_inventory(inventory)
    tables = []
    for name in inventory["tables"]:
        where = f" WHERE t.migration_id<>{literal(migration_id)}" if name == "core_migration.import_runs" else ""
        tables.append(f"""SELECT {literal(name)} AS name,jsonb_build_object('rows',count(*),'sha256',
 encode(sha256(coalesce(string_agg(sha256(convert_to(to_jsonb(t)::text,'UTF8')),
 ''::bytea ORDER BY convert_to(to_jsonb(t)::text,'UTF8')),''::bytea)),'hex')) AS value
 FROM {relation(name)} t{where}""")
    sequences = []
    for name in inventory["sequences"]:
        sequences.append(f"""SELECT {literal(name)} AS name,jsonb_build_object(
 'last_value',s.last_value,'is_called',s.is_called,'start',p.seqstart,'increment',p.seqincrement,
 'min',p.seqmin,'max',p.seqmax,'cache',p.seqcache,'cycle',p.seqcycle) AS value
 FROM {relation(name)} s CROSS JOIN pg_sequence p WHERE p.seqrelid={literal(name)}::regclass""")
    return "SELECT jsonb_build_object('tables',(SELECT jsonb_object_agg(name,value) FROM (" + \
        " UNION ALL ".join(tables) + ") t),'sequences',(SELECT jsonb_object_agg(name,value) FROM (" + \
        " UNION ALL ".join(sequences) + ") s),'inventory',(" + inventory_sql() + "))"


def instance_sql() -> str:
    base = IDENTITY.IDENTITY_SQL.removesuffix(";").removeprefix("SELECT ")
    # pg_control_system requires an explicit offline grant to the NOLOGIN owner.
    return f"""SELECT ({base})::jsonb - 'read_only' || jsonb_build_object(
 'system_identifier',(SELECT system_identifier::text FROM pg_control_system()),
 'listen_addresses',current_setting('listen_addresses'))"""


def ownership_sql() -> str:
    return f"""SELECT encode(sha256(convert_to(coalesce(jsonb_agg(v ORDER BY v::text),'[]'::jsonb)::text,'UTF8')),'hex')
 FROM (
 SELECT jsonb_build_array('schema',n.nspname,pg_get_userbyid(n.nspowner),n.nspacl::text) v
 FROM pg_namespace n WHERE {TABLE_FILTER}
 UNION ALL SELECT jsonb_build_array('relation',n.nspname,c.relname,pg_get_userbyid(c.relowner),c.relacl::text)
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE {TABLE_FILTER}
 UNION ALL SELECT jsonb_build_array('function',n.nspname,p.proname,pg_get_function_identity_arguments(p.oid),
 pg_get_userbyid(p.proowner),p.proacl::text,p.prosecdef)
 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE {TABLE_FILTER}
 ) permissions"""


def snapshot_sql(inventory: dict, migration_id: str) -> str:
    contract = IDENTITY.current_contract()
    return "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL timezone='UTC';\n" + \
        "SELECT jsonb_build_object('instance',(" + instance_sql() + \
        "),'catalog_fingerprint_sha256',(" + IDENTITY.catalog_hash_sql(contract) + \
        "),'ownership_sha256',(" + ownership_sql() + \
        "),'component_fingerprints',(SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract)," + \
        "'trust_policy_seed',(" + IDENTITY.trust_policy_seed_query() + ")," + \
        "'state',(" + state_sql(inventory, migration_id) + ")," + \
        "'native_users',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'personal_account_id',personal_account_id) ORDER BY id),'[]'::jsonb) FROM core_identity.users)," + \
        "'import_runs',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY migration_id),'[]'::jsonb) FROM core_migration.import_runs r));\nCOMMIT;\n"


def source_hashes(*, escrow=False) -> dict:
    result = {**PLANNER.tool_source_hashes(), "financial_writer_sha256": IDENTITY.sha256_file(Path(__file__))}
    if escrow:
        for name in ("legacy_escrow_mapping.py", "legacy_bounty_basis.py"):
            result[name] = IDENTITY.sha256_file(Path(__file__).with_name(name))
    return result


def escrow_profile(material):
    return material.get("scope") == ESCROW_SCOPE


def validate_source_review(review, plan, *, candidate=None, unit_evidence=None, inflight=None):
    composed = candidate is not None or unit_evidence is not None
    fields = {"format_version", "scope", "decision", "source_archive_sha256", "mapping_sha256",
              "identity_map_sha256", "credits_per_usd", "evidence_sha256", "other_rights_policy"}
    if composed:
        fields |= {"escrow_candidate_sha256", "escrow_unit_evidence_sha256", "escrow_policy", "inflight_policy", "inflight_evidence_sha256"}
    require(type(review) is dict and set(review) == fields, "source_review_shape_invalid")
    require(type(review["format_version"]) is int and review["format_version"] == (2 if composed else 1)
            and review["scope"] == (ESCROW_SCOPE if composed else SCOPE), "source_review_profile_invalid")
    require(type(review["credits_per_usd"]) is int and review["credits_per_usd"] == 500000,
            "source_unit_invalid")
    require(review["other_rights_policy"] == ("all-other-rights-remain-disabled-pending-separate-import" if composed
                                             else "remain-disabled-pending-separate-import"),
            "source_other_rights_policy_invalid")
    require(review["decision"] == ("approved-offline-opening-with-legacy-escrow" if composed else "approved-offline-opening"),
            "source_review_required")
    checked_hash(review["evidence_sha256"], "source_review_evidence_missing")
    for key in ("source_archive_sha256", "mapping_sha256", "identity_map_sha256"):
        require(review[key] == plan.get(key), "source_review_binding_mismatch")
    if composed:
        require(type(candidate) is dict and type(unit_evidence) is dict
                and type(inflight) is dict and review["inflight_evidence_sha256"] == digest(inflight)
                and review["escrow_candidate_sha256"] == digest(candidate)
                and review["escrow_unit_evidence_sha256"] == digest(unit_evidence)
                and review["escrow_policy"] == ESCROW_POLICY and review["inflight_policy"] == INFLIGHT_POLICY,
                "escrow_independent_review_missing_or_mismatched")


def validate_review_evidence_files(material):
    files = material["source_files"]
    require(files["source_review_evidence"]["sha256"] == material["source_review"]["evidence_sha256"]
            and files["escrow_policy_evidence"]["sha256"] == material["escrow_unit_evidence"].get("source_policy_evidence_sha256"),
            "escrow_evidence_file_hash_mismatch")


def validate_inflight(material):
    evidence = material.get("inflight_evidence")
    require(type(evidence) is dict and set(evidence) == {"format_version", "scope", "basis", "snapshot_relation",
        "source_archive_sha256", "mapping_sha256", "source_export_sha256", "identity_map_sha256",
        "source_application_revision", "observed_at", "drain_completed_at", "source_process_identity",
        "process_evidence_sha256", "positions", "source_table_counts", "unresolved_operations"}, "inflight_evidence_shape_invalid")
    require(type(evidence["format_version"]) is int and evidence["format_version"] == 1
        and evidence["scope"] == "legacy-nonescrow-inflight-zero-evidence", "inflight_evidence_profile_invalid")
    require(evidence["basis"] == "source-application-drain-and-durable-state-reconciliation"
        and evidence["snapshot_relation"] == "after-drain", "inflight_source_drain_required")
    require(type(evidence["unresolved_operations"]) is list and evidence["unresolved_operations"] == [],
            "inflight_unresolved_operations")
    require(all(evidence[k] == material[k] for k in ("source_archive_sha256", "mapping_sha256", "identity_map_sha256"))
        and evidence["source_export_sha256"] == material["source_files"]["source_export"]["sha256"]
        and evidence["source_application_revision"] == material["escrow_unit_evidence"]["source_application_revision"],
        "inflight_source_binding_mismatch")
    observed = PLANNER.integer(evidence["observed_at"], "inflight_observation_invalid", minimum=1, maximum=253402300799)
    require(observed == material["escrow_batch"]["observed_at"], "inflight_source_binding_mismatch")
    PLANNER.integer(evidence["drain_completed_at"], "inflight_drain_not_before_capture", minimum=1, maximum=observed)
    process = evidence["source_process_identity"]
    require(type(process) is dict and set(process) == {"pid", "started_at", "binary_sha256"}, "inflight_process_identity_missing")
    PLANNER.integer(process["pid"], "inflight_process_identity_invalid", minimum=1)
    PLANNER.integer(process["started_at"], "inflight_process_identity_invalid", minimum=1, maximum=evidence["drain_completed_at"])
    checked_hash(process["binary_sha256"], "inflight_process_identity_invalid")
    require(type(evidence["positions"]) is list and len(evidence["positions"]) == len(material["counters"]),
            "inflight_account_coverage_incomplete")
    expected = {row["user_id"] for row in material["counters"]}
    seen = set()
    for row in evidence["positions"]:
        require(type(row) is dict and set(row) == {"source_user_id", "reserved_units"}, "inflight_position_invalid")
        uid = PLANNER.integer(row["source_user_id"], "inflight_position_invalid", minimum=1)
        require(uid in expected and uid not in seen and type(row["reserved_units"]) is int and row["reserved_units"] == 0,
                "inflight_nonzero_or_unknown_reserved")
        seen.add(uid)
    require(digest(evidence["source_table_counts"]) == digest({name: row["row_count"]
            for name, row in material["source_plan"]["source_evidence"].items()}), "inflight_source_table_coverage_incomplete")
    checked_hash(evidence["process_evidence_sha256"], "inflight_process_evidence_missing")
    require(evidence["process_evidence_sha256"] == material["source_files"]["inflight_process_evidence"]["sha256"],
            "inflight_process_evidence_file_mismatch")


def validate_escrow_source_evidence(material):
    """Check candidate/unit bindings without economics, a target or SQL creation."""
    candidate, evidence = (material.get(k) for k in ("escrow_candidate", "escrow_unit_evidence"))
    import legacy_escrow_mapping as mapper
    require(type(evidence) is dict and set(evidence) == mapper.UNIT_FIELDS
            and type(evidence["format_version"]) is int and evidence["format_version"] == 1
            and evidence["scope"] == "legacy-escrow-source-unit-evidence"
            and evidence["unit"] == "credit_500k_usd" and type(evidence["credits_per_usd"]) is int
            and evidence["credits_per_usd"] == 500000 and evidence["conversion_policy"] == mapper.POLICY
            and type(evidence["source_application_revision"]) is str
            and re.fullmatch(r"[0-9a-f]{40}", evidence["source_application_revision"]) is not None,
            "escrow_unit_evidence_policy_invalid")
    require(type(candidate) is dict and candidate.get("blockers") == []
            and candidate.get("approval_created") is False and type(candidate.get("target_writes")) is int
            and candidate["target_writes"] == 0 and candidate.get("economic_approval_verified") is False,
            "escrow_candidate_unresolved")
    require(all(candidate.get(k) == material[k] for k in ("migration_id", "source_archive_sha256", "mapping_sha256", "identity_map_sha256"))
            and candidate.get("source_export_sha256") == material["source_files"]["source_export"]["sha256"]
            and candidate.get("source_unit_evidence_bound") is True
            and candidate.get("source_unit_authority_verified") is False
            and candidate.get("unit_evidence_sha256") == digest(evidence)
            and candidate.get("candidate_sha256") == digest({k: v for k, v in candidate.items() if k not in ("candidate_sha256", "public_summary")}),
            "escrow_candidate_binding_mismatch")
    for key in ("source_archive_sha256", "mapping_sha256", "identity_map_sha256"):
        checked_hash(candidate.get(key), "escrow_candidate_binding_mismatch")
    require(all(evidence[k] == candidate.get(k) for k in ("source_archive_sha256", "mapping_sha256",
                "source_export_sha256", "identity_records_sha256")), "escrow_unit_evidence_binding_mismatch")
    for key in ("parent_plan_sha256", "parent_audit_row_sha256", "source_policy_evidence_sha256"):
        checked_hash(evidence[key], "escrow_unit_evidence_hash_invalid")
    raw = candidate.get("batch_candidate")
    require(type(raw) is dict and set(raw) == {"migration_id", "source_archive_sha256", "mapping_sha256",
            "unit", "observed_at", "projects"}, "escrow_candidate_shape_invalid")
    return raw


# @mark migration.financial-escrow role=writer
def validate_escrow(material):
    """Preflight the composed batch without granting economic authority.

    The source-only mapper is rerun from the actual private files before any
    write. These checks additionally bind the exact reviewed candidate and the
    Core SQL batch, and check every intermediate native balance for overflow.
    """
    raw = validate_escrow_source_evidence(material)
    candidate, evidence, batch = material["escrow_candidate"], material["escrow_unit_evidence"], material.get("escrow_batch")
    require(type(batch) is dict and digest(batch) == digest({**raw, "review_proof_sha256": material["source_review"]["evidence_sha256"]}),
            "escrow_batch_not_reviewed_candidate")
    require(batch["unit"] == "credit_500k_usd" and all(batch[k] == material[k] for k in
            ("migration_id", "source_archive_sha256", "mapping_sha256")), "escrow_batch_binding_mismatch")
    observed = PLANNER.integer(batch["observed_at"], "escrow_observation_invalid", minimum=1, maximum=253402300799)
    require(type(candidate.get("observed_at")) is int and candidate["observed_at"] == observed,
            "escrow_candidate_binding_mismatch")
    require(all(type(p["reserved_units"]) is int and p["reserved_units"] == 0 and p["observed_at"] == observed
                for p in material["opening"]), "escrow_requires_zero_nonescrow_opening_reserves")
    require(type(batch["projects"]) is list and 1 <= len(batch["projects"]) <= 128,
            "escrow_project_coverage_invalid")
    require(len(canonical(batch).encode()) <= 1040000, "escrow_batch_too_large")
    accounts = {row["personal_account_id"]: row["id"] for row in material["native_users"]}
    project_ids, claim_ids, dispute_ids, totals = set(), set(), set(), {}
    opening = PREFLIGHT.preflight_opening_batch(material["opening"])
    clearing = opening.positions[-1].clearing_after_units
    require(all(type(p) is dict for p in batch["projects"]), "escrow_project_shape_invalid")
    for project in sorted(batch["projects"], key=lambda p: PLANNER.integer(p.get("source_project_id"), "escrow_project_id_invalid", minimum=1)):
        require(type(project) is dict and set(project) == {"source_project_id", "owner_user_id", "payer_account_id",
            "source_row_sha256", "source_parent_plan_sha256", "escrow_units", "status", "claims", "disputes"},
            "escrow_project_shape_invalid")
        pid = PLANNER.integer(project["source_project_id"], "escrow_project_id_invalid", minimum=1)
        require(pid not in project_ids, "escrow_project_duplicate"); project_ids.add(pid)
        owner = PLANNER.integer(project["owner_user_id"], "escrow_owner_invalid", minimum=1)
        payer = PLANNER.integer(project["payer_account_id"], "escrow_owner_invalid", minimum=1)
        require(accounts.get(payer) == owner and project["status"] in ("published", "paused"), "escrow_owner_or_status_invalid")
        for field in ("source_row_sha256", "source_parent_plan_sha256"):
            checked_hash(project[field], "escrow_source_hash_invalid")
        require(project["source_parent_plan_sha256"] == evidence["parent_plan_sha256"], "escrow_parent_plan_mismatch")
        amount = PLANNER.integer(project["escrow_units"], "escrow_amount_invalid", minimum=1, maximum=PREFLIGHT.JS_SAFE_MAX)
        clearing = PLANNER.integer(clearing - amount, "escrow_clearing_prefix_overflow")
        totals[payer] = PLANNER.integer(totals.get(payer, 0) + amount, "escrow_reserved_total_overflow", minimum=0)
        require(type(project["claims"]) is list and len(project["claims"]) <= 2048
                and type(project["disputes"]) is list and len(project["disputes"]) <= 2048,
                "escrow_rights_shape_invalid")
        claims = {}
        for claim in project["claims"]:
            require(type(claim) is dict and set(claim) == {"source_challenge_id", "participant_user_id", "recipient_account_id",
                "source_row_sha256", "rights_status", "source_reward_quota", "payout_units", "status", "rejected_at", "paid_at"},
                "escrow_claim_shape_invalid")
            cid = PLANNER.integer(claim["source_challenge_id"], "escrow_claim_id_invalid", minimum=1)
            require(cid not in claim_ids, "escrow_claim_duplicate"); claim_ids.add(cid); claims[cid] = claim
            participant = PLANNER.integer(claim["participant_user_id"], "escrow_recipient_invalid", minimum=1)
            recipient = PLANNER.integer(claim["recipient_account_id"], "escrow_recipient_invalid", minimum=1)
            require(accounts.get(recipient) == participant and recipient != payer and participant != owner,
                    "escrow_recipient_identity_mismatch")
            checked_hash(claim["source_row_sha256"], "escrow_source_hash_invalid")
            PLANNER.integer(claim["source_reward_quota"], "escrow_claim_amount_invalid", minimum=0, maximum=PREFLIGHT.JS_SAFE_MAX)
            rejected = PLANNER.integer(claim["rejected_at"], "escrow_claim_time_invalid", minimum=0, maximum=observed)
            require(type(claim["paid_at"]) is int and claim["paid_at"] == 0
                    and claim["status"] in ("accepted", "submitted", "rejected"), "escrow_claim_state_invalid")
            if claim["rights_status"] == "active_future_reward":
                PLANNER.integer(claim["payout_units"], "escrow_claim_amount_invalid", minimum=1, maximum=PREFLIGHT.JS_SAFE_MAX)
            else:
                require(claim["rights_status"] == "historical_rejection_guard_only" and claim["payout_units"] is None
                        and claim["status"] == "rejected", "escrow_claim_rights_invalid")
            require(claim["status"] != "rejected" or rejected > 0, "escrow_claim_state_invalid")
        disputes = {}
        for dispute in project["disputes"]:
            require(type(dispute) is dict and set(dispute) == {"source_dispute_id", "source_challenge_id", "source_row_sha256",
                "status", "reward_quota_snapshot", "project_escrow_quota_snapshot", "tip_quota_snapshot"}, "escrow_dispute_shape_invalid")
            did = PLANNER.integer(dispute["source_dispute_id"], "escrow_dispute_id_invalid", minimum=1)
            cid = PLANNER.integer(dispute["source_challenge_id"], "escrow_claim_id_invalid", minimum=1)
            require(did not in dispute_ids and cid in claims and dispute["status"] in ("open", "resolved_denied"),
                    "escrow_dispute_binding_invalid")
            dispute_ids.add(did); disputes.setdefault(cid, []).append(dispute["status"])
            checked_hash(dispute["source_row_sha256"], "escrow_source_hash_invalid")
            for field in ("reward_quota_snapshot", "project_escrow_quota_snapshot", "tip_quota_snapshot"):
                PLANNER.integer(dispute[field], "escrow_dispute_snapshot_invalid", minimum=0, maximum=PREFLIGHT.JS_SAFE_MAX)
        owed = 0
        for cid, claim in claims.items():
            states = disputes.get(cid, [])
            active_rejection = "open" in states or (claim["rejected_at"] > observed - 604800 and "resolved_denied" not in states)
            if claim["rights_status"] == "historical_rejection_guard_only":
                require(not active_rejection, "escrow_active_rejection_basis_missing")
            elif claim["status"] in ("accepted", "submitted") or active_rejection:
                owed += claim["payout_units"]
        require(owed <= amount, "escrow_insufficient_for_frozen_rights")
    return totals


def prepare_material(plan: dict, *, migration_id: str, identities: dict,
                     observation: dict, reservations: dict, review: dict,
                     snapshot: dict, target_contract: dict, runtime_roles: list[str], source_files: dict,
                     escrow_candidate=None, escrow_unit_evidence=None, inflight_evidence=None) -> dict:
    """Bind existing planner adapters to a reviewed source and observed target.

    An opening-only source review never establishes missing Key, subscription,
    refund or escrow semantics. Those remain disabled and require separate work.
    """
    PLANNER.verify_plan_mapping(plan)
    require(type(migration_id) is str and IDENTITY.MIGRATION_ID.fullmatch(migration_id) is not None,
            "migration_id_invalid")
    composed = escrow_candidate is not None or escrow_unit_evidence is not None
    validate_source_review(review, plan, candidate=escrow_candidate, unit_evidence=escrow_unit_evidence, inflight=inflight_evidence)
    require(digest(identities) == plan.get("identity_map_sha256"), "identity_map_mismatch")
    runs = [row for row in snapshot["import_runs"]
            if row.get("manifest", {}).get("scope") == "identity-only"]
    require(len(runs) == 1, "target_identity_import_missing_or_ambiguous")
    PLANNER.verify_target_identity_binding(plan, identities, import_run=runs[0], native_users=snapshot["native_users"])
    require(all(row["source_archive_sha256"] == plan["source_archive_sha256"] for row in snapshot["import_runs"]),
            "target_contains_other_source")
    contract = IDENTITY.current_contract()
    require(all(target_contract.get(key) == contract[key] for key in
                CONTRACT_FIELDS),
            "target_contract_mismatch")
    require(snapshot["component_fingerprints"] == contract["component_fingerprints"]
            and snapshot["catalog_fingerprint_sha256"] == target_contract.get("catalog_fingerprint_sha256"),
            "target_catalog_mismatch")
    IDENTITY.validate_trust_policy_seed(snapshot.get("trust_policy_seed"))
    require(snapshot["trust_policy_seed"] == target_contract.get("trust_policy_seed"), "target_trust_policy_seed_mismatch")
    opening = PLANNER.build_opening_payloads(plan, migration_id=migration_id,
        observation=observation, reservation_reconciliation=reservations)
    counters = PLANNER.build_opening_counter_payloads(plan, opening)
    material = {
        "format_version": 3 if composed else 2, "scope": ESCROW_SCOPE if composed else SCOPE, "seed_contract": SEED,
        "migration_id": migration_id, "source_archive_sha256": plan["source_archive_sha256"],
        "mapping_sha256": plan["mapping_sha256"], "identity_map_sha256": plan["identity_map_sha256"],
        "source_review": review, "identity_import": runs[0],
        "native_users": snapshot["native_users"], "observation": observation,
        "source_plan": plan, "source_files": source_files,
        "reservation_evidence_sha256": opening["reservation_review_evidence_sha256"],
        "opening": opening["payloads"], "counters": counters["payloads"],
        "target": {**{key: snapshot[key] for key in ("instance", "catalog_fingerprint_sha256", "ownership_sha256", "component_fingerprints", "trust_policy_seed")},
                   "source_contract": {key: contract[key] for key in CONTRACT_FIELDS}},
        "before_state": snapshot["state"], "runtime_roles": runtime_roles,
        "tool_source_hashes": source_hashes(escrow=composed),
    }
    if composed:
        material.update(escrow_candidate=escrow_candidate, escrow_unit_evidence=escrow_unit_evidence, inflight_evidence=inflight_evidence,
                        escrow_batch={**escrow_candidate["batch_candidate"], "review_proof_sha256": review["evidence_sha256"]}
                        if type(escrow_candidate.get("batch_candidate")) is dict else None)
    validate_material(material)
    return json.loads(canonical(material))


def validate_material(material: dict) -> None:
    composed = type(material) is dict and escrow_profile(material)
    require(type(material) is dict and type(material.get("format_version")) is int
            and material["format_version"] == (3 if composed else 2) and material.get("scope") == (ESCROW_SCOPE if composed else SCOPE)
            and material.get("seed_contract") == SEED, "material_contract_invalid")
    require(composed or not {"escrow_candidate", "escrow_unit_evidence", "escrow_batch", "inflight_evidence"}.intersection(material),
            "escrow_requires_composed_profile")
    require(type(material.get("migration_id")) is str
            and IDENTITY.MIGRATION_ID.fullmatch(material["migration_id"]) is not None, "migration_id_invalid")
    for key in ("source_archive_sha256", "mapping_sha256", "identity_map_sha256", "reservation_evidence_sha256"):
        checked_hash(material.get(key))
    require(material.get("tool_source_hashes") == source_hashes(escrow=composed), "financial_tool_source_changed")
    files = material.get("source_files")
    require(type(files) is dict and set(files) == SOURCE_FILES | (ESCROW_SOURCE_FILES if composed else frozenset()), "source_files_incomplete")
    for item in files.values():
        require(type(item) is dict and set(item) == {"path", "sha256"}
                and type(item["path"]) is str and Path(item["path"]).is_absolute(), "source_file_shape_invalid")
        checked_hash(item["sha256"])
    require(files["archive"]["sha256"] == material["source_archive_sha256"], "source_archive_binding_mismatch")
    PLANNER.verify_plan_mapping(material["source_plan"])
    require(all(material["source_plan"].get(key) == material[key] for key in
                ("source_archive_sha256", "mapping_sha256", "identity_map_sha256")),
            "material_source_plan_mismatch")
    target = material["target"]
    expected = target["instance"]
    require(type(expected.get("database_oid")) in (str, int) and str(expected["database_oid"]).isascii()
            and str(expected["database_oid"]).isdigit() and int(expected["database_oid"]) > 0, "target_oid_invalid")
    for key in ("database", "role", "database_owner", "session_user"):
        require(type(expected.get(key)) is str and IDENTITY.IDENTIFIER.fullmatch(expected[key]) is not None,
                "target_role_invalid")
    require(expected["database_owner"] == expected["role"] != expected["session_user"]
            and expected.get("effective_role_can_login") is False
            and expected.get("tcp_connection") is False and expected.get("listen_addresses") == "",
            "target_not_offline_owner")
    require(type(expected.get("port")) is int and 0 < expected["port"] <= 65535
            and all(type(expected.get(k)) is str and expected[k] for k in
                    ("postmaster_started_at", "system_identifier", "server_instance_nonce", "server_version_num")),
            "target_instance_incomplete")
    for key in ("catalog_fingerprint_sha256", "ownership_sha256"):
        checked_hash(target.get(key))
    require(target["component_fingerprints"] == IDENTITY.current_contract()["component_fingerprints"],
            "component_source_changed")
    contract = IDENTITY.current_contract()
    require(target.get("source_contract") == {key: contract[key] for key in CONTRACT_FIELDS}, "source_contract_changed")
    IDENTITY.validate_trust_policy_seed(target.get("trust_policy_seed"))
    roles = material.get("runtime_roles")
    require(type(roles) is list and roles and all(type(role) is str and IDENTITY.IDENTIFIER.fullmatch(role)
            for role in roles) and len(roles) == len(set(roles))
            and not {expected["role"], expected["session_user"]}.intersection(roles), "runtime_roles_invalid")
    state = material["before_state"]
    validate_inventory(state["inventory"])
    require(set(state["tables"]) == set(state["inventory"]["tables"])
            and set(state["sequences"]) == set(state["inventory"]["sequences"]), "snapshot_inventory_mismatch")
    for value in state["tables"].values():
        PLANNER.integer(value.get("rows"), "snapshot_count_invalid", minimum=0)
        checked_hash(value.get("sha256"))
    require(all(state["tables"][name] == {"rows": 0, "sha256": sha(b"")} for name in FRESH_EMPTY_TABLES),
            "fresh26_business_or_upgrade_records_not_empty")
    if composed:
        require(set(ESCROW_TABLES) | {"core_billing.legacy_escrow_operations"} <= set(state["tables"]),
                "escrow_target_inventory_incomplete")
        require(all(state["tables"][name] == {"rows": 0, "sha256": sha(b"")} for name in
                    (*ESCROW_TABLES, "core_billing.legacy_escrow_operations")), "escrow_target_not_empty")
    PREFLIGHT.preflight_opening_batch(material["opening"])
    PREFLIGHT.validate_opening_counters(material["opening"], material["counters"])
    require(material["opening"], "empty_opening_not_supported")
    native = {row["id"]: row["personal_account_id"] for row in material["native_users"]}
    require(len(native) == len(material["native_users"]) and len(set(native.values())) == len(native)
            and digest(native) == material["identity_map_sha256"]
            and {row["user_id"]: row["account_id"] for row in material["counters"]} == native,
            "opening_identity_coverage_mismatch")
    PLANNER.verify_target_identity_binding(material["source_plan"], native,
        import_run=material["identity_import"], native_users=material["native_users"])
    # Rebuild both adapters from the bound original plan. An edited payload is
    # not allowed to acquire the same source row hash with different money.
    rebuilt = PLANNER.build_opening_payloads(material["source_plan"], migration_id=material["migration_id"],
        observation=material["observation"], reservation_reconciliation={
            "source_archive_sha256": material["source_archive_sha256"], "mapping_sha256": material["mapping_sha256"],
            "reviewed_evidence_sha256": material["reservation_evidence_sha256"],
            "positions": [{"source_user_id": counter["user_id"], "reserved_units": opening["reserved_units"]}
                          for opening, counter in zip(material["opening"], material["counters"])]})
    require(rebuilt["payloads"] == material["opening"] and
            PLANNER.build_opening_counter_payloads(material["source_plan"], rebuilt)["payloads"] == material["counters"],
            "opening_plan_adapter_mismatch")
    validate_source_review(material["source_review"], material,
        candidate=material.get("escrow_candidate"), unit_evidence=material.get("escrow_unit_evidence"), inflight=material.get("inflight_evidence"))
    for row in material["opening"]:
        require(all(row[key] == material[key] for key in ("migration_id", "source_archive_sha256", "mapping_sha256")),
                "opening_source_mismatch")
    if composed:
        validate_escrow(material)
        validate_inflight(material)
        validate_review_evidence_files(material)
    needed = (4 * len(material["opening"]), sum(bool(row["available_units"] or row["affiliate_units"] or row["reserved_units"])
                                               for row in material["opening"]))
    if composed:
        needed = (needed[0], needed[1] + len(material["escrow_batch"]["projects"]))
    for name, count in zip(CHANGED_SEQUENCES, needed):
        sequence = state["sequences"][name]
        require(type(sequence.get("increment")) is int and sequence["increment"] == 1
                and type(sequence.get("cache")) is int and sequence["cache"] == 1 and sequence.get("cycle") is False
                and type(sequence.get("is_called")) is bool, "noncanonical_ledger_sequence")
        last = PLANNER.integer(sequence.get("last_value"), "sequence_value_invalid", minimum=1)
        maximum = PLANNER.integer(sequence.get("max"), "sequence_value_invalid", minimum=1)
        require(last + int(sequence["is_called"]) + max(count - 1, 0) <= maximum, "ledger_sequence_overflow")


def _guard(material: dict) -> str:
    target = material["target"]
    roles = material["runtime_roles"]
    contract = IDENTITY.current_contract()
    return f"""
 PERFORM pg_stat_clear_snapshot();
 IF ({instance_sql()}) IS DISTINCT FROM {literal(target['instance'])}::jsonb THEN
  RAISE EXCEPTION 'financial_target_instance_mismatch'; END IF;
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database())
  AND pid<>pg_backend_pid()) THEN RAISE EXCEPTION 'financial_target_has_other_clients'; END IF;
 IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=current_user AND NOT rolcanlogin AND NOT rolsuper
  AND NOT rolcreaterole AND NOT rolcreatedb AND NOT rolreplication AND NOT rolbypassrls)
 OR NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=session_user AND rolcanlogin AND NOT rolsuper
  AND NOT rolcreaterole AND NOT rolcreatedb AND NOT rolreplication AND NOT rolbypassrls)
 OR (SELECT count(*) FROM pg_roles WHERE rolname IN ({','.join(literal(r) for r in roles)}) AND NOT rolcanlogin AND NOT rolsuper
  AND NOT rolcreaterole AND NOT rolcreatedb AND NOT rolreplication AND NOT rolbypassrls)<>{len(roles)}
 OR EXISTS(SELECT 1 FROM pg_roles WHERE rolcanlogin AND NOT rolsuper AND rolname<>session_user
  AND has_database_privilege(oid,current_database(),'CONNECT'))
 THEN RAISE EXCEPTION 'financial_offline_roles_mismatch'; END IF;
 IF NOT pg_has_role(session_user,current_user,'MEMBER') THEN RAISE EXCEPTION 'financial_owner_membership_mismatch'; END IF;
 IF EXISTS(SELECT 1 FROM pg_namespace n WHERE {TABLE_FILTER} AND n.nspname<>'public' AND pg_get_userbyid(n.nspowner)<>current_user)
 OR EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
  WHERE {TABLE_FILTER} AND pg_get_userbyid(c.relowner)<>current_user)
 OR EXISTS(SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
  WHERE {TABLE_FILTER} AND pg_get_userbyid(p.proowner)<>current_user)
 THEN RAISE EXCEPTION 'financial_schema_owner_mismatch'; END IF;
 IF ({IDENTITY.catalog_hash_sql(contract)}) IS DISTINCT FROM {literal(target['catalog_fingerprint_sha256'])}
 OR ({ownership_sql()}) IS DISTINCT FROM {literal(target['ownership_sha256'])}
 OR NOT EXISTS(SELECT 1 FROM core_meta.runtime_contract WHERE singleton
  AND encode(catalog_fingerprint,'hex')={literal(target['catalog_fingerprint_sha256'])})
 OR (SELECT jsonb_object_agg(name,encode(fingerprint,'hex')) FROM core_meta.component_contract)
  IS DISTINCT FROM {literal(target['component_fingerprints'])}::jsonb
 OR ({inventory_sql()}) IS DISTINCT FROM {literal(material['before_state']['inventory'])}::jsonb
 OR NOT EXISTS(SELECT 1 FROM core_meta.schema_contract WHERE singleton
  AND version={contract['identity_version']} AND encode(fingerprint,'hex')={literal(contract['identity_fingerprint_sha256'])})
 THEN RAISE EXCEPTION 'financial_target_catalog_mismatch'; END IF;
 IF {IDENTITY.upgrade_receipts_guard_sql()} THEN RAISE EXCEPTION 'financial_upgrade_receipts_not_empty'; END IF;
"""


def _dollar_quote(sql: str) -> str:
    tag = "$lmm_financial_" + sha(sql.encode())[:24] + "$"
    require(tag not in sql, "sql_delimiter_collision")
    # Only replace the four template delimiters. A source string may itself
    # contain the old delimiter, including escaped SQL-looking text.
    prefix, separator, body = sql.partition("END $lmm_financial$;")
    require(bool(separator), "sql_template_invalid")
    prefix = prefix.replace("DO $lmm_financial$", "DO " + tag, 1)
    body = body.replace("DO $lmm_financial$", "DO " + tag, 1)
    middle, separator, suffix = body.rpartition("END $lmm_financial$;")
    require(bool(separator), "sql_template_invalid")
    return prefix + "END " + tag + ";" + middle + "END " + tag + ";" + suffix


def _start(material: dict, *, readonly: bool) -> str:
    # Take a session fence BEFORE choosing the transaction snapshot. A wait inside
    # SERIALIZABLE would not refresh that snapshot. Never retry automatically.
    return f"""SET standard_conforming_strings=on;
SET timezone='UTC';
SET lock_timeout='5s';
SET statement_timeout='120s';
DO $lmm_financial$ BEGIN
 IF NOT pg_try_advisory_lock({FENCE}) THEN RAISE EXCEPTION 'financial_import_busy'; END IF;
END $lmm_financial$;
BEGIN ISOLATION LEVEL SERIALIZABLE{' READ ONLY' if readonly else ''};
SET LOCAL synchronous_commit=on;
SET LOCAL row_security=off;
SET LOCAL search_path=pg_catalog,public;
"""


def escrow_readback_sql() -> str:
    return """
  IF (SELECT count(*) FROM core_billing.legacy_escrow_batch_runs)<>1
   OR NOT EXISTS(SELECT FROM core_billing.legacy_escrow_batch_runs b WHERE b.request=m->'escrow_batch')
   OR (SELECT count(*) FROM core_billing.legacy_escrow_projects)<>jsonb_array_length(m->'escrow_batch'->'projects')
   OR (SELECT count(*) FROM core_billing.legacy_escrow_claims)<>(SELECT count(*) FROM
     jsonb_array_elements(m->'escrow_batch'->'projects') p CROSS JOIN LATERAL jsonb_array_elements(p->'claims') c)
   OR (SELECT count(*) FROM core_billing.legacy_escrow_disputes)<>(SELECT count(*) FROM
     jsonb_array_elements(m->'escrow_batch'->'projects') p CROSS JOIN LATERAL jsonb_array_elements(p->'disputes') d)
   OR (SELECT count(*) FROM core_billing.legacy_escrow_state)<>jsonb_array_length(m->'escrow_batch'->'projects')
   OR EXISTS(SELECT FROM core_billing.legacy_escrow_operations)
   OR EXISTS(SELECT FROM jsonb_array_elements(m->'escrow_batch'->'projects') expected
     LEFT JOIN core_billing.legacy_escrow_projects p ON p.migration_id=m->>'migration_id'
       AND p.source_project_id=(expected->>'source_project_id')::bigint
     LEFT JOIN core_billing.legacy_escrow_state s USING(migration_id,source_project_id)
     LEFT JOIN core_billing.ledger_journals j ON j.id=p.opening_journal_id
     WHERE p.source IS DISTINCT FROM expected OR
       to_jsonb(p)-ARRAY['migration_id','source_archive_sha256','opening_journal_id','source']
         IS DISTINCT FROM expected-ARRAY['claims','disputes']
       OR s.remaining_units IS DISTINCT FROM (expected->>'escrow_units')::bigint
       OR s.revision IS DISTINCT FROM 1::bigint OR s.closed IS DISTINCT FROM false
       OR j.kind IS DISTINCT FROM 'legacy_escrow_opening' OR j.scope IS DISTINCT FROM 'legacy-escrow-opening'
       OR j.amount_units IS DISTINCT FROM (expected->>'escrow_units')::bigint
       OR j.payer_id IS DISTINCT FROM (SELECT id FROM core_billing.ledger_accounts WHERE bucket='clearing')
       OR j.payee_id IS DISTINCT FROM (SELECT id FROM core_billing.ledger_accounts
         WHERE owner_account_id=p.payer_account_id AND bucket='reserved'))
   OR EXISTS(SELECT FROM jsonb_array_elements(m->'escrow_batch'->'projects') p
     CROSS JOIN LATERAL jsonb_array_elements(p->'claims') expected
     LEFT JOIN core_billing.legacy_escrow_claims c ON c.migration_id=m->>'migration_id'
       AND c.source_project_id=(p->>'source_project_id')::bigint AND c.source_challenge_id=(expected->>'source_challenge_id')::bigint
     WHERE c.source IS DISTINCT FROM expected OR to_jsonb(c)-ARRAY['migration_id','source_project_id','source'] IS DISTINCT FROM expected)
   OR EXISTS(SELECT FROM jsonb_array_elements(m->'escrow_batch'->'projects') p
     CROSS JOIN LATERAL jsonb_array_elements(p->'disputes') expected
     LEFT JOIN core_billing.legacy_escrow_disputes d ON d.migration_id=m->>'migration_id'
       AND d.source_project_id=(p->>'source_project_id')::bigint AND d.source_dispute_id=(expected->>'source_dispute_id')::bigint
     WHERE d.source IS DISTINCT FROM expected OR d.source_challenge_id IS DISTINCT FROM (expected->>'source_challenge_id')::bigint
       OR d.source_row_sha256 IS DISTINCT FROM expected->>'source_row_sha256' OR d.status IS DISTINCT FROM expected->>'status')
   OR EXISTS(SELECT FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id
     WHERE a.bucket IN ('clearing','revenue') AND b.balance_units::numeric IS DISTINCT FROM
       CASE WHEN a.bucket='revenue' THEN 0::numeric ELSE
         -(SELECT coalesce(sum((p->>'available_units')::bigint+(p->>'affiliate_units')::bigint+(p->>'reserved_units')::bigint),0)
            FROM jsonb_array_elements(m->'opening') p)
         -(SELECT coalesce(sum((p->>'escrow_units')::bigint),0) FROM jsonb_array_elements(m->'escrow_batch'->'projects') p) END)
  THEN RAISE EXCEPTION 'financial_escrow_exact_readback_failed'; END IF;
"""


def build_apply_sql(material: dict) -> str:
    validate_material(material)
    inventory = material["before_state"]["inventory"]
    state = state_sql(inventory, material["migration_id"])
    guard = _guard(material)
    locks = ",".join(relation(name) for name in inventory["tables"])
    composed = escrow_profile(material)
    scope = material["scope"]
    changed_tables = "ARRAY[" + ",".join(literal(n) for n in (*CHANGED_TABLES, *(ESCROW_TABLES if composed else ()))) + "]::text[]"
    changed_sequences = "ARRAY[" + ",".join(literal(n) for n in CHANGED_SEQUENCES) + "]::text[]"
    escrow_restore = "PERFORM core_billing.import_legacy_escrow_batch(m->'escrow_batch');" if composed else ""
    expected_reserved = """(p->>'reserved_units')::bigint + coalesce((SELECT sum((ep->>'escrow_units')::bigint)
      FROM jsonb_array_elements(m->'escrow_batch'->'projects') ep
      WHERE (ep->>'payer_account_id')::bigint=(p->>'account_id')::bigint),0)""" if composed else "(p->>'reserved_units')::bigint"
    restarts = []
    for name in CHANGED_SEQUENCES:
        seq = material["before_state"]["sequences"][name]
        next_value = seq["last_value"] + int(seq["is_called"])
        # RESTART is transactional. It also makes subsequent nextval calls use
        # rollbackable sequence storage, unlike setval or a plain nextval.
        restarts.append(f"ALTER SEQUENCE {relation(name)} RESTART WITH {next_value};")
    sql = _start(material, readonly=False) + f"""
DO $lmm_financial$
DECLARE m jsonb := {literal(material)}::jsonb; previous core_migration.import_runs%ROWTYPE;
 before_state jsonb; after_state jsonb; v_payload jsonb;
BEGIN
 {guard}
 LOCK TABLE {locks} IN ACCESS EXCLUSIVE MODE;
 {guard}
 IF {IDENTITY.trust_policy_seed_guard_sql(material['target'])} THEN RAISE EXCEPTION 'financial_trust_policy_seed_mismatch'; END IF;
 SELECT ({state}) INTO before_state;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256<>m->>'source_archive_sha256' OR previous.mapping_sha256<>m->>'mapping_sha256'
   OR previous.manifest->'material' IS DISTINCT FROM m
   OR previous.manifest->>'scope' IS DISTINCT FROM {literal(scope)}
   OR previous.manifest->'post_state' IS DISTINCT FROM before_state
  THEN RAISE EXCEPTION 'financial_replay_conflict'; END IF;
 ELSE
  IF EXISTS(SELECT 1 FROM core_billing.opening_import_runs)
   OR EXISTS(SELECT 1 FROM core_migration.import_runs WHERE manifest->>'scope' IN ({literal(SCOPE)},{literal(ESCROW_SCOPE)}))
  THEN RAISE EXCEPTION 'financial_other_import_conflict'; END IF;
  IF before_state IS DISTINCT FROM m->'before_state' THEN RAISE EXCEPTION 'financial_target_state_changed'; END IF;
  IF (SELECT count(*) FROM core_billing.ledger_accounts)<>2 OR (SELECT count(*) FROM core_billing.balance_state)<>2
   OR EXISTS(SELECT 1 FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id
    WHERE a.owner_account_id IS NOT NULL OR a.bucket NOT IN ('clearing','revenue') OR a.unit<>'credit_500k_usd'
    OR b.balance_units<>0 OR b.revision<>0)
   OR EXISTS(SELECT 1 FROM core_billing.ledger_operations) OR EXISTS(SELECT 1 FROM core_billing.ledger_journals)
   OR EXISTS(SELECT 1 FROM core_billing.ledger_entries) OR EXISTS(SELECT 1 FROM core_billing.opening_imports)
   OR EXISTS(SELECT 1 FROM core_billing.account_opening_counters)
  THEN RAISE EXCEPTION 'financial_ledger_seed_mismatch'; END IF;
  IF (SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'personal_account_id',personal_account_id) ORDER BY id),'[]'::jsonb)
   FROM core_identity.users) IS DISTINCT FROM m->'native_users'
   OR NOT EXISTS(SELECT 1 FROM core_migration.import_runs r WHERE to_jsonb(r)=m->'identity_import')
  THEN RAISE EXCEPTION 'financial_identity_binding_mismatch'; END IF;
  {' '.join(restarts)}
  FOR v_payload IN SELECT value FROM jsonb_array_elements(m->'opening') ORDER BY (value->>'account_id')::bigint LOOP
   PERFORM core_billing.import_opening_balance(v_payload);
  END LOOP;
  FOR v_payload IN SELECT value FROM jsonb_array_elements(m->'counters') ORDER BY (value->>'account_id')::bigint LOOP
   PERFORM core_billing.import_account_opening_counters(v_payload);
  END LOOP;
  {escrow_restore}
  SET CONSTRAINTS ALL IMMEDIATE;
  {escrow_readback_sql() if composed else ''}
  IF (SELECT count(*) FROM core_billing.opening_imports)<>jsonb_array_length(m->'opening')
   OR (SELECT count(*) FROM core_billing.account_opening_counters)<>jsonb_array_length(m->'counters')
   OR EXISTS(SELECT 1 FROM jsonb_array_elements(m->'opening') p LEFT JOIN core_billing.opening_imports o
    ON o.account_id=(p->>'account_id')::bigint WHERE o.request IS DISTINCT FROM p)
   OR EXISTS(SELECT 1 FROM jsonb_array_elements(m->'counters') p LEFT JOIN core_billing.account_opening_counters c
    ON c.account_id=(p->>'account_id')::bigint WHERE c.request IS DISTINCT FROM p)
  THEN RAISE EXCEPTION 'financial_exact_readback_failed'; END IF;
  IF EXISTS(SELECT 1 FROM jsonb_array_elements(m->'opening') p CROSS JOIN
    (VALUES ('wallet'),('debt'),('reserved'),('affiliate')) expected(bucket)
    LEFT JOIN core_billing.ledger_accounts a ON a.owner_account_id=(p->>'account_id')::bigint AND a.bucket=expected.bucket
    LEFT JOIN core_billing.balance_state b ON b.ledger_account_id=a.id
    WHERE b.balance_units IS DISTINCT FROM CASE expected.bucket
     WHEN 'wallet' THEN greatest((p->>'available_units')::bigint,0)
     WHEN 'debt' THEN least((p->>'available_units')::bigint,0)
     WHEN 'reserved' THEN {expected_reserved} ELSE (p->>'affiliate_units')::bigint END)
   OR EXISTS(SELECT 1 FROM core_billing.ledger_journals j LEFT JOIN core_billing.ledger_entries e ON e.journal_id=j.id
    GROUP BY j.id HAVING count(e.journal_id)<2 OR sum(e.delta_units)<>0)
   OR EXISTS(SELECT 1 FROM core_billing.balance_state b LEFT JOIN
    (SELECT ledger_account_id,sum(delta_units) units,count(*) entries FROM core_billing.ledger_entries GROUP BY ledger_account_id) e
    ON e.ledger_account_id=b.ledger_account_id WHERE b.balance_units::numeric<>coalesce(e.units,0) OR b.revision<>coalesce(e.entries,0))
   OR (SELECT coalesce(sum(balance_units),0) FROM core_billing.balance_state)<>0
  THEN RAISE EXCEPTION 'financial_ledger_reconciliation_failed'; END IF;
  SELECT ({state}) INTO after_state;
  IF (before_state->'tables') - {changed_tables} IS DISTINCT FROM (after_state->'tables') - {changed_tables}
   OR (before_state->'sequences') - {changed_sequences} IS DISTINCT FROM (after_state->'sequences') - {changed_sequences}
  THEN RAISE EXCEPTION 'financial_unrelated_state_changed'; END IF;
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
   VALUES(m->>'migration_id',m->>'source_archive_sha256',m->>'mapping_sha256',
    jsonb_build_object('scope',{literal(scope)},'material',m,'post_state',after_state));
 END IF;
 {guard}
 IF {IDENTITY.trust_policy_seed_guard_sql(material['target'])} THEN RAISE EXCEPTION 'financial_trust_policy_seed_mismatch'; END IF;
END $lmm_financial$;
COMMIT;
SELECT jsonb_build_object('status','COMMITTED','migration_id',{literal(material['migration_id'])},
 'material_sha256',{literal(digest(material))});
"""
    return _dollar_quote(sql)


def build_resolve_sql(material: dict, original_application_name: str) -> str:
    validate_material(material)
    require(type(original_application_name) is str and re.fullmatch(r"lmm-financial-[0-9a-f]{32}", original_application_name),
            "original_attempt_identity_invalid")
    state = state_sql(material["before_state"]["inventory"], material["migration_id"])
    sql = _start(material, readonly=True) + f"""
DO $lmm_financial$
DECLARE m jsonb := {literal(material)}::jsonb; previous core_migration.import_runs%ROWTYPE; actual jsonb;
BEGIN
 {_guard(material)}
 IF EXISTS(SELECT 1 FROM pg_stat_activity WHERE application_name={literal(original_application_name)}
  AND datid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pid<>pg_backend_pid())
 THEN RAISE EXCEPTION 'financial_original_session_still_active'; END IF;
 SELECT ({state}) INTO actual;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256<>m->>'source_archive_sha256' OR previous.mapping_sha256<>m->>'mapping_sha256'
   OR previous.manifest->'material' IS DISTINCT FROM m OR previous.manifest->>'scope' IS DISTINCT FROM {literal(material['scope'])}
   OR previous.manifest->'post_state' IS DISTINCT FROM actual
  THEN RAISE EXCEPTION 'financial_resolution_conflict'; END IF;
 ELSIF actual IS DISTINCT FROM m->'before_state' THEN RAISE EXCEPTION 'financial_absence_not_proven'; END IF;
 {_guard(material)}
END $lmm_financial$;
SELECT jsonb_build_object('status',CASE WHEN EXISTS(SELECT 1 FROM core_migration.import_runs
 WHERE migration_id={literal(material['migration_id'])}) THEN 'COMMITTED' ELSE 'NOT_APPLIED' END,
 'migration_id',{literal(material['migration_id'])},'material_sha256',{literal(digest(material))});
COMMIT;
"""
    return _dollar_quote(sql)


def verify_approval(material: dict, sql: str, approval: dict) -> None:
    composed = escrow_profile(material)
    require(type(approval) is dict and type(approval.get("format_version")) is int and approval == {
        "format_version": 2 if composed else 1,
        "decision": "approve-offline-opening-with-legacy-escrow" if composed else "approve-offline-opening", "scope": material["scope"],
        "material_sha256": digest(material), "sql_sha256": sha(sql.encode()),
        "source_review_sha256": digest(material["source_review"]),
    }, "offline_approval_mismatch")


def write_private(directory: Path, name: str, value: str) -> Path:
    result = IDENTITY.write_private(directory, name, value)
    fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)
    return result


def read_private(path: Path) -> str:
    return IDENTITY.private_backup_path(path).read_text(encoding="utf-8")


def verify_source_files(material: dict) -> None:
    files = material["source_files"]
    for item in files.values():
        require(IDENTITY.sha256_file(IDENTITY.private_backup_path(Path(item["path"]))) == item["sha256"],
                "source_file_changed")
    load = lambda name: PLANNER.parse_json(read_private(Path(files[name]["path"])))
    require(all(digest(load(name)) == digest(expected) for name, expected in (
        ("financial_plan", material["source_plan"]), ("source_review", material["source_review"]),
        ("identity_manifest", material["identity_import"]["manifest"]))), "source_file_content_mismatch")
    identities = PLANNER.identity_index([PLANNER.parse_json(line) for line in
        read_private(Path(files["identity_map"]["path"])).splitlines() if line])
    require(digest(identities) == material["identity_map_sha256"] and files["identity_map"]["sha256"] ==
            material["source_plan"]["identity_provenance"]["identity_map_file_sha256"], "source_id_map_file_mismatch")
    proof = load("source_export_proof")
    require(proof["source_archive_sha256"] == material["source_archive_sha256"] and
            proof["source_export_sha256"] == files["source_export"]["sha256"], "source_export_file_mismatch")
    source = load("source_receipt")
    opening = PLANNER.build_opening_payloads(material["source_plan"], migration_id=material["migration_id"],
        observation=material["observation"], reservation_reconciliation=load("reservations"))
    require(opening["payloads"] == material["opening"] and
            PLANNER.source_observation(source) == material["observation"], "source_observation_or_reservation_mismatch")
    # These files are bound transitively by the receipt's private paths and the
    # approved export proof's hashes. Recheck them before submission and after
    # a successful transport return, just like the directly listed source files.
    # This reads local files only; it cannot reconnect to the source database.
    columns, baseline, metadata_hashes, _, _ = PLANNER.metadata(source)
    require(digest(metadata_hashes) == digest(proof.get("metadata_hashes")),
            "source_export_metadata_hash_mismatch")
    if escrow_profile(material):
        for name in ("escrow_candidate", "escrow_unit_evidence", "inflight_evidence"):
            require(digest(load(name)) == digest(material[name]), "escrow_source_file_content_mismatch")
        for name in ("source_review_evidence", "escrow_policy_evidence", "inflight_process_evidence"):
            require(bool(read_private(Path(files[name]["path"])).strip()), "escrow_evidence_file_empty")
        # The candidate is not accepted merely because its own hashes agree.
        # Parse the original frozen rows and recompute the maintained mapper.
        selected = sorted(table for table in columns if PLANNER.table_families(table) or PLANNER.FINANCIAL_NAME.search(table))
        require(digest(proof.get("selected_table_counts")) == digest({table: baseline[table] for table in selected}),
                "source_export_table_selection_mismatch")
        tables = PLANNER.read_frozen_export(Path(files["source_export"]["path"]), source,
                                           columns, baseline, selected, proof["source_instance"])
        records = [PLANNER.parse_json(line) for line in read_private(Path(files["identity_map"]["path"])).splitlines() if line]
        import legacy_escrow_mapping as mapper
        candidate = mapper.build(PLANNER, tables=tables, identity_records=records, plan=material["source_plan"],
            migration_id=material["migration_id"], observed_at=material["observation"]["observed_at"],
            source_proof=proof, unit_evidence=material["escrow_unit_evidence"],
            post_parent_claim_evidence=material["escrow_candidate"].get("post_parent_claim_evidence"))
        require(digest(candidate) == digest(material["escrow_candidate"]), "escrow_source_mapper_rebuild_mismatch")


def run_sql(connection: dict, sql: str, output: Path, application_name: str, *, readonly=False,
            label="database") -> tuple[int, str]:
    environment = {key: value for key, value in os.environ.items() if not key.startswith("PG")}
    environment.update(IDENTITY.socket_connection(connection))
    environment.update(PGCONNECT_TIMEOUT="5", PGAPPNAME=application_name,
                       PGOPTIONS=IDENTITY.psql_options(connection, readonly=readonly))
    try:
        result = subprocess.run(["psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1"],
            input=sql, text=True, capture_output=True, env=environment, timeout=150, check=False)
    except (subprocess.TimeoutExpired, OSError) as error:
        evidence = []
        if isinstance(error, subprocess.TimeoutExpired):
            # TimeoutExpired can contain bytes even when text=True was requested.
            for stream, value in (("stdout", error.stdout), ("stderr", error.stderr)):
                if value is not None:
                    if isinstance(value, bytes):
                        value = value.decode("utf-8", errors="replace")
                    evidence.append((label + "." + stream + ".private.txt", value))
        evidence.append((label + ".failure.private.json",
                         canonical({"type": type(error).__name__, "message": str(error)}) + "\n"))
        for name, value in evidence:
            try:
                write_private(output, name, value)
            except OSError:
                # A storage failure must not prevent other evidence writes or
                # replace the original transport exception. Safety checks still raise.
                pass
        raise
    write_private(output, label + ".stderr.private.txt", result.stderr)
    write_private(output, label + ".stdout.private.txt", result.stdout)
    return result.returncode, result.stdout


def execute(material: dict, *, approval: dict, original_sql: str, connection: dict,
            output: Path, original_attempt: dict | None = None) -> dict:
    """One execution, never an automatic retry. All post-start errors are UNKNOWN."""
    validate_material(material)
    require(original_sql == build_apply_sql(material), "original_sql_changed")
    verify_approval(material, original_sql, approval)
    verify_source_files(material)
    owner, login = IDENTITY.connection_roles(connection)
    expected = material["target"]["instance"]
    require(owner == expected["role"] and login == expected["session_user"]
            and connection.get("database") == expected["database"] and connection.get("port") == expected["port"],
            "target_connection_binding_mismatch")
    IDENTITY.socket_connection(connection)  # Reject transport/secret paths before recording a write attempt.
    application_name = "lmm-financial-" + uuid.uuid4().hex
    attempt = {"format_version": 1, "migration_id": material["migration_id"],
        "material_sha256": digest(material), "sql_sha256": sha(original_sql.encode()),
        "application_name": application_name, "operation": "resolve" if original_attempt else "apply"}
    sql = original_sql
    if original_attempt is not None:
        require(original_attempt.get("operation") == "apply"
                and all(original_attempt.get(k) == attempt[k] for k in ("migration_id", "material_sha256", "sql_sha256")),
                "original_attempt_material_mismatch")
        sql = build_resolve_sql(material, original_attempt.get("application_name"))
    write_private(output, "attempt.private.json", canonical(attempt) + "\n")
    write_private(output, "executed.private.sql", sql)
    # Artifacts and attempt identity are durable BEFORE a server can receive SQL.
    outcome = {"status": "UNKNOWN", "migration_id": material["migration_id"], "material_sha256": digest(material)}
    try:
        code, stdout = run_sql(connection, sql, output, application_name)
        if code == 0:
            verify_source_files(material)
            require(source_hashes(escrow=escrow_profile(material)) == material["tool_source_hashes"], "financial_tool_source_changed")
            result = PLANNER.parse_json(stdout)
            allowed = ("COMMITTED", "NOT_APPLIED") if original_attempt else ("COMMITTED",)
            if type(result) is dict and set(result) == set(outcome) and result.get("status") in allowed \
                    and all(result.get(key) == outcome[key] for key in ("migration_id", "material_sha256")):
                outcome = result
    except Exception:
        # After a server may have received SQL, even an unexpected verifier error
        # must not escape as REFUSED (which would imply no write was attempted).
        # The durable original attempt is the only safe recovery reference.
        pass
    try:
        write_private(output, "result.private.json", canonical(outcome) + "\n")
    except Exception:
        outcome = {**outcome, "status": "UNKNOWN"}
    return outcome


PREPARE_ARGUMENTS = ("plan", "source_receipt", "export_proof", "identity_map", "identity_manifest",
                     "source_review", "reservations", "target_receipt", "migration_id", "runtime_role")


def validate_prepare_arguments(args):
    require(all(getattr(args, name, None) for name in PREPARE_ARGUMENTS), "prepare_arguments_missing")
    require(all(getattr(args, name, None) is None for name in ("material", "sql", "approval", "resolve_attempt")),
            "execution_arguments_on_prepare")
    composed = getattr(args, "include_legacy_escrow", False)
    require(all(getattr(args, name, None) for name in ESCROW_SOURCE_FILES) if composed else
            all(getattr(args, name, None) is None for name in ESCROW_SOURCE_FILES), "escrow_profile_arguments_mismatch")
    return composed


def validate_target_receipt(target, plan, migration_id):
    require(type(target) is dict and target.get("status") == "initialized-identity-bound-financial-target"
            and target.get("application_or_worker_started") is False
            and target.get("migration_id") == migration_id
            and target.get("source_archive_sha256") == plan["source_archive_sha256"], "target_receipt_invalid")


# @mark migration.financial-readiness role=implementation
def check_readiness(args, output: Path) -> dict:
    """Diagnose existing private prerequisites without SQL or financial outputs.

    Hash/shape/binding checks are not economic approval or evidence of current
    target/runtime state. Full source replanning and mapper replay remain in
    prepare/execute; this mode does not duplicate the large raw-row DTO.
    """
    composed = validate_prepare_arguments(args)
    checks, values, files = {}, {}, {}

    def check(name, function, dependencies=()):
        if any(checks[key]["status"] != "PASS" for key in dependencies):
            checks[name] = {"status": "BLOCKED", "error_code": "readiness_dependency_unavailable"}
            return
        try:
            function()
            checks[name] = {"status": "PASS"}
        except Error as error:
            # All validators use fixed codes. Never print a path, a source value,
            # a decoder traceback or an operating-system exception message.
            code = str(error)
            checks[name] = {"status": "BLOCKED", "error_code": code if re.fullmatch(r"[a-z][a-z0-9_]{0,127}", code)
                            else "readiness_validation_failed"}
        except (OSError, ValueError, TypeError, KeyError, PREFLIGHT.FinancialPreflightError):
            checks[name] = {"status": "BLOCKED", "error_code": "readiness_input_invalid_or_unavailable"}

    json_inputs = {"plan", "source_receipt", "export_proof", "identity_manifest", "source_review",
                   "reservations", "target_receipt"}
    if composed:
        json_inputs |= {"escrow_candidate", "escrow_unit_evidence", "inflight_evidence"}
    names = json_inputs | {"identity_map"} | (ESCROW_SOURCE_FILES if composed else frozenset())
    for name in sorted(names):
        def load(name=name):
            path = IDENTITY.private_backup_path(getattr(args, name))
            before = IDENTITY.sha256_file(path)
            text = read_private(path)
            require(bool(text.strip()), "readiness_input_empty")
            if name in json_inputs:
                value = PLANNER.parse_json(text)
                require(type(value) is dict, "readiness_input_shape_invalid")
                if name == "plan":
                    require(all(type(value.get(key)) is dict for key in ("public_summary", "source_evidence", "identity_provenance"))
                            and all(type(row) is dict for row in value["source_evidence"].values())
                            and all(type(value.get(key, [])) is list and all(type(row) is dict for row in value.get(key, []))
                                    for key in ("wallet_positions", "wallet_transfers")), "readiness_input_shape_invalid")
                elif name == "source_receipt":
                    require(type(value.get("archive")) is dict and type(value.get("source")) is dict
                            and type(value.get("private_metadata_files")) is list
                            and all(type(path) is str for path in value["private_metadata_files"]), "readiness_input_shape_invalid")
                elif name == "target_receipt":
                    require(type(value.get("connection")) is dict, "readiness_input_shape_invalid")
                values[name] = value
            elif name == "identity_map":
                records = [PLANNER.parse_json(line) for line in text.splitlines() if line]
                require(all(type(row) is dict for row in records), "readiness_input_shape_invalid")
                values[name] = records
            require(IDENTITY.sha256_file(path) == before, "readiness_input_changed")
            files[name] = {"path": str(path), "sha256": before}
        check("input_" + name, load)

    def source_binding():
        source, proof, plan = (values[key] for key in ("source_receipt", "export_proof", "plan"))
        PLANNER.verify_plan_mapping(plan)
        require(source.get("status") == "snapshot-restored-baseline-verified-frozen-and-classified", "source_receipt_status_invalid")
        archive = IDENTITY.private_backup_path(Path(source["archive"]["path"]))
        archive_hash = IDENTITY.sha256_file(archive)
        require(archive_hash == source["archive"]["sha256"] == proof["source_archive_sha256"] == plan["source_archive_sha256"],
                "source_archive_binding_mismatch")
        raw = IDENTITY.private_backup_path(args.export_proof.with_name("source-financial-export.private.jsonl"))
        raw_hash = IDENTITY.sha256_file(raw)
        require(raw_hash == proof["source_export_sha256"], "source_export_file_mismatch")
        _, _, metadata_hashes, _, _ = PLANNER.metadata(source)
        require(digest(metadata_hashes) == digest(proof["metadata_hashes"]), "source_export_metadata_hash_mismatch")
        require(proof.get("source_column_coverage_exact") is True and proof.get("source_row_baseline_exact") is True,
                "source_export_baseline_unproven")
        require(digest(proof["selected_table_counts"]) == digest({name: row["row_count"] for name, row in plan["source_evidence"].items()}),
                "source_export_table_selection_mismatch")
        provenance = PLANNER.bind_identity_provenance(values["identity_manifest"],
            map_file_sha256=files["identity_map"]["sha256"], source_archive_sha256=archive_hash)
        require(digest(provenance) == digest(plan["identity_provenance"]), "identity_provenance_mismatch")
        identities = PLANNER.identity_index(values["identity_map"])
        require(digest(identities) == plan["identity_map_sha256"], "identity_map_mismatch")
        values["observation"] = PLANNER.source_observation(source)
        files.update(archive={"path": str(archive), "sha256": archive_hash}, source_export={"path": str(raw), "sha256": raw_hash})
    check("source_file_bindings", source_binding,
          tuple("input_" + name for name in ("source_receipt", "export_proof", "plan", "identity_manifest", "identity_map")))

    def economic_review():
        validate_source_review(values["source_review"], values["plan"],
            **({"candidate": values["escrow_candidate"], "unit_evidence": values["escrow_unit_evidence"],
                "inflight": values["inflight_evidence"]} if composed else {}))
    review_inputs = ("source_review", "plan", *( ("escrow_candidate", "escrow_unit_evidence", "inflight_evidence") if composed else ()))
    check("economic_review", economic_review, tuple("input_" + name for name in review_inputs))
    check("reservation_evidence", lambda: PLANNER.build_opening_payloads(values["plan"], migration_id=args.migration_id,
          observation=values["observation"], reservation_reconciliation=values["reservations"]),
          ("source_file_bindings", "input_reservations"))
    if composed:
        def source_context():
            plan = values["plan"]
            return {**{key: plan[key] for key in ("source_archive_sha256", "mapping_sha256", "identity_map_sha256")},
                "migration_id": args.migration_id, "source_files": files,
                "escrow_candidate": values["escrow_candidate"], "escrow_unit_evidence": values["escrow_unit_evidence"]}

        def review_context():
            plan = values["plan"]
            return {**source_context(), "source_plan": plan, "source_review": values["source_review"],
                "inflight_evidence": values["inflight_evidence"],
                "escrow_batch": {"observed_at": values["observation"]["observed_at"]},
                "counters": [{"user_id": row["source_user_id"]} for row in plan["wallet_positions"]]}
        check("escrow_unit_and_candidate_bindings", lambda: validate_escrow_source_evidence(source_context()),
              ("source_file_bindings", "input_escrow_candidate", "input_escrow_unit_evidence"))
        check("inflight_source_evidence", lambda: validate_inflight(review_context()),
              ("source_file_bindings", "input_source_review", "input_escrow_candidate", "input_escrow_unit_evidence", "input_inflight_evidence", "input_inflight_process_evidence"))
        check("economic_and_policy_evidence_files", lambda: validate_review_evidence_files(review_context()),
              ("source_file_bindings", "input_source_review", "input_escrow_unit_evidence", "input_escrow_candidate", "input_inflight_evidence",
               "input_source_review_evidence", "input_escrow_policy_evidence"))

    def target_declaration():
        validate_target_receipt(values["target_receipt"], values["plan"], args.migration_id)
        roles = args.runtime_role
        connection = values["target_receipt"]["connection"]
        owner, login = IDENTITY.connection_roles(connection)
        require(type(roles) is list and roles and len(roles) == len(set(roles))
                and all(type(role) is str and IDENTITY.IDENTIFIER.fullmatch(role) for role in roles)
                and not {owner, login}.intersection(roles), "runtime_roles_invalid")
    check("target_and_role_declarations", target_declaration, ("input_target_receipt", "input_plan"))
    blockers = sum(row["status"] == "BLOCKED" for row in checks.values())
    result = {"format_version": 1, "scope": "financial-offline-readiness-v1",
        "status": "OFFLINE_CHECK_BLOCKED" if blockers else "OFFLINE_CHECK_PASSED", "checks": checks, "blocker_count": blockers,
        "target_state": "NOT_CHECKED_OFFLINE", "runtime_provision": "NOT_CHECKED_OFFLINE",
        "source_plan_rebuilt": False, "source_mapper_rebuilt": False, "economic_authority_verified": False,
        "can_apply": False, "material_created": False, "approval_created": False, "sql_created": False,
        "source_queries": 0, "target_queries": 0, "target_writes": 0}
    write_private(output, "readiness.private.json", canonical(result) + "\n")
    return result


def prepare(args, output: Path) -> dict:
    """Read an existing frozen export; query only the explicitly selected target."""
    composed = validate_prepare_arguments(args)
    load = lambda path: PLANNER.parse_json(read_private(path))
    source = load(args.source_receipt)
    plan = load(args.plan)
    identity_manifest = load(args.identity_manifest)
    source_review = load(args.source_review)
    escrow_inputs = ({name: load(getattr(args, name)) for name in
                     ("escrow_candidate", "escrow_unit_evidence", "inflight_evidence")} if composed else {})
    validate_source_review(source_review, plan, candidate=escrow_inputs.get("escrow_candidate"),
                           unit_evidence=escrow_inputs.get("escrow_unit_evidence"), inflight=escrow_inputs.get("inflight_evidence"))
    provenance = PLANNER.bind_identity_provenance(identity_manifest,
        map_file_sha256=IDENTITY.sha256_file(IDENTITY.private_backup_path(args.identity_map)),
        source_archive_sha256=source["archive"]["sha256"])
    identities = PLANNER.identity_index([PLANNER.parse_json(line) for line in read_private(args.identity_map).splitlines() if line])
    # This reuses a prior local read-only export; it cannot connect to the source.
    tables, proof = PLANNER.reuse_frozen_export(source, args.export_proof, output)
    rebuilt = PLANNER.plan_financial(tables, identities, source_sha256=proof["source_archive_sha256"],
        source_timezone=plan.get("source_timezone"), identity_provenance=provenance,
        primary_keys=proof["source_primary_keys"], column_schema=proof["source_column_schema"])
    require(digest(rebuilt) == digest(plan), "source_plan_rebuild_mismatch")
    target = load(args.target_receipt)
    validate_target_receipt(target, plan, args.migration_id)
    connection = target["connection"]
    code, raw = run_sql(connection, "BEGIN READ ONLY;" + inventory_sql() + ";COMMIT;", output,
        "lmm-financial-preview", readonly=True, label="inventory")
    require(code == 0, "target_inventory_read_failed")
    inventory = PLANNER.parse_json(raw)
    code, raw = run_sql(connection, snapshot_sql(inventory, args.migration_id), output,
        "lmm-financial-preview", readonly=True, label="snapshot")
    require(code == 0, "target_snapshot_read_failed")
    snapshot = PLANNER.parse_json(raw)
    IDENTITY.verify_instance(snapshot["instance"], connection, target["instance_identity"], source=False)
    require(snapshot["instance"]["system_identifier"] == target["instance_identity"]["system_identifier"],
            "target_system_identifier_mismatch")
    require((str(snapshot["instance"]["database_oid"]), datetime.fromisoformat(snapshot["instance"]["postmaster_started_at"])) !=
            (str(proof["source_instance"]["database_oid"]), datetime.fromisoformat(proof["source_instance"]["postmaster_started_at"])),
            "source_target_same_database")
    paths = {"archive": Path(source["archive"]["path"]), "source_receipt": args.source_receipt,
        "financial_plan": args.plan, "source_export": Path(proof["reused_source_export_path"]),
        "source_export_proof": args.export_proof, "identity_map": args.identity_map,
        "identity_manifest": args.identity_manifest, "source_review": args.source_review, "reservations": args.reservations}
    if composed:
        paths.update({name: getattr(args, name) for name in ESCROW_SOURCE_FILES})
    files = {name: {"path": str(IDENTITY.private_backup_path(path)), "sha256": IDENTITY.sha256_file(path)}
             for name, path in paths.items()}
    material = prepare_material(plan, migration_id=args.migration_id, identities=identities,
        observation=PLANNER.source_observation(source), reservations=load(args.reservations), review=source_review,
        snapshot=snapshot, target_contract=target["contract"], runtime_roles=args.runtime_role, source_files=files,
        **escrow_inputs)
    verify_source_files(material)
    sql = build_apply_sql(material)
    write_private(output, "material.private.json", canonical(material) + "\n")
    write_private(output, "opening.private.sql", sql)
    # An approval template is NOT authorization. An independent offline review
    # must bind all three hashes and replace the decision with the documented value.
    approval = {"format_version": 2 if composed else 1, "scope": material["scope"], "decision": "REVIEW_REQUIRED",
        "material_sha256": digest(material), "sql_sha256": sha(sql.encode()),
        "source_review_sha256": digest(material["source_review"])}
    write_private(output, "approval-required.private.json", canonical(approval) + "\n")
    return {"status": "PREPARED_NOT_APPLIED", "migration_id": args.migration_id,
            "material_sha256": digest(material), "target_writes": 0}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("material", "sql", "approval", "target-receipt", "plan", "source-receipt",
                 "export-proof", "identity-map", "identity-manifest", "source-review", "reservations",
                 *(name.replace("_", "-") for name in sorted(ESCROW_SOURCE_FILES))):
        parser.add_argument("--" + name, type=Path)
    parser.add_argument("--include-legacy-escrow", action="store_true",
                        help="Prepare the separate reviewed wallet/counter/legacy-escrow profile; requires source, policy and process-drain evidence files")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--migration-id")
    parser.add_argument("--runtime-role", action="append")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--prepare", action="store_true")
    mode.add_argument("--check-readiness", action="store_true", help="Check existing private source prerequisites only; no database access, material, SQL or approval")
    mode.add_argument("--apply", action="store_true")
    mode.add_argument("--resolve-attempt", type=Path)
    args = parser.parse_args()
    output = None
    try:
        if args.prepare or args.check_readiness:
            validate_prepare_arguments(args)
        require(args.prepare or args.check_readiness or (not args.include_legacy_escrow and all(getattr(args, name) is None for name in ESCROW_SOURCE_FILES)),
                "escrow_prepare_arguments_on_execution")
        IDENTITY.private_backup_path(args.output.parent, directory=True)
        args.output.mkdir(mode=0o700)
        output = IDENTITY.private_backup_path(args.output, directory=True)
        if args.check_readiness:
            result = check_readiness(args, output)
        elif args.prepare:
            result = prepare(args, output)
        else:
            require(all((args.material, args.approval, args.sql, args.target_receipt)), "apply_arguments_missing")
            material = PLANNER.parse_json(read_private(args.material))
            approval = PLANNER.parse_json(read_private(args.approval))
            target = PLANNER.parse_json(read_private(args.target_receipt))
            original = PLANNER.parse_json(read_private(args.resolve_attempt)) if args.resolve_attempt else None
            require(target.get("application_or_worker_started") is False, "application_start_not_excluded")
            result = execute(material, approval=approval, original_sql=read_private(args.sql),
                             connection=target["connection"], output=output, original_attempt=original)
        print(canonical(result))
        return 2 if result["status"] == "UNKNOWN" else 1 if result["status"] == "OFFLINE_CHECK_BLOCKED" else 0
    except BrokenPipeError:
        # A missing console ACK does not cancel a committed database transaction.
        return 2
    except (OSError, ValueError, TypeError, KeyError, Error, PREFLIGHT.FinancialPreflightError, subprocess.TimeoutExpired) as error:
        refusal = {"status": "REFUSED", "error_code": str(error) if isinstance(error, Error) else type(error).__name__}
        if output is not None:
            try:
                write_private(output, "refusal.private.json", canonical(refusal) + "\n")
            except OSError:
                pass
        print(canonical(refusal), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
