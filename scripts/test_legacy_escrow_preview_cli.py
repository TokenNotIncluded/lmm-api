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

F = fixtures.F


# @mark migration.legacy-escrow-preview role=test
class EscrowPreviewCLITests(unittest.TestCase):
    write = fixtures.SourceRightsCLITest.write
    invoke = fixtures.SourceRightsCLITest.invoke

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


if __name__ == "__main__":
    unittest.main()
