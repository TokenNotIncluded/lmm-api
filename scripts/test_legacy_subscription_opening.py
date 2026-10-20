"""Synthetic contracts for the UTC subscription writer; no real source or DSN."""
import copy
from pathlib import Path
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import financial_import as F
import legacy_subscription_opening as S
from test_financial_import import source_fixture, snapshot_fixture, material_fixture
from test_migrate_legacy_financial import tables, user, subscription, plan
from test_financial_import_pg import installed_postgres_image, POSTGRES_IMAGES


def fixture(*, period="monthly", **changes):
    source = source_fixture()
    row = subscription(**{"upgrade_group":"", "prev_user_group":"", "downgrade_group":"", "reset_amount":2000, **changes})
    if period == "never":
        row.update(last_reset_time=0, next_reset_time=0)
    source["plan"] = F.PLANNER.plan_financial(tables(users=[user(71), user(72), user(73)], tokens=[],
        user_subscriptions=[row], subscription_plans=[plan(quota_reset_period=period)]),
        source["identities"], source_sha256=source["plan"]["source_archive_sha256"],
        identity_provenance=source["plan"]["identity_provenance"], source_timezone="UTC")
    for value in (source["review"], source["reservations"]):
        value["mapping_sha256"] = source["plan"]["mapping_sha256"]
    snapshot = snapshot_fixture(source)
    for table in S.CHANGED_TABLES:
        snapshot["state"]["inventory"]["tables"].append(table)
        snapshot["state"]["tables"][table] = {"rows": 0, "sha256": F.sha(b"")}
    snapshot["state"]["inventory"]["tables"].sort()
    snapshot["state"]["inventory"]["sequences"].append(S.SEQUENCE)
    snapshot["state"]["inventory"]["sequences"].sort()
    snapshot["state"]["sequences"][S.SEQUENCE] = {"last_value": 1, "is_called": False,
        "start": 1, "increment": 1, "min": 1, "max": 2**63-1, "cache": 1, "cycle": False}
    a = material_fixture(source, snapshot)
    r = source["plan"]["subscription_entitlements"][0]
    review = {"format_version": 1, "scope": S.SCOPE, "source_contract": S.SOURCE_CONTRACT,
        "decision": "approved-offline-subscriptions", "source_archive_sha256": a["source_archive_sha256"],
        "mapping_sha256": a["mapping_sha256"], "identity_map_sha256": a["identity_map_sha256"],
        "credits_per_usd": 500000, "evidence_sha256": "b"*64, "source_timezone": "UTC",
        "renewal_policy": "separate-grant-after-expiry", "policies": [{
            "source_subscription_id": r["source_subscription_id"], "models": ["synthetic-model"],
            "groups": ["synthetic-group"], "source_row_sha256": r["source_row_sha256"],
            "plan_row_sha256": r["plan_row_sha256"]}]}
    return a, review


