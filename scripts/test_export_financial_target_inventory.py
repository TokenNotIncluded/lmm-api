"""Pure refusal tests for native target observations, not database acceptance."""
import hashlib
from pathlib import Path
import tempfile
import unittest

import export_financial_target_inventory as E


def mapping():
    return [{"source_user_id": 11, "target_user_id": 21, "target_personal_account_id": 31}]


def observation():
    return {"observation_details": [{**mapping()[0], "account_id": 31, "account_kind": "personal",
        "buckets": [{"ledger_account_id": i, "bucket": name, "unit": E.UNIT,
                     "balance_units": units, "revision": 4}
                    for i, name, units in [(101,"wallet",80),(102,"debt",-5),(103,"affiliate",-3),(104,"reserved",7)]]}]}


# @mark migration.financial-target-observation role=test
class TargetObservationTests(unittest.TestCase):
    def test_exact_observed_values_and_signed_positions_are_preserved(self):
        result = E.classify_observation(observation(), mapping())
        self.assertEqual(result["inventory"]["records"], [{"source_user_id": 11,"account_id":31,
            "wallet_units":80,"debt_units_signed":-5,"affiliate_units":-3}])
        self.assertFalse(result["target_inventory_provenance_verified"])
        self.assertFalse(result["financial_migration_or_economic_approval"])

    def test_missing_bucket_never_becomes_zero_or_consumable_inventory(self):
        o=observation();o["observation_details"][0]["buckets"].pop(1)
        result=E.classify_observation(o,mapping())
        self.assertIsNone(result["inventory"])
        self.assertIn("target_bucket_missing",[x["code"] for x in result["issues"]])

    def test_missing_balance_preserves_incomplete(self):
        o=observation();o["observation_details"][0]["buckets"][0]["balance_units"]=None
        self.assertIsNone(E.classify_observation(o,mapping())["inventory"])

    def test_missing_owner_does_not_accept_mapped_account_instead(self):
        for account in [None,32,True]:
            with self.subTest(account=account):
                o=observation();o["observation_details"][0]["account_id"]=account
                self.assertIsNone(E.classify_observation(o,mapping())["inventory"])

    def test_foreign_or_duplicate_source_id_is_rejected(self):
        o=observation();o["observation_details"][0]["source_user_id"]=12
        with self.assertRaises(E.H.MappingError):E.classify_observation(o,mapping())

    def test_actual_target_user_must_equal_reviewed_mapping(self):
        o=observation();o["observation_details"][0]["target_user_id"]=22
        with self.assertRaises(E.H.MappingError):E.classify_observation(o,mapping())

    def test_wrong_unit_revision_type_or_sign_is_incomplete(self):
        for field,value in [("unit","USD"),("revision",None),("revision",True),("balance_units",True),("balance_units",-1)]:
            with self.subTest(field=field,value=value):
                o=observation();o["observation_details"][0]["buckets"][0][field]=value
                self.assertIsNone(E.classify_observation(o,mapping())["inventory"])

    def test_positive_debt_cannot_be_native_inventory(self):
        o=observation();o["observation_details"][0]["buckets"][1]["balance_units"]=1
        self.assertIsNone(E.classify_observation(o,mapping())["inventory"])

    def test_duplicate_bucket_or_ledger_id_is_rejected(self):
        for change in ["bucket","ledger_account_id"]:
            o=observation();o["observation_details"][0]["buckets"][1][change]=o["observation_details"][0]["buckets"][0][change]
            with self.assertRaises(E.H.MappingError):E.classify_observation(o,mapping())

    def test_mapping_rejects_bool_string_duplicate_and_extra_fields(self):
        for value in [True,"11",0,2**63]:
            rows=mapping();rows[0]["source_user_id"]=value
            with self.assertRaises(E.H.MappingError):E.validate_mapping(rows,1)
        rows=mapping();rows[0]["wallet_units"]=80
        with self.assertRaises(E.H.MappingError):E.validate_mapping(rows,1)
        with self.assertRaises(E.H.MappingError):E.validate_mapping(mapping()*2,2)
        with self.assertRaises(E.H.MappingError):E.validate_mapping(mapping(),2)

    def test_duplicate_json_keys_rejected(self):
        with self.assertRaises(E.H.MappingError):E.parse_json('{"id":1,"id":2}')

    def test_frozen_code_or_schema_bytes_cannot_be_relabelled_after_change(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/"helper.py";path.write_bytes(b"old implementation\n")
            frozen={"helper":{"path":path,"sha256":hashlib.sha256(path.read_bytes()).hexdigest()}}
            E.verify_frozen_inputs(frozen,private=False)
            path.write_bytes(b"new implementation\n")
            with self.assertRaisesRegex(E.H.MappingError,"observation_input_changed_during_export"):
                E.verify_frozen_inputs(frozen,private=False)
            self.assertEqual(frozen["helper"]["sha256"],hashlib.sha256(b"old implementation\n").hexdigest())

    def test_sql_uses_one_readonly_snapshot_and_actual_ledger_tables(self):
        sql=E.build_sql(mapping(),{"scope":"identity-only"},"test-id")
        self.assertTrue(sql.startswith("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;"))
        self.assertTrue(sql.endswith("ROLLBACK;\n"))
        self.assertIn("pg_current_snapshot()",sql)
        self.assertIn("LEFT JOIN core_billing.balance_state",sql)
        self.assertNotIn("coalesce(b.balance_units",sql.lower())
        for command in ["INSERT INTO", "UPDATE core_", "DELETE FROM", "CREATE TABLE"]:
            self.assertNotIn(command,sql)


if __name__ == "__main__":
    unittest.main()
