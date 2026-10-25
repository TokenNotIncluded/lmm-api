"""Synthetic frozen-file escrow preview checks; no database or approval."""
import hashlib
import sys
from pathlib import Path
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))
import test_legacy_key_reconciliation as fixtures
import test_legacy_escrow_mapping as mapping

F = fixtures.F


# @mark migration.legacy-escrow-preview role=test
class EscrowPreviewCLITests(unittest.TestCase):
    write = fixtures.SourceRightsCLITest.write
    invoke = fixtures.SourceRightsCLITest.invoke
    reseal_source_rows = fixtures.SourceRightsCLITest.reseal_source_rows

    def setUp(self):
        original = fixtures.fixture

        def data():
            result = original()
            for table in ("open_source_bounty_projects", "open_source_bounty_challenges",
                          "open_source_bounty_disputes", "wallet_credit_rebases"):
                result["tables"][table] = []
            return result

        with mock.patch.object(fixtures, "fixture", data):
            fixtures.SourceRightsCLITest.setUp(self)

    def test_unknown_unit_and_empty_rights_remain_blocked_with_bound_private_evidence(self):
        code, output, stdout, stderr = self.invoke(extra=("--escrow-candidate-preview",))
        self.assertEqual((code, stderr), (0, ""))
        path = output / "escrow-candidate.private.json"
        candidate = F.parse_json(path.read_text())
        self.assertIsNone(candidate["batch_candidate"])
        public = F.parse_json(stdout)["escrow_candidate"]
        self.assertIs(public["approval_created"], False)
        self.assertIs(public["inflight_reservations_verified"], False)
        self.assertEqual(public["target_writes"], 0)
        self.assertNotIn("projects", public)
        manifest = F.parse_json((output / "manifest.private.json").read_text())
        self.assertEqual(manifest["escrow_candidate_file_sha256"], hashlib.sha256(path.read_bytes()).hexdigest())
        self.assertEqual(manifest["escrow_candidate_typed_sha256"], F.digest(candidate))
        self.assertEqual(manifest["escrow_mapping_helper_sha256"],
                         F.IDENTITY.sha256_file(ROOT / "legacy_escrow_mapping.py"))
        for artifact in output.iterdir():
            self.assertNotIn(b"SYNTHETIC-SECRET", artifact.read_bytes(), artifact.name)
        self.assertFalse((output / "approval-required.private.json").exists())

    def test_escrow_candidate_requires_offline_source_rights_mode(self):
        code, output, _, stderr = self.invoke(preview=False, extra=("--escrow-candidate-preview",))
        self.assertEqual(code, 1)
        self.assertIn("escrow_candidate_requires_source_rights_preview", stderr)
        self.assertFalse(output.exists())

    def test_unit_evidence_requires_explicit_candidate_mode(self):
        path = self.write("unit.json", {})
        code, output, _, stderr = self.invoke(extra=("--escrow-unit-evidence", str(path)))
        self.assertEqual(code, 1)
        self.assertIn("escrow_unit_evidence_requires_candidate_preview", stderr)
        self.assertFalse(output.exists())

    def test_post_parent_evidence_requires_explicit_candidate_mode(self):
        path = self.write("post-parent.json", {})
        code, output, _, stderr = self.invoke(extra=("--escrow-post-parent-claim-evidence", str(path)))
        self.assertEqual(code, 1)
        self.assertIn("escrow_post_parent_claim_evidence_requires_candidate_preview", stderr)
        self.assertFalse(output.exists())

    def test_post_parent_request_and_explicit_facts_preserve_source_and_review_gates(self):
        source = mapping.post_parent_data()
        for table, rows in source["tables"].items():
            self.tables[table] = rows
        prototype = self.tables[mapping.B.BOUNTY_TABLES[1]][-1]
        for row in self.tables[mapping.B.BOUNTY_TABLES[1]]:
            for key, value in prototype.items():
                row.setdefault(key, "" if type(value) is str else False if type(value) is bool else 0)
        self.tables["users"] = [{"id": uid, "quota": 0, "aff_quota": 0,
            "used_quota": 0, "request_count": 0, "aff_history": 0} for uid in (1, 2, 3)]
        records = [{**row, "target_user_id": row["source_user_id"]} for row in source["identity_records"]]
        map_path = self.write("map.jsonl", "\n".join(map(F.canonical, records)) + "\n")
        identity = F.parse_json((self.inputs / "identity.json").read_text())
        identity["source_target_id_map_sha256"] = F.IDENTITY.sha256_file(map_path)
        self.write("identity.json", identity)
        self.reseal_source_rows()
        catalog_path = self.inputs / "native-catalog-metadata.jsonl"
        catalog = [F.parse_json(line) for line in catalog_path.read_text().splitlines()]
        for constraint in catalog[3]:
            if constraint["table_name"] == "wallet_credit_rebases":
                constraint["definition"] = "PRIMARY KEY (migration_id)"
        self.write(catalog_path.name, "\n".join(map(F.canonical, catalog)) + "\n")
        counts = self.write("native-table-row-count-baseline.jsonl", "\n".join(
            F.canonical({"table": table, "row_count": len(rows)}) for table, rows in sorted(self.tables.items())) + "\n")
        proof = F.parse_json((self.inputs / "proof.json").read_text())
        proof["selected_table_counts"] = {table: len(rows) for table, rows in self.tables.items()}
        proof["metadata_hashes"][counts.name] = F.IDENTITY.sha256_file(counts)
        proof["metadata_hashes"][catalog_path.name] = F.IDENTITY.sha256_file(catalog_path)
        self.write("proof.json", proof)
        source_bytes = (self.inputs / "source-financial-export.private.jsonl").read_bytes()
        code, output, _, stderr = self.invoke(extra=("--escrow-candidate-preview",))
        self.assertEqual((code, stderr), (0, ""))
        unknown = F.parse_json((output / "escrow-candidate.private.json").read_text())
        self.assertIsNone(unknown["batch_candidate"])
        self.assertIsNone(unknown["post_parent_claim_evidence_request"]["unit_evidence_sha256"])
        plan = F.parse_json((output / "financial-plan.private.json").read_text())
        unit = mapping.evidence({"tables": self.tables, "identity_records": records, "plan": plan, "source_proof": proof})
        unit["source_application_revision"] = mapping.M.POST_PARENT_REVISION
        unit_path = self.write("unit.json", unit)
        extra = ("--escrow-candidate-preview", "--escrow-unit-evidence", str(unit_path))
        code, output, _, stderr = self.invoke(extra=extra)
        self.assertEqual((code, stderr), (0, ""))
        blocked = F.parse_json((output / "escrow-candidate.private.json").read_text())
        self.assertIsNone(blocked["batch_candidate"])
        request_path = output / "escrow-post-parent-claim-evidence-request.private.json"
        request = F.parse_json(request_path.read_text())
        self.assertFalse(request["economic_approval_verified"])
        self.assertEqual(request, blocked["post_parent_claim_evidence_request"])
        code, output, stdout, stderr = self.invoke(extra=(*extra,
            "--escrow-post-parent-claim-evidence", str(request_path)))
        self.assertEqual((code, stderr), (0, ""))
        candidate = F.parse_json((output / "escrow-candidate.private.json").read_text())
        self.assertEqual(candidate["blockers"], [])
        self.assertEqual((candidate["project_count"], candidate["claim_count"],
            candidate["accepted_unpaid_claim_count"], candidate["historical_rejection_guard_count"]), (5, 12, 9, 3))
        self.assertTrue(candidate["post_parent_claim_evidence_bound"])
        self.assertFalse(candidate["economic_approval_verified"])
        self.assertNotIn("review_proof_sha256", candidate["batch_candidate"])
        retained = output / "escrow-post-parent-claim-evidence.private.json"
        self.assertEqual(retained.read_bytes(), request_path.read_bytes())
        manifest = F.parse_json((output / "manifest.private.json").read_text())
        self.assertEqual(manifest["escrow_post_parent_claim_evidence_file_sha256"], F.IDENTITY.sha256_file(retained))
        self.assertEqual(manifest["escrow_post_parent_claim_evidence_typed_sha256"], F.digest(request))
        self.assertNotIn("source_owner_user_id", stdout)
        self.assertNotIn("source_reward_quota", stdout)
        self.assertEqual(source_bytes, (self.inputs / "source-financial-export.private.jsonl").read_bytes())
        self.assertEqual(output.stat().st_mode & 0o777, 0o700)
        self.assertTrue(all(p.stat().st_mode & 0o777 == 0o600 for p in output.iterdir()))
        self.assertFalse((output / "approval-required.private.json").exists())

    def test_secret_in_unit_evidence_never_reaches_any_derived_artifact(self):
        path = self.write("unit.json", {"note": "SYNTHETIC-SECRET"})
        code, output, stdout, stderr = self.invoke(
            extra=("--escrow-candidate-preview", "--escrow-unit-evidence", str(path)))
        self.assertEqual(code, 1)
        self.assertNotIn("SYNTHETIC-SECRET", stdout + stderr)
        self.assertFalse((output / "escrow-candidate.private.json").exists())
        self.assertFalse((output / "financial-plan.private.json").exists())
        for artifact in output.iterdir():
            self.assertNotIn(b"SYNTHETIC-SECRET", artifact.read_bytes())

    def test_secret_or_unsafe_post_parent_evidence_is_never_retained(self):
        path = self.write("post-parent.json", {"note": "SYNTHETIC-SECRET"})
        extra = ("--escrow-candidate-preview", "--escrow-post-parent-claim-evidence", str(path))
        code, output, stdout, stderr = self.invoke(extra=extra)
        self.assertEqual(code, 1)
        self.assertNotIn("SYNTHETIC-SECRET", stdout + stderr)
        self.assertFalse((output / "escrow-candidate.private.json").exists())
        self.assertFalse((output / "financial-plan.private.json").exists())
        self.assertTrue(all(b"SYNTHETIC-SECRET" not in p.read_bytes() for p in output.iterdir()))
        path.chmod(0o644)
        code, output, _, stderr = self.invoke(extra=extra)
        self.assertEqual(code, 1)
        self.assertIn("permissions_unsafe", stderr)
        self.assertFalse((output / "escrow-candidate.private.json").exists())


if __name__ == "__main__":
    unittest.main()
