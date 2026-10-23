"""Synthetic offline wallet comparisons; no monetary or database acceptance."""

import copy
import hashlib
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))
import test_legacy_key_reconciliation as fixtures

F, R = fixtures.F, fixtures.R


def inventory(quota=29, affiliate=7):
    return {"format_version": 1, "scope": "legacy-wallet-positions", "unit": "credit_500k_usd",
        "records": [{"source_user_id": fixtures.UID, "account_id": fixtures.ACCOUNT,
            "wallet_units": max(quota, 0), "debt_units_signed": min(quota, 0), "affiliate_units": affiliate}]}


# @mark migration.financial-rights role=test
class WalletComparisonTests(unittest.TestCase):
    def test_signed_positions_match_without_authorizing_units_or_reservations(self):
        args = fixtures.fixture()
        args["tables"]["users"][0].update(quota=-19, aff_quota=-7)
        result = R.wallet_position_reconciliation(F, tables=args["tables"], identities=args["identities"],
                                                target_wallet_inventory=inventory(-19, -7))
        self.assertTrue(result["passed"])
        self.assertEqual(result["status"], "matched-source-integers-only")
        for field in ("target_state_complete", "source_unit_provenance_verified", "reserved_units_verified",
                      "escrow_rights_verified", "economic_approval_created", "target_inventory_provenance_verified",
                      "target_inventory_scope_completeness_verified"):
            self.assertIs(result[field], False, field)
        self.assertEqual(result["target_writes"], 0)

    def test_missing_inventory_is_not_acceptance_even_with_empty_source(self):
        for users in ([], fixtures.fixture()["tables"]["users"]):
            result = R.wallet_position_reconciliation(F, tables={"users": users}, identities={fixtures.UID: fixtures.ACCOUNT})
            self.assertFalse(result["passed"])
            self.assertFalse(result["target_inventory_supplied"])

    def test_zero_wallet_row_must_still_be_observed(self):
        args = fixtures.fixture()
        args["tables"]["users"][0].update(quota=0, aff_quota=0)
        target = inventory(0, 0)
        target["records"] = []
        result = R.wallet_position_reconciliation(F, tables=args["tables"], identities=args["identities"],
                                                target_wallet_inventory=target)
        self.assertFalse(result["passed"])
        self.assertEqual(result["comparison_counts"], {"missing_in_target": 1})

    def test_source_target_plan_comparison_cannot_hide_changes_in_totals(self):
        for field in ("account_id", "wallet_units", "debt_units_signed", "affiliate_units"):
            target = inventory()
            target["records"][0][field] += -1 if field == "debt_units_signed" else 1
            result = R.build(F, **fixtures.fixture(), target_wallet_inventory=target)
            self.assertFalse(result["wallet_reconciliation_passed"])
            self.assertEqual(result["status"], "failed-wallet-reconciliation")
            row = result["wallet_position_reconciliation"]["records"][0]
            self.assertEqual(row["mismatched_fields"], [field])

    def test_unexpected_target_owner_is_preserved_as_failure(self):
        target = inventory()
        target["records"][0]["source_user_id"] += 1
        result = R.build(F, **fixtures.fixture(), target_wallet_inventory=target)
        self.assertEqual(result["wallet_position_reconciliation"]["comparison_counts"],
                         {"missing_in_target": 1, "unexpected_in_target": 1})

    def test_duplicates_types_signs_and_unexpected_metadata_fail_closed(self):
        bad = []
        for field in ("source_user_id", "account_id", "wallet_units", "debt_units_signed", "affiliate_units"):
            for value in (True, 1.0, "1", None):
                target = inventory(); target["records"][0][field] = value; bad.append(target)
        for field, value in (("wallet_units", -1), ("debt_units_signed", 1), ("affiliate_units", 2**63)):
            target = inventory(); target["records"][0][field] = value; bad.append(target)
        for field in ("source_user_id", "account_id"):
            target = inventory(); row = copy.deepcopy(target["records"][0]); row[field] += 1
            target["records"].append(row); bad.append(target)
        target = inventory(); target["records"][0]["metadata"] = "SYNTHETIC-SECRET"; bad.append(target)
        target = inventory(); target["unit"] = "USD"; bad.append(target)
        target = inventory(); target["format_version"] = True; bad.append(target)
        for target in bad:
            with self.subTest(target=target), self.assertRaises(F.MappingError):
                R.validate_target_wallet_inventory(F, target)

    def test_inputs_remain_unchanged_and_public_report_has_no_wallet_positions(self):
        args, target = fixtures.fixture(), inventory()
        before = copy.deepcopy((args, target))
        result = R.build(F, **args, target_wallet_inventory=target)
        self.assertEqual((args, target), before)
        summary = result["public_summary"]["wallet_position_reconciliation"]
        self.assertNotIn("records", summary)
        self.assertTrue(summary["passed"])
        self.assertIs(result["approval_created"], False)
        self.assertIs(result["public_summary"]["reserved_zero_proven"], False)


