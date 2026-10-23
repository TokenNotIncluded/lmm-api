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


# @mark migration.legacy-escrow-map role=test
class EscrowMappingTests(unittest.TestCase):
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
