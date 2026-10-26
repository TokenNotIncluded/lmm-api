#!/usr/bin/env python3
"""Recompute frozen credit-unit facts, never financial approval or application.

The reviewed Go contract is this repository's AGPL source at the pinned revision.
Go's own build-info reader inspects the archived executable without running it.
Reuse the financial planner's typed digests, source metadata and parent parser;
the 244MB raw export is deliberately not read by this command.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys

_SPEC = importlib.util.spec_from_file_location("unit_financial_source", Path(__file__).with_name("migrate-legacy-financial.py"))
F = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(F)
import legacy_bounty_basis as B
import legacy_escrow_mapping as E

LEGACY_REVISION = "cc52aa195c6f2a01d703a1a407be73c31b89867d"
NATIVE_REVISION = "8134849584b612aa07717388bf38dc7fe9bfac4f"
LEGACY_CODE = {
    "apps/api-go/common/currency_units.go": "96cb9778aa8459f6f0a41a66b0ef76b14080a2050cc5047985701f53ba700cc4",
    "apps/api-go/common/public_credit_units.go": "ba0bfd2813812e8e183a0dec656cd47c7ede4acc6b0ba7b49d632e09aa2a2514",
    "apps/api-go/model/currency_units.go": "45842abacf22878c31d387560ddf521d94e0d95a003e3789398801a562948245",
    "apps/api-go/model/option.go": "bc11532a32d8265054a0d729e098b5d833274f6500fc31b96c84c418b5ee82a8",
    "apps/api-go/main.go": "13778a62ec5d158b12a2f9bb447888763a1453f1bdf9f913822df821e5bcb6e8",
    "apps/api-go/model/open_source_bounty.go": "4180a0978a30ca28ff515147184d2ae97a894c04b2ffc9286b6b58033d6841e2",
}
NATIVE_PATH = "services/core/schema/ledger.sql"
NATIVE_SHA256 = "ce973f6fee25904d4fcdc87895ec49e6c53e10c9fb3508df4d7d72ace16df77f"
OPTION_KEYS = {"CreditsPerUSD", "LegacyPricingQuotaPerUnit", "PublicCreditsPerUSD", "QuotaPerUnit"}
HEX = re.compile(r"[0-9a-f]{64}\Z")
require = F.require


def parse_buildinfo(text: str) -> dict:
    result = {}
    for line in text.splitlines():
        parts = line.strip().split("\t")
        if len(parts) == 2 and parts[0] == "build" and "=" in parts[1]:
            key, value = parts[1].split("=", 1)
            if key in ("vcs.revision", "vcs.modified"):
                require(key not in result, "unit_buildinfo_duplicate_field")
                result[key] = value
    require(set(result) == {"vcs.revision", "vcs.modified"}, "unit_buildinfo_missing_fields")
    return result


def validate_code(code: dict) -> None:
    require(code.get("legacy_revision") == LEGACY_REVISION and code.get("native_revision") == NATIVE_REVISION,
            "unit_code_revision_not_reviewed")
    require(code.get("legacy_file_sha256") == LEGACY_CODE, "unit_legacy_code_changed")
    require(code.get("native_ledger_sha256") == NATIVE_SHA256, "unit_native_contract_changed")


def inspect_parent_policy(review: dict, old_unit: dict, text: str, text_sha256: str) -> dict:
    require(review.get("decision") == "approved-offline-opening-with-legacy-escrow"
            and review.get("scope") == "financial-opening-with-legacy-escrow-v1", "unit_old_review_scope_invalid")
    require(review.get("source_archive_sha256") == old_unit.get("source_archive_sha256"), "unit_old_machine_scope_mismatch")
    require(old_unit.get("source_policy_evidence_sha256") == text_sha256, "unit_old_policy_bytes_changed")
    declared = set(re.findall(r"适用归档为\s*([0-9a-f]{64})", text))
    require(len(declared) == 1, "unit_old_policy_archive_declaration_unproven")
    prose_archive = next(iter(declared))
    return {"old_machine_archive_sha256": old_unit["source_archive_sha256"],
            "old_prose_archive_sha256": prose_archive,
            "prose_machine_scope_conflict": prose_archive != old_unit["source_archive_sha256"],
            "role": "historical-policy-reference-only-never-current-batch-approval",
            "old_batch_approval_inherited": False}


def build_facts(*, source: dict, proof: dict, plan: dict, options: dict, runtime: dict,
                buildinfo: dict, binary_sha256: str, code: dict, identities: list,
                old_review: dict, old_unit: dict, old_policy_text: str, old_policy_sha256: str) -> dict:
    """Validate independent bindings and return technical facts with no authority."""
    validate_code(code)
    F.verify_plan_mapping(plan)
    archive = source.get("archive", {}).get("sha256")
    export = proof.get("source_export_sha256")
    require(type(archive) is str and HEX.fullmatch(archive) is not None
            and type(export) is str and HEX.fullmatch(export) is not None, "unit_source_digest_invalid")
    require(all(row.get("source_archive_sha256") == archive for row in (proof, plan, options, runtime)), "unit_source_archive_mismatch")
    require(options.get("source_export_sha256") == export, "unit_source_export_mismatch")
    require(runtime.get("source_revision") == LEGACY_REVISION
            and buildinfo.get("vcs.revision") == LEGACY_REVISION, "unit_runtime_revision_mismatch")
    require(buildinfo.get("vcs.modified") == "false", "unit_binary_modified_or_unknown")
    require(type(binary_sha256) is str and HEX.fullmatch(binary_sha256) is not None
            and runtime.get("binary_sha256") == binary_sha256
            and runtime.get("buildinfo_matches_live_binary") is True, "unit_running_binary_binding_mismatch")
    rows, digests = options.get("rows"), options.get("source_row_sha256")
    require(type(rows) is list and len(rows) == 4 and type(digests) is list and len(digests) == 4, "unit_options_coverage_invalid")
    require(all(type(row) is dict and set(row) == {"key", "value"} for row in rows)
            and {row["key"] for row in rows} == OPTION_KEYS, "unit_options_fields_invalid")
    require(all(row["value"] == "500000" for row in rows), "unit_options_not_reviewed_fixed_values")
    require([F.digest(row) for row in rows] == digests, "unit_option_row_digest_mismatch")
    options_evidence = plan.get("source_evidence", {}).get("options", {})
    require(set(digests) <= set(options_evidence.get("row_hashes", []))
            and type(options_evidence.get("row_count")) is int
            and options_evidence["row_count"] == proof.get("selected_table_counts", {}).get("options"),
            "unit_options_not_bound_to_source_plan")
    account_map = E._identities(F, identities, plan)
    accounts = {uid: row["target_personal_account_id"] for uid, row in account_map.items()}
    audits = plan.get("refund_lineage", {}).get("wallet_rebase_audits", [])
    require(type(audits) is list and len(audits) == 1, "unit_single_parent_required")
    audit = audits[0]
    require(audit.get("source_row_sha256") in plan.get("source_evidence", {}).get("wallet_credit_rebases", {}).get("row_hashes", []),
            "unit_parent_not_bound_to_source_plan")
    parent, _ = B.parent_contract(F, audit, accounts)
    historical = inspect_parent_policy(old_review, old_unit, old_policy_text, old_policy_sha256)
    historical["same_reviewed_code_revision"] = old_unit.get("source_application_revision") == LEGACY_REVISION
    historical["same_parent_plan"] = old_unit.get("parent_plan_sha256") == parent["plan_sha256"]
    historical["same_parent_audit_row"] = old_unit.get("parent_audit_row_sha256") == audit["source_row_sha256"]
    bindings = {"source_archive_sha256": archive, "source_export_sha256": export,
                "mapping_sha256": plan["mapping_sha256"], "identity_map_sha256": plan["identity_map_sha256"],
                "identity_records_sha256": F.digest(sorted(identities, key=lambda row: row["source_user_id"])),
                "source_application_revision": LEGACY_REVISION, "source_binary_sha256": binary_sha256,
                "parent_plan_sha256": parent["plan_sha256"], "parent_audit_row_sha256": audit["source_row_sha256"]}
    result = {"format_version": 1, "scope": "legacy-unit-source-facts-not-approval", "status": "TECHNICAL_FACTS_ONLY",
              "bindings": bindings, "reviewed_code": code, "source_option_row_sha256": digests,
              "technical_propositions": {"reviewed_source_current_ledger_unit": "credit_500k_usd",
                  "reviewed_source_contract_credits_per_usd": 500000, "native_contract_uses_same_integer_unit": True,
                  "native_opening_preserves_signed_integers_without_fx": True,
                  "native_negative_available_moves_to_debt_without_clamping_total": True,
                  "native_escrow_moves_clearing_to_reserved_without_wallet_debit": True,
                  "parent_rebase_semantic_contract_validated": True,
                  "post_parent_claim_requires_separate_exact_facts": True},
              "historical_policy_reference": historical,
              "evidence_boundaries": {"static_binary_read_not_executed": True,
                  "running_process_evidence": "referenced-readonly-observation-not-refreshed-here",
                  "raw_export_rehashed_here": False, "runtime_heap_currency_anchor_sampled": False,
                  "historical_parent_commit_proven_here": False,
                  "native_contract_evidence": "pinned-source-not-live-database-readiness"},
              "unknown_or_outside_scope": ["historical-payment-and-refund-currencies", "tariff-and-rounding-equivalence",
                  "historical-counter-denominations", "unrelated-entitlement-units", "inflight-reservations",
                  "provider-settlement", "current-batch-economic-approval", "final-source-fence"],
              "economic_approval_verified": False, "current_batch_approved": False, "can_apply": False,
              "approval_created": False, "source_queries": 0, "target_queries": 0, "target_writes": 0}
    result["facts_sha256"] = F.digest(result)
    return result


def read_code(repository: Path) -> dict:
    def blob(revision, name):
        value = subprocess.run(["git", "-C", str(repository), "show", revision + ":" + name], capture_output=True, check=False)
        require(value.returncode == 0, "unit_git_source_unavailable")
        return hashlib.sha256(value.stdout).hexdigest()
    result = {"legacy_revision": LEGACY_REVISION, "native_revision": NATIVE_REVISION,
              "legacy_file_sha256": {name: blob(LEGACY_REVISION, name) for name in LEGACY_CODE},
              "native_ledger_sha256": blob(NATIVE_REVISION, NATIVE_PATH)}
    validate_code(result)
    return result


def technical_policy_text(facts: dict) -> str:
    b = facts["bindings"]
    old_scope = ("The old prose archive and old machine archive disagree. "
                 if facts["historical_policy_reference"]["prose_machine_scope_conflict"]
                 else "Old prose and machine scope agree for their historical archive only. ")
    return ("Technical source-unit evidence; NOT owner approval, settlement or apply permission.\n\n"
            + "Fresh source archive: " + b["source_archive_sha256"] + "\n"
            + "Fresh typed mapping: " + b["mapping_sha256"] + "\n"
            + "Recomputed source facts digest: " + facts["facts_sha256"] + "\n\n"
            + "Pinned Go startup validates fixed persistent anchors before serving; production currency initialization is the sole caller that publishes the wallet anchor. Normal option updates cannot change that anchor. Fiat FX is separate. Exact source files and actual archived binary build info are bound in the facts.\n\n"
            + "The pinned native ledger contract uses the same integer unit. Its opening takes signed BIGINT units directly, splits negative available into debt, and does not apply FX. Historical counters and any conversion of their denominations remain outside this proof. Historical escrow opening moves clearing to reserved without a second wallet debit. These statements describe reviewed contracts, not live database readiness.\n\n"
            + "The original parent rebase semantic/hash contract is revalidated. This is not proof of historical COMMIT. Current post-parent claims need their separately verified source facts and cannot be accepted solely from timestamps.\n\n"
            + "Historical owner approval remains attached to its original batch. " + old_scope
            + "Neither the old PASS nor numeric equality is reused as current authority. Historical payment/refund currencies, counter denominations, in-flight state and current financial approval remain unknown or separately required.\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("source-receipt", "export-proof", "financial-plan", "planner-manifest", "identity-map", "options-facts",
                 "runtime-observation", "legacy-binary", "old-source-review", "old-unit-evidence", "old-policy"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--git-repository", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = None
    try:
        paths = {name: F.IDENTITY.private_backup_path(path) for name, path in vars(args).items()
                 if name not in ("output", "git_repository")}
        hashes = {name: F.IDENTITY.sha256_file(path) for name, path in paths.items()}
        tool_paths = (Path(__file__), Path(F.__file__), Path(F.IDENTITY.__file__), Path(B.__file__), Path(E.__file__))
        tool_hashes = {str(path): F.IDENTITY.sha256_file(path) for path in tool_paths}
        F.IDENTITY.private_backup_path(args.output.parent, directory=True)
        args.output.mkdir(mode=0o700)
        output = F.IDENTITY.private_backup_path(args.output, directory=True)
        load = lambda name: F.parse_json(paths[name].read_text())
        source, proof, plan, manifest = (load(name) for name in ("source_receipt", "export_proof", "financial_plan", "planner_manifest"))
        require(source.get("status") == "snapshot-restored-baseline-verified-frozen-and-classified", "unit_source_receipt_status_invalid")
        archive_path = F.IDENTITY.private_backup_path(Path(source["archive"]["path"]))
        require(F.IDENTITY.sha256_file(archive_path) == source["archive"]["sha256"], "unit_archive_bytes_changed")
        require(manifest.get("financial_plan_file_sha256") == hashes["financial_plan"]
                and manifest.get("source_export_sha256") == proof.get("source_export_sha256")
                and manifest.get("source_archive_sha256") == source["archive"]["sha256"], "unit_existing_plan_manifest_mismatch")
        _, _, metadata, _, _ = F.metadata(source)
        require(F.digest(metadata) == F.digest(proof.get("metadata_hashes"))
                and proof.get("source_column_coverage_exact") is True and proof.get("source_row_baseline_exact") is True,
                "unit_source_metadata_proof_mismatch")
        require(F.digest(proof.get("selected_table_counts")) == F.digest({name: row["row_count"] for name, row in plan["source_evidence"].items()}),
                "unit_source_plan_table_coverage_mismatch")
        identities = [F.parse_json(line) for line in paths["identity_map"].read_text().splitlines() if line]
        require(plan["identity_provenance"]["identity_map_file_sha256"] == hashes["identity_map"], "unit_identity_map_file_changed")
        command = ["go", "version", "-m", str(paths["legacy_binary"])]
        inspected = subprocess.run(command, capture_output=True, text=True, check=False, timeout=30)
        F.IDENTITY.write_private(output, "buildinfo.stdout.private.txt", inspected.stdout)
        F.IDENTITY.write_private(output, "buildinfo.stderr.private.txt", inspected.stderr)
        require(inspected.returncode == 0, "unit_go_buildinfo_reader_failed")
        facts = build_facts(source=source, proof=proof, plan=plan, options=load("options_facts"),
            runtime=load("runtime_observation"), buildinfo=parse_buildinfo(inspected.stdout), binary_sha256=hashes["legacy_binary"],
            code=read_code(args.git_repository), identities=identities, old_review=load("old_source_review"),
            old_unit=load("old_unit_evidence"), old_policy_text=paths["old_policy"].read_text(), old_policy_sha256=hashes["old_policy"])
        require(all(F.IDENTITY.sha256_file(path) == hashes[name] for name, path in paths.items()), "unit_input_changed_during_run")
        require(all(F.IDENTITY.sha256_file(Path(path)) == digest for path, digest in tool_hashes.items()), "unit_tool_changed_during_run")
        F.IDENTITY.write_private(output, "source-unit-facts.private.json", F.canonical(facts) + "\n")
        text = technical_policy_text(facts)
        F.IDENTITY.write_private(output, "unit-policy.TECHNICAL-NOT-APPROVAL.private.md", text)
        b = facts["bindings"]
        proposal = {key: b[key] for key in ("source_archive_sha256", "mapping_sha256", "source_export_sha256",
            "identity_records_sha256", "parent_plan_sha256", "parent_audit_row_sha256", "source_application_revision")}
        proposal.update(format_version=1, scope="legacy-escrow-source-unit-evidence", unit=E.UNIT, credits_per_usd=500000,
                        conversion_policy=E.POLICY, source_policy_evidence_sha256=hashlib.sha256(text.encode()).hexdigest())
        # Existing unit-evidence is a source artifact, not approval. Keep that exact
        # consumer shape; non-authority flags live in facts and the run receipt.
        require(set(proposal) == E.UNIT_FIELDS, "unit_consumer_shape_changed")
        F.IDENTITY.write_private(output, "unit-evidence.TECHNICAL-PROPOSAL.private.json", F.canonical(proposal) + "\n")
        receipt = {"scope": "legacy-unit-source-facts-execution", "status": "TECHNICAL_FACTS_ONLY",
            "economic_approval_verified": False, "current_batch_approved": False, "can_apply": False, "approval_created": False,
            "source_queries": 0, "target_queries": 0, "target_writes": 0, "raw_export_read": False,
            "inputs": {name: {"path": str(paths[name]), "sha256": digest} for name, digest in hashes.items()},
            "tool_source_sha256": tool_hashes, "facts_sha256": facts["facts_sha256"],
            "outputs": {path.name: F.IDENTITY.sha256_file(path) for path in output.iterdir() if path.is_file()}}
        F.IDENTITY.write_private(output, "execution-receipt.private.json", F.canonical(receipt) + "\n")
        print(F.canonical({"status": facts["status"], "source_option_count": 4, "facts_sha256": facts["facts_sha256"],
            "old_policy_scope_conflict": facts["historical_policy_reference"]["prose_machine_scope_conflict"], "can_apply": False}))
        return 0
    except (F.MappingError, OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError):
        if output is not None:
            F.IDENTITY.write_private(output, "failure.private.json", '{"status":"REFUSED","error_code":"unit_source_facts_validation_failed","approval_created":false}\n')
        print('{"status":"REFUSED","error_code":"unit_source_facts_validation_failed"}')
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
