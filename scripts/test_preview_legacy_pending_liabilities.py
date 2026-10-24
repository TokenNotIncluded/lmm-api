"""Focused tests for the frozen-export pending-liability fence."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location(
    "pending_preview", ROOT / "preview-legacy-pending-liabilities.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PendingPreviewTest(unittest.TestCase):
    def files(self, directory: Path):
        archive = directory / "source.dump"
        archive.write_bytes(b"isolated archive")
        source_export = directory / "source.jsonl"
        source_export.write_text(
            "\n".join([
                json.dumps({"kind": "identity", "row": {"database": "fixture"}}),
                json.dumps({"kind": "row", "table": "top_ups", "row": {
                    "id": 4, "user_id": 1, "status": "pending", "payment_provider": "fastpay",
                    "trade_no": "TOP-4", "money": 2,
                }}),
                json.dumps({"kind": "row", "table": "top_ups", "row": {
                    "id": 5, "user_id": 1, "status": "success", "payment_provider": "fastpay",
                    "trade_no": "TOP-5", "money": 3,
                }}),
                json.dumps({"kind": "row", "table": "subscription_orders", "row": {
                    "id": 7, "user_id": 1, "status": "pending", "payment_provider": "waffo_pancake",
                    "trade_no": "SUB-7", "money": 4,
                }}),
            ]) + "\n"
        )
        archive_sha = MODULE._sha256_file(archive)
        export_sha = MODULE._sha256_file(source_export)
        identity_map = directory / "identity-mapping.json"
        identity_map.write_text(json.dumps({
            "mapping_sha256": "a" * 64,
            "records": [{"user_id": 1, "account_id": 1}],
        }, separators=(",", ":")))
        id_map = directory / "source-target-id-map.jsonl"
        id_map.write_text('{"source_user_id":1,"target_personal_account_id":1,"target_user_id":1}\n')
        identity_manifest = directory / "identity-manifest.json"
        identity_manifest.write_text(json.dumps({
            "format_version": 2, "scope": "identity-only", "source_archive_sha256": archive_sha,
            "source_export_sha256": "b" * 64, "mapping_sha256": "a" * 64,
            "mapping_file_sha256": MODULE._sha256_file(identity_map),
            "source_target_id_map_sha256": MODULE._sha256_file(id_map),
        }, separators=(",", ":")))
        financial_manifest = directory / "financial-manifest.json"
        financial_manifest.write_text(json.dumps({
            "scope": "financial-typed-planner-only", "migration_id": "fixture-financial",
            "source_archive_sha256": archive_sha, "source_export_sha256": export_sha,
            "identity_mapping_sha256": "a" * 64,
            "identity_manifest_file_sha256": MODULE._sha256_file(identity_manifest),
            "identity_map_file_sha256": MODULE._sha256_file(id_map),
            "source_observation": {
                "observed_at": 1_700_000_000,
                "exact_financial_snapshot_instant_known": False,
                "must_not_be_used_as_original_payment_or_settlement_time": True,
            },
        }, separators=(",", ":")))
        return source_export, archive, identity_map, id_map, identity_manifest, financial_manifest

    def test_extracts_pending_rows_and_fences_provider(self):
        with tempfile.TemporaryDirectory() as raw:
            paths = self.files(Path(raw))
            summary, document = MODULE.preview(
                source_export=paths[0], archive=paths[1], identity_map=paths[2],
                identity_map_bytes=paths[3], identity_manifest=paths[4], financial_manifest=paths[5],
            )
        self.assertEqual(summary["pending_record_count"], 2)
        self.assertEqual(summary["pending_counts_by_table"], {"subscription_orders": 1, "top_ups": 1})
        self.assertEqual(summary["pending_counts_by_provider"], {"fastpay": 1, "waffo_pancake": 1})
        self.assertEqual(summary["gate"], "BLOCKED_PENDING_PROVIDER_RECONCILIATION")
        self.assertIn("owner_only_receipt_writer_contract_missing", summary["fence_reasons"])
        self.assertFalse(summary["provider_io_performed"])
        self.assertEqual(summary["target_writes"], 0)
        self.assertEqual(summary["ledger_writes"], 0)
        self.assertTrue(all(row["spendable"] is False and row["spendable_units"] == 0
                            for row in document["records"]))

    def test_archive_binding_mismatch_rejects_before_preview(self):
        with tempfile.TemporaryDirectory() as raw:
            paths = self.files(Path(raw))
            paths[1].write_bytes(b"changed archive")
            with self.assertRaisesRegex(MODULE.PreviewError, "identity_archive_binding_mismatch"):
                MODULE.preview(
                    source_export=paths[0], archive=paths[1], identity_map=paths[2],
                    identity_map_bytes=paths[3], identity_manifest=paths[4], financial_manifest=paths[5],
                )


if __name__ == "__main__":
    unittest.main()
