"""Synthetic provenance and SQL regressions; pure tests are not PG acceptance."""
from copy import deepcopy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import test_financial_import as FA
import test_migrate_legacy_snapshot as A2

SPEC = importlib.util.spec_from_file_location("trust_import", Path(__file__).with_name("prepare-trust-pricing-import.py"))
T = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(T)


def source_fixture(source=None):
    source = source or FA.source_fixture()
    identities = source["identity_plan"]
    ids = [row["user_id"] for row in identities["records"]]
    policy = {"version": 1, "paid_activation_enabled": True, "decay_period_days": 90,
              "tiers": [{"level": i, "min_paid_credits": v, "discount_ratio": r} for i, (v, r) in enumerate(
                  [(0, "1"), (500000, "1"), (5000000, "0.97"), (25000000, "0.94"), (100000000, "0.90")])],
              "role_tiers": [{"level": 5, "role": 10, "discount_ratio": "0.90"},
                             {"level": 6, "role": 100, "discount_ratio": "0.90"}]}
    basis = {"version": 1, "source_commit": T.LEGACY_COMMIT,
             "source_dump_sha256": source["identity_run"]["source_archive_sha256"],
             "source_receipt_sha256": "a" * 64, "original_manifest_sha256": "b" * 64,
             "source_cutoff": 1792627200, "transaction_read_only": "on", "transaction_isolation": "repeatable read",
             "legacy_policy": policy, "users": []}
    for uid in ids:
        basis["users"].append({"source_user_id": uid, "original_created_at": 1792500000,
              "console_activated_at": 0, "last_api_activity_at": 1792500000, "override_level": None,
              "projection_known": True, "orders": [{"source_order_id": 710, "net_credits": 6000000,
                                                     "paid_at": 1792500000}] if uid == ids[0] else []})
    raw = {"id": 710, "user_id": ids[0], "payment_provider": "stripe", "payment_method": "stripe",
           "status": "success", "settled_amount_micros": 12000000, "expected_amount_micros": 12000000,
           "settlement_currency": "JPY", "money": "12", "create_time": 1792400000,
           "complete_time": 1792500000, "credited_quota": 6000000, "refunded_quota": 0, "amount": 12}
    evidence = {"format_version": 1, "scope": "original-trust-evidence-v1", "projection": T.SOURCE_PROJECTION,
        **{k: basis[k] for k in ("source_commit", "source_dump_sha256", "source_receipt_sha256", "original_manifest_sha256", "source_cutoff")},
        "basis_sha256": T.digest(basis), "identity_mapping_sha256": T.sha((T.I.canonical(identities) + "\n").encode()), "independent_report_sha256": "c" * 64,
        "credits_per_usd": 500000, "original_currencies": ["JPY"], "complete_user_ids": ids, "users": [],
        "orders": [{"source_order_id": 710, "source_user_id": ids[0], "net_credits": 6000000, "paid_at": 1792500000,
                    "source_kind": "successful_external_paid_topup", "projection": T.SOURCE_PROJECTION,
                    "original_currency": "JPY", "original_amount": "12", "credited_credits": 6000000,
                    "refunded_credits": 0, "original_row": raw, "original_row_sha256": T.digest(raw), "net_evidence_sha256": "d" * 64}]}
    spec = T.policy_spec(policy)
    for user in basis["users"]:
        quote = T.effective_quote(spec, user, "user", basis["source_cutoff"])
        evidence["users"].append({"source_user_id": user["source_user_id"], "source_role": 1,
                                  "original_quote": quote, "original_quote_sha256": T.digest(quote)})
    inputs = {"basis": basis, "evidence": evidence, "identity_mapping": identities,
              "user_map": {str(uid): uid for uid in ids}}
    bind_report(inputs)
    return inputs, source



def bind_report(inputs):
    """Rebuild independently collected SYNTHETIC source report after test edits."""
    basis, evidence = inputs["basis"], inputs["evidence"]
    evidence["basis_sha256"] = T.digest(basis)
    evidence["identity_mapping_sha256"] = T.sha((T.I.canonical(inputs["identity_mapping"]) + "\n").encode())
    report = {"format_version": 1, "scope": "original-net-projector-report-v1", "projection": T.SOURCE_PROJECTION,
        **{key: basis[key] for key in ("source_commit", "source_dump_sha256", "source_receipt_sha256", "original_manifest_sha256",
             "source_cutoff", "transaction_read_only", "transaction_isolation")},
        "basis_sha256": evidence["basis_sha256"], "identity_mapping_sha256": evidence["identity_mapping_sha256"],
        "complete_user_ids": evidence["complete_user_ids"], "users": deepcopy(evidence["users"]), "orders": [],
        "projector_source_hashes": deepcopy(T.PROJECTOR_SOURCE_HASHES)}
    for proof in evidence["orders"]:
        audit = {key: proof[key] for key in ("source_order_id", "source_user_id", "credited_credits", "refunded_credits", "net_credits", "paid_at")}
        audit["source_rows"] = [{"table": "top_ups", "row": deepcopy(proof["original_row"])}]
        audit["projection_mode"] = "fully_refunded" if proof["net_credits"] == 0 else "unrebased"
        audit["rebase_cutoff"] = 0
        audit["projector_result"] = {key: proof[key] for key in ("credited_credits", "refunded_credits", "net_credits")}
        proof["net_evidence_sha256"] = T.digest(audit)
        report["orders"].append(audit)
    inputs["source_review_report"] = report
    evidence["independent_report_sha256"] = T.digest(report)
    return input_hashes(inputs)

