#!/usr/bin/env python3
"""Prepare a reviewed, offline fresh26 trust-basis import; never connect or apply.

Run AFTER identity, console and financial imports and BEFORE enabling runtime.
Inventory and snapshot commands emit SELECT-only SQL for an independent runner.
Prepare emits a non-executable SQL candidate and pending review request. Seal
requires a separately supplied owner approval binding the exact candidate bytes.
A lost execution result is UNKNOWN: retain the attempt and run its resolve.sql;
never generate another ID or infer rollback from a disconnected client.
"""
from __future__ import annotations

import argparse
from datetime import datetime
from decimal import Decimal
from fractions import Fraction
import hashlib
import json
import os
from pathlib import Path
import re
import sys

import financial_import as F

I = F.IDENTITY
Error = I.MappingError
require = I.require
ROOT = Path(__file__).resolve().parents[1]
SCOPE = "trust-basis-fresh26-v2"
LEGACY_COMMIT = "cc52aa195c6f2a01d703a1a407be73c31b89867d"
SOURCE_PROJECTION = "legacy-cc52-trustPaidCreditSQL-net-v1"
# Original cc52 source bytes, reviewed alongside the export adapter. The report
# must record these files from its source tree; changing only a commit label is
# insufficient evidence that the old entitlement algorithm actually ran.
PROJECTOR_SOURCE_HASHES = {
    "apps/api-go/model/topup.go": "39f7247c6d5d57f0cef008f2117f72d524e391e5b8c628657f03ace2c1a9be8e",
    "apps/api-go/model/trust_level.go": "2d3f1188fbc92214b329f87aa98c50cae09e1ee20e2008557f702622444342b9",
    "apps/api-go/model/trust_paid_credit_projection.go": "daddeaa8509729166b7ce2d3d917a76d747f77a850d3543b0bf2053ff12e3259",
    "apps/api-go/model/user_topup_projection.go": "2daa33a191f35bfb903dd6e428fd4f2292c22844cd726070eca56637e1f5f9d6",
    "apps/api-go/model/payment_refund.go": "902db7c0b99058e2b8fe18236073eee7b4746700ede056b5edba842fa068e0f5",
    "apps/api-go/model/payment_refund_rebase.go": "21a6a7fe2c772617699154fd944aabfaafb834a3fc11ff6c253c5e0d00551c53",
}
APPROVAL_SLOT = "__LMM_TRUST_OWNER_APPROVAL_SHA256__"
CHANGED = ("core_trust_pricing.policies", "core_trust_pricing.user_bases",
           "core_trust_pricing.legacy_paid_orders")
INPUTS = ("basis", "evidence", "user_map", "identity_mapping", "target_snapshot", "source_review_report")


def canonical(value):
    # Preserve exact decimal text. No binary float enters price calculations.
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True,
                      default=lambda v: format(v, "f") if isinstance(v, Decimal) else fail("unsupported_json_type"))


def fail(code):
    raise Error(code)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def digest(value):
    return sha(canonical(value).encode())


def checked_hash(value):
    require(type(value) is str and re.fullmatch("[a-f0-9]{64}", value) is not None, "invalid_digest")
    return value


def integer(value, low=0, high=9223372036854775807):
    require(type(value) is int and low <= value <= high, "invalid_integer")
    return value


def legacy_refund_target(total, paid, refunded):
    """Original shopspring Decimal.Div precision16, then Round(0), exactly.

    The legacy proportional counter intentionally differs from the rebase
    integer-ratio counter near half ties. Keep both historical domains intact.
    """
    if total <= 0 or paid <= 0 or refunded <= 0:
        return 0
    scale = 10**16
    quotient, remainder = divmod(total * refunded * scale, paid)
    rounded_division = quotient + (2 * remainder >= paid)
    whole, fraction = divmod(rounded_division, scale)
    return min(total, whole + (2 * fraction >= scale))


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate_json_key")
        result[key] = value
    return result


def decode(data):
    return json.loads(data, object_pairs_hook=unique_object, parse_float=Decimal,
                      parse_constant=lambda _: fail("nonfinite_json"))


def ratio(value):
    require(type(value) in (int, str, Decimal), "invalid_ratio")
    if type(value) is str:
        require(re.fullmatch(r"[0-9]+(?:\.[0-9]+)?", value) is not None, "invalid_ratio")
    fraction = Fraction(Decimal(value))
    require(0 < fraction <= 1 and fraction.denominator <= 10**18, "unrepresentable_ratio")
    return {"numerator": fraction.numerator, "denominator": fraction.denominator}


def policy_spec(raw):
    require(type(raw) is dict and type(raw.get("version")) is int and raw.get("version") == 1
            and type(raw.get("paid_activation_enabled")) is bool, "invalid_policy")
    result = {"version": 1, "paid_activation_enabled": raw["paid_activation_enabled"],
              "decay_period_days": integer(raw["decay_period_days"], 0, 3650), "tiers": []}
    require(len(raw["tiers"]) == 5 and len(raw["role_tiers"]) == 2, "invalid_policy_tiers")
    previous = -1
    for level, tier in enumerate(raw["tiers"]):
        require(type(tier["level"]) is int and tier["level"] == level, "invalid_tier_level")
        threshold = integer(tier["min_paid_credits"], 0, 2147483647)
        require(threshold > previous and (level != 0 or threshold == 0), "invalid_tier_threshold")
        previous = threshold
        result["tiers"].append({"level": level, "min_paid_credits": threshold,
                                "discount": ratio(tier["discount_ratio"])})
    for index, (level, role, name) in enumerate(((5, 10, "admin_discount"), (6, 100, "superadmin_discount"))):
        tier = raw["role_tiers"][index]
        require(type(tier["level"]) is int and type(tier["role"]) is int
                and (tier["level"], tier["role"]) == (level, role), "invalid_role_policy")
        result[name] = ratio(tier["discount_ratio"])
    return result



