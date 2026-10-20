"""Opt-in real PG evidence for synthetic UTC subscription migration only.

Uses the existing owned, network-disabled Docker fixture. No source connection,
real credentials, application/worker or existing database is accepted.
"""
import copy
import json
import os
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import financial_import as F
import legacy_subscription_opening as S
import test_financial_import_pg as P
from test_legacy_subscription_opening import fixture


@unittest.skipUnless(os.environ.get("LMM_FINANCIAL_PG_TEST") == "1", "isolated PostgreSQL test not enabled")
class SubscriptionPostgreSQLTests(P.PostgreSQLTests):
    def setUp(self):
        super().setUp()
        candidate, review = fixture()
        snapshot = json.loads(self.sql(F.snapshot_sql(self.inventory, "synthetic-subscription")).stdout)
        candidate["before_state"] = snapshot["state"]
        candidate["target"] = {key:snapshot[key] for key in
            ("instance","catalog_fingerprint_sha256","ownership_sha256","component_fingerprints")}
        candidate["native_users"] = snapshot["native_users"]
        candidate["identity_import"] = snapshot["import_runs"][0]
        self.sub = S.prepare_material(candidate, review, migration_id="synthetic-subscription")
        self.sql_batch = S.build_apply_sql(self.sub)

    def test_sub_exact_reset_usage_and_strict_replay(self):
        before = self.full_state()
        result=json.loads(self.sql(self.sql_batch).stdout)
        self.assertEqual(result["status"],"COMMITTED")
        after=self.full_state()
        self.assertEqual(json.loads(self.sql(self.sql_batch).stdout),result)
        self.assertEqual(after,self.full_state())
        for table in before["tables"]:
            if table not in set(S.CHANGED_TABLES)|{"core_migration.import_runs"}:
                self.assertEqual(before["tables"][table],after["tables"][table],table)
        actual=self.query("SELECT jsonb_build_object('current',s.limit_credits,'used',o.consumed_credits,'future',s.reset_limit_credits,'models',s.models,'groups',s.groups) FROM core_charging.subscriptions s JOIN core_charging.subscription_opening_consumption o ON o.subscription_id=s.id")
        self.assertEqual(actual,{"current":1000,"used":400,"future":2000,
            "models":["synthetic-model"],"groups":["synthetic-group"]})
        for at,limit in ((1706745600,1000),(1709251200,2000)):
            actual=self.query(f"SELECT jsonb_build_object('limit',limit_credits) FROM core_charging.subscription_window(1,{at})")
            self.assertEqual(actual,{"limit":limit})

    def test_sub_conflicts_and_post_application_mutation_refuse_replay(self):
        self.sql(self.sql_batch)
        changed=copy.deepcopy(self.sub)
        changed["migration_id"]="another-subscription"
        self.rejected_unchanged(S.build_apply_sql(changed),"subscription_target_state_changed")
        self.sql("BEGIN ISOLATION LEVEL SERIALIZABLE; UPDATE core_charging.subscriptions SET active=false; COMMIT;")
        self.rejected_unchanged(self.sql_batch,"subscription_replay_conflict")

    def test_sub_partial_failure_rolls_back_tables_and_sequence(self):
        bad=self.sql_batch.replace("  SET CONSTRAINTS ALL IMMEDIATE;", "  PERFORM 1/0;\n  SET CONSTRAINTS ALL IMMEDIATE;")
        self.rejected_unchanged(bad,"division by zero")
        self.assertEqual(self.query("SELECT count(*)::int FROM core_charging.subscriptions"),0)

    def test_sub_unknown_ack_resolution_uses_exact_state(self):
        name="lmm-subscription-"+"a"*32
        resolver=S.build_resolve_sql(self.sub,name)
        self.assertEqual(json.loads(self.sql(resolver).stdout)["status"],"NOT_APPLIED")
        self.sql(self.sql_batch)
        self.assertEqual(json.loads(self.sql(resolver).stdout)["status"],"COMMITTED")
        self.sql("BEGIN ISOLATION LEVEL SERIALIZABLE; UPDATE core_charging.subscriptions SET active=false; COMMIT;")
        self.rejected_unchanged(resolver,"subscription_resolution_conflict")

    def test_sub_bad_target_and_calendar_boundary_are_atomic(self):
        bad=copy.deepcopy(self.sub)
        bad["financial_attestation"]["target"]["instance"]["server_instance_nonce"]="wrong"
        self.rejected_unchanged(S.build_apply_sql(bad),"financial_target_instance_mismatch")
        # Tamper SQL after generation to exercise the actual DB calendar trigger.
        bad=self.sql_batch.replace("(p->>'opening_end')::bigint,", "(p->>'opening_end')::bigint-1,",1)
        self.rejected_unchanged(bad,"opening subscription reset mismatch")


def load_tests(loader, tests, pattern):
    # The shared parent owns the existing wallet tests. Run only new subscription
    # cases here; do not count inherited tests twice or run two extra fixtures.
    return unittest.TestSuite(SubscriptionPostgreSQLTests(name) for name in
        loader.getTestCaseNames(SubscriptionPostgreSQLTests) if name.startswith("test_sub_"))


if __name__ == "__main__":
    unittest.main()
