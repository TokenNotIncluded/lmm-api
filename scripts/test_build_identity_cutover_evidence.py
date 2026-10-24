#!/usr/bin/env python3
"""Synthetic tests for build-identity-cutover-evidence.py."""

from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location(
    "identity_evidence", Path(__file__).with_name("build-identity-cutover-evidence.py")
)
P = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(P)


class Checks(unittest.TestCase):
    def write(self, root: Path, name: str, value: str) -> Path:
        path = root / name
        path.write_text(value, encoding="utf-8")
        return path

    def fixture(self) -> Path:
        root = Path(tempfile.mkdtemp())
        archive = "a" * 64
        summary = {
            "sessions_imported": 0,
            "api_keys_imported": 0,
            "financial_rows_written": 0,
            "functional_gaps": [
                "legacy_oauth_and_passkeys_archived_only",
                "financial_data_not_migrated",
            ],
        }
        mapping = self.write(root, P.FILES["mapping"], '{"records":[]}\n')
        source_export = self.write(root, P.FILES["source_export"], '{"source":"frozen"}\n')
        source_baseline = self.write(root, P.FILES["source_baseline"], '{"baseline":true}\n')
        id_map = self.write(root, P.FILES["id_map"], '{"source_user_id":1,"target_user_id":1}\n')
        sql = self.write(root, P.FILES["sql"], "BEGIN;\n-- private SQL fixture\nCOMMIT;\n")
        target_import = self.write(
            root,
            P.FILES["target_import"],
            json.dumps({"status": "identity-import-verified", "migration_id": "fixture-identity"}) + "\n",
        )
        target_receipt = self.write(
            root,
            P.FILES["target_receipt"],
            json.dumps(
                {
                    "status": "initialized-empty-identity-target",
                    "migration_id": "fixture-identity",
                    "source_archive_sha256": archive,
                    "registry_provisioned": True,
                    "application_or_worker_started": False,
                    "instance_identity": {
                        "database_oid": 42,
                        "postmaster_started_at": "2026-10-24T00:00:00+00:00",
                        "server_version_num": "180006",
                        "server_instance_nonce": "e" * 64,
                    },
                },
                sort_keys=True,
            ),
        )
        manifest = {
            "format_version": 2,
            "scope": "identity-only",
            "source_archive_sha256": archive,
            "source_export_sha256": hashlib.sha256(source_export.read_bytes()).hexdigest(),
            "source_baseline_proof_sha256": hashlib.sha256(source_baseline.read_bytes()).hexdigest(),
            "source_target_id_map_sha256": hashlib.sha256(id_map.read_bytes()).hexdigest(),
            "mapping_file_sha256": hashlib.sha256(mapping.read_bytes()).hexdigest(),
            "mapping_sha256": "b" * 64,
            "import_contract_sha256": "c" * 64,
            "summary": summary,
        }
        report = {
            "status": "identity-import-verified",
            "apply": True,
            "migration_id": "fixture-identity",
            "source_archive_sha256": archive,
            "mapping_sha256": manifest["mapping_sha256"],
            "mapping_file_sha256": manifest["mapping_file_sha256"],
            "identity_contract": {"identity_version": 5, "identity_fingerprint_sha256": "d" * 64},
            **summary,
        }
        manifest["identity_contract"] = report["identity_contract"]
        manifest_path = self.write(root, P.FILES["manifest"], json.dumps(manifest, sort_keys=True) + "\n")
        report_path = self.write(root, P.FILES["report"], json.dumps(report, sort_keys=True) + "\n")
        independent = {
            "status": "PASS",
            "scope": "identity-cutover",
            "source_frozen": True,
            "target_actual_instance_match": True,
            "mapping_status": "passed",
            "target_comparison_status": "passed",
            "legacy_inventory": {
                "sessions": 723,
                "api_keys": 815,
                "passkeys": 8,
                "oauth_bindings": 235,
                "oauth_server_tokens": 1214,
                "mfa_factors": 2,
                "mfa_backup_codes": 8,
            },
            "reauth_required": 2172,
            "rekey_required": 815,
            "reenroll_required": 18,
            "actions": {
                "sessions": "reauth_required",
                "api_keys": "rekey_required",
                "passkeys": "reenroll_required",
                "oauth_bindings": "reauth_required",
                "oauth_server_tokens": "reauth_required",
                "mfa_factors": "reenroll_required",
                "mfa_backup_codes": "reenroll_required",
            },
            "source_archive_sha256": archive,
            "manifest_sha256": hashlib.sha256(manifest_path.read_bytes()).hexdigest(),
            "report_sha256": hashlib.sha256(report_path.read_bytes()).hexdigest(),
            "identity_import_sql_sha256": hashlib.sha256(sql.read_bytes()).hexdigest(),
            "target_import_sha256": hashlib.sha256(target_import.read_bytes()).hexdigest(),
            "target_receipt_sha256": hashlib.sha256(target_receipt.read_bytes()).hexdigest(),
        }
        self.write(root, P.FILES["independent"], json.dumps(independent, sort_keys=True) + "\n")
        return root

    def test_builds_sanitized_identity_block(self):
        root = self.fixture()
        result = P.build(root)
        self.assertEqual(result["status"], "PASS")
        identity = result["identity"]
        self.assertEqual(identity["legacy_inventory"]["api_keys"], 815)
        self.assertEqual(len([key for key in identity if key.endswith("_sha256")]), 9)
        self.assertNotIn("private SQL fixture", json.dumps(result))
        self.assertNotIn("initialized-empty-identity-target", json.dumps(result))

    def test_missing_independent_pass_is_not_run(self):
        root = self.fixture()
        (root / P.FILES["independent"]).unlink()
        with self.assertRaisesRegex(P.EvidenceError, "required evidence missing"):
            P.build(root)

    def test_target_instance_mismatch_blocks(self):
        root = self.fixture()
        path = root / P.FILES["independent"]
        value = json.loads(path.read_text())
        value["target_actual_instance_match"] = False
        path.write_text(json.dumps(value, sort_keys=True) + "\n")
        with self.assertRaisesRegex(P.EvidenceError, "independent identity boundary missing"):
            P.build(root)

    def test_source_freeze_and_artifact_hashes_are_bound(self):
        root = self.fixture()
        path = root / P.FILES["sql"]
        path.write_text(path.read_text() + "-- changed\n")
        with self.assertRaisesRegex(P.EvidenceError, "independent artifact hash mismatch"):
            P.build(root)

    def test_action_coverage_is_fail_closed(self):
        root = self.fixture()
        path = root / P.FILES["independent"]
        value = json.loads(path.read_text())
        value["rekey_required"] = 0
        path.write_text(json.dumps(value, sort_keys=True) + "\n")
        with self.assertRaisesRegex(P.EvidenceError, "identity action coverage incomplete"):
            P.build(root)

    def test_existing_local_layout_is_verified_then_stays_not_run_without_inventory(self):
        source = self.fixture()
        backup = Path(tempfile.mkdtemp())
        apply = backup / "identity-apply"
        apply.mkdir()
        for key, name in P.FILES.items():
            if key in {"target_receipt", "independent"}:
                continue
            shutil.copy2(source / name, apply / name)
        shutil.copy2(source / P.FILES["target_receipt"], backup / P.FILES["target_receipt"])
        independent_dir = backup / "independent-identity"
        independent_dir.mkdir()
        actual = {
            "status": "PASS",
            "scope": "real-snapshot-local-identity-independent-readback",
            "archive_and_all_frozen_artifact_hashes_verified": True,
            "binary_compiled_schema_manifest_verified": True,
            "exact_SQL_regenerated_equal": True,
            "no_write_or_SSH": True,
            "source_mapping_recomputed_equal": True,
            "source_users_equal_applied_export": True,
            "target_actual_instance_matches_receipt": True,
            "password_verifiers_exact": 754,
            "privileged_facts_exact_19_fields": 3,
            "users_exact_fields": 1216,
        }
        (independent_dir / "identity-independent-result.private.json").write_text(
            json.dumps(actual, sort_keys=True) + "\n", encoding="utf-8"
        )
        frozen = {
            str((apply / P.FILES[key]).resolve()): P.digest(apply / P.FILES[key])
            for key in ("manifest", "report", "sql", "target_import")
        }
        frozen[str((backup / P.FILES["target_receipt"]).resolve())] = P.digest(
            backup / P.FILES["target_receipt"]
        )
        (independent_dir / "frozen-inputs.private.json").write_text(
            json.dumps(frozen, sort_keys=True) + "\n", encoding="utf-8"
        )
        with self.assertRaisesRegex(P.EvidenceError, "legacy inventory evidence missing"):
            P.build(apply)


if __name__ == "__main__":
    unittest.main(verbosity=2)
