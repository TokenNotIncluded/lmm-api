#!/usr/bin/env python3
"""Read-only shape checks for legacy provider reconciliation evidence.

This module never contacts a provider, verifies a signature, writes a receipt,
or changes a settlement state.  It only checks whether a future provider
contract has enough hash-bound fields to be reviewed by an independently
implemented verifier.  Every result remains ``unknown`` and non-spendable.
"""

from __future__ import annotations

import hashlib
import json
from typing import Any


HEX = "0123456789abcdef"
CONTRACT = "legacy-provider-verification-contract.v1"
UNKNOWN = "unknown"

PROVIDERS = {
    "epay": {
        "revision": "epay-md5-cny-v1",
        "required_source": ("trade_no",),
        "required_evidence": (
            "merchant_sha256", "environment", "currency", "amount_raw_sha256",
            "provider_transaction_sha256", "provider_event_id_sha256",
            "payload_sha256", "signature_sha256", "signature_verification",
            "provider_status", "observed_at",
        ),
    },
    "waffo_pancake": {
        "revision": "waffo-pancake-hmac-v1",
        "required_source": (
            "trade_no", "provider_transaction_id", "provider_event_id",
            "provider_product_id", "provider_store_id", "settlement_currency",
        ),
        "required_evidence": (
            "merchant_sha256", "environment", "currency", "amount_raw_sha256",
            "provider_transaction_sha256", "provider_event_id_sha256",
            "store_sha256", "product_sha256", "payload_sha256", "signature_sha256",
            "signature_verification", "provider_status", "observed_at",
        ),
    },
    # No reviewed FastPay wire contract or independent verifier exists in the
    # new architecture.  Keeping this explicit prevents a generic parser from
    # accidentally turning a pending row into accepted evidence.
    "fastpay": None,
}

BASE_EVIDENCE_FIELDS = frozenset({
    "contract_revision", "source_row_sha256", "merchant_sha256", "environment",
    "currency", "amount_raw_sha256", "provider_transaction_sha256",
    "provider_event_id_sha256", "payload_sha256", "signature_sha256",
    "signature_verification",
    "provider_status", "observed_at",
})
WAFFO_EVIDENCE_FIELDS = BASE_EVIDENCE_FIELDS | {"store_sha256", "product_sha256"}
BASE_HASH_FIELDS = frozenset({
    "source_row_sha256", "merchant_sha256", "amount_raw_sha256",
    "provider_transaction_sha256", "provider_event_id_sha256", "payload_sha256",
    "signature_sha256",
})
WAFFO_HASH_FIELDS = BASE_HASH_FIELDS | {"store_sha256", "product_sha256"}
STATUS_VALUES = frozenset({"pending", "paid", "failed", "refunded", "unknown"})
SIGNATURE_VALUES = frozenset({"not_run", "verified", "failed"})
ENVIRONMENTS = frozenset({"test", "live"})


