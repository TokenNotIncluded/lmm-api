"""Synthetic history/scoped-zero-coverage contracts; never accesses real data."""
import copy
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).parent))
import financial_import as F
import legacy_history_import as H
from test_financial_import import source_fixture, snapshot_fixture, material_fixture


def raw_log(log_id=1, **changes):
    return {"id":log_id,"user_id":71,"token_id":4,"created_at":1700000000,"type":2,"quota":17,
        "prompt_tokens":3,"completion_tokens":4,"username":"synthetic-user","token_name":"synthetic-key",
        "model_name":"synthetic-model","channel":1,"group":"synthetic-group","request_id":"req",
        "upstream_request_id":"upstream","other":'{"pricing_credits_per_usd":500000,"secret":"never-persist-this"}',**changes}


def source_files(plan):
    return {k:{"path":"/home/test/Backups/"+k+".json","sha256":plan["source"]["source_export_sha256"] if k=="history_export" else "c"*64}
            for k in ("history_export","credential_mapping","source_review")}


def fixture(logs=None):
    source=source_fixture();snapshot=snapshot_fixture(source)
    for table in H.CHANGED_TABLES:
        snapshot["state"]["inventory"]["tables"].append(table)
        snapshot["state"]["tables"][table]={"rows":0,"sha256":F.sha(b"")}
    snapshot["state"]["inventory"]["tables"].sort()
    snapshot["state"]["inventory"]["sequences"].append(H.SEQUENCE)
    snapshot["state"]["inventory"]["sequences"].sort()
    snapshot["state"]["sequences"][H.SEQUENCE]={"last_value":1,"is_called":False,"start":1,
        "increment":1,"min":1,"max":2**63-1,"cache":1,"cycle":False}
    a=material_fixture(source,snapshot)
    receipt={"scope":"a02-credential-mapping-receipt","source_archive_sha256":a["source_archive_sha256"],
        "records":[{"source_key_id":4,"target_credential_id":400,"target_user_id":71,"target_account_id":1},
                   {"source_key_id":5,"target_credential_id":500,"target_user_id":72,"target_account_id":2}]}
    p=H.build_plan([raw_log()] if logs is None else logs,a,receipt,
        source_export_sha256="e"*64,source_namespace="synthetic-history")
    review={"format_version":1,"scope":H.SCOPE,"decision":"approved-offline-history-scopes",
        **{k:p["source"][k] for k in ("source_archive_sha256","source_export_sha256","source_table_sha256","credential_mapping_sha256")},
        "mapping_sha256":p["mapping_sha256"],"identity_map_sha256":a["identity_map_sha256"],
        "complete_source_table":True,"complete_identity_and_key_mapping":True,"evidence_sha256":"a"*64}
    return a,receipt,p,review


class BasisTests(unittest.TestCase):
    def test_exact_numeric_anchors_and_unknown_invalid_metadata(self):
        for raw in ("500000","500000.0","5e5","500000000e-3"):
            self.assertEqual(H.captured_basis('{"pricing_credits_per_usd":'+raw+'}'),500000)
        self.assertEqual(H.captured_basis('{"pricing_credits_per_usd":1.'+'0'*62+'}'),1)
        for raw in ('"500000"','null','true','0','-1','1.5','1e19','9007199254740992','0.0000001e19'):
            self.assertIsNone(H.captured_basis('{"pricing_credits_per_usd":'+raw+'}'),raw)
        for raw in ('[]','{bad','{"pricing_credits_per_usd":500000,"bad":NaN}',
                    '{"pricing_credits_per_usd":500000,"pricing_credits_per_usd":3}', ' '*65537):
            self.assertIsNone(H.captured_basis(raw))


