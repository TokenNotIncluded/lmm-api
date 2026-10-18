"""Development-only counter profile checks; SQL execution needs independent PG."""
import copy
from decimal import Decimal
import io
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import test_legacy_financial_opening as base

F, O = base.F, base.O


def counter(wallet=None, **changes):
    wallet = base.payload() if wallet is None else wallet
    result = {k: wallet[k] for k in (
        "account_id", "migration_id", "source_entity_key", "source_archive_sha256",
        "mapping_sha256", "source_row_sha256", "observed_at")}
    result.update(user_id=wallet["account_id"], used_quota=-11,
                  request_count=17, aff_history=-5)
    return {**result, **changes}


class CounterProjectionTests(unittest.TestCase):
    def test_raw_signed_counters_are_bound_without_creating_money(self):
        wallet = base.payload()
        old = O.expected_readback(F, [wallet])
        value = O.expected_readback(F, [wallet], counter_payloads=[counter(wallet)])
        self.assertNotEqual(value["domain"], old["domain"])
        self.assertEqual({k: v for k, v in value.items()
                          if k not in ("domain", "historical_counters")},
                         {k: v for k, v in old.items() if k != "domain"})
        self.assertEqual(value["historical_counters"],
                         [{**counter(wallet), "request": counter(wallet)}])

    def test_default_projection_still_excludes_counter_rows(self):
        self.assertEqual(O.expected_readback(F, [base.payload()])["domain"], O.DOMAIN)
        self.assertNotIn("historical_counters", O.expected_readback(F, [base.payload()]))

    def test_missing_extra_duplicate_and_different_user_counter_rows_rejected(self):
        for rows in ([], [counter(), counter()], [counter(user_id=2)],
                     [counter(account_id=2)], [counter(extra=0)]):
            with self.subTest(rows=rows), self.assertRaises(F.MappingError):
                O.expected_readback(F, [base.payload()], counter_payloads=rows)

    def test_counter_types_bounds_and_original_binding_are_not_coerced(self):
        for field in ("used_quota", "request_count", "aff_history"):
            for value in (True, False, Decimal("0"), "0", None,
                          F.JS_SAFE_MAX + 1, -F.JS_SAFE_MAX - 1):
                with self.subTest(field=field, value=value), self.assertRaises(F.MappingError):
                    O.expected_readback(F, [base.payload()],
                                        counter_payloads=[counter(**{field: value})])
        for field, value in (("source_row_sha256", "e" * 64),
                             ("source_archive_sha256", "e" * 64),
                             ("mapping_sha256", "e" * 64),
                             ("migration_id", "other"), ("observed_at", True)):
            with self.subTest(field=field), self.assertRaises(F.MappingError):
                O.expected_readback(F, [base.payload()],
                                    counter_payloads=[counter(**{field: value})])


class CounterEntryTests(unittest.TestCase):
    def test_opt_in_is_refused_outside_an_explicit_opening_mode_before_io(self):
        argv = ["financial", "--source-receipt", "/absent", "--identity-map", "/absent",
                "--identity-manifest", "/absent", "--output", "/must-not-write",
                "--migration-id", "synthetic", "--include-opening-counters"]
        error = io.StringIO()
        with patch("sys.argv", argv), patch("sys.stderr", error), \
                patch.object(F.IDENTITY, "load_receipt") as source:
            self.assertEqual(F.main(), 1)
        source.assert_not_called()
        self.assertIn("opening_counters_require_opening_mode", error.getvalue())

    def test_general_apply_stays_refused_even_with_counter_opt_in(self):
        argv = ["financial", "--source-receipt", "/absent", "--identity-map", "/absent",
                "--identity-manifest", "/absent", "--output", "/must-not-write",
                "--migration-id", "synthetic", "--apply", "--include-opening-counters"]
        error = io.StringIO()
        with patch("sys.argv", argv), patch("sys.stderr", error), \
                patch.object(F.IDENTITY, "load_receipt") as source:
            self.assertEqual(F.main(), 1)
        source.assert_not_called()
        self.assertIn("financial_apply_contract_not_frozen", error.getvalue())


