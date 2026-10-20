"""Opt-in synthetic PG history writer evidence; no old database/real source.

Requires the canonical scope_receipts DDL to be installed by the normal schema
fixture. Never injects DDL or relaxes the installed catalog for this test.
"""
import copy
import json
import os
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).parent))
import financial_import as F
import legacy_history_import as H
import test_financial_import_pg as P
from test_legacy_history_import import fixture, source_files


@unittest.skipUnless(os.environ.get("LMM_FINANCIAL_PG_TEST")=="1","isolated PostgreSQL test not enabled")
class HistoryPostgreSQLTests(P.PostgreSQLTests):
    def setUp(self):
        super().setUp()
        self.sql("INSERT INTO core_identity.credentials(id,digest,kind,user_id,user_version,owner_account_id,expires_at) "
            "OVERRIDING SYSTEM VALUE VALUES(400,decode(repeat('e',64),'hex'),'api_key',71,1,1,'2100-01-01'),"
            "(500,decode(repeat('f',64),'hex'),'api_key',72,1,2,'2100-01-01')")
        self.history=self.capture_history()
        self.history_sql=H.build_apply_sql(self.history)

    def capture_history(self,logs=None):
        a,keys,p,review=fixture(logs)
        snapshot=json.loads(self.sql(F.snapshot_sql(self.inventory,"synthetic-history")).stdout)
        a["before_state"]=snapshot["state"]
        a["target"]={k:snapshot[k] for k in ("instance","catalog_fingerprint_sha256","ownership_sha256","component_fingerprints")}
        a["native_users"]=snapshot["native_users"]
        a["identity_import"]=snapshot["import_runs"][0]
        return H.prepare_material(a,p,review,migration_id="synthetic-history",source_files=source_files(p))

    def test_history_exact_log_and_zero_scopes_replay_without_money_changes(self):
        before=self.full_state()
        result=json.loads(self.sql(self.history_sql).stdout)
        self.assertEqual(result["status"],"COMMITTED")
        after=self.full_state()
        self.assertEqual(json.loads(self.sql(self.history_sql).stdout),result)
        self.assertEqual(self.full_state(),after)
        for table in before["tables"]:
            if table not in set(H.CHANGED_TABLES)|{"core_migration.import_runs"}:
                self.assertEqual(before["tables"][table],after["tables"][table],table)
        actual=self.query("SELECT jsonb_build_object('rows',(SELECT count(*) FROM core_history.log_records),"
            "'scopes',(SELECT count(*) FROM core_history.scope_receipts),'empty_user',EXISTS(SELECT 1 FROM core_history.scope_receipts WHERE user_id=73 AND token_id IS NULL),"
            "'empty_key',EXISTS(SELECT 1 FROM core_history.scope_receipts WHERE user_id=72 AND token_id=500),"
            "'basis',(SELECT pricing_credits_per_usd FROM core_history.log_records))")
        self.assertEqual(actual,{"rows":1,"scopes":5,"empty_user":True,"empty_key":True,"basis":500000})
        expected=H.canonical_sha(self.history["plan"]["scopes"][2])
        self.assertEqual(self.query("SELECT to_jsonb(source_scope_sha256) FROM core_history.scope_receipts WHERE user_id=73 AND token_id IS NULL"),expected)

    def test_history_fully_empty_source_still_has_real_scope_receipts(self):
        m=self.capture_history([])
        self.sql(H.build_apply_sql(m))
        self.assertEqual(self.query("SELECT count(*)::int FROM core_history.log_records"),0)
        self.assertEqual(self.query("SELECT count(*)::int FROM core_history.scope_receipts"),5)
        self.assertTrue(self.query("SELECT bool_and(complete) FROM core_history.scope_receipts"))

    def test_history_wrong_target_key_and_scope_partial_failure_are_atomic(self):
        bad=copy.deepcopy(self.history)
        bad["financial_attestation"]["target"]["instance"]["server_instance_nonce"]="wrong"
        self.rejected_unchanged(H.build_apply_sql(bad),"financial_target_instance_mismatch")
        self.sql("UPDATE core_identity.credentials SET kind='session' WHERE id=400")
        # Business CAS rejects changed credential before any source/sequence write.
        self.rejected_unchanged(self.history_sql,"history_target_state_changed")

    def test_history_partial_scope_insert_rolls_back_every_table_and_sequence(self):
        bad=self.history_sql.replace("  SET CONSTRAINTS ALL IMMEDIATE;","  PERFORM 1/0;\n  SET CONSTRAINTS ALL IMMEDIATE;")
        self.rejected_unchanged(bad,"division by zero")
        self.assertEqual(self.query("SELECT count(*)::int FROM core_history.sources"),0)

    def test_history_other_id_and_changed_scope_refuse_strict_replay(self):
        self.sql(self.history_sql)
        bad=copy.deepcopy(self.history);bad["migration_id"]="another-history"
        self.rejected_unchanged(H.build_apply_sql(bad),"history_target_state_changed")
        self.sql("UPDATE core_history.scope_receipts SET complete=false WHERE user_id=73")
        self.rejected_unchanged(self.history_sql,"history_replay_conflict")

    def test_history_unknown_resolution_and_runtime_cannot_write(self):
        resolver=H.build_resolve_sql(self.history,"lmm-history-"+"a"*32)
        self.assertEqual(json.loads(self.sql(resolver).stdout)["status"],"NOT_APPLIED")
        self.sql(self.history_sql)
        self.assertEqual(json.loads(self.sql(resolver).stdout)["status"],"COMMITTED")
        self.assertFalse(self.query("SELECT has_table_privilege('a03_runtime','core_history.scope_receipts','INSERT,UPDATE,DELETE,TRUNCATE')"))
        self.sql("DELETE FROM core_history.scope_receipts WHERE token_id=500")
        self.rejected_unchanged(resolver,"history_resolution_conflict")


def load_tests(loader,tests,pattern):
    return unittest.TestSuite(HistoryPostgreSQLTests(n) for n in loader.getTestCaseNames(HistoryPostgreSQLTests)
                             if n.startswith("test_history_"))


if __name__=="__main__":unittest.main()