class HistoryContractTests(unittest.TestCase):
    def test_whitelist_preserves_raw_counters_and_omits_opaque_metadata(self):
        a,keys,p,review=fixture([raw_log(quota=-7,other='{"unknown":"never-persist-this"}')])
        r=p["rows"][0]
        self.assertEqual(r["quota"],-7);self.assertIsNone(r["pricing_credits_per_usd"])
        self.assertEqual(r["source_row_sha256"],F.digest(raw_log(quota=-7,other='{"unknown":"never-persist-this"}')))
        self.assertNotIn("other",r);self.assertNotIn("never-persist-this",F.canonical(p))

    def test_empty_scope_receipts_cover_all_mapped_users_and_keys(self):
        a,keys,p,review=fixture([])
        self.assertEqual(len(p["scopes"]),5)
        self.assertTrue(all(s["rows"] == [] for s in p["scopes"]))
        self.assertEqual({(s["user_id"],s["token_id"]) for s in p["scopes"]},
                         {(71,None),(72,None),(73,None),(71,400),(72,500)})
        self.assertEqual(len({H.canonical_sha(s) for s in p["scopes"]}),5)
        H.prepare_material(a,p,review,migration_id="synthetic-empty-history",source_files=source_files(p))

    def test_user_and_key_scopes_bind_exact_row_sets(self):
        a,keys,p,review=fixture([raw_log(2,token_id=0),raw_log(1)])
        scopes={(s["user_id"],s["token_id"]):s for s in p["scopes"]}
        self.assertEqual([r["source_log_id"] for r in scopes[(71,None)]["rows"]],[1,2])
        self.assertEqual([r["source_log_id"] for r in scopes[(71,400)]["rows"]],[1])
        self.assertEqual(scopes[(72,500)]["rows"],[])
        original=H.canonical_sha(scopes[(71,None)])
        changed=copy.deepcopy(scopes[(71,None)]);changed["source_export_sha256"]="f"*64
        self.assertNotEqual(original,H.canonical_sha(changed))

    def test_missing_mapping_wrong_ownership_and_unknown_fields_reject_entire_batch(self):
        a,keys,_,_=fixture()
        for log in (raw_log(user_id=0),raw_log(user_id=999),raw_log(token_id=999),raw_log(user_id=72),
                    raw_log(quota=True),raw_log(username="x"*257),raw_log(channel=-1)):
            with self.subTest(log=log["id"]),self.assertRaises(F.Error):
                H.build_plan([log],a,keys,source_export_sha256="e"*64,source_namespace="synthetic")
        with self.assertRaises(F.Error):
            H.build_plan([raw_log(),raw_log()],a,keys,source_export_sha256="e"*64,source_namespace="synthetic")

    def test_complete_flags_and_exact_review_are_required_even_for_empty(self):
        a,keys,p,review=fixture([])
        for key,value in (("complete_source_table",False),("complete_identity_and_key_mapping",False),
            ("mapping_sha256","f"*64),("source_archive_sha256","f"*64),("format_version",True)):
            r=copy.deepcopy(review);r[key]=value
            with self.subTest(key=key),self.assertRaises(F.Error):H.prepare_material(a,p,r,migration_id="history",source_files=source_files(p))

    def test_tampered_scope_and_source_row_count_cannot_reuse_review(self):
        a,keys,p,review=fixture()
        for mutate in (lambda p:p["scopes"].pop(),lambda p:p["rows"][0].update(owner_account_id=2),
                       lambda p:p["source"].update(row_count=0)):
            q=copy.deepcopy(p);mutate(q);q["mapping_sha256"]=F.digest({k:v for k,v in q.items() if k!="mapping_sha256"})
            with self.assertRaises(F.Error):H.prepare_material(a,q,review,migration_id="history",source_files=source_files(p))

    def test_sql_is_atomic_whitelisted_and_approval_bound(self):
        a,keys,p,review=fixture();m=H.prepare_material(a,p,review,migration_id="history",source_files=source_files(p))
        sql=H.build_apply_sql(m)
        self.assertNotIn("never-persist-this",sql)
        for s in ("history_exact_readback_failed","history_unrelated_state_changed","history_credential_binding_mismatch",
                  "history_identity_binding_mismatch","INSERT INTO core_history.scope_receipts","synchronous_commit=on"):
            self.assertIn(s,sql)
        self.assertLess(sql.index("pg_try_advisory_lock"),sql.index("BEGIN ISOLATION LEVEL SERIALIZABLE"))
        self.assertGreater(sql.rindex("'COMMITTED'"),sql.index("COMMIT;"))
        self.assertNotIn("INSERT INTO core_billing.ledger",sql)
        approval={"format_version":1,"scope":H.SCOPE,"decision":"approve-offline-history-scopes",
            "material_sha256":F.digest(m),"sql_sha256":F.sha(sql.encode()),"source_review_sha256":F.digest(review)}
        H.verify_approval(m,sql,approval)
        approval["format_version"]=True
        with self.assertRaises(F.Error):H.verify_approval(m,sql,approval)

    def test_resolution_is_readonly_and_original_attempt_is_mandatory(self):
        a,keys,p,review=fixture();m=H.prepare_material(a,p,review,migration_id="history",source_files=source_files(p))
        sql=H.build_resolve_sql(m,"lmm-history-"+"a"*32)
        self.assertIn("SERIALIZABLE READ ONLY",sql);self.assertNotIn("INSERT INTO",sql)
        self.assertIn("history_original_attempt_active",sql);self.assertIn("history_absence_not_proven",sql)
        with self.assertRaises(F.Error):H.build_resolve_sql(m,"another-session")


