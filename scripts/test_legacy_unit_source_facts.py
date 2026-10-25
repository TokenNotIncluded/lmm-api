"""Synthetic unit-source provenance and non-authority regressions; no database."""
from copy import deepcopy
import hashlib
from pathlib import Path
from types import SimpleNamespace
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import legacy_unit_source_facts as M
import test_legacy_escrow_mapping as fixtures


def inputs():
    data = fixtures.data()
    plan = data["plan"]
    rows = [{"key": name, "value": "500000"} for name in sorted(M.OPTION_KEYS)]
    digests = [M.F.digest(row) for row in rows]
    plan["source_evidence"]["options"] = {"row_count": 4, "row_hashes": list(digests)}
    fixtures.reseal(plan)
    audit = plan["refund_lineage"]["wallet_rebase_audits"][0]
    old_archive = "e" * 64
    text = "适用归档为 " + "f" * 64 + "。\n旧来源技术政策提案，不是新批批准。\n"
    text_hash = hashlib.sha256(text.encode()).hexdigest()
    return {"source": {"archive": {"sha256": "a" * 64}},
        "proof": {"source_archive_sha256": "a" * 64, "source_export_sha256": "b" * 64,
                  "selected_table_counts": {"options": 4}},
        "plan": plan,
        "options": {"scope": "raw-source-options-facts-not-approval", "source_archive_sha256": "a" * 64,
                    "source_export_sha256": "b" * 64, "rows": rows, "source_row_sha256": digests},
        "runtime": {"source_archive_sha256": "a" * 64, "source_revision": M.LEGACY_REVISION,
                    "binary_sha256": "9" * 64, "buildinfo_matches_live_binary": True},
        "buildinfo": {"vcs.revision": M.LEGACY_REVISION, "vcs.modified": "false"},
        "binary_sha256": "9" * 64,
        "code": {"legacy_revision": M.LEGACY_REVISION, "native_revision": M.NATIVE_REVISION,
                 "legacy_file_sha256": dict(M.LEGACY_CODE), "native_ledger_sha256": M.NATIVE_SHA256},
        "identities": data["identity_records"],
        "old_review": {"decision": "approved-offline-opening-with-legacy-escrow",
                       "scope": "financial-opening-with-legacy-escrow-v1", "source_archive_sha256": old_archive},
        "old_unit": {"source_archive_sha256": old_archive, "source_policy_evidence_sha256": text_hash,
                     "source_application_revision": M.LEGACY_REVISION,
                     "parent_plan_sha256": audit["declared_plan_sha256"],
                     "parent_audit_row_sha256": audit["source_row_sha256"]},
        "old_policy_text": text, "old_policy_sha256": text_hash}


