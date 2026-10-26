"""Opening contract and driver tests. Database evidence lives in the PG suite."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

_SCRIPTS_DIR = str(Path(__file__).parent)
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

import financial_import as F
from test_migrate_legacy_financial import tables, user


def source_fixture():
    identities = {71: 1, 72: 2, 73: 3}
    archive_hash = F.sha(b"synthetic-frozen-archive")
    id_map = "".join(F.canonical({"source_user_id": uid, "target_user_id": uid,
        "target_personal_account_id": aid}) + "\n" for uid, aid in identities.items())
    identity_plan = F.IDENTITY.map_identity([
        {"id": uid, "username": "financial-fixture-" + str(uid), "password": "$2b$10$" + "." * 53,
         "display_name": "Financial fixture", "email": "", "setting": "{}", "role": 1,
         "status": 1, "auth_version": 1, "deleted_at": None, "trust_level_override": 0,
         "console_activated_at": 0} for uid in identities], [], source_sha256=archive_hash)
    manifest = {"format_version": 2, "scope": "identity-only", "source_archive_sha256": archive_hash,
        "mapping_sha256": identity_plan["mapping_sha256"], "source_target_id_map_sha256": F.sha(id_map.encode()),
        "identity_contract": F.IDENTITY.identity_contract(), "developer_access_policy": F.IDENTITY.DEVELOPER_POLICY,
        "initialized_core_seed_contract": F.IDENTITY.CORE_SEED_CONTRACT,
        "source_export_sha256": "e" * 64, "source_baseline_proof_sha256": "f" * 64,
        "summary": identity_plan["summary"],
        "import_tool_sha256": F.IDENTITY.sha256_file(Path(F.IDENTITY.__file__)),
        "import_contract_sha256": F.sha(F.IDENTITY.canonical(F.IDENTITY.current_contract()).encode())}
    provenance = F.PLANNER.bind_identity_provenance(manifest,
        map_file_sha256=F.sha(id_map.encode()), source_archive_sha256=archive_hash)
    plan = F.PLANNER.plan_financial(tables(users=[user(71, quota=-120), user(72, quota=0, aff_quota=-5),
        user(73, quota=43, aff_quota=0)], tokens=[], user_subscriptions=[], subscription_plans=[]),
        identities, source_sha256=archive_hash, identity_provenance=provenance)
    observation = F.PLANNER.source_observation({"started_utc": "20261001T000000Z"})
    reservations = {"source_archive_sha256": archive_hash, "mapping_sha256": plan["mapping_sha256"],
        "reviewed_evidence_sha256": "d" * 64,
        "positions": [{"source_user_id": uid, "reserved_units": 7 if uid == 71 else 0} for uid in identities]}
    review = {"format_version": 1, "scope": F.SCOPE, "decision": "approved-offline-opening",
        "credits_per_usd": 500000, "source_archive_sha256": archive_hash,
        "mapping_sha256": plan["mapping_sha256"], "identity_map_sha256": plan["identity_map_sha256"],
        "evidence_sha256": "e" * 64, "other_rights_policy": "remain-disabled-pending-separate-import"}
    files = {name: {"path": "/home/test/Backups/" + name, "sha256": "f" * 64} for name in F.SOURCE_FILES}
    files["archive"]["sha256"] = archive_hash
    files["identity_map"]["sha256"] = F.sha(id_map.encode())
    return {"plan": plan, "identities": identities, "identity_plan": identity_plan, "observation": observation,
        "reservations": reservations, "review": review, "files": files,
        "identity_run": {"migration_id": "synthetic-identity", "source_archive_sha256": archive_hash,
            "mapping_sha256": manifest["mapping_sha256"], "manifest": manifest,
            "completed_at": "2026-10-01T00:00:01+00:00"}}


def trust_seed_fixture():
    return {"singleton": True, "revision": 1, "spec": copy.deepcopy(F.IDENTITY.DEFAULT_TRUST_POLICY),
        "source_digest": F.sha(b"core-native-trust-policy-v1"), "source_cutoff": 1790812800,
        "native_user_floor": 1, "native_since": "2026-10-01T00:00:00+00:00"}


def snapshot_fixture(source):
    names = sorted(set(F.CHANGED_TABLES) | set(F.FRESH_EMPTY_TABLES) | {"core_trust_pricing.policies","core_identity.users", "core_identity.accounts",
        "core_migration.import_runs", "core_meta.runtime_contract", "core_meta.component_contract", "public.unrelated"})
    inventory = {"tables": names, "sequences": sorted(F.CHANGED_SEQUENCES)}
    state = {"inventory": inventory, "tables": {name: {"rows": 0, "sha256": F.sha(b"")} for name in names},
        "sequences": {name: {"last_value": 2 if "accounts" in name else 1,
            "is_called": "accounts" in name, "start": 1, "increment": 1, "min": 1,
            "max": 2**63 - 1, "cache": 1, "cycle": False} for name in F.CHANGED_SEQUENCES}}
    return {"state": state, "native_users": [{"id": uid, "personal_account_id": aid}
            for uid, aid in source["identities"].items()], "import_runs": [source["identity_run"]],
        "instance": {"database": "a03_test", "database_owner": "a03_owner", "role": "a03_owner",
            "session_user": "a03_operator", "effective_role_can_login": False,
            "database_oid": "12345", "system_identifier": "7777777", "listen_addresses": "",
            "postmaster_started_at": "2026-10-01 00:00:00+00", "server_instance_nonce": "synthetic-instance",
            "server_version_num": "180000", "port": 5432, "tcp_connection": False},
        "catalog_fingerprint_sha256": "a" * 64, "ownership_sha256": "b" * 64,
        "component_fingerprints": F.IDENTITY.current_contract()["component_fingerprints"],
        "trust_policy_seed": trust_seed_fixture()}


def material_fixture(source=None, snapshot=None, runtime_roles=None):
    source = source or source_fixture()
    snapshot = snapshot or snapshot_fixture(source)
    return F.prepare_material(source["plan"], migration_id="synthetic-financial",
        identities=source["identities"], observation=source["observation"], reservations=source["reservations"],
        review=source["review"], snapshot=snapshot, runtime_roles=runtime_roles or ["a03_runtime"], source_files=source["files"],
        target_contract={**F.IDENTITY.current_contract(), "catalog_fingerprint_sha256": snapshot["catalog_fingerprint_sha256"],
                         "trust_policy_seed": snapshot["trust_policy_seed"]})


def approval_fixture(material, sql):
    composed = F.escrow_profile(material)
    return {"format_version": 2 if composed else 1, "scope": material["scope"],
        "decision": "approve-offline-opening-with-legacy-escrow" if composed else "approve-offline-opening",
        "material_sha256": F.digest(material), "sql_sha256": F.sha(sql.encode()),
        "source_review_sha256": F.digest(material["source_review"])}


def escrow_material_fixture(source=None, snapshot=None, runtime_roles=None):
    """Synthetic SQL contract fixture, not original-source or drain evidence.

    The private-file suite exercises actual mapper/export rederivation. These
    hand-written source declarations deliberately make the money oracle easy:
    11+7 moves from clearing to the same payer's reserved, never from wallet.
    """
    source = copy.deepcopy(source or source_fixture())
    snapshot = copy.deepcopy(snapshot or snapshot_fixture(source))
    for name in (*F.ESCROW_TABLES, "core_billing.legacy_escrow_operations"):
        snapshot["state"]["tables"][name] = {"rows": 0, "sha256": F.sha(b"")}
    snapshot["state"]["inventory"]["tables"] = sorted(snapshot["state"]["tables"])
    for row in source["reservations"]["positions"]:
        row["reserved_units"] = 0
    plan = source["plan"]
    observed = source["observation"]["observed_at"]
    id_records = [{"source_user_id": uid, "target_user_id": uid, "target_personal_account_id": aid}
                  for uid, aid in source["identities"].items()]
    evidence = {"format_version": 1, "scope": "legacy-escrow-source-unit-evidence",
        "source_archive_sha256": plan["source_archive_sha256"], "mapping_sha256": plan["mapping_sha256"],
        "source_export_sha256": source["files"]["source_export"]["sha256"],
        "identity_records_sha256": F.digest(id_records), "parent_plan_sha256": "1" * 64,
        "parent_audit_row_sha256": "2" * 64, "unit": "credit_500k_usd", "credits_per_usd": 500000,
        "conversion_policy": "preserve-exact-parent-after-integers", "source_application_revision": "c" * 40,
        "source_policy_evidence_sha256": F.sha(b"SYNTHETIC escrow unit review\r\n")}
    claim = lambda cid, uid, aid, amount: {"source_challenge_id": cid, "participant_user_id": uid,
        "recipient_account_id": aid, "source_row_sha256": "d" * 64, "rights_status": "active_future_reward",
        "source_reward_quota": amount * 10, "payout_units": amount, "status": "accepted", "rejected_at": 0, "paid_at": 0}
    projects = [{"source_project_id": pid, "owner_user_id": 71, "payer_account_id": 1,
        "source_row_sha256": "e" * 64, "source_parent_plan_sha256": "1" * 64, "escrow_units": amount,
        "status": "published", "claims": [claim(cid, uid, aid, payout)], "disputes": []}
        for pid, amount, cid, uid, aid, payout in ((101, 11, 1001, 72, 2, 5), (102, 7, 1002, 73, 3, 4))]
    projects[0]["claims"].append({**claim(1003, 73, 3, 1), "status": "rejected",
        "rights_status": "historical_rejection_guard_only", "payout_units": None, "rejected_at": observed - 604801})
    projects[0]["disputes"].append({"source_dispute_id": 2001, "source_challenge_id": 1001,
        "source_row_sha256": "f" * 64, "status": "open", "reward_quota_snapshot": 500,
        "project_escrow_quota_snapshot": 1100, "tip_quota_snapshot": 0})
    batch = {"migration_id": "synthetic-financial", "source_archive_sha256": plan["source_archive_sha256"],
        "mapping_sha256": plan["mapping_sha256"], "unit": "credit_500k_usd", "observed_at": observed, "projects": projects}
    candidate = {"format_version": 1, "scope": "legacy-escrow-candidate-only",
        "economic_approval_verified": False, "approval_created": False, "target_writes": 0, "blockers": [],
        "source_unit_evidence_bound": True, "source_unit_authority_verified": False,
        "unit_evidence_sha256": F.digest(evidence), "identity_records_sha256": F.digest(id_records),
        "identity_map_sha256": plan["identity_map_sha256"], "source_export_sha256": evidence["source_export_sha256"],
        **{key: batch[key] for key in ("migration_id", "source_archive_sha256", "mapping_sha256", "observed_at")},
        "batch_candidate": batch}
    candidate["candidate_sha256"] = F.digest(candidate)
    process_hash = F.sha(b"SYNTHETIC drain process log\r\n")
    inflight = {"format_version": 1, "scope": "legacy-nonescrow-inflight-zero-evidence",
        "basis": "source-application-drain-and-durable-state-reconciliation", "snapshot_relation": "after-drain",
        **{key: plan[key] for key in ("source_archive_sha256", "mapping_sha256", "identity_map_sha256")},
        "source_export_sha256": evidence["source_export_sha256"], "source_application_revision": "c" * 40,
        "observed_at": observed, "drain_completed_at": observed - 1,
        "source_process_identity": {"pid": 99, "started_at": observed - 100, "binary_sha256": "b" * 64},
        "process_evidence_sha256": process_hash,
        "positions": copy.deepcopy(source["reservations"]["positions"]),
        "source_table_counts": {name: row["row_count"] for name, row in plan["source_evidence"].items()},
        "unresolved_operations": []}
    review = {**source["review"], "format_version": 2, "scope": F.ESCROW_SCOPE,
        "decision": "approved-offline-opening-with-legacy-escrow",
        "other_rights_policy": "all-other-rights-remain-disabled-pending-separate-import",
        "escrow_candidate_sha256": F.digest(candidate), "escrow_unit_evidence_sha256": F.digest(evidence),
        "inflight_evidence_sha256": F.digest(inflight), "escrow_policy": F.ESCROW_POLICY, "inflight_policy": F.INFLIGHT_POLICY,
        "evidence_sha256": F.sha(b"SYNTHETIC independent economic review\r\n")}
    files = copy.deepcopy(source["files"])
    for name in F.ESCROW_SOURCE_FILES:
        files[name] = {"path": "/home/test/Backups/" + name, "sha256": "f" * 64}
    files["source_review_evidence"]["sha256"] = review["evidence_sha256"]
    files["escrow_policy_evidence"]["sha256"] = evidence["source_policy_evidence_sha256"]
    files["inflight_process_evidence"]["sha256"] = process_hash
    return F.prepare_material(plan, migration_id="synthetic-financial", identities=source["identities"],
        observation=source["observation"], reservations=source["reservations"], review=review,
        snapshot=snapshot, runtime_roles=runtime_roles or ["a03_runtime"], source_files=files,
        target_contract={**F.IDENTITY.current_contract(), "catalog_fingerprint_sha256": snapshot["catalog_fingerprint_sha256"],
                         "trust_policy_seed": snapshot["trust_policy_seed"]},
        escrow_candidate=candidate, escrow_unit_evidence=evidence, inflight_evidence=inflight)


def reseal_escrow(material):
    candidate = material["escrow_candidate"]
    candidate["candidate_sha256"] = F.digest({k: v for k, v in candidate.items() if k not in ("candidate_sha256", "public_summary")})
    material["source_review"]["escrow_candidate_sha256"] = F.digest(candidate)
    material["source_review"]["inflight_evidence_sha256"] = F.digest(material["inflight_evidence"])
    material["escrow_batch"] = {**copy.deepcopy(candidate["batch_candidate"]),
        "review_proof_sha256": material["source_review"]["evidence_sha256"]}


class ComposedEscrowTests(unittest.TestCase):
    def test_separate_profile_and_approval_cannot_reuse_opening_authority(self):
        material = escrow_material_fixture()
        self.assertEqual((material["format_version"], material["scope"]), (3, F.ESCROW_SCOPE))
        sql = F.build_apply_sql(material)
        F.verify_approval(material, sql, approval_fixture(material, sql))
        old = approval_fixture(material, sql)
        old.update(format_version=1, scope=F.SCOPE, decision="approve-offline-opening")
        with self.assertRaisesRegex(F.Error, "approval_mismatch"):
            F.verify_approval(material, sql, old)
        opening = material_fixture()
        opening["escrow_batch"] = material["escrow_batch"]
        with self.assertRaisesRegex(F.Error, "composed_profile"):
            F.validate_material(opening)

    def test_candidate_is_not_itself_an_approval_and_escrow_never_debits_wallet(self):
        material = escrow_material_fixture()
        self.assertEqual(F.validate_escrow(material), {1: 18})
        self.assertEqual(material["opening"][0]["available_units"], -120)
        self.assertTrue(all(row["reserved_units"] == 0 for row in material["opening"]))
        sql = F.build_apply_sql(material)
        self.assertIn("PERFORM core_billing.import_legacy_escrow_batch(m->'escrow_batch');", sql)
        self.assertIn("financial_escrow_exact_readback_failed", sql)
        self.assertIn("ALTER SEQUENCE", sql)
        self.assertLess(sql.index("import_account_opening_counters(v_payload)"), sql.index("import_legacy_escrow_batch(m->'escrow_batch')"))

    def test_unknown_or_wrong_process_evidence_and_partial_coverage_fail(self):
        changes = [("snapshot_relation", "online-not-drained"), ("unresolved_operations", ["pending-refund"]),
                   ("drain_completed_at", 253402300799), ("positions", []), ("source_table_counts", {}),
                   ("source_process_identity", None), ("mapping_sha256", "0" * 64)]
        for key, value in changes:
            m = escrow_material_fixture()
            m["inflight_evidence"][key] = value
            reseal_escrow(m)
            with self.subTest(key=key), self.assertRaises(F.Error):
                F.build_apply_sql(m)
        for value in (None, True, 1):
            m = escrow_material_fixture()
            m["inflight_evidence"]["positions"][0]["reserved_units"] = value
            reseal_escrow(m)
            with self.subTest(reserved=value), self.assertRaises(F.Error):
                F.build_apply_sql(m)

    def test_review_and_actual_evidence_file_hashes_cannot_be_dangling(self):
        for name in ("source_review_evidence", "escrow_policy_evidence", "inflight_process_evidence"):
            m = escrow_material_fixture()
            m["source_files"][name]["sha256"] = "0" * 64
            with self.subTest(file=name), self.assertRaises(F.Error):
                F.build_apply_sql(m)
        for key in ("escrow_candidate_sha256", "escrow_unit_evidence_sha256", "inflight_evidence_sha256"):
            m = escrow_material_fixture()
            m["source_review"][key] = "0" * 64
            with self.subTest(review_key=key), self.assertRaises(F.Error):
                F.build_apply_sql(m)

    def test_business_shape_or_replayed_claim_errors_refuse_the_whole_batch(self):
        for field, value in (("recipient_account_id", 1), ("payout_units", True), ("paid_at", True),
                             ("source_reward_quota", 1.0), ("status", "approved")):
            m = escrow_material_fixture()
            m["escrow_candidate"]["batch_candidate"]["projects"][-1]["claims"][0][field] = value
            with self.subTest(field=field), self.assertRaises(F.Error):
                reseal_escrow(m)
                F.build_apply_sql(m)
        m = escrow_material_fixture()
        m["escrow_candidate"]["batch_candidate"]["projects"][-1]["claims"][0]["source_challenge_id"] = 1001
        reseal_escrow(m)
        with self.assertRaisesRegex(F.Error, "claim_duplicate"):
            F.build_apply_sql(m)

    def test_sequence_budget_includes_each_escrow_project(self):
        m = escrow_material_fixture()
        m["before_state"]["sequences"]["core_billing.ledger_journals_id_seq"]["max"] = 4
        with self.assertRaisesRegex(F.Error, "sequence_overflow"):
            F.build_apply_sql(m)

    def test_combined_clearing_prefix_is_checked_before_any_sql(self):
        m = escrow_material_fixture()
        opening = m["opening"][0]
        m["opening"] = [{**opening, "account_id": i + 1, "source_entity_key": f"legacy.users/{i + 71}",
                         "available_units": F.PREFLIGHT.JS_SAFE_MAX, "affiliate_units": 0} for i in range(1024)]
        m["escrow_candidate"]["batch_candidate"]["projects"][0]["escrow_units"] = 2048
        reseal_escrow(m)
        with self.assertRaisesRegex(F.Error, "clearing_prefix_overflow"):
            F.validate_escrow(m)


class MaterialTests(unittest.TestCase):
    def setUp(self):
        self.material = material_fixture()

    def test_actual_planner_contract_retains_debt_affiliate_and_counters(self):
        self.assertEqual(self.material["opening"][0]["available_units"], -120)
        self.assertEqual(self.material["opening"][0]["affiliate_units"], 38)
        self.assertEqual(self.material["opening"][0]["reserved_units"], 7)
        self.assertEqual(self.material["counters"][0]["used_quota"], 9001)
        self.assertEqual(self.material["counters"][0]["user_id"], 71)
        self.assertEqual(self.material["counters"][0]["account_id"], 1)

    def test_prepare_does_not_modify_arguments(self):
        source = source_fixture()
        snapshot = snapshot_fixture(source)
        before = copy.deepcopy((source, snapshot))
        material_fixture(source, snapshot)
        self.assertEqual((source, snapshot), before)

    def test_edited_payload_cannot_reuse_source_row_hash(self):
        self.material["opening"][0]["available_units"] += 1
        with self.assertRaisesRegex(F.Error, "adapter_mismatch"):
            F.build_apply_sql(self.material)

    def test_unknown_unit_and_unreviewed_rights_fail(self):
        for field, values in {"credits_per_usd": [1, True, 500000.0, None],
            "decision": ["pending"], "other_rights_policy": ["activate-all"]}.items():
            for value in values:
                with self.subTest(field=field, value=value):
                    m = copy.deepcopy(self.material)
                    m["source_review"][field] = value
                    with self.assertRaises(F.Error):
                        F.build_apply_sql(m)

    def test_wrong_mapping_or_source_fails(self):
        for key in ("source_archive_sha256", "mapping_sha256", "identity_map_sha256"):
            m = copy.deepcopy(self.material)
            m[key] = "0" * 64
            with self.subTest(key=key), self.assertRaises(F.Error):
                F.build_apply_sql(m)

    def test_changed_source_plan_fails(self):
        self.material["source_plan"]["wallet_positions"][0]["signed_available_units"] = 1
        with self.assertRaisesRegex(F.Error, "mapping_hash_mismatch"):
            F.validate_material(self.material)

    def test_identity_row_ownership_fails(self):
        self.material["native_users"][0]["personal_account_id"] = 4
        with self.assertRaisesRegex(F.Error, "coverage_mismatch"):
            F.validate_material(self.material)

    def test_wrong_identity_import_manifest_fails(self):
        self.material["identity_import"]["manifest"]["mapping_sha256"] = "0" * 64
        with self.assertRaisesRegex(F.Error, "import_run_mismatch"):
            F.validate_material(self.material)

    def test_network_runtime_or_login_owner_fails(self):
        for key, value in (("tcp_connection", True), ("listen_addresses", "127.0.0.1"),
                           ("effective_role_can_login", True), ("session_user", "a03_owner")):
            m = copy.deepcopy(self.material)
            m["target"]["instance"][key] = value
            with self.subTest(key=key), self.assertRaisesRegex(F.Error, "offline_owner"):
                F.build_apply_sql(m)

    def test_missing_target_boot_nonce_system_id_fails(self):
        for key in ("system_identifier", "postmaster_started_at", "server_instance_nonce"):
            m = copy.deepcopy(self.material)
            del m["target"]["instance"][key]
            with self.subTest(key=key), self.assertRaisesRegex(F.Error, "instance_incomplete"):
                F.build_apply_sql(m)

    def test_source_code_and_component_changes_fail(self):
        for key in ("financial_writer_sha256", "financial_preflight_helper_sha256"):
            m = copy.deepcopy(self.material)
            m["tool_source_hashes"][key] = "0" * 64
            with self.assertRaisesRegex(F.Error, "tool_source_changed"):
                F.build_apply_sql(m)
        self.material["target"]["component_fingerprints"]["ledger"] = "0" * 64
        with self.assertRaisesRegex(F.Error, "component_source_changed"):
            F.build_apply_sql(self.material)

    def test_old_material_or_relabelled_source_contract_cannot_be_reused(self):
        for change in ({"format_version": 1}, {"seed_contract": "identity-bound-zero-ledger-v1"}):
            material = copy.deepcopy(self.material)
            material.update(change)
            with self.subTest(change=change), self.assertRaisesRegex(F.Error, "material_contract_invalid"):
                F.build_apply_sql(material)
        material = copy.deepcopy(self.material)
        material["target"]["source_contract"]["installer_contract_sha256"] = "0" * 64
        with self.assertRaisesRegex(F.Error, "source_contract_changed"):
            F.build_apply_sql(material)

    def test_every_fresh26_empty_table_is_bound(self):
        for name in F.FRESH_EMPTY_TABLES:
            material = copy.deepcopy(self.material)
            material["before_state"]["tables"][name] = {"rows": 1, "sha256": "0" * 64}
            with self.subTest(table=name), self.assertRaisesRegex(F.Error, "fresh26_business_or_upgrade_records_not_empty"):
                F.build_apply_sql(material)

    def test_trust_seed_requires_owner_capture_and_exact_default_fields(self):
        material = copy.deepcopy(self.material)
        del material["target"]["trust_policy_seed"]
        with self.assertRaisesRegex(F.Error, "trust_seed_receipt"):
            F.build_apply_sql(material)
        material = copy.deepcopy(self.material)
        material["target"]["trust_policy_seed"]["spec"]["paid_activation_enabled"] = False
        with self.assertRaisesRegex(F.Error, "trust_seed_receipt_not_default"):
            F.build_apply_sql(material)
        sql = F.build_apply_sql(self.material)
        self.assertEqual(sql.count("financial_trust_policy_seed_mismatch"), 2)
        self.assertIn("financial_upgrade_receipts_not_empty", sql)
        self.assertNotIn("UPDATE core_trust_pricing", sql)

    def test_missing_inventory_or_duplicate_relation_fails(self):
        for inventory in ({"tables": [], "sequences": []},
                          {"tables": ["x.y", "x.y"], "sequences": []}):
            with self.assertRaises(F.Error):
                F.validate_inventory(inventory)

    def test_relation_names_cannot_be_sql(self):
        for name in ("core.foo;drop", "core.foo.bar", "core.\"foo", "core.foo\\bar", "core.foo\x00"):
            with self.assertRaises(F.Error):
                F.relation(name)

    def test_sequence_cache_cycle_boolean_or_overflow_rejected(self):
        for key, value in (("cache", 2), ("cache", True), ("increment", True), ("cycle", True),
                           ("last_value", 2**63 - 1), ("max", 5)):
            m = copy.deepcopy(self.material)
            m["before_state"]["sequences"][F.CHANGED_SEQUENCES[0]][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(F.Error):
                F.build_apply_sql(m)

    def test_sequence_last_available_ids_fit_exactly(self):
        seq = self.material["before_state"]["sequences"][F.CHANGED_SEQUENCES[0]]
        seq.update(last_value=2**63 - 13, is_called=True)
        F.validate_material(self.material)
        seq["last_value"] += 1
        with self.assertRaisesRegex(F.Error, "sequence_overflow"):
            F.validate_material(self.material)

    def test_source_file_inventory_and_archive_hash_are_required(self):
        del self.material["source_files"]["archive"]
        with self.assertRaisesRegex(F.Error, "source_files_incomplete"):
            F.validate_material(self.material)


class SQLTests(unittest.TestCase):
    def setUp(self):
        self.material = material_fixture()
        self.sql = F.build_apply_sql(self.material)

    def test_sql_is_deterministic_and_serializable_with_session_fence_first(self):
        self.assertEqual(self.sql, F.build_apply_sql(copy.deepcopy(self.material)))
        self.assertLess(self.sql.index("pg_try_advisory_lock"), self.sql.index("BEGIN ISOLATION LEVEL SERIALIZABLE"))
        self.assertIn("SET LOCAL synchronous_commit=on", self.sql)
        self.assertIn("ACCESS EXCLUSIVE MODE", self.sql)
        self.assertNotIn("setval(", self.sql)
        self.assertEqual(self.sql.count("ALTER SEQUENCE"), 2)

    def test_success_output_follows_commit_not_a_precommit_receipt(self):
        self.assertLess(self.sql.index("COMMIT;"), self.sql.rindex("'COMMITTED'"))
        self.assertIn("financial_exact_readback_failed", self.sql)
        self.assertIn("financial_ledger_reconciliation_failed", self.sql)
        self.assertIn("financial_unrelated_state_changed", self.sql)

    def test_sql_has_runtime_role_and_live_catalog_checks(self):
        for fragment in ("financial_offline_roles_mismatch", "pg_control_system()", "financial_schema_owner_mismatch",
                         "has_database_privilege", "pg_stat_clear_snapshot()", "financial_target_catalog_mismatch"):
            self.assertIn(fragment, self.sql)

    def test_marker_in_private_source_is_preserved_inside_outer_quote(self):
        marker = "END $lmm_financial$; DO $lmm_financial$ SELECT 'inert';"
        self.material["target"]["instance"]["server_instance_nonce"] = marker
        sql = F.build_apply_sql(self.material)
        self.assertIn(marker.replace("'", "''"), sql)
        self.assertTrue(sql.startswith("SET standard_conforming_strings=on"))

    def test_resolver_is_readonly_and_binds_original_attempt(self):
        sql = F.build_resolve_sql(self.material, "lmm-financial-" + "a" * 32)
        self.assertIn("BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY", sql)
        self.assertIn("financial_absence_not_proven", sql)
        self.assertNotIn("import_opening_balance(", sql)
        self.assertNotIn("INSERT INTO", sql)
        for app in ("", "lmm-financial-x", "lmm-financial-' OR 1=1"):
            with self.assertRaises(F.Error):
                F.build_resolve_sql(self.material, app)

    def test_exact_approval_not_boolean_equivalent_or_template(self):
        approval = approval_fixture(self.material, self.sql)
        F.verify_approval(self.material, self.sql, approval)
        for key, value in (("format_version", True), ("decision", "REVIEW_REQUIRED"),
                           ("sql_sha256", "0" * 64), ("source_review_sha256", "0" * 64)):
            with self.subTest(key=key), self.assertRaises(F.Error):
                F.verify_approval(self.material, self.sql, {**approval, key: value})


class DriverTests(unittest.TestCase):
    def setUp(self):
        self.material = material_fixture()
        self.sql = F.build_apply_sql(self.material)
        self.approval = approval_fixture(self.material, self.sql)
        self.connection = {"role": "a03_owner", "login_role": "a03_operator", "set_role": "a03_owner",
                           "database": "a03_test", "port": 5432}
        self.result = {"status": "COMMITTED", "migration_id": self.material["migration_id"],
                       "material_sha256": F.digest(self.material)}

    def execute(self, output, **kwargs):
        return F.execute(self.material, approval=self.approval, original_sql=self.sql,
                         connection=self.connection, output=output, **kwargs)

    def test_changed_approval_sql_or_files_never_reaches_database(self):
        with tempfile.TemporaryDirectory() as root, patch.object(F, "run_sql") as run:
            for sql in (self.sql + "\n", self.sql.replace("SERIALIZABLE", "READ COMMITTED")):
                with self.assertRaisesRegex(F.Error, "original_sql_changed"):
                    F.execute(self.material, approval=self.approval, original_sql=sql,
                              connection=self.connection, output=Path(root))
            with patch.object(F, "verify_source_files", side_effect=F.Error("source_file_changed")):
                with self.assertRaisesRegex(F.Error, "source_file_changed"):
                    self.execute(Path(root))
            run.assert_not_called()

    def test_file_and_attempt_are_written_before_single_database_call(self):
        with tempfile.TemporaryDirectory() as root, patch.object(F, "verify_source_files"), \
             patch.object(F.IDENTITY, "socket_connection", return_value={}):
            output = Path(root)
            def transport(*args, **kwargs):
                self.assertTrue((output / "attempt.private.json").is_file())
                self.assertEqual((output / "executed.private.sql").read_text(), self.sql)
                self.assertEqual((output / "attempt.private.json").stat().st_mode & 0o777, 0o600)
                return 0, F.canonical(self.result)
            with patch.object(F, "run_sql", side_effect=transport) as run:
                self.assertEqual(self.execute(output), self.result)
                run.assert_called_once()

    def test_nonzero_exit_lost_ack_bad_json_timeout_are_unknown_without_retry(self):
        values = [(1, ""), (0, ""), (0, "not-json"), (1, F.canonical(self.result)),
                  (0, F.canonical({**self.result, "migration_id": "wrong"})),
                  subprocess.TimeoutExpired("synthetic", 1), OSError("synthetic-private-error")]
        for value in values:
            with self.subTest(value=type(value).__name__), tempfile.TemporaryDirectory() as root, \
                 patch.object(F, "verify_source_files"), patch.object(F.IDENTITY, "socket_connection", return_value={}):
                with patch.object(F, "run_sql", **({"side_effect": value} if isinstance(value, Exception)
                                                  else {"return_value": value})) as run:
                    result = self.execute(Path(root))
                    self.assertEqual(result, {**self.result, "status": "UNKNOWN"})
                    run.assert_called_once()

    def test_transport_exceptions_save_private_evidence_and_remain_unknown(self):
        cases = [
            (subprocess.TimeoutExpired("synthetic", 1, output=b"partial stdout\n",
                                       stderr=b"partial stderr\n"),
             {"database.stdout.private.txt": "partial stdout\n",
              "database.stderr.private.txt": "partial stderr\n"}),
            (OSError("synthetic private failure"), {}),
        ]
        for error, streams in cases:
            with self.subTest(error=type(error).__name__), tempfile.TemporaryDirectory() as root, \
                 patch.object(F, "verify_source_files"), \
                 patch.object(F.IDENTITY, "socket_connection", return_value={}), \
                 patch.object(F.IDENTITY, "psql_options", return_value=""), \
                 patch.object(F.subprocess, "run", side_effect=error) as run:
                output = Path(root)
                self.assertEqual(self.execute(output)["status"], "UNKNOWN")
                run.assert_called_once()
                failure = F.PLANNER.parse_json((output / "database.failure.private.json").read_text())
                self.assertEqual(failure, {"type": type(error).__name__, "message": str(error)})
                for name, expected in streams.items():
                    self.assertEqual((output / name).read_text(), expected)
                for name in (*streams, "database.failure.private.json", "result.private.json",
                             "attempt.private.json", "executed.private.sql"):
                    self.assertEqual((output / name).stat().st_mode & 0o777, 0o600)
                self.assertEqual(F.PLANNER.parse_json((output / "result.private.json").read_text())["status"], "UNKNOWN")

    def test_transport_evidence_writes_are_independent_and_preserve_exception(self):
        names = ("database.stdout.private.txt", "database.stderr.private.txt",
                 "database.failure.private.json")
        for failed in names:
            for direct in (True, False):
                with self.subTest(failed=failed, direct=direct), tempfile.TemporaryDirectory() as root, \
                     patch.object(F, "verify_source_files"), \
                     patch.object(F.IDENTITY, "socket_connection", return_value={}), \
                     patch.object(F.IDENTITY, "psql_options", return_value=""):
                    output = Path(root)
                    original_write = F.write_private
                    attempted = []
                    def write(directory, name, value):
                        attempted.append(name)
                        if name == failed:
                            raise OSError("synthetic storage failure")
                        return original_write(directory, name, value)
                    error = subprocess.TimeoutExpired("synthetic", 1, output=b"synthetic stdout",
                                                       stderr=b"synthetic stderr")
                    with patch.object(F, "write_private", side_effect=write), \
                         patch.object(F.subprocess, "run", side_effect=error) as run:
                        if direct:
                            with self.assertRaises(subprocess.TimeoutExpired) as caught:
                                F.run_sql(self.connection, "synthetic sql", output, "synthetic")
                            self.assertIs(caught.exception, error)
                        else:
                            self.assertEqual(self.execute(output)["status"], "UNKNOWN")
                            self.assertEqual(F.PLANNER.parse_json(
                                (output / "result.private.json").read_text())["status"], "UNKNOWN")
                        run.assert_called_once()
                    self.assertEqual([name for name in attempted if name in names], list(names))
                    for name in names:
                        self.assertEqual((output / name).exists(), name != failed)
                        if name != failed:
                            self.assertEqual((output / name).stat().st_mode & 0o777, 0o600)

    def test_transport_evidence_does_not_swallow_safety_errors(self):
        error = subprocess.TimeoutExpired("synthetic", 1, output=b"synthetic stdout")
        with tempfile.TemporaryDirectory() as root, \
             patch.object(F.IDENTITY, "socket_connection", return_value={}), \
             patch.object(F.IDENTITY, "psql_options", return_value=""), \
             patch.object(F.subprocess, "run", side_effect=error), \
             patch.object(F, "write_private", side_effect=F.Error("synthetic safety refusal")):
            with self.assertRaisesRegex(F.Error, "synthetic safety refusal"):
                F.run_sql(self.connection, "synthetic sql", Path(root), "synthetic")

    def test_resolve_only_uses_matching_original_identity(self):
        attempt = {"operation": "apply", "migration_id": self.material["migration_id"],
            "material_sha256": F.digest(self.material), "sql_sha256": F.sha(self.sql.encode()),
            "application_name": "lmm-financial-" + "a" * 32}
        with tempfile.TemporaryDirectory() as root, patch.object(F, "verify_source_files"), \
             patch.object(F.IDENTITY, "socket_connection", return_value={}), \
             patch.object(F, "run_sql", return_value=(0, F.canonical({**self.result, "status": "NOT_APPLIED"}))) as run:
            result = self.execute(Path(root), original_attempt=attempt)
            self.assertEqual(result["status"], "NOT_APPLIED")
            self.assertIn("READ ONLY", run.call_args.args[1])
        with tempfile.TemporaryDirectory() as root, patch.object(F, "verify_source_files"), \
             patch.object(F.IDENTITY, "socket_connection", return_value={}), patch.object(F, "run_sql") as run:
            attempt["migration_id"] = "another-id"
            with self.assertRaisesRegex(F.Error, "original_attempt_material_mismatch"):
                self.execute(Path(root), original_attempt=attempt)
            run.assert_not_called()

    def test_unknown_remains_unknown_when_private_result_cannot_be_saved(self):
        original_write = F.write_private
        def write(directory, name, value):
            if name == "result.private.json":
                raise OSError("synthetic disk full")
            return original_write(directory, name, value)
        with tempfile.TemporaryDirectory() as root, patch.object(F, "verify_source_files"), \
             patch.object(F.IDENTITY, "socket_connection", return_value={}), \
             patch.object(F, "run_sql", return_value=(0, F.canonical(self.result))), \
             patch.object(F, "write_private", side_effect=write):
            self.assertEqual(self.execute(Path(root))["status"], "UNKNOWN")


if __name__ == "__main__":
    unittest.main()
