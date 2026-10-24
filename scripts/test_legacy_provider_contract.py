#!/usr/bin/env python3
"""Tests for the source-only legacy provider verification contract."""

import hashlib
import importlib.util
import unittest
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "legacy_provider_contract", Path(__file__).with_name("legacy_provider_contract.py")
)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC and SPEC.loader
SPEC.loader.exec_module(MODULE)


def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()


def base(provider="epay"):
    refs = {"trade_no": "EPAY-1001"}
    raw_currency = {"settlement_currency": "CNY"}
    if provider == "waffo_pancake":
        refs.update({
            "provider_transaction_id": "pay_1001",
            "provider_event_id": "evt_1001",
            "provider_product_id": "prod_1001",
            "provider_store_id": "store_1001",
        })
        raw_currency = {"settlement_currency": "USD"}
    return {
        "record_id": "a" * 64,
        "source_row_sha256": "b" * 64,
        "source_table": "top_ups",
        "source_row_id": 1,
        "source_row_ordinal": 0,
        "source_user_id": 7,
        "target_user_id": 7,
        "target_account_id": 9,
        "operation_kind": "topup_payment",
        "legacy_status": "pending",
        "provider": provider,
        "references": refs,
        "raw_amount": {"money": "10.00"},
        "raw_currency": raw_currency,
        "raw_row": {"id": 1, "user_id": 7, "status": "pending", **refs},
        "settlement_state": "unknown",
        "spendable": False,
        "spendable_units": 0,
        "disposition": "deferred-source-only-no-target-write",
    }


def evidence(record, provider="epay"):
    refs = record["references"]
    source_transaction = refs.get("provider_transaction_id", refs["trade_no"])
    value = {
        "contract_revision": "waffo-pancake-hmac-v1" if provider == "waffo_pancake" else "epay-md5-cny-v1",
        "source_row_sha256": record["source_row_sha256"],
        "merchant_sha256": "c" * 64,
        "environment": "live",
        "currency": record["raw_currency"]["settlement_currency"],
        "amount_raw_sha256": digest('{"money":"10.00"}'),
        "provider_transaction_sha256": digest(source_transaction),
        "provider_event_id_sha256": digest(refs.get("provider_event_id", "evt_1001")),
        "payload_sha256": "d" * 64,
        "signature_sha256": "e" * 64,
        "signature_verification": "verified",
        "provider_status": "paid",
        "observed_at": 1_800_000_000,
    }
    if provider == "waffo_pancake":
        value["store_sha256"] = digest(refs["provider_store_id"])
        value["product_sha256"] = digest(refs["provider_product_id"])
    return value


class ProviderContractTests(unittest.TestCase):
    def test_fastpay_is_explicitly_unsupported_and_fenced(self):
        result = MODULE.analyze(base("fastpay"))
        self.assertEqual(result["classification"], "UNSUPPORTED_PROVIDER_CONTRACT")
        self.assertIn("provider_contract", result["missing"])
        self.assertEqual(result["settlement_state"], "unknown")
        self.assertFalse(result["spendable"])

    def test_epay_without_explicit_source_currency_stays_incomplete(self):
        record = base("epay")
        record["raw_currency"] = {}
        result = MODULE.analyze(record)
        self.assertEqual(result["classification"], "INCOMPLETE_PROVIDER_EVIDENCE")
        self.assertIn("source_currency", result["missing"])
        self.assertIn("external_evidence", result["missing"])

    def test_valid_shape_is_still_unverified_and_non_spendable(self):
        record = base("epay")
        result = MODULE.analyze(record, evidence(record))
        self.assertEqual(result["classification"], "SHAPE_VALID_REQUIRES_INDEPENDENT_VERIFIER")
        self.assertEqual(result["verification_status"], "UNVERIFIED")
        self.assertEqual(result["settlement_state"], "unknown")
        self.assertFalse(result["spendable"])
        self.assertEqual(result["ledger_writes"], 0)

    def test_waffo_bindings_are_hash_checked(self):
        record = base("waffo_pancake")
        candidate = evidence(record, "waffo_pancake")
        candidate["store_sha256"] = "f" * 64
        result = MODULE.analyze(record, candidate)
        self.assertEqual(result["classification"], "INCOMPLETE_PROVIDER_EVIDENCE")
        self.assertIn("store_binding", result["missing"])

    def test_strict_evidence_shape_rejects_secret_payload_fields(self):
        record = base("epay")
        candidate = evidence(record)
        candidate["raw_payload"] = "signed-provider-payload"
        result = MODULE.analyze(record, candidate)
        self.assertEqual(result["classification"], "INCOMPLETE_PROVIDER_EVIDENCE")
        self.assertEqual(result["missing"], ["evidence_shape"])

    def test_summary_never_claims_settlement(self):
        result = MODULE.summarize([base("epay"), base("fastpay"), base("waffo_pancake")])
        self.assertEqual(result["verification_status"], "UNVERIFIED")
        self.assertEqual(result["settlement_state"], "unknown")
        self.assertFalse(result["spendable"])
        self.assertEqual(result["ledger_writes"], 0)


if __name__ == "__main__":
    unittest.main()