def fixture():
    inputs, source = source_fixture()
    snapshot = FA.snapshot_fixture(source)
    snapshot["instance"]["server_instance_nonce"] = "f" * 64
    snapshot["state"]["tables"]["core_trust_pricing.policies"]["rows"] = 1
    snapshot["identities"] = [{"id": r["user_id"], "personal_account_id": r["account_id"],
                    "platform_role": r["platform_role"], "active": r["active"], "auth_version": r["auth_version"],
                    "account_active": r["active"]} for r in inputs["identity_mapping"]["records"]]
    inputs["target_snapshot"] = snapshot
    return inputs, input_hashes(inputs)


def historical_order(inputs, currency, provider="stripe", money=1):
    """The original cc52 unrebased order can prove credits without a currency."""
    proof = inputs["evidence"]["orders"][0]
    proof.update(original_currency=currency, original_amount=str(money), credited_credits=500000,
                 refunded_credits=0, net_credits=500000)
    proof["original_row"].update(payment_provider=provider, payment_method=provider,
        credited_quota=500000, amount=1, money=money, settled_amount_micros=0,
        expected_amount_micros=0, settlement_currency=currency)
    proof["original_row_sha256"] = T.digest(proof["original_row"])
    inputs["evidence"]["original_currencies"] = [currency]
    user = inputs["basis"]["users"][0]
    user["orders"][0]["net_credits"] = 500000
    fact = inputs["evidence"]["users"][0]
    fact["original_quote"] = T.effective_quote(T.policy_spec(inputs["basis"]["legacy_policy"]),
        user, "user", inputs["basis"]["source_cutoff"])
    fact["original_quote_sha256"] = T.digest(fact["original_quote"])
    return bind_report(inputs)


def input_bytes(key, value):
    return ((T.I.canonical(value) + "\n") if key == "identity_mapping" else T.canonical(value)).encode()


def input_hashes(inputs):
    return {key: T.sha(input_bytes(key, value)) for key, value in inputs.items()}


def material_fixture():
    inputs, hashes = fixture()
    return T.prepare_material(inputs, hashes, migration_id="synthetic-trust", runtime_roles=["a03_runtime"])


def approved_candidate(material):
    """Synthetic owner fixture only; production CLI never creates approval."""
    candidate = T.candidate_sql(material)
    approval = {"format_version": 2, "decision": "approve-offline-trust-basis", "scope": T.SCOPE,
                "migration_id": material["migration_id"], "material_sha256": T.digest(material),
                "candidate_sql_sha256": T.sha(candidate.encode()), "input_hashes": material["input_hashes"],
                "owner_review_evidence_sha256": "3" * 64}
    return candidate, T.canonical(approval).encode()