def validate_order_source(proof):
    """Mirror cc52 topup.go's original eligibility, without FX conversion.

    Currency is an opaque original label, including an explicitly stored empty
    string. Missing/null evidence is different from the old NOT NULL default ''.
    Gross/NET credit normalization still requires the separately bound original
    projector report, including rebases/refunds; it is never inferred from money.
    """
    row = proof.get("original_row")
    require(type(row) is dict and digest(row) == proof.get("original_row_sha256"), "original_order_row_changed")
    require(type(row.get("id")) is int and type(row.get("user_id")) is int
            and row.get("id") == proof["source_order_id"] and row.get("user_id") == proof["source_user_id"],
            "original_order_identity_mismatch")
    require(row.get("payment_provider") is None or type(row["payment_provider"]) is str, "unknown_order_source")
    require(row.get("payment_method") is None or type(row["payment_method"]) is str, "unknown_order_source")
    provider, method = row.get("payment_provider") or "", row.get("payment_method") or ""
    require(type(provider) is str and type(method) is str, "unknown_order_source")
    require(provider in {"epay", "stripe", "creem", "waffo", "waffo_pancake"}
            or (not provider and method in {"stripe", "creem", "waffo", "waffo_pancake", "alipay", "wxpay"}),
            "unknown_order_source")
    require(provider != "balance" and method != "balance", "ineligible_internal_order")
    require(not (provider.lower() == "epay" and method.lower() in {"ldc", "linuxdo", "linux_do", "linuxdo_credit"}),
            "ineligible_internal_order")
    require(row.get("status") == "success", "order_not_successful")
    settled = integer(row.get("settled_amount_micros"))
    expected = integer(row.get("expected_amount_micros"))
    money = row.get("money")
    require(type(money) in (int, str, Decimal), "original_money_invalid")
    money = Decimal(money)
    require(money.is_finite() and (settled > 0 or (settled == 0 and money > 0)), "order_paid_evidence_missing")
    currency = proof.get("original_currency")
    require(type(currency) is str and len(currency) <= 16
            and "\0" not in currency and row.get("settlement_currency") == currency, "original_currency_unproven")
    require(not (provider.lower() == "epay" and method.lower() == "epay"
                 and (currency.upper() != "CNY" or (expected <= 0 and settled <= 0))), "ineligible_epay_order")
    amount = proof.get("original_amount")
    require(type(amount) is str and Decimal(amount).is_finite()
            and Decimal(amount) == (Decimal(settled) / 1000000 if settled > 0 else money), "original_amount_mismatch")
    created, completed = integer(row.get("create_time")), integer(row.get("complete_time"))
    require(proof["paid_at"] == (completed if completed > 0 else created), "original_paid_time_mismatch")
    credited_quota = integer(row.get("credited_quota"), -9223372036854775808)
    amount = integer(row.get("amount"), -9223372036854775808)
    gross = credited_quota if credited_quota > 0 else amount if provider == "creem" or method == "creem" else amount * 500000
    require(gross > 0, "original_gross_credit_not_positive")



