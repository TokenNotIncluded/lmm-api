import copy
import unittest

import prepare_wallet_history_snapshot as importer


class PrepareWalletTopupHistoryTests(unittest.TestCase):
    def setUp(self):
        self.export = {
            "source_namespace": importer.NAMESPACE,
            "observed_at": 1_700_000_200,
            "source_row_count": 2,
            "source_user_ids": [11, 12],
            "rows": [
                {
                    "id": 301,
                    "user_id": 11,
                    "amount": 100,
                    "money_text": "2.000",
                    "trade_no": "ORD%_301",
                    "payment_method": "stripe",
                    "status": "success",
                    "create_time": 1_700_000_100,
                    "complete_time": 1_700_000_101,
                    "credited_quota": 500_000,
                    "platform_amount_micros": 0,
                    "settlement_currency": "USD",
                },
                {
                    "id": 302,
                    "user_id": 11,
                    "amount": 50,
                    "money_text": "1.25",
                    "trade_no": "ORD-302",
                    "payment_method": "creem",
                    "status": "failed",
                    "create_time": 1_700_000_102,
                    "complete_time": 1_700_000_103,
                    "credited_quota": 0,
                    "platform_amount_micros": 0,
                    "settlement_currency": "",
                },
            ],
        }
        self.mapping = [
            {"source_user_id": 11, "target_user_id": 71, "target_personal_account_id": 701},
            {"source_user_id": 12, "target_user_id": 72, "target_personal_account_id": 702},
        ]

    def build(self, export=None, mapping=None, expected=2):
        return importer.prepare(
            copy.deepcopy(self.export if export is None else export),
            copy.deepcopy(self.mapping if mapping is None else mapping),
            expected_rows=expected,
            archive_sha256="a" * 64,
            export_sha256="b" * 64,
        )

    def test_preserves_rows_identity_currency_decimal_and_exact_snapshot_plan(self):
        manifest = self.build()
        self.assertEqual(manifest["source_row_count"], 2)
        self.assertEqual(manifest["per_source_user_counts"], {"11": 2, "12": 0})
        self.assertEqual(manifest["counts_by_status"], {"success": 1, "failed": 1})
        self.assertEqual(manifest["rows"][0]["record"]["user_id"], 71)
        self.assertEqual(manifest["rows"][0]["record"]["trade_no"], "ORD%_301")
        self.assertEqual(manifest["rows"][0]["money_text"], "2.000")
        self.assertNotIn("currency", manifest["rows"][1]["record"])
        self.assertEqual(manifest["rows"][1]["record"]["settlement_currency"], "")
        self.assertEqual(manifest["rows"][1]["record"]["amount_unit"], "CREDIT")
        sql = importer.import_sql(manifest)
        self.assertIn("SERIALIZABLE", sql)
        self.assertIn("wallet_history_import_conflict", sql)
        self.assertIn("exact_manifest_match", importer.verification_sql(manifest))

    def test_count_and_identity_gaps_block_without_truncating_rows(self):
        with self.assertRaisesRegex(ValueError, "source_expected_count_mismatch"):
            self.build(expected=3)
        mapping = [row for row in self.mapping if row["source_user_id"] != 12]
        with self.assertRaisesRegex(ValueError, "identity_map_missing_source_users:12"):
            self.build(mapping=mapping)
        export = copy.deepcopy(self.export)
        export["rows"].pop()
        with self.assertRaisesRegex(ValueError, "export_row_count_mismatch"):
            self.build(export=export)

    def test_duplicate_orders_and_unverified_status_block(self):
        export = copy.deepcopy(self.export)
        export["rows"][1]["id"] = export["rows"][0]["id"]
        with self.assertRaisesRegex(ValueError, "source_order_id_duplicate"):
            self.build(export=export)
        export = copy.deepcopy(self.export)
        export["rows"][0]["status"] = "settled?"
        with self.assertRaisesRegex(ValueError, "source_order_status_unknown"):
            self.build(export=export)


if __name__ == "__main__":
    unittest.main()
