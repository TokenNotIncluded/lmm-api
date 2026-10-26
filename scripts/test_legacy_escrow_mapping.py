"""Synthetic mapper tests, not real-money import or independent acceptance."""

import copy
from decimal import Decimal
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))
import test_legacy_bounty_basis as fixtures
import legacy_escrow_mapping as M

F, B = fixtures.F, fixtures.B


def reseal(plan):
    plan.pop("mapping_sha256", None)
    plan.pop("public_summary", None)
    plan["mapping_sha256"] = F.digest(plan)
    plan["public_summary"] = {"mapping_sha256": plan["mapping_sha256"]}


def data(parent=None, tables=None):
    if parent is None:
        parent, tables = fixtures.fixture()
    source = fixtures.args(parent, tables)
    plan = source["plan"]
    plan.update(source_archive_sha256="a" * 64, identity_map_sha256=F.digest(source["identities"]))
    reseal(plan)
    return dict(tables=source["tables"], plan=plan,
        identity_records=[{"source_user_id": uid, "target_user_id": uid + 10000,
            "target_personal_account_id": aid} for uid, aid in source["identities"].items()],
        migration_id="synthetic-composed-v1", observed_at=1000000,
        source_proof={"source_archive_sha256": "a" * 64, "source_export_sha256": "b" * 64,
            "selected_table_counts": {name: len(rows) for name, rows in source["tables"].items()}})


def evidence(args):
    accounts = {row["source_user_id"]: row["target_personal_account_id"] for row in args["identity_records"]}
    binding = B.build(F, tables=args["tables"], identities=accounts, plan=args["plan"])["parent_plan_bindings"][0]
    return {"format_version": 1, "scope": "legacy-escrow-source-unit-evidence",
        "source_archive_sha256": args["plan"]["source_archive_sha256"],
        "mapping_sha256": args["plan"]["mapping_sha256"],
        "source_export_sha256": args["source_proof"]["source_export_sha256"],
        "identity_records_sha256": F.digest(sorted(args["identity_records"], key=lambda row: row["source_user_id"])),
        "parent_plan_sha256": binding["plan_sha256"],
        "parent_audit_row_sha256": binding["source_audit_row_sha256"],
        "unit": M.UNIT, "credits_per_usd": 500000, "conversion_policy": M.POLICY,
        "source_application_revision": "c" * 40, "source_policy_evidence_sha256": "d" * 64}


