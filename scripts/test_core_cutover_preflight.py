#!/usr/bin/env python3
"""Synthetic contract checks for core-cutover-preflight.py."""

import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("preflight", Path(__file__).with_name("core-cutover-preflight.py"))
P = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(P)


class Checks(unittest.TestCase):
    def base(self):
        sha = "a" * 64
        contracts = {}
        for name in P.CONTRACTS:
            contracts[name] = {
                "config": "reviewed",
                "credentials": "reviewed",
                "contract": "reviewed",
                "probe": "passed",
                "contract_sha256": sha,
                "extension": "reviewed",
                "provider": "external" if name == "payments" else "none",
            }
        return {
            "protocol": P.PROTOCOL,
            "host": "ArchDmit",
            "source_revision": "b" * 64,
            "core": {
                "ready": {
                    "http_status": 200,
                    "database": True,
                    "model_ready_status": 200,
                    "capabilities": {
                        "models": True,
                        "models_streaming": True,
                        "payments": True,
                        "commerce": True,
                        "assistant": True,
                        "assistant_tools": True,
                        "tool_market": True,
                    },
                },
                "public_edge": {"mode": "candidate", "candidate_selected": True},
                "contracts": contracts,
            },
            "extensions": {
                "ready_http_status": 200,
                "modules": ["assistant", "toolmarket"],
                "host_credential": "reviewed",
            },
            "migration": {
                "status": "completed",
                "unknown_liabilities": 0,
                "reconciliation": "passed",
                "writer_fenced": True,
                "active_writers": 0,
                "legacy_workers_stopped": True,
            },
        }

    def test_complete_record_is_ready(self):
        result = P.evaluate(P.parse(self.base()))
        self.assertEqual(result["status"], "READY")
        self.assertTrue(result["safe_to_cutover"])
        self.assertEqual(result["unsafe_enablement"], [])

    def test_archdmit_like_disabled_record_is_blocked(self):
        record = self.base()
        record["core"]["ready"]["model_ready_status"] = 503
        record["core"]["ready"]["capabilities"].update({
            "models": False, "models_streaming": False, "payments": False,
            "commerce": False, "assistant": False, "assistant_tools": False,
            "tool_market": False,
        })
        record["core"]["public_edge"] = {"mode": "legacy", "candidate_selected": False}
        record["extensions"]["modules"] = []
        record["migration"].update({"status": "blocked", "unknown_liabilities": 82,
                                   "reconciliation": "not_run", "writer_fenced": False,
                                   "active_writers": 1, "legacy_workers_stopped": False})
        result = P.evaluate(P.parse(record))
        self.assertEqual(result["status"], "BLOCKED")
        self.assertFalse(result["safe_to_cutover"])
        boundary = next(g for g in result["gates"] if g["name"] == "cutover_boundary")
        self.assertIn("migration.unknown_liabilities", boundary["missing"])
        self.assertIn("core.public_edge.candidate_selected", boundary["missing"])

    def test_enabled_capability_without_contract_is_unsafe(self):
        record = self.base()
        record["core"]["contracts"]["payments"] = {
            "config": "absent", "credentials": "absent", "contract": "absent",
            "probe": "not_run", "contract_sha256": None, "extension": "absent",
            "provider": "none",
        }
        result = P.evaluate(P.parse(record))
        self.assertEqual(result["status"], "BLOCKED")
        self.assertIn("payments", result["unsafe_enablement"])

    def test_duplicate_or_secret_fields_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "evidence.json"
            path.write_text('{"protocol":"core-cutover-preflight.v1","protocol":"x"}')
            with self.assertRaises(ValueError):
                P.run(path)
            record = self.base()
            record["core"]["contracts"]["model_forwarding"]["api_key"] = "secret"
            with self.assertRaises(ValueError):
                P.parse(record)

    def test_test_provider_cannot_pass_production_payment_gate(self):
        record = self.base()
        record["core"]["contracts"]["payments"]["provider"] = "stripe-test"
        result = P.evaluate(P.parse(record))
        gate = next(g for g in result["gates"] if g["name"] == "payments")
        self.assertEqual(gate["status"], "BLOCKED")
        self.assertIn("payments.external_provider_contract", gate["missing"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
