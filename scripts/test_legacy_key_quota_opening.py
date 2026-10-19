#!/usr/bin/env python3
"""Pure offline unit tests for legacy key quota opening writer.

Strictly offline: no PostgreSQL, no Docker, no psql, no external network.
Verifies canonical material validation, receipt bindings, duplicate rejection,
boundary checks, and generated SQL structure/invariants.
"""
from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
import sys
import unittest

_SCRIPTS_DIR = str(Path(__file__).resolve().parent)
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

import legacy_key_quota_opening as KQ


def sample_fixture():
    token = {
        "id": 11,
        "user_id": 71,
        "used_quota": 10,
        "remain_quota": 90,
        "unlimited_quota": False,
        "key": "lmmk_" + "a" * 64,
        "status": 1,
    }
    secret_sha = hashlib.sha256(token["key"].encode("utf-8")).hexdigest()
    binding = {
        "source_key_id": 11,
        "target_credential_id": 201,
        "target_user_id": 71,
        "target_account_id": 1,
        "source_secret_sha256": secret_sha,
    }
    receipt = {
        "scope": "a02-credential-mapping-receipt",
        "source_archive_sha256": "a" * 64,
        "records": [binding],
    }
    user_map = {71: {"user_id": 71, "account_id": 1}}
    plan = KQ.build_plan_from_tokens(
        [token], user_map, receipt,
        migration_id="independent-key-draft",
        source_archive_sha256="a" * 64
    )
    return plan, token, receipt, user_map


def sample_target():
    return {
        "connection": {
            "role": "a03_owner",
            "login_role": "a03_operator",
            "set_role": "a03_owner",
        },
        "instance_identity": {
            "database": "test_db",
            "database_oid": 16384,
            "port": 5432,
            "cluster_system_identifier": "7000000000000000000",
            "server_version_num": 180000,
            "postmaster_started_at": "2026-10-19T00:00:00Z",
            "server_instance_nonce": "test-nonce-1234",
        },
        "contract": {
            "identity_version": 4,
            "identity_fingerprint_sha256": "0" * 64,
            "catalog_fingerprint_sha256": "c" * 64,
            "catalog_query": "SELECT 1 AS definition",
            "component_fingerprints": {},
        },
    }


class PurePlanBoundaryTests(unittest.TestCase):
    def test_positive_draft_plan_is_accepted(self):
        plan, _, _, _ = sample_fixture()
        KQ.validate_key_quota_plan(plan)

    def test_changed_records_with_stale_mapping_digest_are_rejected(self):
        plan, _, _, _ = sample_fixture()
        plan["key_records"][0]["used_units"] += 1
        with self.assertRaises(KQ.Error):
            KQ.validate_key_quota_plan(plan)

    def test_duplicate_source_key_cannot_be_assigned_to_two_target_credentials(self):
        plan, _, _, _ = sample_fixture()
        other = copy.deepcopy(plan["key_records"][0])
        other["target_credential_id"] = 202
        plan["key_records"].append(other)
        plan["mapping_sha256"] = KQ.digest(plan["key_records"])
        with self.assertRaises(KQ.Error):
            KQ.validate_key_quota_plan(plan)

    def test_duplicate_target_credential_id_is_rejected(self):
        plan, _, _, _ = sample_fixture()
        other = copy.deepcopy(plan["key_records"][0])
        other["source_key_id"] = 999
        plan["key_records"].append(other)
        plan["mapping_sha256"] = KQ.digest(plan["key_records"])
        with self.assertRaises(KQ.Error):
            KQ.validate_key_quota_plan(plan)

    def test_credential_receipt_from_another_archive_is_rejected(self):
        _, token, receipt, user_map = sample_fixture()
        receipt["source_archive_sha256"] = "b" * 64
        with self.assertRaises(KQ.Error):
            KQ.build_plan_from_tokens(
                [token], user_map, receipt,
                migration_id="independent-key-draft",
                source_archive_sha256="a" * 64
            )

    def test_credential_receipt_scope_mismatch(self):
        _, token, receipt, user_map = sample_fixture()
        receipt["scope"] = "wrong-scope"
        with self.assertRaises(KQ.Error):
            KQ.build_plan_from_tokens(
                [token], user_map, receipt,
                migration_id="independent-key-draft",
                source_archive_sha256="a" * 64
            )

    def test_unmapped_user_is_rejected(self):
        _, token, receipt, _ = sample_fixture()
        with self.assertRaises(KQ.Error):
            KQ.build_plan_from_tokens(
                [token], {}, receipt,
                migration_id="independent-key-draft",
                source_archive_sha256="a" * 64
            )

    def test_binding_user_or_account_mismatch(self):
        _, token, receipt, user_map = sample_fixture()
        bad_map = {71: {"user_id": 999, "account_id": 1}}
        with self.assertRaises(KQ.Error):
            KQ.build_plan_from_tokens(
                [token], bad_map, receipt,
                migration_id="independent-key-draft",
                source_archive_sha256="a" * 64
            )

    def test_source_secret_hash_mismatch(self):
        _, token, receipt, user_map = sample_fixture()
        token["key"] = "lmmk_" + "f" * 64
        with self.assertRaises(KQ.Error):
            KQ.build_plan_from_tokens(
                [token], user_map, receipt,
                migration_id="independent-key-draft",
                source_archive_sha256="a" * 64
            )

    def test_negative_used_units_rejected(self):
        plan, _, _, _ = sample_fixture()
        plan["key_records"][0]["used_units"] = -1
        plan["mapping_sha256"] = KQ.digest(plan["key_records"])
        with self.assertRaises(KQ.Error):
            KQ.validate_key_quota_plan(plan)

    def test_bigint_overflow_rejected(self):
        plan, _, _, _ = sample_fixture()
        plan["key_records"][0]["used_units"] = KQ.BIGINT_MAX
        plan["key_records"][0]["remaining_units"] = 1
        plan["mapping_sha256"] = KQ.digest(plan["key_records"])
        with self.assertRaises(KQ.Error):
            KQ.validate_key_quota_plan(plan)

    def test_unlimited_flag_validation(self):
        plan, _, _, _ = sample_fixture()
        plan["key_records"][0]["unlimited"] = "not-a-bool"
        plan["mapping_sha256"] = KQ.digest(plan["key_records"])
        with self.assertRaises(KQ.Error):
            KQ.validate_key_quota_plan(plan)

    def test_empty_records_rejected(self):
        plan, _, _, _ = sample_fixture()
        plan["key_records"] = []
        plan["mapping_sha256"] = KQ.digest(plan["key_records"])
        with self.assertRaises(KQ.Error):
            KQ.validate_key_quota_plan(plan)

    def test_duplicate_token_id_in_source_tokens_rejected(self):
        _, token, receipt, user_map = sample_fixture()
        tokens = [token, copy.deepcopy(token)]
        with self.assertRaises(KQ.Error):
            KQ.build_plan_from_tokens(
                tokens, user_map, receipt,
                migration_id="independent-key-draft",
                source_archive_sha256="a" * 64
            )

    def test_duplicate_source_key_in_credential_receipt_rejected(self):
        _, token, receipt, user_map = sample_fixture()
        receipt["records"].append(copy.deepcopy(receipt["records"][0]))
        with self.assertRaises(KQ.Error):
            KQ.build_plan_from_tokens(
                [token], user_map, receipt,
                migration_id="independent-key-draft",
                source_archive_sha256="a" * 64
            )


