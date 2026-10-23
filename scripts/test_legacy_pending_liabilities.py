"""Tests for the source-only unresolved payment liability contract."""
import copy
import importlib.util
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


F = load("financial_planner", ROOT / "migrate-legacy-financial.py")
L = load("pending_liabilities", ROOT / "legacy_pending_liabilities.py")


class PendingLiabilityTest(unittest.TestCase):
    def rows(self):
        return {
            "subscription_orders": [
                {"id": 9, "user_id": 71, "status": "pending", "payment_provider": "waffo_pancake",
                 "trade_no": "SUB-9", "provider_subscription_id": "", "money": 3,
                 "expected_amount_micros": 3_000_000, "settlement_currency": "USD"},
                {"id": 10, "user_id": 72, "status": "success", "payment_provider": "epay",
                 "trade_no": "SUB-10", "money": 4},
            ],
            "top_ups": [
                {"id": 5, "user_id": 71, "status": "pending", "payment_provider": "fastpay",
                 "trade_no": "TOP-5", "amount": 2, "money": 2, "settlement_currency": "CNY",
                 "provider_transaction_id": None},
                {"id": 6, "user_id": 73, "status": "failed", "payment_provider": "epay",
                 "trade_no": "TOP-6", "amount": 1},
            ],
        }

    def preview(self, tables=None, identities=None, target_users=None):
        identities = identities or {71: 101, 72: 102, 73: 103}
        target_users = target_users if target_users is not None else {uid: uid for uid in identities}
        return L.build(F, tables=tables or self.rows(), identities=identities, target_users=target_users,
                       migration_id="pending-liabilities-test", observed_at=1_800_000_000,
                       source_archive_sha256="a" * 64, mapping_sha256="b" * 64,
                       source_export_sha256="c" * 64)

    def test_only_pending_rows_are_preserved_without_spendable_units(self):
        preview = self.preview()
        self.assertTrue(L.verify(F, preview))
        self.assertEqual(preview["public_summary"]["record_count"], 2)
        self.assertEqual(preview["public_summary"]["counts_by_table"], {"subscription_orders": 1, "top_ups": 1})
        self.assertEqual(preview["public_summary"]["target_writes"], 0)
        self.assertEqual(preview["public_summary"]["ledger_writes"], 0)
        self.assertTrue(all(not row["spendable"] and row["spendable_units"] == 0 for row in preview["records"]))
        self.assertTrue(all(row["settlement_state"] == "unknown" for row in preview["records"]))

    def test_raw_rows_and_amount_currency_facts_are_preserved(self):
        tables = self.rows()
        original = copy.deepcopy(tables)
        preview = self.preview(tables)
        self.assertEqual(tables, original)
        topup = next(row for row in preview["records"] if row["source_table"] == "top_ups")
        self.assertEqual(topup["raw_amount"], {"amount": 2, "money": 2})
        self.assertEqual(topup["raw_currency"], {"settlement_currency": "CNY"})
        self.assertEqual(topup["raw_row"]["provider_transaction_id"], None)
        self.assertNotIn("SUB-10", F.canonical(preview))

    def test_typed_json_artifact_round_trips_without_decoding_decimal_tags(self):
        preview = self.preview()
        persisted = json.loads(F.canonical(preview))
        self.assertTrue(L.verify(F, persisted))

    def test_unmapped_owner_is_retained_but_marked_for_review(self):
        tables = self.rows()
        tables["top_ups"].append({"id": 7, "user_id": 73, "status": "pending", "payment_provider": "epay", "trade_no": "TOP-7", "amount": 1})
        preview = self.preview(tables=tables, identities={71: 101, 72: 102}, target_users={71: 71, 72: 72})
        row = next(row for row in preview["records"] if row["source_user_id"] == 73)
        self.assertIsNone(row["target_account_id"])
        self.assertEqual(preview["public_summary"]["unmapped_owner_count"], 1)
        self.assertTrue(L.verify(F, preview))

    def test_preview_hash_and_row_hash_reject_tampering(self):
        preview = self.preview()
        preview["records"][0]["raw_row"]["money"] = 99
        with self.assertRaisesRegex(F.MappingError, "pending_liability_preview_hash_mismatch"):
            L.verify(F, preview)

        preview = self.preview()
        preview["records"][0]["spendable_units"] = 1
        preview["preview_sha256"] = F.digest({key: value for key, value in preview.items()
                                               if key not in ("preview_sha256", "public_summary")})
        with self.assertRaisesRegex(F.MappingError, "pending_liability_spendable_contract_invalid"):
            L.verify(F, preview)

    def test_policy_and_record_binding_cannot_be_relaxed(self):
        preview = self.preview()
        preview["policy"]["ledger_entry_policy"] = "allowed"
        preview["preview_sha256"] = F.digest({key: value for key, value in preview.items()
                                               if key not in ("preview_sha256", "public_summary")})
        with self.assertRaisesRegex(F.MappingError, "pending_liability_policy_invalid"):
            L.verify(F, preview)

        preview = self.preview()
        preview["records"][0]["record_id"] = "0" * 64
        preview["preview_sha256"] = F.digest({key: value for key, value in preview.items()
                                               if key not in ("preview_sha256", "public_summary")})
        with self.assertRaisesRegex(F.MappingError, "pending_liability_record_binding_mismatch"):
            L.verify(F, preview)

    def test_import_request_is_handoff_only(self):
        preview = self.preview()
        request = L.build_import_request(F, preview, target_schema_contract_sha256="d" * 64)
        self.assertEqual(request["scope"], L.IMPORT_SCOPE)
        self.assertFalse(request["applied"])
        self.assertTrue(request["requires_owner_writer"])
        self.assertTrue(request["requires_provider_reconciliation"])
        self.assertEqual(request["target_writes"], 0)
        self.assertEqual(request["ledger_writes"], 0)
        self.assertEqual(request["spendable_units"], 0)

    def test_owner_apply_sql_is_dedicated_and_zero_spend(self):
        preview = self.preview()
        sql = L.build_owner_apply_sql(F, preview)
        self.assertIn("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE", sql)
        self.assertIn("SET LOCAL synchronous_commit='on'", sql)
        self.assertIn("core_billing.import_legacy_pending_liability", sql)
        self.assertNotIn("import_opening_balance", sql)
        self.assertNotIn("import_legacy_escrow_batch", sql)
        self.assertNotIn("post_ledger", sql)
        self.assertNotIn("UPDATE core_billing", sql)
        self.assertEqual(sql.count("import_legacy_pending_liability"), 2)
        self.assertIn('"source_row_key":"id:9"', sql)

    def test_owner_apply_sql_refuses_incomplete_owner_mapping(self):
        tables = self.rows()
        tables["top_ups"].append({"id": 7, "user_id": 73, "status": "pending", "payment_provider": "epay", "trade_no": "TOP-7", "amount": 1})
        preview = self.preview(tables=tables, target_users={71: 71})
        with self.assertRaisesRegex(F.MappingError, "pending_liability_owner_mapping_incomplete"):
            L.build_owner_apply_sql(F, preview)

    def test_readback_sql_is_read_only_and_checks_zero_spend(self):
        sql = L.build_readback_sql(F, self.preview())
        self.assertIn("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY", sql)
        self.assertIn("source_rows_sha256", sql)
        self.assertIn("spendable_true_count", sql)
        self.assertIn("spendable_units_sum", sql)
        self.assertIn("WHERE migration_id='pending-liabilities-test'", sql)
        self.assertNotIn("WHERE migration_id='\"pending-liabilities-test\"'", sql)
        self.assertNotIn("INSERT ", sql.upper())
        self.assertNotIn("UPDATE ", sql.upper())
        self.assertNotIn("DELETE ", sql.upper())

    def test_same_material_is_replay_noop_and_different_material_conflicts(self):
        first = self.preview()
        second = self.preview()
        self.assertEqual(L.replay_status(F, first, second), "replay-noop")
        changed = self.preview(tables={**self.rows(), "top_ups": self.rows()["top_ups"] + [
            {"id": 7, "user_id": 71, "status": "pending", "payment_provider": "epay", "trade_no": "TOP-7", "amount": 1}
        ]})
        self.assertEqual(L.replay_status(F, first, changed), "conflict-material-mismatch")

    def test_duplicate_source_id_is_rejected(self):
        tables = self.rows()
        tables["top_ups"].append(dict(tables["top_ups"][0], trade_no="TOP-other"))
        with self.assertRaisesRegex(F.MappingError, "pending_liability_source_id_duplicate"):
            self.preview(tables)

    def test_invalid_hash_or_status_is_rejected(self):
        with self.assertRaisesRegex(F.MappingError, "pending_liability_source_archive_sha256_invalid"):
            L.build(F, tables=self.rows(), identities={71: 101}, migration_id="pending-liabilities-test",
                    observed_at=1_800_000_000, source_archive_sha256="bad", mapping_sha256="b" * 64,
                    source_export_sha256="c" * 64)
        tables = self.rows()
        tables["top_ups"][0]["status"] = "settled"
        # A non-pending row is intentionally omitted, leaving a valid empty top-up family.
        self.assertEqual(self.preview(tables)["public_summary"]["record_count"], 1)


if __name__ == "__main__":
    unittest.main()