class CounterApprovalTests(unittest.TestCase):
    def setUp(self):
        reviewed = base.OpeningReviewTests()
        reviewed.setUp()
        self.proof, self.evidence = reviewed.proof, reviewed.evidence
        self.identity_manifest = {"format_version": 1, "scope": "identity-only",
            "source_archive_sha256": "a" * 64, "source_target_id_map_sha256": "b" * 64,
            "mapping_sha256": "c" * 64}
        provenance = F.bind_identity_provenance(self.identity_manifest,
            map_file_sha256="b" * 64, source_archive_sha256="a" * 64)
        self.plan = F.plan_financial(base.facts.tables(users=[base.facts.user(
            used_quota=-11, request_count=17, aff_history=-5)]), {71: 2},
            source_sha256="a" * 64, identity_provenance=provenance)
        self.evidence = {**self.evidence, "mapping_sha256": self.plan["mapping_sha256"]}
        self.old_review = {**reviewed.review, "mapping_sha256": self.plan["mapping_sha256"]}
        self.review = {**self.old_review, "format_version": 2, "scope": O.COUNTER_REVIEW_SCOPE,
            "included_families": O.COUNTER_INCLUDED, "excluded_families": O.COUNTER_EXCLUDED,
            "historical_counter_policy": O.COUNTER_POLICY,
            "counter_source_facts_sha256": F.digest(O.counter_source_facts(F, self.plan))}
        self.opening = F.build_opening_payloads(self.plan, migration_id="synthetic-opening",
            observation=F.source_observation({"started_utc": "20261011T174353Z"}),
            reservation_reconciliation={"source_archive_sha256": "a" * 64,
                "mapping_sha256": self.plan["mapping_sha256"], "reviewed_evidence_sha256": "f" * 64,
                "positions": [{"source_user_id": 71, "reserved_units": 0}]})
        typed = base.OpeningTypedContractTests()
        typed.setUp()
        self.target = copy.deepcopy(typed.target)
        self.contract = F.IDENTITY.current_contract()
        self.target["contract"] = {**self.contract, "catalog_fingerprint_sha256": "d" * 64}
        self.target["identity_binding_sha256"] = "d" * 64
        self.target["identity_registry_row"] = {"migration_id": "synthetic-identity"}
        self.snapshot = {O.COUNTER_TABLE: {"rows": 0, "sha256": "e" * 64}, "sequence-state": {
            name: {"owner": "opening_o", "definition": {"seqincrement": 1, "seqcache": 1,
                "seqmin": 1, "seqmax": F.BIGINT_MAX, "seqcycle": False},
                "last_value": 1, "is_called": False} for name in O.TOUCHED}}

    def review_result(self, review=None, *, include=True):
        return O.validate_review(F, self.plan, self.proof, self.evidence,
            self.review if review is None else review, evidence_sha256="f" * 64,
            include_counters=include)

    def approval(self, *, include=True):
        return O.make_approval(F, plan=self.plan, opening=self.opening, target=self.target,
            contract=self.contract, file_hashes={"source": {}, "target_receipt": "f" * 64,
                "review": {}}, review=self.review if include else self.old_review,
            snapshot=self.snapshot, include_counters=include)

    def sql(self, approval=None, *, include=True, **changes):
        return O.build_import_sql(F, approval=self.approval() if approval is None else approval,
            target=self.target, contract=self.contract, plan=self.plan, identities={71: 2},
            identity_manifest=self.identity_manifest, snapshot=self.snapshot,
            include_counters=include, **changes)

    def test_explicit_review_binds_all_source_counters_but_does_not_prove_new_funds(self):
        self.assertEqual(self.review_result()["positions"], [{"source_user_id": 71, "reserved_units": 0}])
        facts = O.counter_source_facts(F, self.plan)
        self.assertEqual((facts[0]["used_quota"], facts[0]["request_count"], facts[0]["aff_history"]), (-11, 17, -5))
        self.assertEqual((facts[0]["source_user_id"], facts[0]["account_id"]), (71, 2))

    def test_old_review_cannot_approve_new_profile_and_new_review_cannot_approve_old(self):
        with self.assertRaises(F.MappingError):
            self.review_result(self.old_review)
        with self.assertRaises(F.MappingError):
            self.review_result(include=False)
        self.review_result(self.old_review, include=False)

    def test_missing_or_changed_counter_review_facts_fail_closed(self):
        for key, value in (("format_version", True), ("format_version", Decimal("2")),
                           ("counter_source_facts_sha256", "e" * 64),
                           ("historical_counter_policy", "convert-current-price"),
                           ("included_families", ["wallet-opening"]),
                           ("excluded_families", O.EXCLUDED)):
            review = {**self.review, key: value}
            with self.subTest(key=key), self.assertRaises(F.MappingError):
                self.review_result(review)
        for key in ("counter_source_facts_sha256", "historical_counter_policy", "included_families"):
            review = copy.deepcopy(self.review)
            del review[key]
            with self.subTest(missing=key), self.assertRaises(F.MappingError):
                self.review_result(review)

    def test_counter_review_never_waives_unknown_reservations(self):
        self.evidence["table_assessments"][0]["opening_holds"] = "unknown"
        with self.assertRaisesRegex(F.MappingError, "unknown_opening_rights_rejected"):
            self.review_result()

    def test_new_approval_binds_counter_payload_and_separate_domain(self):
        approved, old = self.approval(), self.approval(include=False)
        self.assertEqual((approved["format_version"], approved["scope"]), (3, O.COUNTER_SCOPE))
        self.assertEqual((old["format_version"], old["scope"]), (2, O.SCOPE))
        self.assertNotIn("counter_payloads", old)
        self.assertNotIn("historical-counters", approved["excluded_families"])
        self.assertEqual(approved["review"]["full_counter_payload_set_sha256"], F.digest(approved["counter_payloads"]))
        self.assertEqual(approved["review"]["expected_complete_readback_sha256"],
            F.digest(O.expected_readback(F, approved["payloads"], counter_payloads=approved["counter_payloads"])))

    def test_sql_profiles_cannot_be_mixed_even_if_payload_hashes_are_valid(self):
        with self.assertRaises(F.MappingError):
            self.sql(self.approval(include=False), include=True)
        with self.assertRaises(F.MappingError):
            self.sql(include=False)

    def test_approval_version_and_payload_types_cannot_alias_integer_contracts(self):
        for field, value in (("format_version", Decimal("3.0")), ("format_version", 3.0),
                             ("format_version", "3"), ("format_version", True),
                             ("credits_per_usd", Decimal("500000"))):
            approved = self.approval()
            approved[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(F.MappingError):
                self.sql(approved)

    def test_approval_mutation_does_not_change_the_module_family_policy(self):
        approved = self.approval()
        approved["included_families"].append("subscriptions")
        approved["excluded_families"].remove("subscriptions")
        self.assertEqual(O.COUNTER_INCLUDED, ["wallet-opening", "historical-counters"])
        self.assertIn("subscriptions", O.COUNTER_EXCLUDED)
        with self.assertRaises(F.MappingError):
            self.sql(approved)

    def test_rehashed_changed_counters_cannot_override_original_source_plan(self):
        for key, value in (("used_quota", -12), ("request_count", 18), ("aff_history", -6),
                           ("user_id", 72), ("account_id", 3), ("observed_at", True),
                           ("used_quota", Decimal("-11"))):
            approved = self.approval()
            approved["counter_payloads"][0][key] = value
            approved["review"]["full_counter_payload_set_sha256"] = F.digest(approved["counter_payloads"])
            with self.subTest(key=key, value=value), self.assertRaises(F.MappingError):
                self.sql(approved)

    def test_generated_sql_keeps_counter_write_inside_same_transaction_and_full_readback(self):
        sql = self.sql()
        self.assertLess(sql.index("BEGIN ISOLATION LEVEL SERIALIZABLE"), sql.index("result := core_billing.import_opening_balance"))
        self.assertLess(sql.index("result := core_billing.import_opening_balance"), sql.index("result := core_billing.import_account_opening_counters"))
        self.assertLess(sql.index("result := core_billing.import_account_opening_counters"), sql.index("SET CONSTRAINTS ALL IMMEDIATE"))
        self.assertEqual(sql.count("COMMIT;"), 1)
        self.assertIn("raw_readback", sql)
        self.assertIn(O.COUNTER_DOMAIN, sql)
        old = self.sql(self.approval(include=False), include=False)
        self.assertNotIn("result := core_billing.import_account_opening_counters", old)

    def test_reconcile_fences_before_readonly_snapshot_and_refuses_live_original_backend(self):
        backend = {"pid": 123, "backend_start": "synthetic"}
        sql = self.sql(reconcile_attempt={"prestate": self.snapshot}, original_backend=backend)
        self.assertTrue(sql.startswith("SELECT pg_advisory_lock(741803121);\nBEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY;"))
        self.assertIn("opening original backend has not ended", sql)
        self.assertIn("confirmed-business-rollback", sql)

    def test_counter_import_does_not_allow_additional_sequences(self):
        approved = self.approval()
        self.assertEqual(set(self.snapshot["sequence-state"]), O.TOUCHED)
        self.assertEqual(len(approved["counter_payloads"]), len(approved["payloads"]))
        self.assertEqual(O.TOUCHED, {"core_billing.ledger_accounts_id_seq", "core_billing.ledger_journals_id_seq"})

    def test_wrong_profile_original_attempt_is_rejected_before_target_query(self):
        approved = self.approval()
        attempt = {"format_version": 1, "scope": O.SCOPE,
                   "migration_id": approved["migration_id"], "approval_sha256": F.digest(approved)}
        with patch.object(O, "read_json", return_value=(None, attempt)), \
                patch.object(F.IDENTITY, "psql") as query:
            with self.assertRaisesRegex(F.MappingError, "opening_attempt_material_mismatch"):
                O.reconcile(F, SimpleNamespace(opening_attempt="/original", include_opening_counters=True),
                    target=self.target, approval=approved, snapshot=self.snapshot, output=None)
        query.assert_not_called()


class CounterRunAcknowledgementTests(unittest.TestCase):
    """Exercise the real run call site with private files and a transport seam.

    No SQL or COMMIT runs here. Independent nativePG owns durability acceptance.
    Source/target identity reads are prevalidated synthetic transport fixtures;
    approval construction, actual file hashes/lease and post-ACK checks are real.
    """
    def exercise_ack(self, *, include=True, status="applied", mutation=None):
        fixture = CounterApprovalTests()
        fixture.setUp()
        with tempfile.TemporaryDirectory(prefix="counter-ack-") as temporary, \
                patch.object(F.IDENTITY, "BACKUP_ROOT", Path(temporary)):
            inputs = Path(temporary) / "inputs"
            inputs.mkdir(mode=0o700)
            write = lambda name, value: F.IDENTITY.write_private(inputs, name, F.canonical(value) + "\n")
            tables = base.facts.tables(users=[base.facts.user(used_quota=-11, request_count=17, aff_history=-5)])
            proof = {**fixture.proof, "selected_table_counts": {name: len(rows) for name, rows in tables.items()}}
            evidence = {**fixture.evidence, "exported_table_counts": proof["selected_table_counts"],
                "table_assessments": [{"table": name, "source_rows_sha256": F.digest(sorted(rows, key=F.canonical)),
                    "opening_holds": "proven-none", "review_basis": "Synthetic transport-only source row review; no real economic claims."}
                    for name, rows in sorted(tables.items())]}
            evidence_path = write("evidence.json", evidence)
            review = {**(fixture.review if include else fixture.old_review),
                      "reservation_evidence_file_sha256": F.IDENTITY.sha256_file(evidence_path)}
            review_path = write("review.json", review)
            proof_path = write("export-proof.json", proof)
            write("source-financial-export.private.jsonl", {"synthetic_transport_fixture": True})
            archive = write("synthetic-archive.fixture", {"not_a_real_database_archive": True})
            source = {"archive": {"path": str(archive)}, "started_utc": "20261011T174353Z"}
            source_path = write("source.json", source)
            map_path = write("map.json", {"source_user_id": 71, "target_personal_account_id": 2})
            manifest_path = write("identity.json", fixture.identity_manifest)
            identity_run = {"migration_id": "synthetic-identity", "source_archive_sha256": "a" * 64,
                            "mapping_sha256": "c" * 64, "manifest": fixture.identity_manifest}
            binding = {"identity_run": identity_run, "native_users": [{"id": 71, "personal_account_id": 2}]}
            lease = write("target.lease", {})
            st = lease.stat()
            target = {**fixture.target, "pre_import_snapshot": fixture.snapshot,
                "identity_registry_row": identity_run, "identity_binding_sha256": F.digest(binding),
                "identity_import": {"migration_id": "synthetic-identity", "source_archive_sha256": "a" * 64,
                                    "mapping_sha256": "c" * 64},
                "lease_file": str(lease), "lease_identity": {"device": st.st_dev, "inode": st.st_ino}}
            target_path = write("target.json", target)
            counters = O.counter_payloads_for(F, fixture.plan, fixture.opening["payloads"]) if include else None
            projection = O.expected_readback(F, fixture.opening["payloads"], counter_payloads=counters)
            poststate = {**fixture.snapshot, O.COUNTER_TABLE: {"rows": 1 if include else 0,
                                                           "sha256": "e" * 64}}
            state, calls = fixture.snapshot, []

            def transport(connection, sql, **options):
                nonlocal state
                label = options["label"]
                calls.append(label)
                if label == "opening-prestate":
                    return F.canonical(state)
                if label == "opening-apply":
                    self.assertFalse(options["readonly"])
                    state = poststate
                    ack_projection = copy.deepcopy(projection)
                    if mutation is not None:
                        mutation(ack_projection)
                    return F.canonical({"status": status, "projection": ack_projection, "poststate": poststate}) + "\n"
                if label == "opening-poststate":
                    return F.canonical(state)
                self.fail("unexpected transport phase")

            def run(label, *, apply, approval=None):
                output = Path(temporary) / label
                output.mkdir(mode=0o700)
                F.IDENTITY.write_private(output, "financial-plan.private.json", F.canonical(fixture.plan) + "\n")
                F.IDENTITY.write_private(output, "manifest.private.json", "{}\n")
                args = SimpleNamespace(include_opening_counters=include, reuse_export_proof=proof_path,
                    target_receipt=target_path, reservation_evidence=evidence_path, reservation_review=review_path,
                    migration_id="synthetic-opening", opening_preview=not apply, apply_opening=apply,
                    reconcile_opening=False, opening_approval=approval, opening_attempt=None)
                return O.run(F, args, output=output, source=source, tables=tables, proof=proof,
                    plan=fixture.plan, planner_manifest={}, identities={71: 2},
                    identity_manifest=fixture.identity_manifest, source_path=source_path,
                    map_path=map_path, identity_manifest_path=manifest_path)

            with patch.object(O, "inspect_target", return_value=binding), \
                    patch.object(F.IDENTITY, "psql", side_effect=transport):
                preview = run("preview", apply=False)
                self.assertEqual(preview["status"], "opening-preview-review-required")
                self.assertEqual(calls, ["opening-prestate"])
                if include:
                    self.assertEqual(preview["scope"], O.COUNTER_SCOPE)
                if status == "replay-noop":
                    state = poststate
                approval = Path(temporary) / "preview/opening-approval-candidate.private.json"
                if mutation is None:
                    result = run("apply", apply=True, approval=approval)
                    self.assertEqual(result["status"], "opening-" + status)
                    if include:
                        self.assertEqual(result["historical_counter_rows_verified"], 1)
                        self.assertEqual(result["historical_counter_rows_newly_imported"], int(status == "applied"))
                    self.assertTrue((Path(temporary) / "apply/opening-success.private.json").is_file())
                else:
                    with self.assertRaisesRegex(F.MappingError, "opening_outcome_unknown_reconcile_original_attempt"):
                        run("apply", apply=True, approval=approval)
                    unknown = F.parse_json((Path(temporary) / "apply/opening-outcome-unknown.private.json").read_text())
                    self.assertEqual(unknown["error_code"], "opening_apply_projection_invalid")
                    self.assertFalse(unknown["automatic_retry"])
                    self.assertFalse((Path(temporary) / "apply/opening-success.private.json").exists())
                self.assertEqual(calls.count("opening-apply"), 1)

    def test_run_accepts_complete_counter_profile_ack_for_apply_and_exact_replay(self):
        for status in ("applied", "replay-noop"):
            with self.subTest(status=status):
                self.exercise_ack(status=status)

    def test_run_default_profile_still_accepts_wallet_only_ack(self):
        for status in ("applied", "replay-noop"):
            with self.subTest(status=status):
                self.exercise_ack(include=False, status=status)

    def test_run_missing_wrong_domain_and_tampered_counter_ack_stay_unknown(self):
        mutations = {
            "missing-counter-profile": lambda value: value.pop("historical_counters"),
            "wrong-scope-domain": lambda value: value.update(domain=O.DOMAIN),
            "changed-raw-counter": lambda value: value["historical_counters"][0].update(used_quota=-12),
            "counter-type-alias": lambda value: value["historical_counters"][0].update(request_count=True),
        }
        for name, mutation in mutations.items():
            with self.subTest(case=name):
                self.exercise_ack(mutation=mutation)


if __name__ == "__main__":
    unittest.main()