def _hash(value: Any) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _canonical(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def _digest_json(value: Any) -> str:
    return _hash(_canonical(value))


def _is_hash(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(c in HEX for c in value)


def _source_value(record: dict[str, Any], field: str) -> Any:
    refs = record.get("references")
    raw_row = record.get("raw_row")
    if not isinstance(refs, dict) or not isinstance(raw_row, dict):
        return None
    if field in refs:
        return refs[field]
    return raw_row.get(field)


def _source_present(record: dict[str, Any], field: str) -> bool:
    value = _source_value(record, field)
    return isinstance(value, str) and bool(value.strip())


def _validate_evidence(evidence: Any, provider: str) -> list[str]:
    if evidence is None:
        return ["external_evidence"]
    expected_fields = WAFFO_EVIDENCE_FIELDS if provider == "waffo_pancake" else BASE_EVIDENCE_FIELDS
    if not isinstance(evidence, dict) or set(evidence) != expected_fields:
        return ["evidence_shape"]
    missing: list[str] = []
    if not isinstance(evidence["contract_revision"], str) or not evidence["contract_revision"]:
        missing.append("contract_revision")
    hash_fields = WAFFO_HASH_FIELDS if provider == "waffo_pancake" else BASE_HASH_FIELDS
    for field in hash_fields:
        if not _is_hash(evidence[field]):
            missing.append(field)
    if evidence["environment"] not in ENVIRONMENTS:
        missing.append("environment")
    if not isinstance(evidence["currency"], str) or not (1 <= len(evidence["currency"]) <= 16):
        missing.append("currency")
    if evidence["signature_verification"] not in SIGNATURE_VALUES:
        missing.append("signature_verification")
    if evidence["provider_status"] not in STATUS_VALUES:
        missing.append("provider_status")
    if type(evidence["observed_at"]) is not int or evidence["observed_at"] <= 0:
        missing.append("observed_at")
    return sorted(set(missing))


def analyze(record: dict[str, Any], evidence: dict[str, Any] | None = None) -> dict[str, Any]:
    """Return a sanitized review classification without accepting settlement."""
    if not isinstance(record, dict):
        raise ValueError("record_shape")
    provider = record.get("provider")
    if not isinstance(provider, str) or provider not in PROVIDERS:
        return _result(provider if isinstance(provider, str) else "unknown", None, ["provider_contract"])
    contract = PROVIDERS[provider]
    if contract is None:
        return _result(provider, None, ["provider_contract"])

    missing = [field for field in contract["required_source"] if not _source_present(record, field)]
    raw_amount = record.get("raw_amount")
    raw_currency = record.get("raw_currency")
    if not isinstance(raw_amount, dict) or not raw_amount:
        missing.append("source_amount")
    if provider == "epay" and (not isinstance(raw_currency, dict) or not raw_currency.get("settlement_currency")):
        # The legacy ePay wire profile carries no currency.  A CNY assumption
        # is not evidence, so the source must retain an explicit currency fact.
        missing.append("source_currency")
    if provider == "waffo_pancake" and (
        not isinstance(raw_currency, dict) or not raw_currency.get("settlement_currency")
    ):
        missing.append("source_currency")
    evidence_missing = _validate_evidence(evidence, provider)
    missing.extend(evidence_missing)

    if evidence is not None and not evidence_missing:
        if evidence["contract_revision"] != contract["revision"]:
            missing.append("contract_revision_mismatch")
        if evidence["source_row_sha256"] != record.get("source_row_sha256"):
            missing.append("source_row_binding")
        transaction_source = (
            _source_value(record, "provider_transaction_id")
            if provider == "waffo_pancake"
            else _source_value(record, "trade_no")
        )
        if isinstance(transaction_source, str) and evidence["provider_transaction_sha256"] != _hash(transaction_source):
            missing.append("provider_transaction_binding")
        if provider == "waffo_pancake":
            event_source = _source_value(record, "provider_event_id")
            store_source = _source_value(record, "provider_store_id")
            product_source = _source_value(record, "provider_product_id")
            if isinstance(event_source, str) and evidence["provider_event_id_sha256"] != _hash(event_source):
                missing.append("provider_event_binding")
            if isinstance(store_source, str) and evidence["store_sha256"] != _hash(store_source):
                missing.append("store_binding")
            if isinstance(product_source, str) and evidence["product_sha256"] != _hash(product_source):
                missing.append("product_binding")
        if isinstance(raw_amount, dict) and evidence["amount_raw_sha256"] != _digest_json(raw_amount):
            missing.append("amount_binding")
        if isinstance(raw_currency, dict) and evidence["currency"] and provider == "waffo_pancake":
            expected_currency = raw_currency.get("settlement_currency")
            if isinstance(expected_currency, str) and evidence["currency"] != expected_currency:
                missing.append("currency_binding")

    classification = "SHAPE_VALID_REQUIRES_INDEPENDENT_VERIFIER" if not missing else "INCOMPLETE_PROVIDER_EVIDENCE"
    return _result(provider, contract["revision"], sorted(set(missing)), classification=classification)


def _result(provider: str, revision: str | None, missing: list[str], *, classification: str = "UNSUPPORTED_PROVIDER_CONTRACT") -> dict[str, Any]:
    return {
        "protocol": CONTRACT,
        "provider": provider,
        "contract_revision": revision,
        "classification": classification,
        "verification_status": "UNVERIFIED",
        "settlement_state": UNKNOWN,
        "spendable": False,
        "spendable_units": 0,
        "missing": sorted(set(missing)),
        "provider_io_performed": False,
        "target_writes": 0,
        "ledger_writes": 0,
    }


def summarize(records: list[dict[str, Any]], evidence_by_record: dict[str, dict[str, Any]] | None = None) -> dict[str, Any]:
    evidence_by_record = evidence_by_record or {}
    results = [analyze(row, evidence_by_record.get(row.get("record_id"))) for row in records]
    counts: dict[str, int] = {}
    for item in results:
        counts[item["classification"]] = counts.get(item["classification"], 0) + 1
    return {
        "protocol": CONTRACT,
        "record_count": len(results),
        "classification_counts": dict(sorted(counts.items())),
        "verification_status": "UNVERIFIED",
        "settlement_state": UNKNOWN,
        "spendable": False,
        "spendable_units": 0,
        "provider_io_performed": False,
        "target_writes": 0,
        "ledger_writes": 0,
        "results": results,
    }