def five_eight():
    parent, _ = fixtures.fixture()
    projects = [fixtures.make_source(B.BOUNTY_TABLES[0], 10 + i, owner_user_id=1,
        status="published", escrow_quota=1000, reward_quota=200, net_reward_quota=100,
        reward_slots=10, created_at=1) for i in range(5)]
    claims = [fixtures.make_source(B.BOUNTY_TABLES[1], 100 + i, project_id=10 + i % 5,
        participant_user_id=2, status="accepted", reward_quota=100, created_at=2) for i in range(8)]
    guards = [fixtures.make_source(B.BOUNTY_TABLES[1], 200 + i, project_id=10 + i,
        participant_user_id=2, status="rejected", reward_quota=100, rejected_at=1, created_at=2) for i in range(3)]
    snapshot = {"snapshot_at": parent["snapshot_at"], "entities": {
        "bounty_projects": projects, "bounty_challenges": claims + guards, "bounty_disputes": []}}
    parent["entity_updates"] = B.make_entities(snapshot, {1, 2}, lambda n: (n + 5) // 10, include_bounties=True)
    fixtures.seal(parent)
    tables = {table: [] for table in B.BOUNTY_TABLES}
    for e in parent["entity_updates"]:
        tables[e["table"]].append({**e["source"], **{k: v["after_credit"] for k, v in e["updates"].items()}})
    tables[B.BOUNTY_TABLES[1]].extend(fixtures.make_source(B.BOUNTY_TABLES[1], 300 + i,
        project_id=10 + i % 5, participant_user_id=2, status="approved", reward_quota=100,
        paid_at=100, created_at=2) for i in range(19))
    return data(parent, tables)


def with_dispute(*, challenge=2, status="open"):
    parent, tables = fixtures.fixture()
    dispute = fixtures.make_source(B.BOUNTY_TABLES[2], 4, project_id=1,
        challenge_id=challenge, opened_by_user_id=2, against_user_id=1, status=status,
        reward_quota_snapshot=100, project_escrow_quota_snapshot=1000,
        challenge_status_snapshot="submitted" if challenge == 2 else "rejected", created_at=3)
    snapshot = {"snapshot_at": parent["snapshot_at"], "entities": {
        "bounty_projects": [], "bounty_challenges": [], "bounty_disputes": [dispute]}}
    for entry in parent["entity_updates"]:
        key = "bounty_projects" if entry["table"] == B.BOUNTY_TABLES[0] else "bounty_challenges"
        snapshot["entities"][key].append(entry["source"])
    parent["entity_updates"] = B.make_entities(snapshot, {1, 2}, lambda n: (n + 5) // 10, include_bounties=True)
    parent["other_credit_bases"] = B.prepare_disputes(snapshot, {1, 2}, lambda n: (n + 5) // 10, include=True)
    fixtures.seal(parent)
    tables = {table: [] for table in B.BOUNTY_TABLES}
    for entry in parent["entity_updates"]:
        tables[entry["table"]].append({**entry["source"], **{key: value["after_credit"] for key, value in entry["updates"].items()}})
    return data(parent, tables)


def post_parent_data():
    """Five/eight historical shape plus the ninth, freshly accepted right."""
    args = five_eight()
    parent = args["tables"]["wallet_credit_rebases"][0]["plan"]
    created = parent["snapshot_at"] + 10
    args["tables"][B.BOUNTY_TABLES[1]].append(fixtures.make_source(
        B.BOUNTY_TABLES[1], 999, project_id=10, participant_user_id=3,
        status="accepted", reward_quota=10, created_at=created,
        accepted_at=created, updated_at=created, github_handle="synthetic-contributor",
        issue_url="", pull_request_url="", submission_note="", review_note="",
        encrypted_review_message="", contributor_rating_score=0,
        contributor_rating_comment="", contributor_rated_at=0,
        owner_rating_score=0, owner_rating_comment="", owner_rated_at=0,
        owner_rating_overturned=False))
    args = data(parent, args["tables"])
    args["observed_at"] = created + 10
    args["identity_records"].append({"source_user_id": 3, "target_user_id": 10003,
        "target_personal_account_id": 103})
    args["plan"]["identity_map_sha256"] = F.digest({1: 101, 2: 102, 3: 103})
    reseal(args["plan"])
    return args


def refresh_source(args):
    """Rebind changed synthetic facts without changing the historical parent."""
    for table, rows in args["tables"].items():
        item = args["plan"]["source_evidence"][table]
        item.update(row_count=len(rows), row_hashes=[F.digest(row) for row in rows])
        if table in B.BOUNTY_TABLES:
            item["source_records"] = [{"source_primary_key": {"id": row["id"]},
                "source_export_ordinal": i, "source_row_sha256": F.digest(row)} for i, row in enumerate(rows)]
        args["source_proof"]["selected_table_counts"][table] = len(rows)
    reseal(args["plan"])


def post_parent_evidence(args):
    unit = evidence(args)
    unit["source_application_revision"] = M.POST_PARENT_REVISION
    request = M.build(F, **args, unit_evidence=unit)["post_parent_claim_evidence_request"]
    return unit, request


# @mark migration.legacy-escrow-map role=test
class EscrowMappingTests(unittest.TestCase):
    def test_post_parent_accepted_claim_requires_explicit_bound_source_facts(self):
        args = post_parent_data(); unit, proof = post_parent_evidence(args)
        missing = M.build(F, **args, unit_evidence=unit)
        self.assertIsNone(missing["batch_candidate"])
        self.assertIn("escrow_post_parent_claim_evidence_required", missing["blockers"])
        before = copy.deepcopy((args, unit, proof))
        result = M.build(F, **args, unit_evidence=unit, post_parent_claim_evidence=proof)
        self.assertEqual((args, unit, proof), before)
        self.assertEqual(result["blockers"], [])
        self.assertEqual((result["project_count"], result["claim_count"],
            result["accepted_unpaid_claim_count"], result["historical_rejection_guard_count"]), (5, 12, 9, 3))
        self.assertEqual(result["lineage"], missing["lineage"])
        self.assertEqual(result["lineage"]["lineage_status"], "not-proven-exact-parent-after")
        new = [c for p in result["batch_candidate"]["projects"] for c in p["claims"]
               if c["source_challenge_id"] == 999][0]
        self.assertEqual((new["rights_status"], new["payout_units"], new["participant_user_id"]),
                         ("active_future_reward", 10, 10003))
        self.assertEqual(new["source_row_sha256"], F.digest(args["tables"][B.BOUNTY_TABLES[1]][-1]))
        self.assertEqual(result["post_parent_claim_evidence"], proof)
        self.assertFalse(result["economic_approval_verified"])
        self.assertNotIn("review_proof_sha256", result["batch_candidate"])
        budget = proof["project_budgets"][0]
        project = next(p for p in result["projects"] if p["source_project_id"] == budget["source_project_id"])
        self.assertEqual(budget["source_integer_owed"], project["source_integer_owed"])
        self.assertEqual(budget["source_integer_owed"] + budget["source_integer_uncommitted"],
                         project["source_escrow_quota"])
        self.assertEqual(result, M.build(F, **args, unit_evidence=unit, post_parent_claim_evidence=proof))

    def test_post_parent_source_facts_cannot_prove_an_unknown_unit(self):
        args = post_parent_data()
        request = M.build(F, **args)["post_parent_claim_evidence_request"]
        self.assertIsNone(request["unit_evidence_sha256"])
        result = M.build(F, **args, post_parent_claim_evidence=request)
        self.assertIsNone(result["batch_candidate"])
        self.assertFalse(result["post_parent_claim_evidence_bound"])
        self.assertIn("escrow_source_unit_evidence_required", result["blockers"])
        self.assertEqual((result["claim_count"], result["accepted_unpaid_claim_count"]), (12, 9))
        self.assertTrue(all(c["payout_units"] is None for p in result["projects"] for c in p["candidate"]["claims"]))

    def test_post_parent_evidence_rejects_repeated_weakened_or_changed_facts(self):
        args = post_parent_data(); unit, proof = post_parent_evidence(args)
        changes = [(key, "0" * 64) for key in ("source_archive_sha256", "source_export_sha256", "mapping_sha256",
            "identity_records_sha256", "parent_plan_sha256", "parent_audit_row_sha256", "unit_evidence_sha256",
            "source_policy_evidence_sha256")]
        changes += [("format_version", True), ("contract", "timestamps-only"), ("economic_approval_verified", True),
                    ("source_application_revision", "c" * 40), ("observed_at", args["observed_at"] + 1)]
        for key, value in changes:
            bad = copy.deepcopy(proof); bad[key] = value
            with self.subTest(key=key), self.assertRaisesRegex(F.MappingError, "post_parent_claim_evidence_mismatch"):
                M.build(F, **args, unit_evidence=unit, post_parent_claim_evidence=bad)
        for field in ("source_reward_quota", "source_owner_user_id", "source_participant_user_id", "source_project_id",
                      "source_challenge_id", "created_at", "source_row_sha256", "source_project_row_sha256", "status"):
            bad = copy.deepcopy(proof); bad["claims"][0][field] = "tampered"
            with self.subTest(field=field), self.assertRaisesRegex(F.MappingError, "post_parent_claim_evidence_mismatch"):
                M.build(F, **args, unit_evidence=unit, post_parent_claim_evidence=bad)
        for field in ("source_integer_owed", "source_integer_uncommitted", "source_escrow_quota",
                      "complete_project_claim_rows_sha256", "complete_project_dispute_rows_sha256"):
            bad = copy.deepcopy(proof); bad["project_budgets"][0][field] = 0
            with self.subTest(field=field), self.assertRaisesRegex(F.MappingError, "post_parent_claim_evidence_mismatch"):
                M.build(F, **args, unit_evidence=unit, post_parent_claim_evidence=bad)
        for mode in ("duplicate", "omit", "extra"):
            bad = copy.deepcopy(proof)
            if mode == "duplicate": bad["claims"].append(copy.deepcopy(bad["claims"][0]))
            elif mode == "omit": bad["claims"] = []
            else: bad["approved"] = True
            with self.subTest(mode=mode), self.assertRaisesRegex(F.MappingError, "post_parent_claim_evidence_mismatch"):
                M.build(F, **args, unit_evidence=unit, post_parent_claim_evidence=bad)

    def test_post_parent_contract_rejects_later_states_timing_rewards_and_negative_values(self):
        changes = (("status", "submitted"), ("status", "rejected"), ("created_at", 1000000),
            ("created_at", 1000030), ("accepted_at", 0), ("updated_at", 1000011),
            ("submitted_at", 1), ("reviewed_at", 1), ("rejected_at", 1), ("tip_quota", 1),
            ("reward_quota", 11), ("reward_quota", 0), ("owner_rating_score", 1),
            ("owner_rating_overturned", True), ("pull_request_url", "https://github.com/a/b/pull/1"),
            ("github_handle", "@synthetic"), ("github_handle", "synthetic "))
        for field, value in changes:
            args = post_parent_data(); args["tables"][B.BOUNTY_TABLES[1]][-1][field] = value; refresh_source(args)
            with self.subTest(field=field, value=value):
                result = M.build(F, **args, unit_evidence=evidence(args))
                self.assertIsNone(result["batch_candidate"])
                self.assertIn("escrow_exact_parent_after_lineage_required", result["blockers"])
        for field in ("reward_quota", "tip_quota", "accepted_at", "created_at", "updated_at"):
            args = post_parent_data(); args["tables"][B.BOUNTY_TABLES[1]][-1][field] = -1; refresh_source(args)
            with self.subTest(negative=field), self.assertRaises(F.MappingError): M.build(F, **args)
        args = post_parent_data(); args["tables"][B.BOUNTY_TABLES[1]][-1]["participant_user_id"] = 1; refresh_source(args)
        with self.assertRaisesRegex(F.MappingError, "claim_payer_is_recipient"): M.build(F, **args)

    def test_post_parent_proof_cannot_rebind_changed_source_parent_or_owner(self):
        original = post_parent_data(); unit, proof = post_parent_evidence(original)
        for table, index, field, value in ((B.BOUNTY_TABLES[0], 0, "owner_user_id", 3),
                (B.BOUNTY_TABLES[0], 0, "escrow_quota", 101),
                (B.BOUNTY_TABLES[1], -1, "participant_user_id", 2),
                (B.BOUNTY_TABLES[1], -1, "reward_quota", 11)):
            args = copy.deepcopy(original); args["tables"][table][index][field] = value; refresh_source(args)
            with self.subTest(field=field), self.assertRaises(F.MappingError):
                M.build(F, **args, unit_evidence=evidence(args), post_parent_claim_evidence=proof)
        wrong = copy.deepcopy(unit); wrong["source_application_revision"] = "c" * 40
        request = M.build(F, **original, unit_evidence=wrong)["post_parent_claim_evidence_request"]
        with self.assertRaisesRegex(F.MappingError, "post_parent_source_revision_unsupported"):
            M.build(F, **original, unit_evidence=wrong, post_parent_claim_evidence=request)

    def test_post_parent_overcommitment_and_duplicate_sources_never_produce_batch(self):
        args = post_parent_data()
        # A fully source-bound fact attachment still cannot erase existing owed
        # amounts or spend the same project budget twice.
        args["tables"][B.BOUNTY_TABLES[1]][0]["reward_quota"] = 100
        refresh_source(args)
        unit, proof = post_parent_evidence(args)
        result = M.build(F, **args, unit_evidence=unit, post_parent_claim_evidence=proof)
        self.assertIsNone(result["batch_candidate"])
        self.assertIn("escrow_cannot_cover_unpaid_commitments", result["blockers"])
        self.assertLess(proof["project_budgets"][0]["source_integer_uncommitted"], 0)
        args = post_parent_data(); args["tables"][B.BOUNTY_TABLES[1]].append(copy.deepcopy(args["tables"][B.BOUNTY_TABLES[1]][-1]))
        refresh_source(args)
        with self.assertRaisesRegex(F.MappingError, "id_duplicate"): M.build(F, **args)

    def test_post_parent_same_participant_retry_and_slot_contracts_stay_distinct(self):
        args = post_parent_data()
        created = args["tables"][B.BOUNTY_TABLES[1]][-1]["created_at"]
        args["tables"][B.BOUNTY_TABLES[1]].append(fixtures.make_source(B.BOUNTY_TABLES[1], 888,
            project_id=10, participant_user_id=3, status="rejected", reward_quota=10,
            rejected_at=created - 1, created_at=2))
        args["tables"][B.BOUNTY_TABLES[2]].append(fixtures.make_source(B.BOUNTY_TABLES[2], 777,
            project_id=10, challenge_id=888, status="resolved_paid", created_at=created - 1))
        refresh_source(args)
        result = M.build(F, **args)
        # Source retry SQL exempts only resolved_denied here. A resolved_paid
        # marker does not make a still-rejected row a valid new acceptance.
        self.assertNotIn("post_parent_claim_evidence_request", result)
        self.assertIsNone(result["batch_candidate"])

    def test_unknown_unit_never_produces_batch_or_approval(self):
        result = M.build(F, **data())
        self.assertIsNone(result["batch_candidate"])
        self.assertIn("escrow_source_unit_evidence_required", result["blockers"])
        self.assertIsNone(result["projects"][0]["candidate"]["escrow_units"])
        self.assertEqual(result["projects"][0]["source_escrow_quota"], 100)
        self.assertEqual(result["public_summary"]["status"], "blocked-for-source-evidence")

    def test_structured_bound_evidence_produces_review_only_batch_without_rescaling(self):
        args = data(); before = copy.deepcopy(args)
        result = M.build(F, **args, unit_evidence=evidence(args))
        self.assertEqual(args, before)
        self.assertEqual(result["blockers"], [])
        self.assertEqual(result["public_summary"]["status"], "candidate-requires-economic-review")
        batch = result["batch_candidate"]
        self.assertNotIn("review_proof_sha256", batch)
        self.assertEqual(set(batch), {"migration_id", "source_archive_sha256", "mapping_sha256", "unit", "observed_at", "projects"})
        project = batch["projects"][0]
        self.assertEqual((project["owner_user_id"], project["payer_account_id"], project["escrow_units"]), (10001, 101, 100))
        self.assertEqual(project["source_parent_plan_sha256"], result["source_parent_plan_bindings"][0]["plan_sha256"])
        active, guard = project["claims"]
        self.assertEqual((active["participant_user_id"], active["recipient_account_id"], active["payout_units"]), (10002, 102, 10))
        self.assertEqual((guard["source_reward_quota"], guard["payout_units"]), (100, None))
        self.assertEqual(result["wallet_debit_policy"], "restore-clearing-to-reserved-without-wallet-debit")
        for flag in ("approval_created", "economic_approval_verified", "import_payload_created", "current_amounts_rescaled",
                     "dispute_snapshots_rescaled", "source_unit_authority_verified", "inflight_reservations_verified"):
            self.assertIs(result[flag], False, flag)
        self.assertEqual(result["target_writes"], 0)
        self.assertNotIn("projects", result["public_summary"])

    def test_five_projects_same_payer_eight_unpaid_and_nineteen_paid_not_recredited(self):
        args = five_eight()
        result = M.build(F, **args, unit_evidence=evidence(args))
        self.assertEqual(result["blockers"], [])
        self.assertEqual((result["project_count"], result["accepted_unpaid_claim_count"],
            result["historical_rejection_guard_count"], result["skipped_paid_claim_count"]), (5, 8, 3, 19))
        self.assertEqual({p["payer_account_id"] for p in result["batch_candidate"]["projects"]}, {101})
        self.assertEqual(result["claim_count"], 11)
        self.assertFalse(any(c["paid_at"] for p in result["batch_candidate"]["projects"] for c in p["claims"]))

    def test_evidence_is_bound_to_source_export_plan_parent_and_all_identity_fields(self):
        args = data()
        for field in ("source_archive_sha256", "source_export_sha256", "mapping_sha256", "identity_records_sha256",
                      "parent_plan_sha256", "parent_audit_row_sha256"):
            value = evidence(args); value[field] = "0" * 64
            with self.subTest(field=field), self.assertRaisesRegex(F.MappingError, "unit_evidence_binding_mismatch"):
                M.build(F, **args, unit_evidence=value)
        for field, value in (("format_version", True), ("credits_per_usd", True), ("credits_per_usd", Decimal(500000)),
                             ("unit", "USD"), ("conversion_policy", "scale-by-parent-factor"),
                             ("source_application_revision", "fake"), ("source_policy_evidence_sha256", "")):
            unit = evidence(args); unit[field] = value
            with self.subTest(field=field), self.assertRaises(F.MappingError):
                M.build(F, **args, unit_evidence=unit)
        for unit in (True, "credit_500k_usd", {}, {"unit": M.UNIT}):
            with self.subTest(unit=unit), self.assertRaises(F.MappingError):
                M.build(F, **args, unit_evidence=unit)

    def test_identity_duplicates_owner_changes_and_noninteger_ids_refuse(self):
        for field in ("source_user_id", "target_user_id", "target_personal_account_id"):
            args = data(); args["identity_records"][1][field] = args["identity_records"][0][field]
            with self.subTest(field=field), self.assertRaises(F.MappingError): M.build(F, **args)
            for value in (True, 1.0, "1", 0, 2**63):
                args = data(); args["identity_records"][0][field] = value
                with self.subTest(field=field, value=value), self.assertRaises(F.MappingError): M.build(F, **args)
        args = data(); args["identity_records"][0]["target_personal_account_id"] = 999
        with self.assertRaisesRegex(F.MappingError, "identity_plan_mismatch"): M.build(F, **args)

    def test_missing_or_inexact_source_proof_refuses(self):
        args = data(); args["source_proof"]["source_archive_sha256"] = "0" * 64
        with self.assertRaisesRegex(F.MappingError, "proof_archive_mismatch"): M.build(F, **args)
        for value in (True, Decimal(1), None):
            args = data(); args["source_proof"]["selected_table_counts"][B.BOUNTY_TABLES[0]] = value
            with self.subTest(value=value), self.assertRaises(F.MappingError): M.build(F, **args)
        args = data(); del args["tables"][B.BOUNTY_TABLES[2]]
        with self.assertRaisesRegex(F.MappingError, "table_coverage_incomplete"): M.build(F, **args)

    def test_changed_current_parent_after_stays_blocked_not_rescaled(self):
        parent, tables = fixtures.fixture()
        tables[B.BOUNTY_TABLES[0]][0]["escrow_quota"] += 1
        args = data(parent, tables)
        result = M.build(F, **args, unit_evidence=evidence(args))
        self.assertIsNone(result["batch_candidate"])
        self.assertIn("escrow_exact_parent_after_lineage_required", result["blockers"])
        self.assertFalse(result["current_amounts_rescaled"])

    def test_duplicate_project_and_cross_project_claim_refuse(self):
        parent, tables = fixtures.fixture(); tables[B.BOUNTY_TABLES[0]].append(copy.deepcopy(tables[B.BOUNTY_TABLES[0]][0]))
        with self.assertRaisesRegex(F.MappingError, "id_duplicate"): M.build(F, **data(parent, tables))
        parent, tables = fixtures.fixture(); tables[B.BOUNTY_TABLES[1]][0]["project_id"] = 999
        result = M.build(F, **data(parent, tables))
        self.assertIsNone(result["batch_candidate"])
        self.assertIn("escrow_continuing_claim_outside_funded_projects", result["blockers"])

    def test_capture_observation_must_not_precede_parent_or_accept_wrong_types(self):
        args = data(); args["observed_at"] -= 1
        result = M.build(F, **args)
        self.assertIn("escrow_observation_precedes_parent_snapshot", result["blockers"])
        for value in (True, Decimal(1000000), 0, 253402300800):
            args = data(); args["observed_at"] = value
            with self.subTest(value=value), self.assertRaises(F.MappingError): M.build(F, **args)

    def test_open_dispute_preserves_historical_snapshots_without_rescaling(self):
        args = with_dispute()
        result = M.build(F, **args, unit_evidence=evidence(args))
        self.assertEqual(result["blockers"], [])
        project = result["batch_candidate"]["projects"][0]
        self.assertEqual(project["escrow_units"], 100)
        self.assertEqual(project["disputes"][0]["reward_quota_snapshot"], 100)
        self.assertEqual(project["disputes"][0]["project_escrow_quota_snapshot"], 1000)
        self.assertEqual(project["claims"][0]["payout_units"], 10)
        self.assertFalse(result["dispute_snapshots_rescaled"])

    def test_open_dispute_on_rejected_claim_preserves_current_reward_basis(self):
        args = with_dispute(challenge=3)
        result = M.build(F, **args, unit_evidence=evidence(args))
        self.assertEqual(result["blockers"], [])
        rejected = result["batch_candidate"]["projects"][0]["claims"][1]
        self.assertEqual((rejected["rights_status"], rejected["payout_units"]), ("active_future_reward", 10))

    def test_resolved_paid_dispute_is_not_a_new_unpaid_right(self):
        args = with_dispute(challenge=3, status="resolved_paid")
        result = M.build(F, **args, unit_evidence=evidence(args))
        self.assertIsNone(result["batch_candidate"])
        self.assertIn("escrow_resolved_paid_dispute_not_new_payable_right", result["blockers"])

    def test_candidate_hash_binds_exact_private_content(self):
        result = M.build(F, **data())
        self.assertEqual(result["candidate_sha256"], F.digest({k: v for k, v in result.items()
            if k not in ("candidate_sha256", "public_summary")}))


if __name__ == "__main__":
    unittest.main()