class TrustImportTests(unittest.TestCase):
    def prepare(self, inputs, hashes):
        return T.prepare_material(inputs, hashes, migration_id="synthetic-trust", runtime_roles=["a03_runtime"])

    def test_complete_basis_and_multilingual_a2_hash_domain(self):
        material = material_fixture()
        self.assertEqual(material["derived"]["native_user_floor"], 74)
        self.assertEqual(material["derived"]["quotes_at_source_cutoff"][0]["effective_level"], 2)
        self.assertEqual(material["derived"]["quotes_at_source_cutoff"][0]["discount"], {"numerator": 97, "denominator": 100})
        T.validate_material(material)
        inputs, hashes = fixture()
        identities = inputs["identity_mapping"]
        identities["records"][0]["display_name"] = "原价权利 🐦"
        identities["mapping_sha256"] = T.sha(T.I.canonical({"source_sha256": inputs["basis"]["source_dump_sha256"], "records": identities["records"]}).encode())
        hashes["identity_mapping"] = T.digest(identities)
        inputs["evidence"]["identity_mapping_sha256"] = hashes["identity_mapping"]
        hashes = bind_report(inputs)
        T.derive_inputs(inputs, hashes)

    def test_incomplete_duplicate_or_remapped_users_refused(self):
        for change in (lambda i: i["user_map"].pop("73"), lambda i: i["basis"]["users"].pop(),
                       lambda i: i["user_map"].update({"73": 74}), lambda i: i["identity_mapping"]["records"].pop(),
                       lambda i: i["target_snapshot"]["identities"].pop()):
            inputs, hashes = fixture()
            change(inputs)
            with self.assertRaises(T.Error):
                self.prepare(inputs, hashes)

    def test_duplicate_orders_unknown_source_and_incomplete_evidence_refused(self):
        for change in (lambda i: i["basis"]["users"][0]["orders"].append(deepcopy(i["basis"]["users"][0]["orders"][0])),
                       lambda i: i["evidence"]["orders"][0].update(source_kind="gift"),
                       lambda i: i["evidence"]["orders"].clear(),
                       lambda i: i["evidence"]["orders"][0].update(net_credits=1)):
            inputs, hashes = fixture()
            change(inputs)
            with self.assertRaises(T.Error):
                self.prepare(inputs, hashes)

    def test_original_provider_predicate_and_opaque_currency_preservation(self):
        inputs, _ = source_fixture()
        proof = inputs["evidence"]["orders"][0]
        for currency in ("JPY", "LDC", "historical-unit"):
            proof["original_currency"] = proof["original_row"]["settlement_currency"] = currency
            proof["original_row_sha256"] = T.digest(proof["original_row"])
            T.validate_order_source(proof)
        for provider, method, currency in (("unknown", "stripe", "USD"), ("epay", "ldc", "LDC"),
                                           ("epay", "epay", "USD"), ("balance", "balance", "USD")):
            proof["original_row"].update(payment_provider=provider, payment_method=method, settlement_currency=currency)
            proof["original_currency"] = currency
            proof["original_row_sha256"] = T.digest(proof["original_row"])
            with self.assertRaises(T.Error):
                T.validate_order_source(proof)

    def test_historical_empty_and_unknown_currency_preserves_exact_audit_and_credits(self):
        for provider in ("stripe", "creem"):
            for currency in ("", "unknown", "UNKNOWN", " usd ", "   "):
                with self.subTest(provider=provider, currency=currency):
                    inputs, _ = fixture()
                    hashes = historical_order(inputs, currency, provider)
                    proof = inputs["evidence"]["orders"][0]
                    T.validate_order_source(proof)
                    material = self.prepare(inputs, hashes)
                    self.assertEqual(material["derived"]["orders"][0]["net_credits"], 500000)
                    self.assertEqual(material["derived"]["quotes_at_source_cutoff"][0]["effective_level"], 1)
                    self.assertEqual(material["inputs"]["evidence"]["original_currencies"], [currency])
                    self.assertEqual(material["inputs"]["evidence"]["orders"][0], proof)
                    self.assertEqual(material["inputs"]["source_review_report"]["orders"][0]["source_rows"][0]["row"],
                                     proof["original_row"])
                    candidate, approval = approved_candidate(material)
                    sealed = T.seal_sql(material, candidate, approval, T.sha(approval))
                    self.assertIn(T.canonical(proof), sealed)
                    self.assertNotIn(T.APPROVAL_SLOT, sealed)

    def test_missing_mismatched_or_malformed_currency_evidence_is_not_invented(self):
        changes = (
            lambda i: i["evidence"]["orders"][0].pop("original_currency"),
            lambda i: i["evidence"]["orders"][0].update(original_currency=None),
            lambda i: i["evidence"]["orders"][0]["original_row"].pop("settlement_currency"),
            lambda i: i["evidence"]["orders"][0]["original_row"].update(settlement_currency=None),
            lambda i: i["evidence"]["orders"][0]["original_row"].update(settlement_currency="USD"),
            lambda i: i["evidence"].update(original_currencies=[]),
            lambda i: i["evidence"].update(original_currencies=["USD"]),
            lambda i: i["evidence"].update(original_currencies=["", ""]),
            lambda i: i["evidence"].update(original_currencies=["", None]),
            lambda i: i["evidence"].update(original_currencies=[[]]),
        )
        for change in changes:
            inputs, _ = fixture()
            historical_order(inputs, "")
            change(inputs)
            proof = inputs["evidence"]["orders"][0]
            proof["original_row_sha256"] = T.digest(proof["original_row"])
            with self.assertRaises(T.Error):
                self.prepare(inputs, bind_report(inputs))
        for currency in (None, 1, [], "x" * 17, "\0"):
            with self.subTest(currency=currency):
                inputs, _ = fixture()
                hashes = historical_order(inputs, currency)
                with self.assertRaises(T.Error):
                    self.prepare(inputs, hashes)

    def test_changed_original_currency_requires_new_owner_approval(self):
        inputs, _ = fixture()
        material = self.prepare(inputs, historical_order(inputs, ""))
        candidate, approval = approved_candidate(material)
        changed = self.prepare(inputs, historical_order(inputs, "unknown"))
        changed_candidate = T.candidate_sql(changed)
        self.assertNotEqual(changed["input_hashes"]["evidence"], material["input_hashes"]["evidence"])
        self.assertNotEqual(changed["derived"]["orders"][0]["source_digest"], material["derived"]["orders"][0]["source_digest"])
        self.assertNotEqual(candidate, changed_candidate)
        with self.assertRaisesRegex(T.Error, "owner_approval_binding_mismatch"):
            T.seal_sql(changed, changed_candidate, approval, T.sha(approval))

    def test_historical_currency_does_not_supply_paid_or_credit_evidence(self):
        inputs, _ = fixture()
        # A larger monetary amount never replaces the explicit original credits.
        hashes = historical_order(inputs, "", money=1000)
        self.assertEqual(self.prepare(inputs, hashes)["derived"]["orders"][0]["net_credits"], 500000)
        for values, error in (({"money": 0}, "order_paid_evidence_missing"),
                              ({"credited_quota": 0, "amount": 0}, "original_gross_credit_not_positive"),
                              ({"credited_quota": 1}, "unrebased_credit_unit_mismatch")):
            inputs, _ = fixture()
            historical_order(inputs, "")
            proof = inputs["evidence"]["orders"][0]
            proof["original_row"].update(values)
            proof["original_row_sha256"] = T.digest(proof["original_row"])
            with self.assertRaisesRegex(T.Error, error):
                self.prepare(inputs, bind_report(inputs))

    def test_epay_keeps_cny_and_paid_amount_requirements(self):
        for currency, expected, settled, accepted in (("", 1, 0, False), ("unknown", 1, 0, False),
                ("USD", 1, 0, False), ("CNY", 0, 0, False), ("CNY", 1, 0, True),
                ("cny", 1, 0, True), ("CNY", 0, 1000000, True)):
            with self.subTest(currency=currency, expected=expected, settled=settled):
                inputs, _ = fixture()
                historical_order(inputs, currency, "epay")
                proof = inputs["evidence"]["orders"][0]
                proof["original_row"].update(expected_amount_micros=expected, settled_amount_micros=settled)
                proof["original_row_sha256"] = T.digest(proof["original_row"])
                hashes = bind_report(inputs)
                if accepted:
                    self.assertEqual(self.prepare(inputs, hashes)["derived"]["orders"][0]["net_credits"], 500000)
                else:
                    with self.assertRaisesRegex(T.Error, "ineligible_epay_order"):
                        self.prepare(inputs, hashes)

    def test_post_cutoff_native_still_requires_nonblank_currency(self):
        for currency in ("unknown", "", "   "):
            with self.subTest(currency=currency):
                inputs, _ = fixture()
                historical_order(inputs, currency)
                proof = inputs["evidence"]["orders"][0]
                proof["original_row"]["expected_amount_micros"] = 1000000
                proof["original_row_sha256"] = T.digest(proof["original_row"])
                bind_report(inputs)
                plan = {"version": 1, "kind": "offline_credit_balance_rebase_preview", "migration_id": "synthetic-rebase",
                    "plan_sha256": "d" * 64, "has_complete_history": True, "snapshot_state": "frozen_writers_stopped",
                    "usd_credit_conversion": 500000, "snapshot_at": 1792000000, "divisor": "2", "rounding": "half-away-from-zero",
                    "exact_factor": {"numerator": 1, "denominator": 2},
                    "fx_source": {"kind": "frozen_production_option", "key": "USDExchangeRate", "value": "2"},
                    "user_ids": [proof["source_user_id"]], "refund_bases": [], "pending_bases": [], "blocked_pending_bases": [],
                    "noncash_topups": [], "option_entries": []}
                parent = {"migration_id": "synthetic-rebase", "plan_sha256": "d" * 64, "plan": plan}
                audit = inputs["source_review_report"]["orders"][0]
                audit.update(projection_mode="post_cutoff_native", rebase_cutoff=plan["snapshot_at"],
                    parent_plan_hashes={"synthetic-rebase": {"declared_plan_sha256": "d" * 64, "observed_plan_sha256": T.digest(plan)}})
                audit["source_rows"].append({"table": "wallet_credit_rebases", "row": parent})
                proof["net_evidence_sha256"] = T.digest(audit)
                inputs["evidence"]["independent_report_sha256"] = T.digest(inputs["source_review_report"])
                if currency.strip(" "):
                    self.assertEqual(self.prepare(inputs, input_hashes(inputs))["derived"]["orders"][0]["net_credits"], 500000)
                else:
                    with self.assertRaisesRegex(T.Error, "post_cutoff_order_mismatch"):
                        self.prepare(inputs, input_hashes(inputs))

    def test_fully_refunded_zero_rows_kept(self):
        inputs, hashes = fixture()
        user, proof = inputs["basis"]["users"][0], inputs["evidence"]["orders"][0]
        user["orders"][0]["net_credits"] = 0
        proof.update(net_credits=0, refunded_credits=6000000)
        proof["original_row"]["refunded_quota"] = 6000000
        proof["original_row_sha256"] = T.digest(proof["original_row"])
        fact = inputs["evidence"]["users"][0]
        fact["original_quote"] = T.effective_quote(T.policy_spec(inputs["basis"]["legacy_policy"]), user, "user", inputs["basis"]["source_cutoff"])
        fact["original_quote_sha256"] = T.digest(fact["original_quote"])
        hashes = bind_report(inputs)
        material = self.prepare(inputs, hashes)
        self.assertEqual(material["derived"]["orders"][0]["net_credits"], 0)
        self.assertEqual(material["derived"]["users"][0]["order_count"], 1)
        self.assertEqual(material["derived"]["quotes_at_source_cutoff"][0]["effective_level"], 0)

    def test_unknown_projection_stays_unavailable(self):
        inputs, hashes = fixture()
        inputs["basis"]["users"][0]["projection_known"] = False
        with self.assertRaisesRegex(T.Error, "original_price_rights_mismatch"):
            self.prepare(inputs, hashes)
        fact = inputs["evidence"]["users"][0]
        fact["original_quote"] = {"status": "unavailable"}
        fact["original_quote_sha256"] = T.digest(fact["original_quote"])
        hashes = bind_report(inputs)
        self.assertFalse(self.prepare(inputs, hashes)["derived"]["users"][0]["projection_known"])

    def test_tampered_seed_roles_upgrade_or_basis_rows_refused(self):
        changes = [lambda i: i["target_snapshot"]["trust_policy_seed"].update(native_user_floor=74),
                   lambda i: i["target_snapshot"]["trust_policy_seed"]["spec"].update(paid_activation_enabled=False),
                   lambda i: i["target_snapshot"]["instance"].update(effective_role_can_login=True),
                   lambda i: i["target_snapshot"]["instance"].update(listen_addresses="localhost")]
        for table in T.CHANGED[1:] + ("core_meta.schema_upgrades", "core_meta.component_upgrades"):
            changes.append(lambda i, table=table: i["target_snapshot"]["state"]["tables"][table].update(rows=1))
        for change in changes:
            inputs, hashes = fixture()
            change(inputs)
            with self.assertRaises(T.Error):
                self.prepare(inputs, hashes)

    def test_real_approval_required_and_changed_material_expires_it(self):
        material = material_fixture()
        candidate, approval = approved_candidate(material)
        sealed = T.seal_sql(material, candidate, approval, T.sha(approval))
        self.assertNotIn(T.APPROVAL_SLOT, sealed)
        self.assertIn("owner_approval_not_sealed", candidate)
        for changed in (b'{}', approval.replace(b'approve-offline', b'deny-offline')):
            with self.assertRaises(T.Error):
                T.seal_sql(material, candidate, changed, T.sha(changed))
        with self.assertRaisesRegex(T.Error, "candidate_sql_changed"):
            T.seal_sql(material, candidate + "-- change\n", approval, T.sha(approval))
        changed = deepcopy(material)
        changed["input_hashes"]["basis"] = "0" * 64
        with self.assertRaises(T.Error):
            T.seal_sql(changed, candidate, approval, T.sha(approval))

    def test_old_receipt_relabel_and_existing_original_attempt_refused(self):
        material = material_fixture()
        material["format_version"] = 1
        with self.assertRaisesRegex(T.Error, "trust_material_contract_invalid"):
            T.validate_material(material)
        inputs, hashes = fixture()
        inputs["target_snapshot"]["import_runs"].append({"migration_id": "synthetic-trust"})
        with self.assertRaisesRegex(T.Error, "original_attempt_requires_resolution"):
            self.prepare(inputs, hashes)

    def test_sql_preserves_nontrust_tables_all_sequences_and_native_since(self):
        sql = T.candidate_sql(material_fixture())
        self.assertIn("native_user_floor=74", sql)
        self.assertNotIn("native_since=", sql)
        for marker in ("trust_nontrust_table_or_sequence_changed", "trust_original_receipt_conflict", "trust_exact_prestate_changed"):
            self.assertIn(marker, sql)
        code = A2.without_literals_and_comments(sql)
        for table in ("core_identity.accounts", "core_identity.users", "core_billing.ledger_accounts", "core_billing.ledger_journals"):
            self.assertNotIn("INSERT INTO " + table, code)
            self.assertNotIn("UPDATE " + table, code)
        self.assertNotIn("setval(", code)
        self.assertNotIn("CREATE TABLE", code)
        self.assertEqual(code.count("BEGIN ISOLATION LEVEL SERIALIZABLE"), 1)

    def test_unknown_resolution_never_infers_rollback_or_retries(self):
        sql = T.resolve_sql(material_fixture(), "a" * 64)
        self.assertIn("UNKNOWN_ACTIVE_ORIGINAL", sql)
        self.assertIn("UNKNOWN_NO_MATCHING_RECEIPT_REQUIRES_OWNER_RECONCILIATION", sql)
        self.assertIn("READ ONLY", sql)
        for token in ("INSERT", "UPDATE", "DELETE", "TRUNCATE", "COMMIT"):
            self.assertNotIn(token, A2.without_literals_and_comments(sql))

    def test_original_projector_report_and_audit_records_must_be_present_unchanged(self):
        for mutate in (lambda i: i["source_review_report"]["projector_source_hashes"].update({"apps/api-go/model/topup.go": "0" * 64}),
                       lambda i: i["source_review_report"]["orders"][0]["source_rows"].clear(),
                       lambda i: i["source_review_report"]["orders"][0]["projector_result"].update(net_credits=1),
                       lambda i: i["source_review_report"]["users"].pop()):
            inputs, hashes = fixture()
            # Immutable source hash identities are still the original files.
            mutate(inputs)
            with self.assertRaises(T.Error):
                self.prepare(inputs, hashes)

    def test_audited_rebase_requires_linked_parent_plan_child_and_original_counters(self):
        inputs, hashes = fixture()
        audit = inputs["source_review_report"]["orders"][0]
        audit["projection_mode"], audit["rebase_cutoff"] = "audited_rebase", 1792550000
        proof = inputs["evidence"]["orders"][0]
        def refresh(values=inputs):
            a = values["source_review_report"]["orders"][0]
            p = values["evidence"]["orders"][0]
            p["net_evidence_sha256"] = T.digest(a)
            values["evidence"]["independent_report_sha256"] = T.digest(values["source_review_report"])
            return input_hashes(values)
        with self.assertRaisesRegex(T.Error, "rebase_audit_source_rows_missing"):
            self.prepare(inputs, refresh())
        raw = proof["original_row"]
        raw.update(credited_quota=12000000, platform_amount_micros=12000000,
                   refunded_amount_micros=0, pending_credit_rebase_key="")
        proof["original_row_sha256"] = T.digest(raw)
        child = {"migration_id": "synthetic-rebase", "top_up_id": 710, "user_id": 71,
            "original_credited_quota": 12000000, "original_refunded_quota": 0,
            "original_refunded_amount_micros": 0, "original_paid_amount_micros": 12000000,
            "refundable_quota": 6000000, "rebased_debited_quota": 0}
        base = {key: value for key, value in child.items() if key != "migration_id"}
        base["source"] = {**raw, "effective_credited_quota": 12000000}
        plan = {"version": 1, "kind": "offline_credit_balance_rebase_preview", "migration_id": "synthetic-rebase",
                "plan_sha256": "d" * 64, "has_complete_history": True, "snapshot_state": "frozen_writers_stopped",
                "usd_credit_conversion": 500000, "snapshot_at": 1792550000, "divisor": "2", "rounding": "half-away-from-zero",
                "exact_factor": {"numerator": 1, "denominator": 2}, "fx_source": {"kind": "frozen_production_option", "key": "USDExchangeRate", "value": "2"},
                "user_ids": [71], "snapshot_all_users": True, "include_pending_topups": True, "orphan_pending_user_ids": [],
                "option_entries": [{"key": "QuotaPerUnit", "before": "500000"}], "refund_bases": [base],
                "pending_bases": [], "blocked_pending_bases": [], "noncash_topups": []}
        audit["source_rows"] = [{"table": "top_ups", "row": deepcopy(raw)},
            {"table": "wallet_credit_rebases", "row": {"migration_id": "synthetic-rebase", "plan_sha256": "d" * 64, "plan": plan}},
            {"table": "wallet_topup_credit_rebases", "row": child}]
        audit["parent_plan_hashes"] = {"synthetic-rebase": {"declared_plan_sha256": "d" * 64, "observed_plan_sha256": T.digest(plan)}}
        result = self.prepare(inputs, refresh())
        self.assertEqual(result["derived"]["orders"][0]["net_credits"], 6000000)
        # No new refund must retain the original counter. The original legacy
        # delta function returns zero before recomputing its cumulative target.
        no_new = deepcopy(inputs)
        p = no_new["evidence"]["orders"][0]
        a = no_new["source_review_report"]["orders"][0]
        p["original_row"].update(refunded_quota=1, refunded_amount_micros=2)
        p["original_row_sha256"] = T.digest(p["original_row"])
        a["source_rows"][0]["row"] = deepcopy(p["original_row"])
        a["source_rows"][2]["row"].update(original_refunded_quota=1, original_refunded_amount_micros=2)
        plan = a["source_rows"][1]["row"]["plan"]
        plan["refund_bases"][0].update(original_refunded_quota=1, original_refunded_amount_micros=2)
        plan["refund_bases"][0]["source"].update(refunded_quota=1, refunded_amount_micros=2)
        a["parent_plan_hashes"]["synthetic-rebase"]["observed_plan_sha256"] = T.digest(plan)
        self.assertEqual(self.prepare(no_new, refresh(no_new))["derived"]["orders"][0]["net_credits"], 6000000)
        for mutate in (lambda a: a["source_rows"][1].update(row={}),
                       lambda a: a["source_rows"][2].update(row={}),
                       lambda a: a["source_rows"][2]["row"].update(user_id=72),
                       lambda a: a["source_rows"][2]["row"].update(rebased_debited_quota=1),
                       lambda a: a.update(rebase_cutoff=1792550001),
                       lambda a: a["source_rows"][2]["row"].update(original_credited_quota=12000001)):
            changed = deepcopy(inputs)
            mutate(changed["source_review_report"]["orders"][0])
            with self.assertRaises(T.Error):
                self.prepare(changed, refresh(changed))

    def test_original_decimal_refund_counter_keeps_double_rounding_semantics(self):
        self.assertEqual(T.legacy_refund_target(1, 100000000000000001, 50000000000000000), 1)
        self.assertEqual(T.F.PLANNER.rebased_refund_target(1, 100000000000000001, 50000000000000000), 0)
        self.assertEqual(T.legacy_refund_target(100, 200, 1), 1)
        self.assertEqual(T.legacy_refund_target(100, 200, 0), 0)
        self.assertEqual(T.legacy_refund_target(100, 200, 201), 100)

    def test_prepare_seal_cli_requires_actual_review_bytes_and_rechecks_sources(self):
        inputs, _ = fixture()
        hashes = historical_order(inputs, "")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            command = [sys.executable, "-B", T.__file__, "prepare", "--migration-id", "synthetic-trust",
                       "--runtime-role", "a03_runtime", "--output", str(root / "prepared")]
            for key, value in inputs.items():
                path = root / (key + ".json")
                path.write_bytes(input_bytes(key, value))
                flag = "--" + key.replace("_", "-")
                command.extend((flag, str(path), flag + "-sha256", hashes[key]))
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["status"], "PENDING_OWNER_REVIEW")
            self.assertFalse((root / "prepared/owner-import.private.sql").exists())
            material = T.decode((root / "prepared/material.private.json").read_bytes())
            self.assertEqual(material["inputs"]["evidence"]["orders"][0]["original_currency"], "")
            candidate = (root / "prepared/owner-candidate.private.sql").read_text()
            _, approval_bytes = approved_candidate(material)
            approval = T.decode(approval_bytes)
            review = b"SYNTHETIC UNIT TEST OWNER REVIEW; NO REAL DATA OR APPROVAL"
            (root / "owner-review.txt").write_bytes(review)
            approval["owner_review_evidence_sha256"] = T.sha(review)
            approval_bytes = T.canonical(approval).encode()
            (root / "approval.json").write_bytes(approval_bytes)
            seal = [sys.executable, "-B", T.__file__, "seal", "--prepared", str(root / "prepared"),
                    "--approval", str(root / "approval.json"), "--approval-sha256", T.sha(approval_bytes),
                    "--owner-review-evidence", str(root / "owner-review.txt"), "--output", str(root / "sealed")]
            result = subprocess.run(seal, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout)["status"], "PREPARED_NOT_SUBMITTED")
            self.assertNotIn(T.APPROVAL_SLOT, (root / "sealed/owner-import.private.sql").read_text())
            self.assertTrue((root / "sealed/resolve.private.sql").is_file())
            (root / "owner-review.txt").write_bytes(review + b"changed")
            seal[-1] = str(root / "changed-review")
            result = subprocess.run(seal, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("owner_review_evidence_changed", result.stderr)
            self.assertFalse((root / "changed-review").exists())
            (root / "owner-review.txt").write_bytes(review)
            (root / "basis.json").write_bytes(b"{}")
            seal[-1] = str(root / "changed-source")
            result = subprocess.run(seal, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("input_file_changed", result.stderr)
            self.assertFalse((root / "changed-source").exists())

    def test_duplicate_json_keys_nonfinite_and_cli_help(self):
        for value in (b'{"x":1,"x":2}', b'{"x":NaN}'):
            with self.assertRaises(T.Error):
                T.decode(value)
        result = subprocess.run([sys.executable, "-B", str(Path(T.__file__)), "--help"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)


# Reuse the guarded resource-owner fixture transport, without rerunning the
# financial suite through inheritance. Every case first performs real A2 and
# financial import, then proves the trust transaction preserves their writes.
import test_financial_import_pg as PG


class _TrustPGCases:
    def setUp(self):
        PG.PostgreSQLTests.setUp(self)
        self.sql(self.apply_sql)  # Existing owner-reviewed opening fixture.
        inputs, _ = source_fixture(self.source)
        inputs["target_snapshot"] = json.loads(self.sql(T.snapshot_sql(self.inventory, "synthetic-trust")).stdout)
        hashes = input_hashes(inputs)
        self.material = T.prepare_material(inputs, hashes, migration_id="synthetic-trust", runtime_roles=[self.runtime_role])
        candidate, approval = approved_candidate(self.material)
        self.candidate = candidate
        self.approval_sha256 = T.sha(approval)
        self.apply_sql = T.seal_sql(self.material, candidate, approval, self.approval_sha256)
        self.before = self.full_state()

    def trust_result(self, sql):
        output = self.sql(sql).stdout
        rows = [json.loads(line) for line in output.splitlines() if line.lstrip().startswith("{")]
        self.assertEqual(len(rows), 1, output)
        return rows[0]

    def test_current26_after_finance_strict_replay_and_full_conservation(self):
        self.assertEqual(self.trust_result(self.apply_sql)["status"], "COMMITTED")
        after = self.full_state()
        self.assertEqual(after["sequences"], self.before["sequences"])
        allowed = set(T.CHANGED) | {"core_migration.import_runs"}
        self.assertEqual({k: v for k, v in after["tables"].items() if k not in allowed},
                         {k: v for k, v in self.before["tables"].items() if k not in allowed})
        self.assertEqual(self.trust_result(self.apply_sql)["status"], "COMMITTED")
        self.assertEqual(self.full_state(), after)
        policy = self.query(T.I.trust_policy_seed_query())
        self.assertEqual(policy["native_user_floor"], 74)
        self.assertEqual(policy["native_since"], self.material["inputs"]["target_snapshot"]["trust_policy_seed"]["native_since"])
        self.assertEqual(self.query("SELECT count(*) FROM core_trust_pricing.user_bases"), 3)
        self.assertEqual(self.query("SELECT manifest#>>'{source_evidence,orders,0,original_currency}' FROM core_migration.import_runs WHERE migration_id='synthetic-trust'"), "JPY")

    def test_historical_blank_and_unknown_currency_readback_replay_and_conservation(self):
        inputs = deepcopy(self.material["inputs"])
        historical_order(inputs, "")
        extra = deepcopy(inputs["evidence"]["orders"][0])
        extra.update(source_order_id=711, original_currency="unknown")
        extra["original_row"].update(id=711, payment_provider="creem", payment_method="creem", settlement_currency="unknown")
        extra["original_row_sha256"] = T.digest(extra["original_row"])
        inputs["evidence"]["orders"].append(extra)
        inputs["evidence"]["original_currencies"] = ["", "unknown"]
        user = inputs["basis"]["users"][0]
        user["orders"].append({"source_order_id": 711, "net_credits": 500000, "paid_at": extra["paid_at"]})
        fact = inputs["evidence"]["users"][0]
        fact["original_quote"] = T.effective_quote(T.policy_spec(inputs["basis"]["legacy_policy"]),
            user, "user", inputs["basis"]["source_cutoff"])
        fact["original_quote_sha256"] = T.digest(fact["original_quote"])
        material = T.prepare_material(inputs, bind_report(inputs), migration_id="synthetic-trust", runtime_roles=[self.runtime_role])
        candidate, approval = approved_candidate(material)
        sql = T.seal_sql(material, candidate, approval, T.sha(approval))
        self.assertEqual(self.trust_result(sql)["status"], "COMMITTED")
        after = self.full_state()
        self.assertEqual(after["sequences"], self.before["sequences"])
        allowed = set(T.CHANGED) | {"core_migration.import_runs"}
        self.assertEqual({k: v for k, v in after["tables"].items() if k not in allowed},
                         {k: v for k, v in self.before["tables"].items() if k not in allowed})
        self.assertEqual(self.query("SELECT manifest->'source_evidence' FROM core_migration.import_runs WHERE migration_id='synthetic-trust'"),
                         inputs["evidence"])
        self.assertEqual(self.query("SELECT jsonb_agg(jsonb_build_array(source_order_id,net_credits) ORDER BY source_order_id) FROM core_trust_pricing.legacy_paid_orders"),
                         [[710, 500000], [711, 500000]])
        self.assertEqual(self.trust_result(sql)["status"], "COMMITTED")
        self.assertEqual(self.full_state(), after)

    def test_unapproved_candidate_and_midtransaction_failure_leave_everything_unchanged(self):
        self.rejected_unchanged(self.candidate, "owner_approval_not_sealed")
        sql = self.apply_sql.replace("  UPDATE core_trust_pricing.policies SET", "  RAISE EXCEPTION 'synthetic_trust_failure';\n  UPDATE core_trust_pricing.policies SET", 1)
        self.assertNotEqual(sql, self.apply_sql)
        self.rejected_unchanged(sql, "synthetic_trust_failure")

    def test_exact_prestate_conflict_and_replay_conflict_are_atomic(self):
        self.sql("UPDATE core_trust_pricing.policies SET source_cutoff=source_cutoff+1;")
        self.rejected_unchanged(self.apply_sql, "trust_exact_prestate_changed")

    def test_modified_basis_prevents_same_id_replay(self):
        self.trust_result(self.apply_sql)
        self.sql("UPDATE core_trust_pricing.user_bases SET last_api_activity_at=last_api_activity_at-1 WHERE user_id=71;")
        self.rejected_unchanged(self.apply_sql, "trust_basis_or_order_readback_conflict")

    def test_owner_role_gate_and_missing_order_proof_do_not_change_existing_finance(self):
        self.sql("ALTER ROLE " + self.runtime_role + " LOGIN;", admin=True)
        try:
            self.rejected_unchanged(self.apply_sql, "financial_offline_roles_mismatch")
        finally:
            self.sql("ALTER ROLE " + self.runtime_role + " NOLOGIN;", admin=True)
        self.assertEqual(self.full_state(), self.before)

    def test_unknown_absent_receipt_stays_unknown_and_committed_receipt_is_readonly(self):
        resolve = T.resolve_sql(self.material, self.approval_sha256)
        self.assertEqual(self.trust_result(resolve)["status"], "UNKNOWN_NO_MATCHING_RECEIPT_REQUIRES_OWNER_RECONCILIATION")
        self.assertEqual(self.full_state(), self.before)
        self.trust_result(self.apply_sql)
        after = self.full_state()
        self.assertEqual(self.trust_result(resolve)["status"], "RECEIPT_FOUND_VERIFY_EXACT_STATE")
        self.assertEqual(self.full_state(), after)


def _copy_fixture_methods(destination, bases):
    names = {"owner_role", "installer_role", "runtime_role", "other_role", "docker", "setUpClass", "tearDownClass",
             "cleanup", "command", "sql", "query", "create_test_database", "install_fresh_core", "capture_material",
             "full_state", "rejected_unchanged", "cleanup_database", "idle_sql"}
    for base in bases:
        for name in names:
            if name in base.__dict__:
                setattr(destination, name, base.__dict__[name])


@unittest.skipUnless(os.environ.get("LMM_FINANCIAL_PG_TEST") == "1", "isolated PostgreSQL18 not enabled")
class PostgreSQLTrustTests(_TrustPGCases, unittest.TestCase):
    pass


_copy_fixture_methods(PostgreSQLTrustTests, [PG.PostgreSQLTests])


@unittest.skipUnless(os.environ.get("LMM_FINANCIAL_PG_FIXTURE_RECEIPT"), "matched Core init-db Unix PG18 not requested")
class UnixPostgreSQLTrustTests(_TrustPGCases, unittest.TestCase):
    pass


_copy_fixture_methods(UnixPostgreSQLTrustTests, [PG.PostgreSQLTests, PG.UnixPostgreSQLTests])


if __name__ == "__main__":
    unittest.main()