# @mark migration.financial-rights role=test
class WalletCLITests(unittest.TestCase):
    setUp = fixtures.SourceRightsCLITest.setUp
    write = fixtures.SourceRightsCLITest.write
    invoke = fixtures.SourceRightsCLITest.invoke

    def wallet_invoke(self, target, **kwargs):
        path = self.write("wallet.json", target)
        return self.invoke(extra=("--target-wallet-inventory", str(path)), **kwargs)

    def test_cli_retains_exact_bytes_and_hashes_without_approval(self):
        original = json.dumps(inventory(), indent=2) + "\r\n\n"
        path = self.write("wallet.json", original)
        code, output, stdout, stderr = self.invoke(extra=("--target-wallet-inventory", str(path)))
        self.assertEqual((code, stderr), (0, ""))
        self.assertEqual((output / "target-wallet-inventory.private.json").read_bytes(), path.read_bytes())
        manifest = F.parse_json((output / "manifest.private.json").read_text())
        self.assertEqual(manifest["target_wallet_inventory_file_sha256"], hashlib.sha256(path.read_bytes()).hexdigest())
        self.assertEqual(manifest["target_wallet_inventory_typed_sha256"], F.digest(inventory()))
        public = F.parse_json(stdout)
        self.assertTrue(public["wallet_reconciliation_passed"])
        self.assertFalse(public["wallet_position_reconciliation"]["source_unit_provenance_verified"])
        self.assertEqual(public["target_writes"], 0)
        for artifact in output.iterdir():
            self.assertNotIn(b"SYNTHETIC-SECRET", artifact.read_bytes())

    def test_cli_comparison_mismatch_exits_nonzero_and_keeps_evidence(self):
        target = inventory(); target["records"][0]["wallet_units"] += 1
        code, output, stdout, stderr = self.wallet_invoke(target)
        self.assertEqual((code, stderr), (1, ""))
        self.assertFalse(F.parse_json(stdout)["wallet_reconciliation_passed"])
        self.assertTrue((output / "source-rights-reconciliation.private.json").is_file())
        self.assertTrue((output / "target-wallet-inventory.private.json").is_file())

    def test_wallet_inventory_cannot_be_used_outside_offline_preview(self):
        code, output, _, stderr = self.wallet_invoke(inventory(), preview=False)
        self.assertEqual(code, 1)
        self.assertIn("target_wallet_inventory_requires_source_rights_preview", stderr)
        self.assertFalse(output.exists())

    def test_invalid_metadata_is_rejected_before_artifact_publication(self):
        target = inventory(); target["records"][0]["note"] = "SYNTHETIC-SECRET"
        code, output, stdout, stderr = self.wallet_invoke(target)
        self.assertEqual(code, 1)
        self.assertNotIn("SYNTHETIC-SECRET", stdout + stderr)
        self.assertFalse((output / "financial-plan.private.json").exists())
        self.assertFalse((output / "target-wallet-inventory.private.json").exists())


if __name__ == "__main__":
    unittest.main()