class PureSqlGenerationInvariantTests(unittest.TestCase):
    def setUp(self):
        self.plan, self.token, self.receipt, self.user_map = sample_fixture()
        self.target = sample_target()
        self.sql = KQ.build_apply_sql(self.plan, self.target)

    def test_sql_contains_advisory_lock_and_serializable(self):
        self.assertIn("BEGIN ISOLATION LEVEL SERIALIZABLE;", self.sql)
        self.assertIn(f"SELECT pg_advisory_xact_lock({KQ.FENCE});", self.sql)

    def test_sql_locks_tables_and_queries_sequence_relation(self):
        self.assertIn("LOCK TABLE core_charging.budgets IN ACCESS EXCLUSIVE MODE;", self.sql)
        self.assertNotIn("LOCK TABLE core_charging.budgets_id_seq", self.sql)
        self.assertIn("LOCK TABLE core_charging.budget_opening_consumption IN ACCESS EXCLUSIVE MODE;", self.sql)
        self.assertIn("LOCK TABLE core_migration.import_runs IN ACCESS EXCLUSIVE MODE;", self.sql)
        self.assertIn("LOCK TABLE core_identity.credentials IN SHARE MODE;", self.sql)
        self.assertIn("FROM core_charging.budgets_id_seq;", self.sql)

    def test_sql_contains_target_identity_guards(self):
        self.assertIn("opening target identity mismatch", self.sql)
        self.assertIn("opening target has another client", self.sql)
        self.assertIn("opening installed contract mismatch", self.sql)

    def test_sql_contains_replay_manifest_and_stale_hash_guards(self):
        self.assertIn("key_quota_import_conflict: migration_id already recorded with different material", self.sql)
        self.assertIn("key_quota_import_conflict: mapping_sha256 mismatch with actual records", self.sql)
        self.assertIn("existing_run.manifest = ", self.sql)

    def test_sql_contains_target_secret_digest_check(self):
        secret_sha = self.plan["key_records"][0]["source_secret_sha256"]
        self.assertIn(f"digest=decode('{secret_sha}', 'hex')", self.sql)
        self.assertIn("target credential % invalid, mismatched ownership, or secret digest mismatch", self.sql)

    def test_sql_uses_overriding_system_value_and_no_direct_nextval(self):
        self.assertIn("OVERRIDING SYSTEM VALUE", self.sql)
        self.assertNotIn("nextval(", self.sql)

    def test_sql_has_exact_immutable_import_run_marker(self):
        # Must strictly preserve marker for independent test assertion
        self.assertIn("-- 8. Record Immutable Import Run", self.sql)
        self.assertIn("INSERT INTO core_migration.import_runs", self.sql)

    def test_sql_transactionally_advances_sequence(self):
        self.assertIn("EXECUTE 'ALTER SEQUENCE core_charging.budgets_id_seq RESTART WITH ' || v_next_budget_id;", self.sql)

    def test_sql_deterministic_and_canonical(self):
        sql2 = KQ.build_apply_sql(self.plan, self.target)
        self.assertEqual(self.sql, sql2)


if __name__ == "__main__":
    unittest.main()
