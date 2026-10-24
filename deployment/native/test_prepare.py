#!/usr/bin/env python3
"""Synthetic local tooling checks, not migration or independent acceptance."""
import argparse
import datetime as dt
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("native_prepare", Path(__file__).with_name("prepare.py"))
P = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(P)


class Checks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.candidate = self.root / "candidate"
        (self.candidate / "web").mkdir(parents=True)
        (self.candidate / "web/index.html").write_text("synthetic web")
        self.release = {"kind": "lmm-native-stage-v1", "formal_release": False,
                        "components": {"web": {"version": "1.2.3", "files": P.inventory(self.candidate / "web")}}}
        (self.candidate / "release.json").write_text(json.dumps(self.release))
        self.preflight = self.root / "preflight.json"
        identity = {
            "migration_status": "identity_only", "mapping_status": "passed",
            "target_comparison_status": "passed",
            "legacy_inventory": {field: 0 for field in P.IDENTITY_INVENTORY_FIELDS},
            **{field: "d" * 64 for field in P.IDENTITY_HASH_FIELDS},
            "reauth_required": 0, "rekey_required": 0, "reenroll_required": 0,
            "actions": {field: action for field, action in P.IDENTITY_ACTIONS.items()},
        }
        gates = [{"name": name, "status": "READY", "missing": []}
                 for name in sorted(P.PREFLIGHT_GATES)]
        self.preflight.write_text(json.dumps({
            "protocol": "core-cutover-preflight.v1",
            "status": "READY",
            "safe_to_cutover": True,
            "public_edge": {"mode": "candidate", "candidate_selected": True},
            "identity": identity,
            "gates": gates,
            "unsafe_enablement": [],
            "secret_values_checked": "schema_only",
            "source_commit": "e" * 40,
            "release": {
                "source_commit": "e" * 40,
                "runtime_source_commit": "e" * 40,
                "core_artifact_sha256": "1" * 64,
                "extensions_artifact_sha256": "2" * 64,
                "runtime_core_artifact_sha256": "1" * 64,
                "runtime_extensions_artifact_sha256": "2" * 64,
                "sbom_sha256": "3" * 64,
                "schema_fingerprint_sha256": "4" * 64,
                "provider_matrix_sha256": "5" * 64,
                "identity_evidence_sha256": "6" * 64,
                "financial_evidence_sha256": "7" * 64,
                "edge_config_sha256": "8" * 64,
                "runtime_release_identity_sha256": "9" * 64,
                "previous_writer_epoch": 7,
                "writer_epoch": 8,
            },
        }))
        self.manifest = self.root / "release-manifest.json"
        self.manifest.write_text(json.dumps({
            "protocol": "release-manifest.v1",
            **json.loads(self.preflight.read_text())["release"],
        }))
        self.args = argparse.Namespace(release_dir=self.candidate, release_sha256=P.digest(self.candidate / "release.json"),
                                       review=self.root / "activation.json", review_sha256="", host="synthetic-host",
                                       database_identity_sha256="a" * 64, mode="cutover", previous_dir=None, previous_sha256=None)
        self.record = {"kind": "lmm-native-activation-v1", "status": "ready", "mode": "cutover",
                       "host": self.args.host, "target_database_identity_sha256": "a" * 64,
                       "release_sha256": self.args.release_sha256, "configuration_sha256": {},
                       "cutover_preflight": {"path": str(self.preflight), "sha256": P.digest(self.preflight)},
                       "cutover_manifest": {"path": str(self.manifest), "sha256": P.digest(self.manifest)},
                       "target_runtime_check_passed": True, "all_valid_writes_preserved": True,
                       "backup": {"sha256": "b" * 64, "restore_verified": True},
                       "migration": {"status": "completed", "unresolved_count": 0,
                                     "financial_reconciliation_passed": True, "source_final_boundary_sha256": "c" * 64},
                       "final_fence": {"writes_fenced": True, "active_writers": 0, "legacy_workers_stopped": True,
                                       "source_final_boundary_sha256": "c" * 64,
                                       "observed_at": dt.datetime.now(dt.timezone.utc).isoformat()}}

    def run_gate(self):
        self.args.review.write_text(json.dumps(self.record))
        self.args.review_sha256 = P.digest(self.args.review)
        return P.gate(self.args)

    def test_synthetic_complete_record_and_changed_payload(self):
        self.assertEqual(self.run_gate()["status"], "activation-record-and-files-verified")
        (self.candidate / "web/index.html").write_text("modified")
        with self.assertRaises(ValueError):
            self.run_gate()

    def test_incomplete_cutover_rejected(self):
        mutations = [("status", "dry-run"), ("target_runtime_check_passed", False),
                     ("all_valid_writes_preserved", False), ("migration", {}), ("backup", {}), ("final_fence", {})]
        for key, value in mutations:
            with self.subTest(key=key):
                previous = self.record[key]
                self.record[key] = value
                with self.assertRaises((ValueError, KeyError)):
                    self.run_gate()
                self.record[key] = previous

    def test_web_only_cutover_does_not_require_core_preflight(self):
        self.record.pop("cutover_preflight")
        self.assertEqual(self.run_gate()["status"], "activation-record-and-files-verified")

    def test_core_cutover_preflight_is_required_and_pinned(self):
        current = {"components": {"core": {
            "source_revision": "e" * 40,
            "archive_sha256": "1" * 64,
        }}}
        self.assertEqual(
            P.verify_cutover_preflight(current, self.record, "e" * 40),
            (
                self.record["cutover_preflight"]["sha256"],
                self.record["cutover_manifest"]["sha256"],
            ),
        )
        with self.assertRaisesRegex(ValueError, "expected candidate"):
            P.verify_cutover_preflight(current, self.record, "f" * 40)

        self.preflight.write_text(json.dumps({**json.loads(self.preflight.read_text()), "status": "BLOCKED"}))
        self.record["cutover_preflight"]["sha256"] = P.digest(self.preflight)
        with self.assertRaises(ValueError):
            P.verify_cutover_preflight(current, self.record)

        self.preflight.write_text(json.dumps({
            "protocol": "core-cutover-preflight.v1",
            "status": "READY",
            "safe_to_cutover": True,
            "source_commit": "e" * 40,
            "release": {
                "source_commit": "e" * 40,
                "runtime_source_commit": "e" * 40,
                "core_artifact_sha256": "1" * 64,
                "extensions_artifact_sha256": "2" * 64,
                "runtime_core_artifact_sha256": "1" * 64,
                "runtime_extensions_artifact_sha256": "2" * 64,
                "sbom_sha256": "3" * 64,
                "schema_fingerprint_sha256": "4" * 64,
                "provider_matrix_sha256": "5" * 64,
                "identity_evidence_sha256": "6" * 64,
                "financial_evidence_sha256": "7" * 64,
                "edge_config_sha256": "8" * 64,
                "runtime_release_identity_sha256": "9" * 64,
                "previous_writer_epoch": 7,
                "writer_epoch": 8,
            },
        }))
        self.record["cutover_preflight"]["sha256"] = P.digest(self.preflight)
        with self.assertRaises(ValueError):
            P.verify_cutover_preflight({"components": {"core": {
                "source_revision": "f" * 40,
                "archive_sha256": "1" * 64,
            }}}, self.record)
        with self.assertRaises(ValueError):
            P.verify_cutover_preflight({"components": {"core": {
                "source_revision": "e" * 40,
                "archive_sha256": "f" * 64,
            }}}, self.record)

        self.manifest.write_text(json.dumps({
            "protocol": "release-manifest.v1",
            **{**json.loads(self.preflight.read_text())["release"], "writer_epoch": 9},
        }))
        self.record["cutover_manifest"]["sha256"] = P.digest(self.manifest)
        with self.assertRaises(ValueError):
            P.verify_cutover_preflight(current, self.record)

        self.record["cutover_preflight"]["sha256"] = "f" * 64
        with self.assertRaises(ValueError):
            P.verify_cutover_preflight(current, self.record)

    def test_preflight_result_requires_all_identity_route_and_gate_evidence(self):
        current = {"components": {"core": {
            "source_revision": "e" * 40,
            "archive_sha256": "1" * 64,
        }}}
        original = json.loads(self.preflight.read_text())
        for field, value in (
            ("gates", []),
            ("identity", None),
            ("public_edge", {"mode": "legacy", "candidate_selected": False}),
            ("unsafe_enablement", ["payments"]),
        ):
            with self.subTest(field=field):
                mutated = dict(original)
                mutated[field] = value
                self.preflight.write_text(json.dumps(mutated))
                self.record["cutover_preflight"]["sha256"] = P.digest(self.preflight)
                with self.assertRaises(ValueError):
                    P.verify_cutover_preflight(current, self.record, "e" * 40)
        mutated = dict(original)
        mutated["identity"] = dict(original["identity"])
        mutated["identity"]["legacy_inventory"] = dict(original["identity"]["legacy_inventory"])
        mutated["identity"]["legacy_inventory"]["api_keys"] = 1
        self.preflight.write_text(json.dumps(mutated))
        self.record["cutover_preflight"]["sha256"] = P.digest(self.preflight)
        with self.assertRaises(ValueError):
            P.verify_cutover_preflight(current, self.record, "e" * 40)
        self.preflight.write_text(json.dumps(original))
        self.record["cutover_preflight"]["sha256"] = P.digest(self.preflight)

    def test_fence_and_boundary_rejected(self):
        for key, value in [("active_writers", 1), ("legacy_workers_stopped", False),
                           ("source_final_boundary_sha256", "d" * 64),
                           ("observed_at", "2000-01-01T00:00:00Z")]:
            with self.subTest(key=key):
                previous = self.record["final_fence"][key]
                self.record["final_fence"][key] = value
                with self.assertRaises(ValueError):
                    self.run_gate()
                self.record["final_fence"][key] = previous

    def test_symlinks_duplicate_fields_and_pin_rejected(self):
        self.args.review.write_text('{"status":"ready","status":"ready"}')
        with self.assertRaises(ValueError):
            P.pinned_json(self.args.review, P.digest(self.args.review))
        with self.assertRaises(ValueError):
            P.pinned_json(self.args.review, "f" * 64)
        (self.candidate / "web/link").symlink_to("index.html")
        with self.assertRaises(ValueError):
            P.inventory(self.candidate)

    def test_maintenance_503_eta_and_no_overwrite(self):
        output = self.root / "maintenance.conf"
        eta = (dt.datetime.now(dt.timezone.utc) + dt.timedelta(minutes=12)).isoformat()
        P.maintenance(eta, output)
        text = output.read_text()
        self.assertIn("return 503", text)
        self.assertIn("Retry-After", text)
        self.assertIn("no-store", text)
        self.assertIn(eta, text)
        with self.assertRaises(FileExistsError):
            P.maintenance(eta, output)
        with self.assertRaises(ValueError):
            P.maintenance("2000-01-01T00:00:00Z", self.root / "old.conf")

    def test_versions_are_forward_only(self):
        self.assertGreater(P.version("1.2.4"), P.version("1.2.3"))
        self.assertGreater(P.version("1.2.3"), P.version("1.2.3-rc.1"))
        with self.assertRaises(ValueError):
            P.version("latest")

    def test_real_portable_web_archive_staging_and_pin(self):
        spec = importlib.util.spec_from_file_location("distribution_test", P.ROOT / "scripts/build-distribution.py")
        distribution = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(distribution)
        inputs = self.root / "web-input"
        inputs.mkdir()
        (inputs / "index.html").write_text("synthetic archive")
        archive = distribution.build(P.ROOT, "web", "1.2.3-preview", "e" * 40,
                                     "any", inputs, self.root / "archives", 0)
        plan = self.root / "plan.json"
        value = {"kind": "lmm-native-artifact-plan-v1", "platform": "linux-amd64",
                 "components": {"web": {"archive": str(archive), "sha256": P.digest(archive),
                                        "source_revision": "e" * 40}}}
        plan.write_text(json.dumps(value))
        output = self.root / "stage"
        result = P.stage(plan, P.digest(plan), output)
        self.assertEqual(P.release(output, result)["components"]["web"]["version"], "1.2.3-preview")
        with self.assertRaises(ValueError):
            P.stage(plan, P.digest(plan), output)
        value["components"]["web"]["sha256"] = "f" * 64
        plan.write_text(json.dumps(value))
        with self.assertRaises(ValueError):
            P.stage(plan, P.digest(plan), self.root / "bad-stage")
        self.assertFalse((self.root / "bad-stage").exists())

    def test_software_update_rejects_downgrade_and_schema_change(self):
        old = self.root / "previous"
        import shutil
        shutil.copytree(self.candidate, old)
        self.args.mode = self.record["mode"] = "update"
        self.args.previous_dir = old
        self.args.previous_sha256 = P.digest(old / "release.json")
        self.record["previous_release_sha256"] = self.args.previous_sha256
        self.record["schema"] = {"before_sha256": "d" * 64, "after_sha256": "d" * 64,
                                 "ordinary_startup_only": True}
        for version, passed in [("1.2.2", False), ("1.2.4", True)]:
            self.release["components"]["web"]["version"] = version
            (self.candidate / "release.json").write_text(json.dumps(self.release))
            self.args.release_sha256 = self.record["release_sha256"] = P.digest(self.candidate / "release.json")
            if passed:
                self.run_gate()
            else:
                with self.assertRaises(ValueError):
                    self.run_gate()
        self.record["schema"]["after_sha256"] = "f" * 64
        with self.assertRaises(ValueError):
            self.run_gate()


if __name__ == "__main__":
    unittest.main(verbosity=2)
