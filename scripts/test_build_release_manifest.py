#!/usr/bin/env python3
"""Synthetic checks for build-release-manifest.py."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "manifest", Path(__file__).with_name("build-release-manifest.py")
)
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


class Checks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.value = {
            "source_commit": "a" * 40,
            "core_artifact_sha256": "1" * 64,
            "extensions_artifact_sha256": "2" * 64,
            "sbom_sha256": "3" * 64,
            "schema_fingerprint_sha256": "4" * 64,
            "provider_matrix_sha256": "5" * 64,
            "identity_evidence_sha256": "6" * 64,
            "financial_evidence_sha256": "7" * 64,
            "edge_config_sha256": "8" * 64,
            "runtime_release_identity_sha256": "9" * 64,
            "runtime_source_commit": "a" * 40,
            "runtime_core_artifact_sha256": "1" * 64,
            "runtime_extensions_artifact_sha256": "2" * 64,
            "previous_writer_epoch": 4,
            "writer_epoch": 5,
        }

    def test_builds_canonical_manifest_and_protects_output(self):
        source = self.root / "input.json"
        output = self.root / "release-manifest.json"
        source.write_text(json.dumps({"release": self.value}))
        digest = M.build(source, output)
        self.assertEqual(len(digest), 64)
        rendered = json.loads(output.read_text())
        self.assertEqual(rendered["protocol"], "release-manifest.v1")
        self.assertEqual(rendered["source_commit"], "a" * 40)
        self.assertEqual(output.stat().st_mode & 0o777, 0o600)
        with self.assertRaises(ValueError):
            M.build(source, output)

    def test_rejects_mismatch_and_unknown_fields(self):
        source = self.root / "input.json"
        value = dict(self.value, runtime_core_artifact_sha256="f" * 64)
        source.write_text(json.dumps(value))
        with self.assertRaises(ValueError):
            M.parse(source)
        value = dict(self.value, unexpected="x")
        source.write_text(json.dumps(value))
        with self.assertRaises(ValueError):
            M.parse(source)

    def test_rejects_non_monotonic_epoch_and_duplicate_fields(self):
        source = self.root / "input.json"
        source.write_text(json.dumps(dict(self.value, writer_epoch=4)))
        with self.assertRaises(ValueError):
            M.parse(source)
        source.write_text(
            '{"source_commit":"' + "a" * 40
            + '","source_commit":"' + "b" * 40 + '"}'
        )
        with self.assertRaises(ValueError):
            M.parse(source)


if __name__ == "__main__":
    unittest.main(verbosity=2)