def validate_audited_lineage(audit, proof, cutoff):
    """Validate actual original audit rows, reusing the existing refund mapper.

    A named empty table is never evidence. The observed full plan digest and
    historical declared digest have distinct domains; no self-containing plan
    is relabelled as if its declared SHA were a hash of its final serialization.
    """
    rows = audit["source_rows"]
    tables = {name: [entry["row"] for entry in rows if entry["table"] == name]
              for name in ("top_ups", "wallet_credit_rebases", "wallet_topup_credit_rebases")}
    parents = tables["wallet_credit_rebases"]
    require(parents, "rebase_audit_source_rows_missing")
    normalized, plan_material, plans = [], {}, {}
    for parent in parents:
        mid = parent.get("migration_id")
        require(type(mid) is str and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,99}", mid)
                and mid not in plans, "rebase_parent_identity_invalid")
        raw = parent.get("plan")
        require(type(raw) in (str, dict), "rebase_parent_plan_missing")
        plan = decode(raw) if type(raw) is str else raw
        require(type(plan) is dict and plan.get("migration_id") == mid
                and plan.get("plan_sha256") == checked_hash(parent.get("plan_sha256")), "rebase_parent_plan_hash_binding_mismatch")
        require(type(plan.get("version")) is int and plan["version"] == 1
                and plan.get("kind") == "offline_credit_balance_rebase_preview"
                and plan.get("has_complete_history") is True and plan.get("snapshot_state") == "frozen_writers_stopped"
                and type(plan.get("usd_credit_conversion")) is int and plan["usd_credit_conversion"] == 500000,
                "rebase_parent_plan_contract_invalid")
        integer(plan.get("snapshot_at"), 1, cutoff)
        for key in ("refund_bases", "pending_bases", "blocked_pending_bases", "noncash_topups", "user_ids", "option_entries"):
            require(type(plan.get(key)) is list, "rebase_parent_plan_incomplete")
        ids = [integer(uid, 1) for uid in plan["user_ids"]]
        require(ids and len(ids) == len(set(ids)), "rebase_parent_users_invalid")
        divisor = plan.get("divisor")
        require(type(divisor) is str and len(divisor) <= 64 and re.fullmatch(r"[0-9]+(?:\.[0-9]+)?", divisor), "rebase_frozen_divisor_invalid")
        factor = Fraction(Decimal(divisor))
        require(factor > 1 and plan.get("rounding") in ("half-away-from-zero", "toward-zero")
                and canonical(plan.get("exact_factor")) == canonical({"numerator": factor.denominator, "denominator": factor.numerator})
                and plan.get("fx_source", {}).get("kind") == "frozen_production_option"
                and plan["fx_source"].get("key") == "USDExchangeRate"
                and type(plan["fx_source"].get("value")) is str
                and Fraction(Decimal(plan["fx_source"]["value"])) == factor, "rebase_frozen_factor_mismatch")
        plan_material[mid] = {"declared_plan_sha256": parent["plan_sha256"],
                              "observed_plan_sha256": sha(raw.encode()) if type(raw) is str else digest(raw)}
        plans[mid] = plan
        # Pass normalized JSON to the existing helper, whose explicit JSONB
        # path preserves the original declared hash without pretending that
        # historical source serialization is still available.
        normalized.append({**parent, "plan": plan})
    require(canonical(audit.get("parent_plan_hashes")) == canonical(plan_material), "rebase_full_plan_material_changed")
    require(audit["rebase_cutoff"] == max(plan["snapshot_at"] for plan in plans.values()), "rebase_cutoff_mismatch")
    tables["wallet_credit_rebases"] = normalized
    identity_map = {integer(row.get("user_id"), 1): integer(row.get("user_id"), 1) for row in tables["top_ups"]}
    lineage = F.PLANNER.map_refund_lineage(tables, identity_map)
    require(not any(row["blockers"] for group in ("wallet_refund_children", "wallet_rebase_audits", "topup_refund_lineage")
                    for row in lineage[group]), "rebase_refund_lineage_invalid")
    row = proof["original_row"]
    mode = audit["projection_mode"]
    if mode == "audited_rebase":
        children = [child for child in tables["wallet_topup_credit_rebases"] if child.get("top_up_id") == proof["source_order_id"]]
        require(len(children) == 1, "rebase_child_missing_or_duplicate")
        child = children[0]
        require(child.get("user_id") == proof["source_user_id"] and child.get("migration_id") in plans,
                "rebase_child_identity_mismatch")
        plan = plans[child["migration_id"]]
        bases = [base for base in plan["refund_bases"] if base.get("top_up_id") == proof["source_order_id"]]
        require(len(bases) == 1 and proof["source_user_id"] in plan["user_ids"], "rebase_plan_order_missing")
        base = bases[0]
        fields = ("top_up_id", "user_id", "original_credited_quota", "original_refunded_quota",
                  "original_refunded_amount_micros", "original_paid_amount_micros", "refundable_quota")
        require(all(type(base.get(key)) is int and base[key] == child[key] for key in fields)
                and type(base.get("rebased_debited_quota")) is int and base["rebased_debited_quota"] == 0,
                "rebase_child_plan_mismatch")
        frozen = base.get("source")
        require(type(frozen) is dict and frozen.get("id") == row["id"] and frozen.get("user_id") == row["user_id"]
                and frozen.get("status") == "success", "rebase_original_source_missing")
        for key in ("amount", "platform_amount_micros", "credited_quota", "expected_amount_micros", "settled_amount_micros",
                    "payment_provider", "payment_method", "settlement_currency"):
            require(key in frozen and key in row and canonical(frozen[key]) == canonical(row[key]), "rebase_original_source_changed")
        require(type(frozen.get("money")) in (int, str, Decimal) and Decimal(frozen["money"]).is_finite()
                and Decimal(frozen["money"]) == Decimal(row["money"]), "rebase_original_source_changed")
        paid = frozen["settled_amount_micros"] if frozen["settled_amount_micros"] > 0 else frozen["expected_amount_micros"]
        if paid <= 0:
            money_micros = Fraction(Decimal(frozen["money"])) * 1000000
            whole, fractional = divmod(money_micros.numerator, money_micros.denominator)
            paid = whole + (2 * fractional >= money_micros.denominator)
        require(child["original_paid_amount_micros"] == paid, "rebase_original_paid_counter_mismatch")
        credited = frozen["credited_quota"]
        if credited <= 0:
            if frozen["payment_provider"] == "creem" or frozen["payment_method"] == "creem":
                credited = frozen["amount"]
            else:
                options = [option.get("before") for option in plan["option_entries"] if option.get("key") == "QuotaPerUnit"]
                require(len(options) == 1 and type(options[0]) is str and len(options[0]) <= 64
                        and re.fullmatch(r"[0-9]+(?:\.[0-9]+)?", options[0]), "rebase_frozen_quota_missing")
                quota = Fraction(Decimal(options[0]))
                require(quota > 0 and frozen["amount"] > 0, "rebase_frozen_quota_invalid")
                credited = frozen["amount"] * quota.numerator // quota.denominator
        require(integer(credited, 1) == child["original_credited_quota"], "rebase_original_credit_projection_mismatch")
        require(frozen.get("effective_credited_quota") == child["original_credited_quota"]
                and frozen.get("refunded_quota") == child["original_refunded_quota"]
                and frozen.get("refunded_amount_micros") == child["original_refunded_amount_micros"]
                and not row.get("pending_credit_rebase_key"), "rebase_original_counters_mismatch")
        remaining = child["original_paid_amount_micros"] - child["original_refunded_amount_micros"]
        refunded_since = row["refunded_amount_micros"] - child["original_refunded_amount_micros"]
        require(remaining > 0 and 0 <= refunded_since <= remaining, "rebase_original_paid_counter_mismatch")
        cumulative_original = legacy_refund_target(child["original_credited_quota"], child["original_paid_amount_micros"], row["refunded_amount_micros"])
        # proportionalRefundDelta returns zero without a NEW fiat refund,
        # retaining the actual original counter even if a recomputation differs.
        expected_original = child["original_refunded_quota"] if refunded_since == 0 else max(child["original_refunded_quota"], cumulative_original)
        require(row["refunded_quota"] == expected_original, "rebase_original_refund_projection_mismatch")
        factor = Fraction(Decimal(plan["divisor"]))
        original_remaining = child["original_credited_quota"] - child["original_refunded_quota"]
        quotient, remainder = divmod(original_remaining * factor.denominator, factor.numerator)
        expected_pool = quotient + (plan["rounding"] == "half-away-from-zero" and 2 * remainder >= factor.numerator)
        require(child["refundable_quota"] == expected_pool
                and proof["credited_credits"] == child["refundable_quota"]
                and proof["refunded_credits"] == child["rebased_debited_quota"], "rebase_net_unit_mismatch")
    elif mode == "post_cutoff_native":
        # cc52's post-cutoff projector separately requires SQL TRIM(currency)
        # to be nonempty; historical unrebased orders have no such condition.
        require(row["create_time"] > audit["rebase_cutoff"] and row["complete_time"] > audit["rebase_cutoff"]
                and row["credited_quota"] > 0 and row["expected_amount_micros"] > 0
                and bool(row["settlement_currency"].strip(" "))
                and not row.get("pending_credit_rebase_key")
                and row.get("pending_credit_rebase_original_quota", 0) == 0
                and row.get("pending_credit_rebase_effective_quota", 0) == 0
                and proof["credited_credits"] == row["credited_quota"] and proof["refunded_credits"] == row["refunded_quota"],
                "post_cutoff_order_mismatch")
    else:
        matches = [(plan, base) for plan in plans.values() for base in plan["pending_bases"]
                   if base.get("top_up_id") == proof["source_order_id"]]
        require(len(matches) == 1, "pending_rebase_basis_missing_or_duplicate")
        plan, base = matches[0]
        require(plan.get("include_pending_topups") is True and base.get("user_id") == proof["source_user_id"]
                and proof["source_user_id"] in plan["user_ids"]
                and row.get("pending_credit_rebase_key") == plan["migration_id"]
                and type(base.get("original_quota")) is int and base["original_quota"] > 0
                and type(base.get("effective_quota")) is int and base["effective_quota"] > 0
                and row.get("pending_credit_rebase_original_quota") == base["original_quota"]
                and row.get("pending_credit_rebase_effective_quota") == base["effective_quota"]
                and row["credited_quota"] == base["effective_quota"] and row["complete_time"] > plan["snapshot_at"]
                and proof["credited_credits"] == row["credited_quota"] and proof["refunded_credits"] == row["refunded_quota"],
                "pending_rebase_source_mismatch")
        require(type(base.get("source")) is dict and base["source"].get("id") == row["id"]
                and base["source"].get("user_id") == row["user_id"], "pending_rebase_original_source_missing")