class SourceContractTests(unittest.TestCase):
    def test_current_and_future_reset_amounts_are_separate(self):
        a, review = fixture()
        r = S.build_records(a["source_plan"], review)[0]
        self.assertEqual((r["limit_credits"], r["opening_used"], r["reset_limit_credits"]), (1000,400,2000))
        self.assertEqual((r["opening_start"],r["opening_end"]), (1706745600,1709251200))

    def test_never_and_unlimited_preserve_historical_consumption(self):
        a, review = fixture(period="never",amount_total=0,reset_amount=None)
        r = S.build_records(a["source_plan"],review)[0]
        self.assertTrue(r["unlimited"])
        self.assertEqual((r["period"],r["opening_start"],r["opening_end"],r["opening_used"]),
                         ("grant",1706745600,1740787200,400))
        a,review=fixture(amount_total=0,reset_amount=None)
        self.assertTrue(S.build_records(a["source_plan"],review)[0]["reset_unlimited"])

    def test_explicit_zero_is_finite_exhausted_and_cancelled_is_inactive(self):
        a, review = fixture(amount_total=0,reset_amount=0,status="cancelled")
        r = S.build_records(a["source_plan"],review)[0]
        self.assertFalse(r["unlimited"])
        self.assertFalse(r["active"])
        self.assertEqual(r["opening_used"],400)

    def test_unknown_finite_reset_and_policy_effects_are_rejected(self):
        for changes in ({"reset_amount":None},{"allow_wallet_overflow":True},{"upgrade_group":"vip"}):
            with self.subTest(changes=changes):
                # fixture names these keys explicitly; mutate after planner mapping instead.
                a, review=fixture()
                p=a["source_plan"]
                p["subscription_entitlements"][0].update(changes)
                p.pop("mapping_sha256");p.pop("public_summary")
                p["mapping_sha256"]=F.digest(p)
                review["mapping_sha256"]=p["mapping_sha256"]
                with self.assertRaises(F.Error): S.build_records(p,review)

    def test_source_review_never_accepts_wrong_units_timezone_or_missing_policy(self):
        a, review = fixture()
        for key,value in (("credits_per_usd",True),("credits_per_usd",1000000),("source_timezone","Asia/Taipei"),
                          ("policies",[]),("evidence_sha256",""),("mapping_sha256","c"*64)):
            r=copy.deepcopy(review);r[key]=value
            with self.subTest(key=key,value=value),self.assertRaises(F.Error):S.build_records(a["source_plan"],r)

    def test_policy_is_explicit_and_exact_source_bound(self):
        a,r=fixture()
        for key,value in (("models",[]),("groups",["x","x"]),("source_row_sha256","e"*64)):
            changed=copy.deepcopy(r);changed["policies"][0][key]=value
            with self.subTest(key=key),self.assertRaises(F.Error):S.build_records(a["source_plan"],changed)

    def test_sql_has_atomic_full_state_guard_and_no_wallet_posts(self):
        a,r=fixture();m=S.prepare_material(a,r,migration_id="synthetic-subscription")
        sql=S.build_apply_sql(m)
        self.assertIn("BEGIN ISOLATION LEVEL SERIALIZABLE",sql)
        self.assertLess(sql.index("pg_try_advisory_lock"),sql.index("BEGIN ISOLATION"))
        for text in ("synchronous_commit=on","subscription_replay_conflict","subscription_identity_binding_mismatch",
                     "subscription_unrelated_state_changed","subscription_readback_failed","ALTER SEQUENCE"):
            self.assertIn(text,sql)
        self.assertNotIn("PERFORM core_billing.import_opening_balance",sql)
        self.assertNotIn("setval(",sql)
        self.assertGreater(sql.rindex("'COMMITTED'"),sql.index("COMMIT;"))

    def test_tampering_and_noncanonical_sequence_rejected(self):
        a,r=fixture();m=S.prepare_material(a,r,migration_id="synthetic-subscription")
        m["records"][0]["limit_credits"]=999;m["mapping_sha256"]=F.digest(m["records"])
        with self.assertRaises(F.Error):S.build_apply_sql(m)
        a["before_state"]["sequences"][S.SEQUENCE]["cache"]=20
        with self.assertRaises(F.Error):S.prepare_material(a,r,migration_id="synthetic-subscription")

    def test_approval_and_resolution_bind_exact_material(self):
        a,r=fixture();m=S.prepare_material(a,r,migration_id="synthetic-subscription");sql=S.build_apply_sql(m)
        approval={"format_version":1,"scope":S.SCOPE,"decision":"approve-offline-subscriptions",
            "material_sha256":F.digest(m),"sql_sha256":F.sha(sql.encode()),"source_review_sha256":F.digest(r)}
        S.verify_approval(m,sql,approval)
        approval["format_version"]=True
        with self.assertRaises(F.Error):S.verify_approval(m,sql,approval)
        resolve=S.build_resolve_sql(m,"lmm-subscription-"+"a"*32)
        self.assertIn("SERIALIZABLE READ ONLY",resolve)
        self.assertIn("subscription_absence_not_proven",resolve)
        self.assertNotIn("INSERT INTO",resolve)


class ImageDiscoveryTests(unittest.TestCase):
    def test_digest_only_ci_image_is_found_without_pull(self):
        calls=[]
        def docker(*args,**kwargs):
            calls.append(args)
            return subprocess.CompletedProcess(args,0,"sha256:"+"a"*64+"\n","")
        ref,image,major=installed_postgres_image(docker)
        self.assertEqual(ref,POSTGRES_IMAGES[0][0]);self.assertIn("@sha256:",ref)
        self.assertEqual(major,18);self.assertEqual(image,"sha256:"+"a"*64)
        self.assertEqual(len(calls),1)
        self.assertEqual(calls[0][:2],("image","inspect"))

    def test_tag_fallback_retains_major_and_invalid_identity_rejected(self):
        def docker(*args,**kwargs):
            return subprocess.CompletedProcess(args,0 if args[2]=="postgres:17-alpine" else 1,
                                               "sha256:"+"b"*64,"")
        self.assertEqual(installed_postgres_image(docker)[2],17)
        with self.assertRaises(AssertionError):
            installed_postgres_image(lambda *a,**k:subprocess.CompletedProcess(a,0,"bad-image", ""))


if __name__ == "__main__":
    unittest.main()
