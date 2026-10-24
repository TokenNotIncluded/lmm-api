#!/usr/bin/env python3
"""Executable localhost-only edge compatibility check."""

import unittest

from edge_cutover_harness import run_harness


class EdgeCutoverHarnessTests(unittest.TestCase):
    def test_candidate_fixture_contract(self):
        result = run_harness()
        self.assertEqual(result, {
            "checks": 12,
            "listen": "127.0.0.1",
            "nginx_test": True,
            "status": "PASS",
        })


if __name__ == "__main__":
    unittest.main(verbosity=2)