def effective_quote(policy, user, role, at):
    """Parity check against the reviewed Core evaluate() at the source cutoff.

    It is a comparison of imported evidence, never a new spend authority. The
    original quote below must come from the original source projector.
    """
    if role in ("admin", "superadmin"):
        level, source = (5 if role == "admin" else 6), "role"
    elif user["override_level"] is not None:
        level, source = user["override_level"], "override"
    else:
        if user["projection_known"] is not True:
            return {"status": "unavailable"}
        positive = [order for order in user["orders"] if order["net_credits"] > 0]
        total = sum(order["net_credits"] for order in positive)
        activated = user["console_activated_at"] > 0 or (policy["paid_activation_enabled"]
            and bool(positive) and total >= policy["tiers"][1]["min_paid_credits"])
        level = 0 if not activated else max([1] + [tier["level"] for tier in policy["tiers"][2:]
                                                      if total >= tier["min_paid_credits"]])
        anchor = max(user["original_created_at"], user["last_api_activity_at"],
                     max([order["paid_at"] for order in positive], default=0))
        period = policy["decay_period_days"] * 86400
        if level > 1 and anchor > 0 and period > 0:
            level -= min((at - anchor) // period, level - 1)
        source = "automatic"
    discount = policy["superadmin_discount"] if level == 6 else policy["admin_discount"] if level == 5 else policy["tiers"][level]["discount"]
    return {"status": "available", "effective_level": level, "level_source": source, "discount": discount}


def source_contract():
    return {key: value for key, value in I.current_contract().items() if key != "catalog_query"}


def tool_hashes():
    paths = [Path(__file__), Path(F.__file__), Path(I.__file__),
             ROOT / "scripts/migrate-legacy-financial.py", ROOT / "scripts/legacy_financial_preflight.py"]
    return {str(path.relative_to(ROOT)): I.sha256_file(path) for path in paths}


def derive_inputs(inputs, hashes):
    basis, evidence, mapping, identities = (inputs[key] for key in INPUTS[:4])
    require(basis.get("version") == 1 and type(basis.get("version")) is int
            and basis.get("source_commit") == LEGACY_COMMIT, "wrong_legacy_basis")
    require(basis.get("transaction_read_only") == "on" and basis.get("transaction_isolation") == "repeatable read",
            "source_not_readonly_snapshot")
    cutoff = integer(basis["source_cutoff"], 1, 253402300799)
    for key in ("source_dump_sha256", "source_receipt_sha256", "original_manifest_sha256"):
        checked_hash(basis.get(key))
    require(evidence.get("format_version") == 1 and type(evidence.get("format_version")) is int
            and evidence.get("scope") == "original-trust-evidence-v1"
            and evidence.get("projection") == SOURCE_PROJECTION, "source_evidence_contract_invalid")
    for key in ("source_commit", "source_dump_sha256", "source_receipt_sha256", "original_manifest_sha256", "source_cutoff"):
        require(evidence.get(key) == basis[key], "source_evidence_binding_mismatch")
    require(evidence.get("basis_sha256") == hashes["basis"]
            and evidence.get("identity_mapping_sha256") == hashes["identity_mapping"], "source_evidence_binding_mismatch")
    require(checked_hash(evidence.get("independent_report_sha256")) == hashes["source_review_report"],
            "source_review_report_hash_mismatch")
    report = inputs["source_review_report"]
    require(type(report) is dict and type(report.get("format_version")) is int and report["format_version"] == 1
            and report.get("scope") == "original-net-projector-report-v1" and report.get("projection") == SOURCE_PROJECTION
            and report.get("transaction_read_only") == "on" and report.get("transaction_isolation") == "repeatable read",
            "source_review_report_contract_invalid")
    require(report.get("projector_source_hashes") == PROJECTOR_SOURCE_HASHES, "original_projector_source_mismatch")
    for key in ("source_commit", "source_dump_sha256", "source_receipt_sha256", "original_manifest_sha256", "source_cutoff"):
        require(report.get(key) == basis[key], "source_review_report_binding_mismatch")
    require(report.get("basis_sha256") == hashes["basis"] and report.get("identity_mapping_sha256") == hashes["identity_mapping"],
            "source_review_report_binding_mismatch")
    require(type(evidence.get("credits_per_usd")) is int and evidence["credits_per_usd"] == 500000,
            "original_credit_unit_unproven")
    currencies = evidence.get("original_currencies")
    require(type(currencies) is list
            and all(type(v) is str and len(v) <= 16 and "\0" not in v for v in currencies)
            and currencies == sorted(set(currencies)), "original_currency_set_invalid")
    users = basis.get("users")
    require(type(users) is list and users and type(mapping) is dict, "missing_users")
    ids = sorted(integer(user["source_user_id"], 1) for user in users)
    require(ids == sorted(set(ids)) and set(mapping) == set(map(str, ids)), "incomplete_or_duplicate_user_map")
    require(all(type(mapping[str(uid)]) is int and mapping[str(uid)] == uid for uid in ids), "identity_same_id_required")
    require(evidence.get("complete_user_ids") == ids, "incomplete_original_user_coverage")
    records = identities.get("records")
    require(type(records) is list and [record["user_id"] for record in records] == ids, "incomplete_identity_mapping")
    require(sha(I.canonical({"source_sha256": basis["source_dump_sha256"], "records": records}).encode()) == identities.get("mapping_sha256"),
            "identity_mapping_digest_mismatch")
    facts = evidence.get("users")
    require(type(facts) is list and sorted(integer(row["source_user_id"], 1) for row in facts) == ids,
            "incomplete_original_quote_evidence")
    require(canonical(report.get("users")) == canonical(facts) and report.get("complete_user_ids") == ids,
            "source_review_report_user_mismatch")
    facts = {row["source_user_id"]: row for row in facts}
    order_facts = evidence.get("orders")
    require(type(order_facts) is list, "missing_order_evidence")
    order_ids = [integer(row["source_order_id"], 1) for row in order_facts]
    require(len(order_ids) == len(set(order_ids)), "duplicate_order_evidence")
    order_facts = dict(zip(order_ids, order_facts))
    audited = report.get("orders")
    require(type(audited) is list and sorted(integer(row["source_order_id"], 1) for row in audited) == sorted(order_ids),
            "source_review_report_order_coverage_mismatch")
    audited = {row["source_order_id"]: row for row in audited}
    spec = policy_spec(basis["legacy_policy"])
    rows, orders, quotes, seen = [], [], [], set()
    identity_index = {row["user_id"]: row for row in records}
    for original in sorted(users, key=lambda u: u["source_user_id"]):
        user = dict(original)
        uid = user["source_user_id"]
        fact = facts[uid]
        role = identity_index[uid]["platform_role"]
        require(type(fact.get("source_role")) is int and {1: "user", 10: "admin", 100: "superadmin"}.get(fact["source_role"]) == role,
                "original_role_mismatch")
        for key in ("original_created_at", "console_activated_at", "last_api_activity_at"):
            integer(user[key], 0, cutoff)
        override = user["override_level"]
        require(override is None or type(override) is int, "invalid_override")
        # Preserve the original getter's defined corrupt-override -> L0 behavior.
        user["override_level"] = override if override is None or 0 <= override <= 4 else 0
        require(type(user.get("projection_known")) is bool, "missing_projection_known")
        require(type(user.get("orders")) is list and len(user["orders"]) <= 10000, "invalid_order_set")
        total = 0
        for order in user["orders"]:
            oid = integer(order["source_order_id"], 1)
            require(oid not in seen, "duplicate_source_order")
            seen.add(oid)
            net = integer(order["net_credits"])
            total = integer(total + net)
            integer(order["paid_at"], 1, cutoff)
            proof = order_facts.get(oid)
            require(type(proof) is dict and type(proof.get("source_user_id")) is int and proof.get("source_user_id") == uid
                    and type(proof.get("net_credits")) is int and proof.get("net_credits") == net
                    and type(proof.get("paid_at")) is int and proof.get("paid_at") == order["paid_at"], "missing_or_mismatched_order_proof")
            require(proof.get("source_kind") == "successful_external_paid_topup"
                    and proof.get("projection") == SOURCE_PROJECTION, "unknown_order_source")
            require(proof.get("original_currency") in currencies, "unknown_original_currency")
            amount = proof.get("original_amount")
            require(type(amount) is str and re.fullmatch(r"[0-9]+(?:\.[0-9]+)?", amount) is not None
                    and Decimal(amount) > 0, "original_amount_unproven")
            credited = integer(proof.get("credited_credits"))
            refunded = integer(proof.get("refunded_credits"))
            require(credited > 0 and refunded <= credited and credited - refunded == net,
                    "original_net_reconciliation_mismatch")
            validate_order_source(proof)
            audit = audited[oid]
            require(digest(audit) == checked_hash(proof.get("net_evidence_sha256")), "net_audit_record_changed")
            require(all(audit.get(key) == proof[key] for key in
                        ("source_order_id", "source_user_id", "credited_credits", "refunded_credits", "net_credits", "paid_at")),
                    "net_audit_record_mismatch")
            source_rows = audit.get("source_rows")
            require(type(source_rows) is list and source_rows and all(type(row) is dict
                    and type(row.get("table")) is str and re.fullmatch(r"[a-z_][a-z_0-9]*", row["table"])
                    and type(row.get("row")) is dict for row in source_rows), "net_audit_rows_missing")
            require(any(row["table"] == "top_ups" and canonical(row["row"]) == canonical(proof["original_row"])
                        for row in source_rows), "net_audit_original_order_missing")
            mode = audit.get("projection_mode")
            require(mode in {"unrebased", "fully_refunded", "audited_rebase", "audited_pending", "post_cutoff_native"},
                    "net_audit_projection_mode_unknown")
            row = proof["original_row"]
            gross = row["credited_quota"] if row["credited_quota"] > 0 else row["amount"] if row.get("payment_provider") == "creem" or row.get("payment_method") == "creem" else row["amount"] * 500000
            original_refund = integer(row.get("refunded_quota"))
            if mode in {"unrebased", "fully_refunded"}:
                require(proof["credited_credits"] == gross and proof["refunded_credits"] == original_refund,
                        "unrebased_credit_unit_mismatch")
                require(mode != "fully_refunded" or original_refund == gross, "full_refund_evidence_mismatch")
                integer(audit.get("rebase_cutoff"), 0, cutoff)
                require(audit.get("rebase_cutoff") == 0 or mode == "fully_refunded", "unrebased_global_cutoff_mismatch")
            else:
                integer(audit.get("rebase_cutoff"), 1, cutoff)
                tables = {row["table"] for row in source_rows}
                require("wallet_credit_rebases" in tables and (mode != "audited_rebase" or "wallet_topup_credit_rebases" in tables),
                        "rebase_audit_source_rows_missing")
                validate_audited_lineage(audit, proof, cutoff)
            require(canonical(audit.get("projector_result")) == canonical({"credited_credits": credited,
                    "refunded_credits": refunded, "net_credits": net}), "original_net_projector_result_mismatch")
            orders.append({"source_order_id": oid, "user_id": uid, "net_credits": net, "paid_at": order["paid_at"],
                           "revision": 1, "source_digest": digest({"order": order, "evidence": proof})})
        require(digest(fact.get("original_quote")) == checked_hash(fact.get("original_quote_sha256")), "original_quote_digest_mismatch")
        quote = effective_quote(spec, user, role, cutoff)
        require(canonical(fact.get("original_quote")) == canonical(quote), "original_price_rights_mismatch")
        quotes.append({"user_id": uid, "original_override": override, "normalized_override": user["override_level"], **quote})
        rows.append({"user_id": uid, **{key: user[key] for key in ("original_created_at", "console_activated_at",
                     "last_api_activity_at", "override_level", "projection_known")}, "order_count": len(user["orders"]),
                     "revision": 1, "source_digest": digest({"user": original, "evidence": fact})})
    require(seen == set(order_facts), "incomplete_or_extra_order_evidence")
    floor = integer(max(ids) + 1, 1)
    return {"policy_spec": spec, "users": rows, "orders": sorted(orders, key=lambda r: r["source_order_id"]),
            "quotes_at_source_cutoff": quotes, "native_user_floor": floor, "source_cutoff": cutoff}


def snapshot_sql(inventory, migration_id):
    F.validate_inventory(inventory)
    require(type(migration_id) is str and I.MIGRATION_ID.fullmatch(migration_id), "migration_id_invalid")
    # A native policy row has a dynamic timestamp and cutoff; capture exact bytes
    # in this one transaction, then bind them to the approval and both readbacks.
    query = F.snapshot_sql(inventory, migration_id)
    return query.replace("'native_users',", "'identities',(SELECT coalesce(jsonb_agg(jsonb_build_object('id',u.id,'personal_account_id',u.personal_account_id," +
        "'platform_role',u.platform_role,'active',u.active,'auth_version',u.auth_version,'account_active',a.active) ORDER BY u.id),'[]'::jsonb) " +
        "FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id),'native_users',", 1)


def validate_snapshot(snapshot, inputs, derived, migration_id, runtime_roles):
    contract = source_contract()
    target = {key: snapshot[key] for key in ("instance", "catalog_fingerprint_sha256", "ownership_sha256", "component_fingerprints")}
    target["source_contract"] = contract
    inst = target["instance"]
    for key in ("role", "session_user", "database", "database_owner"):
        require(type(inst.get(key)) is str and I.IDENTIFIER.fullmatch(inst[key]), "target_role_invalid")
    require(inst["role"] == inst["database_owner"] != inst["session_user"]
            and inst.get("effective_role_can_login") is False and inst.get("tcp_connection") is False
            and inst.get("listen_addresses") == "", "target_not_offline_owner")
    require(str(inst.get("server_version_num", "")).isdigit() and 180000 <= int(inst["server_version_num"]) < 190000,
            "isolated_pg18_required")
    for key in ("database_oid", "port", "system_identifier"):
        require(str(inst.get(key, "")).isdigit() and int(inst[key]) > 0, "target_instance_incomplete")
    require(type(inst.get("postmaster_started_at")) is str, "target_instance_incomplete")
    datetime.fromisoformat(inst["postmaster_started_at"])
    require(type(inst.get("server_instance_nonce")) is str and 0 < len(inst["server_instance_nonce"]) <= 256
            and "\0" not in inst["server_instance_nonce"], "target_instance_incomplete")
    require(type(runtime_roles) is list and runtime_roles and len(runtime_roles) == len(set(runtime_roles))
            and all(type(role) is str and I.IDENTIFIER.fullmatch(role) for role in runtime_roles)
            and not {inst["role"], inst["session_user"]}.intersection(runtime_roles), "runtime_roles_invalid")
    for key in ("catalog_fingerprint_sha256", "ownership_sha256"):
        checked_hash(target[key])
    require(target["component_fingerprints"] == contract["component_fingerprints"], "target_component_contract_mismatch")
    state = snapshot["state"]
    F.validate_inventory(state["inventory"])
    require(set(state["tables"]) == set(state["inventory"]["tables"])
            and set(state["sequences"]) == set(state["inventory"]["sequences"]), "snapshot_inventory_mismatch")
    for value in state["tables"].values():
        integer(value["rows"])
        checked_hash(value["sha256"])
    for table in CHANGED[1:] + ("core_meta.schema_upgrades", "core_meta.component_upgrades"):
        require(table in state["tables"] and state["tables"][table]["rows"] == 0, "fresh_trust_or_upgrade_receipts_not_empty")
    require(state["tables"][CHANGED[0]]["rows"] == 1, "trust_policy_seed_invalid")
    I.validate_trust_policy_seed(snapshot["trust_policy_seed"])
    records = inputs["identity_mapping"]["records"]
    expected = [{"id": r["user_id"], "personal_account_id": r["account_id"], "platform_role": r["platform_role"],
                 "active": r["active"], "auth_version": r["auth_version"], "account_active": r["active"]} for r in records]
    require(snapshot["identities"] == expected, "target_identity_coverage_mismatch")
    runs = snapshot.get("import_runs")
    require(type(runs) is list and not any(row.get("migration_id") == migration_id for row in runs),
            "original_attempt_requires_resolution")
    require(all(row.get("source_archive_sha256") == inputs["basis"]["source_dump_sha256"] for row in runs), "target_other_source_receipt")
    identity_runs = [r for r in runs if r.get("manifest", {}).get("scope") == "identity-only"]
    require(len(identity_runs) == 1, "identity_import_missing_or_ambiguous")
    allowed = {"identity-only", "console-only", F.SCOPE}
    require(all(row.get("manifest", {}).get("scope") in allowed for row in runs), "unknown_or_previous_import_receipt")
    I.validate_import_material(inputs["identity_mapping"], inputs["basis"]["source_dump_sha256"], identity_runs[0]["manifest"])
    require(identity_runs[0]["mapping_sha256"] == inputs["identity_mapping"]["mapping_sha256"], "identity_receipt_mapping_mismatch")
    identity_file_hash = sha((I.canonical(inputs["identity_mapping"]) + "\n").encode())
    require(inputs["evidence"]["identity_mapping_sha256"] == identity_file_hash
            and identity_runs[0]["manifest"].get("mapping_file_sha256", identity_file_hash) == identity_file_hash,
            "identity_original_mapping_file_mismatch")
    return target


def prepare_material(inputs, hashes, *, migration_id, runtime_roles):
    require(type(migration_id) is str and I.MIGRATION_ID.fullmatch(migration_id), "migration_id_invalid")
    require(set(inputs) == set(INPUTS) and set(hashes) == set(INPUTS), "input_set_invalid")
    for value in hashes.values():
        checked_hash(value)
    derived = derive_inputs(inputs, hashes)
    target = validate_snapshot(inputs["target_snapshot"], inputs, derived, migration_id, runtime_roles)
    return json.loads(canonical({"format_version": 2, "scope": SCOPE, "migration_id": migration_id,
        "application_name": "lmm-trust-" + digest({"migration_id": migration_id, "hashes": hashes})[:32],
        "source_archive_sha256": inputs["basis"]["source_dump_sha256"], "input_hashes": hashes,
        "inputs": inputs, "derived": derived, "target": target, "runtime_roles": runtime_roles,
        "before_state": inputs["target_snapshot"]["state"], "tool_source_hashes": tool_hashes()}))


def validate_material(material):
    require(type(material) is dict and type(material.get("format_version")) is int
            and material["format_version"] == 2 and material.get("scope") == SCOPE, "trust_material_contract_invalid")
    require(material.get("tool_source_hashes") == tool_hashes(), "trust_tool_source_changed")
    expected = prepare_material(material["inputs"], material["input_hashes"],
                                migration_id=material["migration_id"], runtime_roles=material["runtime_roles"])
    require(material == expected, "trust_material_changed")


def protected_sql(material):
    # The receipt excludes only this attempt. Its own exact contents are checked
    # separately. Preserve every other table, all previous receipts and ALL
    # sequence values and definitions, including finance writes made before us.
    state = F.state_sql(material["before_state"]["inventory"], material["migration_id"])
    names = "ARRAY[" + ",".join(F.literal(name) for name in CHANGED) + "]"
    return f"SELECT jsonb_set(s,'{{tables}}',(s->'tables')-{names}) FROM ({state}) value(s)"


def expected_rows_sql(material):
    d = material["derived"]
    users = F.literal(d["users"])
    orders = F.literal(d["orders"])
    return f"""
 IF (SELECT coalesce(jsonb_agg(to_jsonb(t)-'source_digest'||jsonb_build_object('source_digest',encode(source_digest,'hex')) ORDER BY user_id),'[]'::jsonb) FROM core_trust_pricing.user_bases t) IS DISTINCT FROM {users}::jsonb
 OR (SELECT coalesce(jsonb_agg(to_jsonb(t)-'source_digest'||jsonb_build_object('source_digest',encode(source_digest,'hex')) ORDER BY source_order_id),'[]'::jsonb) FROM core_trust_pricing.legacy_paid_orders t) IS DISTINCT FROM {orders}::jsonb
 THEN RAISE EXCEPTION 'trust_basis_or_order_readback_conflict'; END IF;
"""


def candidate_sql(material):
    validate_material(material)
    snapshot = material["inputs"]["target_snapshot"]
    d = material["derived"]
    mid = F.literal(material["migration_id"])
    approval = APPROVAL_SLOT
    receipt = {"format_version": 2, "scope": SCOPE, "material_sha256": digest(material),
               "input_hashes": material["input_hashes"], "tool_source_hashes": material["tool_source_hashes"],
               "source_contract": material["target"]["source_contract"], "source_evidence": material["inputs"]["evidence"],
               "imported_rights": d, "approval_sha256": approval,
               "candidate_contract": "sha256-of-exact-unsealed-owner-candidate-sql-v1"}
    original = dict(snapshot["trust_policy_seed"])
    expected_policy = {**original, "revision": original["revision"] + 1, "spec": d["policy_spec"],
                       "source_digest": material["input_hashes"]["basis"], "source_cutoff": d["source_cutoff"],
                       "native_user_floor": d["native_user_floor"]}
    protected = dict(material["before_state"])
    protected["tables"] = {k: v for k, v in protected["tables"].items() if k not in CHANGED}
    sql = f"""-- Candidate: the owner approval slot MUST be sealed after separate review.
-- Never execute a new attempt after UNKNOWN. Use this attempt's resolve.sql.
\\set ON_ERROR_STOP on
SET application_name={F.literal(material['application_name'])};
SET standard_conforming_strings=on;
SET timezone='UTC';
SET lock_timeout='5s';
SET statement_timeout='120s';
DO $pre$ BEGIN
 IF '{approval}' !~ '^[a-f0-9]{{64}}$' THEN RAISE EXCEPTION 'owner_approval_not_sealed'; END IF;
 IF NOT pg_try_advisory_lock(741803121) THEN RAISE EXCEPTION 'offline_import_busy'; END IF;
END $pre$;
BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL synchronous_commit=on;
SET LOCAL row_security=off;
SET LOCAL search_path=pg_catalog,public;
SELECT pg_advisory_xact_lock(741803219);
DO $body$
DECLARE r RECORD; previous core_migration.import_runs%ROWTYPE; before_protected JSONB;
BEGIN
{F._guard(material)}
 FOR r IN SELECT n.nspname,c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE {F.TABLE_FILTER} AND c.relkind IN ('r','p') ORDER BY n.nspname,c.relname LOOP
  EXECUTE format('LOCK TABLE %I.%I IN ACCESS EXCLUSIVE MODE',r.nspname,r.relname);
 END LOOP;
 IF {I.upgrade_receipts_guard_sql()} THEN RAISE EXCEPTION 'trust_upgrade_receipts_not_empty'; END IF;
 IF ({protected_sql(material)}) IS DISTINCT FROM {F.literal(protected)}::jsonb THEN
  RAISE EXCEPTION 'trust_protected_state_changed'; END IF;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id={mid};
 IF FOUND THEN
  IF previous.source_archive_sha256<>{F.literal(material['source_archive_sha256'])}
   OR previous.mapping_sha256<>{F.literal(digest(d))} OR previous.manifest IS DISTINCT FROM {F.literal(receipt)}::jsonb THEN
   RAISE EXCEPTION 'trust_original_receipt_conflict'; END IF;
 ELSE
  IF EXISTS(SELECT 1 FROM core_migration.import_runs WHERE manifest->>'scope'={F.literal(SCOPE)}) THEN
   RAISE EXCEPTION 'trust_other_attempt_requires_resolution'; END IF;
  IF ({F.state_sql(material['before_state']['inventory'], material['migration_id'])}) IS DISTINCT FROM {F.literal(material['before_state'])}::jsonb
   OR ({I.trust_policy_seed_query()}) IS DISTINCT FROM {F.literal(original)}::jsonb THEN
   RAISE EXCEPTION 'trust_exact_prestate_changed'; END IF;
  INSERT INTO core_trust_pricing.user_bases(user_id,original_created_at,console_activated_at,last_api_activity_at,
    override_level,projection_known,order_count,revision,source_digest)
  SELECT user_id,original_created_at,console_activated_at,last_api_activity_at,override_level,projection_known,order_count,revision,decode(source_digest,'hex')
  FROM jsonb_to_recordset({F.literal(d['users'])}::jsonb) AS t(user_id bigint,original_created_at bigint,
    console_activated_at bigint,last_api_activity_at bigint,override_level smallint,projection_known boolean,
    order_count bigint,revision bigint,source_digest text);
  INSERT INTO core_trust_pricing.legacy_paid_orders(source_order_id,user_id,net_credits,paid_at,revision,source_digest)
  SELECT source_order_id,user_id,net_credits,paid_at,revision,decode(source_digest,'hex')
  FROM jsonb_to_recordset({F.literal(d['orders'])}::jsonb) AS t(source_order_id bigint,user_id bigint,
    net_credits bigint,paid_at bigint,revision bigint,source_digest text);
  UPDATE core_trust_pricing.policies SET revision={expected_policy['revision']},spec={F.literal(d['policy_spec'])}::jsonb,
    source_digest=decode({F.literal(material['input_hashes']['basis'])},'hex'),source_cutoff={d['source_cutoff']},
    native_user_floor={d['native_user_floor']} WHERE singleton;
 END IF;
{expected_rows_sql(material)}
 IF ({I.trust_policy_seed_query()}) IS DISTINCT FROM {F.literal(expected_policy)}::jsonb THEN
  RAISE EXCEPTION 'trust_policy_or_native_fence_readback_conflict'; END IF;
 IF ({protected_sql(material)}) IS DISTINCT FROM {F.literal(protected)}::jsonb THEN
  RAISE EXCEPTION 'trust_nontrust_table_or_sequence_changed'; END IF;
{F._guard(material)}
 IF previous.migration_id IS NULL THEN
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
  VALUES({mid},{F.literal(material['source_archive_sha256'])},{F.literal(digest(d))},{F.literal(receipt)}::jsonb);
 END IF;
END $body$;
COMMIT;
SELECT jsonb_build_object('status','COMMITTED','migration_id',{mid},'material_sha256',{F.literal(digest(material))},'approval_sha256','{approval}');
SELECT pg_advisory_unlock(741803121);
"""
    # Source strings can contain SQL-looking dollar tags; delimiters are unique
    # over the full payload and are never treated as caller-provided SQL.
    tag = "$lmm_trust_" + sha(sql.encode())[:24] + "$"
    require(tag not in sql, "sql_delimiter_collision")
    sql = sql.replace("DO $body$", "DO " + tag, 1)
    head, found, tail = sql.rpartition("END $body$;")
    require(bool(found), "sql_template_invalid")
    return head + "END " + tag + ";" + tail


def verify_approval(material, candidate, approval):
    require(type(approval) is dict and set(approval) == {"format_version", "decision", "scope", "migration_id",
        "material_sha256", "candidate_sql_sha256", "input_hashes", "owner_review_evidence_sha256"}, "owner_approval_contract_invalid")
    checked_hash(approval.get("owner_review_evidence_sha256"))
    require(type(approval.get("format_version")) is int and approval["format_version"] == 2
        and approval.get("decision") == "approve-offline-trust-basis"
        and approval.get("scope") == SCOPE and approval.get("migration_id") == material["migration_id"]
        and approval.get("material_sha256") == digest(material)
        and approval.get("candidate_sql_sha256") == sha(candidate.encode())
        and approval.get("input_hashes") == material["input_hashes"], "owner_approval_binding_mismatch")


def seal_sql(material, candidate, approval_bytes, expected_approval_sha256):
    validate_material(material)
    require(candidate == candidate_sql(material), "candidate_sql_changed")
    require(sha(approval_bytes) == checked_hash(expected_approval_sha256), "approval_file_changed")
    approval = decode(approval_bytes)
    verify_approval(material, candidate, approval)
    require(all(APPROVAL_SLOT not in canonical(material[key]) for key in material), "approval_slot_collision")
    return candidate.replace(APPROVAL_SLOT, expected_approval_sha256)


def resolve_sql(material, approval_sha256):
    validate_material(material)
    checked_hash(approval_sha256)
    # No advisory lock wait and no mutations. An active original backend keeps
    # the result UNKNOWN; absence alone is never sufficient to authorize retry.
    return f"""\\set ON_ERROR_STOP on
BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL timezone='UTC';
SET LOCAL row_security=off;
SELECT jsonb_build_object('status',CASE
 WHEN ({F.instance_sql()}) IS DISTINCT FROM {F.literal(material['target']['instance'])}::jsonb THEN 'UNKNOWN_TARGET_INSTANCE_MISMATCH'
 WHEN EXISTS(SELECT 1 FROM pg_stat_activity WHERE datid=(SELECT oid FROM pg_database WHERE datname=current_database())
  AND application_name={F.literal(material['application_name'])} AND pid<>pg_backend_pid()) THEN 'UNKNOWN_ACTIVE_ORIGINAL'
 WHEN EXISTS(SELECT 1 FROM core_migration.import_runs WHERE migration_id={F.literal(material['migration_id'])}
  AND manifest->>'material_sha256'={F.literal(digest(material))} AND manifest->>'approval_sha256'={F.literal(approval_sha256)}) THEN 'RECEIPT_FOUND_VERIFY_EXACT_STATE'
 ELSE 'UNKNOWN_NO_MATCHING_RECEIPT_REQUIRES_OWNER_RECONCILIATION' END,
 'migration_id',{F.literal(material['migration_id'])},'instance',({F.instance_sql()}),
 'expected_instance',{F.literal(material['target']['instance'])}::jsonb,
 'original_receipt',(SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE migration_id={F.literal(material['migration_id'])}),
 'state',({F.state_sql(material['before_state']['inventory'], material['migration_id'])}));
ROLLBACK;
"""


def fresh_output(path):
    require(path.is_absolute() and not path.exists(), "fresh_private_output_required")
    path.mkdir(mode=0o700)


def write(path, value):
    with path.open("x", encoding="utf-8") as handle:
        os.chmod(path, 0o600)
        handle.write(value)


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    inventory = sub.add_parser("inventory", help="emit SELECT-only inventory SQL")
    inventory.add_argument("--output", required=True, type=Path)
    snap = sub.add_parser("snapshot", help="emit SELECT-only target snapshot SQL")
    snap.add_argument("--inventory", required=True, type=Path)
    snap.add_argument("--migration-id", required=True)
    snap.add_argument("--output", required=True, type=Path)
    prep = sub.add_parser("prepare", help="prepare pending owner review; no approved SQL")
    for key in INPUTS:
        prep.add_argument("--" + key.replace("_", "-"), required=True, type=Path)
        prep.add_argument("--" + key.replace("_", "-") + "-sha256", required=True)
    prep.add_argument("--migration-id", required=True)
    prep.add_argument("--runtime-role", action="append", required=True)
    prep.add_argument("--output", required=True, type=Path)
    seal = sub.add_parser("seal", help="bind independently supplied owner approval; never execute")
    seal.add_argument("--prepared", required=True, type=Path)
    seal.add_argument("--approval", required=True, type=Path)
    seal.add_argument("--approval-sha256", required=True)
    seal.add_argument("--owner-review-evidence", required=True, type=Path)
    seal.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.command == "inventory":
        write(args.output, "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n" + F.inventory_sql() + ";\nROLLBACK;\n")
    elif args.command == "snapshot":
        write(args.output, snapshot_sql(decode(args.inventory.read_bytes()), args.migration_id))
    elif args.command == "prepare":
        inputs, hashes, paths = {}, {}, {}
        for key in INPUTS:
            path = getattr(args, key).resolve()
            raw = path.read_bytes()
            expected = checked_hash(getattr(args, key + "_sha256"))
            require(sha(raw) == expected, "input_file_changed")
            inputs[key], hashes[key], paths[key] = decode(raw), expected, str(path)
        material = prepare_material(inputs, hashes, migration_id=args.migration_id, runtime_roles=args.runtime_role)
        candidate = candidate_sql(material)
        fresh_output(args.output)
        write(args.output / "material.private.json", canonical(material) + "\n")
        write(args.output / "input-paths.private.json", canonical(paths) + "\n")
        write(args.output / "owner-candidate.private.sql", candidate)
        request = {"status": "PENDING_OWNER_REVIEW", "scope": SCOPE, "migration_id": material["migration_id"],
                   "material_sha256": digest(material), "candidate_sql_sha256": sha(candidate.encode()), "input_hashes": hashes,
                   "required_approval_fields": ["format_version=2", "decision=approve-offline-trust-basis", "scope", "migration_id",
                    "material_sha256", "candidate_sql_sha256", "input_hashes", "owner_review_evidence_sha256"],
                   "preview": material["derived"], "native_since_preserved": inputs["target_snapshot"]["trust_policy_seed"]["native_since"],
                   "all_other_tables_and_sequences": "must remain byte-exact", "applied": False}
        write(args.output / "review-request.private.json", canonical(request) + "\n")
        print(canonical({"status": "PENDING_OWNER_REVIEW", "users": len(material["derived"]["users"]), "orders": len(material["derived"]["orders"]), "applied": False}))
    else:
        material = decode((args.prepared / "material.private.json").read_bytes())
        candidate = (args.prepared / "owner-candidate.private.sql").read_text()
        paths = decode((args.prepared / "input-paths.private.json").read_bytes())
        require(set(paths) == set(INPUTS), "input_set_invalid")
        for key in INPUTS:
            raw = Path(paths[key]).read_bytes()
            require(sha(raw) == material["input_hashes"][key], "input_file_changed")
            require(canonical(decode(raw)) == canonical(material["inputs"][key]), "material_input_bytes_mismatch")
        approval_bytes = args.approval.read_bytes()
        approval = decode(approval_bytes)
        require(sha(args.owner_review_evidence.read_bytes()) == approval.get("owner_review_evidence_sha256"), "owner_review_evidence_changed")
        sql = seal_sql(material, candidate, approval_bytes, args.approval_sha256)
        fresh_output(args.output)
        write(args.output / "owner-import.private.sql", sql)
        write(args.output / "resolve.private.sql", resolve_sql(material, args.approval_sha256))
        write(args.output / "attempt.private.json", canonical({"status": "PREPARED_NOT_SUBMITTED", "migration_id": material["migration_id"],
            "application_name": material["application_name"], "material_sha256": digest(material), "approval_sha256": args.approval_sha256,
            "sql_sha256": sha(sql.encode()), "unknown_result_policy": "retain this directory; inspect original receipt with resolve.private.sql; never change ID or assume rollback"}) + "\n")
        print(canonical({"status": "PREPARED_NOT_SUBMITTED", "applied": False}))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (Error, KeyError, TypeError, ValueError, OSError) as exc:
        print("trust import refused: " + str(exc), file=sys.stderr)
        raise SystemExit(1)