class DriverTests(unittest.TestCase):
    def setUp(self):
        a,keys,p,review=fixture()
        self.m=H.prepare_material(a,p,review,migration_id="history",source_files=source_files(p))
        self.sql=H.build_apply_sql(self.m)
        self.approval={"format_version":1,"scope":H.SCOPE,"decision":"approve-offline-history-scopes",
            "material_sha256":F.digest(self.m),"sql_sha256":F.sha(self.sql.encode()),"source_review_sha256":F.digest(review)}
        self.connection={"database":"a03_test","port":5432}

    def call(self, run, *, source_check=None, original_attempt=None, write=None):
        with patch.object(H,"verify_source_files",side_effect=source_check), \
             patch.object(F.IDENTITY,"connection_roles",return_value=("a03_owner","a03_operator")), \
             patch.object(F.IDENTITY,"socket_connection"), \
             patch.object(F,"write_private",side_effect=write), \
             patch.object(F,"run_sql",side_effect=run):
            return H.execute(self.m,approval=self.approval,original_sql=self.sql,
                connection=self.connection,output=Path("/home/test/Backups/history-attempt"),original_attempt=original_attempt)

    def test_transport_failure_and_malformed_ack_are_unknown_without_retry(self):
        for response in ((1,""),(0,"bad-json"),(0,'{"status":"COMMITTED"}')):
            calls=[]
            def run(*args,**kwargs):calls.append(args);return response
            result=self.call(run)
            self.assertEqual(result["status"],"UNKNOWN");self.assertEqual(len(calls),1)

    def test_source_refusal_and_attempt_write_failure_precede_transport(self):
        calls=[]
        def run(*args,**kwargs):calls.append(args);raise AssertionError("must not submit")
        with self.assertRaises(F.Error):self.call(run,source_check=F.Error("history_source_file_changed"))
        self.assertFalse(calls)
        with self.assertRaises(OSError):self.call(run,write=OSError("synthetic durable-write failure"))
        self.assertFalse(calls)

    def test_post_transport_source_change_is_unknown(self):
        result=self.call(lambda *a,**k:(0,F.canonical({"status":"COMMITTED","migration_id":"history",
            "material_sha256":F.digest(self.m)})),source_check=[None,F.Error("source_changed")])
        self.assertEqual(result["status"],"UNKNOWN")

    def test_original_attempt_resolution_and_durable_order(self):
        events=[]
        original={"operation":"apply","migration_id":"history","material_sha256":F.digest(self.m),
            "sql_sha256":F.sha(self.sql.encode()),"application_name":"lmm-history-"+"a"*32}
        def write(path,name,value):events.append(name)
        def run(connection,sql,output,name):
            self.assertEqual(events,["attempt.private.json","executed.private.sql"])
            self.assertIn("SERIALIZABLE READ ONLY",sql)
            return 0,F.canonical({"status":"NOT_APPLIED","migration_id":"history","material_sha256":F.digest(self.m)})
        self.assertEqual(self.call(run,original_attempt=original,write=write)["status"],"NOT_APPLIED")
        self.assertEqual(events[-1],"result.private.json")
        original["material_sha256"]="f"*64
        with self.assertRaises(F.Error):self.call(run,original_attempt=original)


if __name__=="__main__":unittest.main()