class UnitSourceFactsTests(unittest.TestCase):
    def test_recomputes_bindings_and_never_inherits_batch_authority(self):
        data = inputs()
        before = deepcopy(data)
        result = M.build_facts(**data)
        self.assertEqual(data, before)
        self.assertEqual(result["status"], "TECHNICAL_FACTS_ONLY")
        for name in ("can_apply", "economic_approval_verified", "current_batch_approved", "approval_created"):
            self.assertIs(result[name], False)
        self.assertEqual(result["source_queries"], 0)
        self.assertEqual(result["target_queries"], 0)
        self.assertEqual(result["target_writes"], 0)
        self.assertEqual(result["bindings"]["source_archive_sha256"], "a" * 64)
        self.assertTrue(result["technical_propositions"]["parent_rebase_semantic_contract_validated"])
        self.assertTrue(result["historical_policy_reference"]["same_parent_plan"])
        self.assertTrue(result["historical_policy_reference"]["prose_machine_scope_conflict"])
        self.assertFalse(result["historical_policy_reference"]["old_batch_approval_inherited"])
        self.assertNotIn("target_personal_account_id", M.F.canonical(result))
        self.assertEqual(result["facts_sha256"], M.F.digest({k: v for k, v in result.items() if k != "facts_sha256"}))

    def test_code_and_runtime_revisions_cannot_drift(self):
        for target, field in (("code", "legacy_revision"), ("code", "native_revision"),
                              ("runtime", "source_revision"), ("buildinfo", "vcs.revision")):
            data = inputs(); data[target][field] = "1" * 40
            with self.subTest(target=target, field=field), self.assertRaises(M.F.MappingError):
                M.build_facts(**data)

    def test_reviewed_code_and_native_contract_bytes_are_pinned(self):
        for kind in ("legacy", "native"):
            data = inputs()
            if kind == "legacy":
                data["code"]["legacy_file_sha256"][next(iter(M.LEGACY_CODE))] = "0" * 64
            else:
                data["code"]["native_ledger_sha256"] = "0" * 64
            with self.subTest(kind=kind), self.assertRaises(M.F.MappingError):
                M.build_facts(**data)

    def test_git_collector_rehashes_actual_stdout_instead_of_trusting_a_label(self):
        with patch.object(M.subprocess, "run", return_value=SimpleNamespace(returncode=0, stdout=b"changed source")):
            with self.assertRaisesRegex(M.F.MappingError, "unit_legacy_code_changed"):
                M.read_code(Path("/synthetic/repository"))

    def test_modified_unknown_or_wrong_binary_is_not_observed_runtime(self):
        for value in ("true", "unknown", False, None):
            data = inputs(); data["buildinfo"]["vcs.modified"] = value
            with self.subTest(value=value), self.assertRaises(M.F.MappingError):
                M.build_facts(**data)
        data = inputs(); data["binary_sha256"] = "8" * 64
        with self.assertRaisesRegex(M.F.MappingError, "unit_running_binary_binding_mismatch"):
            M.build_facts(**data)
        data = inputs(); data["runtime"]["buildinfo_matches_live_binary"] = 1
        with self.assertRaises(M.F.MappingError):
            M.build_facts(**data)

    def test_archive_export_and_plan_mutations_are_rejected(self):
        for target in ("source", "proof", "options", "runtime"):
            data = inputs()
            if target == "source": data[target]["archive"]["sha256"] = "c" * 64
            else: data[target]["source_archive_sha256"] = "c" * 64
            with self.subTest(target=target), self.assertRaises(M.F.MappingError): M.build_facts(**data)
        data = inputs(); data["options"]["source_export_sha256"] = "c" * 64
        with self.assertRaises(M.F.MappingError): M.build_facts(**data)
        data = inputs(); data["plan"]["refund_lineage"]["wallet_rebase_audits"][0]["source_migration_id"] = "tampered"
        with self.assertRaisesRegex(M.F.MappingError, "financial_plan_mapping_hash_mismatch"):
            M.build_facts(**data)

    def test_option_values_row_fields_and_bound_membership_cannot_be_replaced(self):
        for value in ("500001", "500000.0", 500000, True, "NaN"):
            data = inputs(); data["options"]["rows"][0]["value"] = value
            with self.subTest(value=value), self.assertRaises(M.F.MappingError): M.build_facts(**data)
        data = inputs(); data["options"]["rows"][0]["future_unit"] = "unknown"
        with self.assertRaisesRegex(M.F.MappingError, "unit_options_fields_invalid"): M.build_facts(**data)
        data = inputs(); data["options"]["source_row_sha256"][0] = "c" * 64
        with self.assertRaisesRegex(M.F.MappingError, "unit_option_row_digest_mismatch"): M.build_facts(**data)
        data = inputs(); data["plan"]["source_evidence"]["options"]["row_hashes"] = ["c" * 64]; fixtures.reseal(data["plan"])
        with self.assertRaisesRegex(M.F.MappingError, "unit_options_not_bound_to_source_plan"): M.build_facts(**data)

    def test_missing_duplicate_and_inconsistent_options_do_not_inherit_defaults(self):
        for mutation in ("missing", "duplicate", "count"):
            data = inputs()
            if mutation == "missing": data["options"]["rows"].pop()
            elif mutation == "duplicate": data["options"]["rows"][1] = deepcopy(data["options"]["rows"][0])
            else: data["proof"]["selected_table_counts"]["options"] = 5
            with self.subTest(mutation=mutation), self.assertRaises(M.F.MappingError): M.build_facts(**data)

    def test_parent_semantics_are_not_proven_by_500000_alone(self):
        data = inputs(); data["plan"]["refund_lineage"]["wallet_rebase_audits"][0]["full_plan_unknown_fields_preserved"]["usd_credit_conversion"] = 500001
        fixtures.reseal(data["plan"])
        with self.assertRaises(M.F.MappingError): M.build_facts(**data)
        data = inputs(); data["plan"]["source_evidence"]["wallet_credit_rebases"]["row_hashes"] = []; fixtures.reseal(data["plan"])
        with self.assertRaisesRegex(M.F.MappingError, "unit_parent_not_bound_to_source_plan"): M.build_facts(**data)

    def test_old_prose_scope_conflict_or_even_matching_scope_never_means_ready(self):
        data = inputs()
        result = M.build_facts(**data)
        self.assertTrue(result["historical_policy_reference"]["prose_machine_scope_conflict"])
        self.assertFalse(result["can_apply"])
        self.assertIn("old machine archive disagree", M.technical_policy_text(result))
        data["old_policy_text"] = "适用归档为 " + "e" * 64
        digest = hashlib.sha256(data["old_policy_text"].encode()).hexdigest()
        data["old_policy_sha256"] = digest; data["old_unit"]["source_policy_evidence_sha256"] = digest
        result = M.build_facts(**data)
        self.assertFalse(result["historical_policy_reference"]["prose_machine_scope_conflict"])
        self.assertFalse(result["economic_approval_verified"])
        self.assertFalse(result["current_batch_approved"])
        text = M.technical_policy_text(result)
        self.assertNotIn("archive disagree", text)
        self.assertIn("agree for their historical archive only", text)
        self.assertIn("Historical counters and any conversion of their denominations remain outside this proof", text)

    def test_old_machine_or_policy_byte_changes_cannot_be_hidden(self):
        data = inputs(); data["old_unit"]["source_archive_sha256"] = "a" * 64
        with self.assertRaisesRegex(M.F.MappingError, "unit_old_machine_scope_mismatch"): M.build_facts(**data)
        data = inputs(); data["old_policy_sha256"] = "c" * 64
        with self.assertRaisesRegex(M.F.MappingError, "unit_old_policy_bytes_changed"): M.build_facts(**data)

    def test_buildinfo_reader_requires_unique_exact_static_fields(self):
        raw = "binary\n\tbuild\tvcs.revision=" + M.LEGACY_REVISION + "\n\tbuild\tvcs.modified=false\n"
        self.assertEqual(M.parse_buildinfo(raw), {"vcs.revision": M.LEGACY_REVISION, "vcs.modified": "false"})
        for text in ("binary\n", raw + "\tbuild\tvcs.modified=false\n"):
            with self.assertRaises(M.F.MappingError): M.parse_buildinfo(text)


if __name__ == "__main__":
    unittest.main()
