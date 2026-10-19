"""Real PostgreSQL key quota and opening consumption tests in Docker.

Exercises scripts/legacy_key_quota_opening.py against PostgreSQL 18-alpine.
Follows the pattern of test_financial_import_pg.py:
- Ephemeral network-isolated container
- tmpfs mounts, non-default roles, offline owner checks
- Verifies finite limits, negative remaining, unlimited display basis, zero quota,
  replay idempotency, conflict rejection, sequence conservation, and admission logic.
"""
from __future__ import annotations

import copy
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import unittest
import uuid

_SCRIPTS_DIR = str(Path(__file__).parent)
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

import legacy_key_quota_opening as KQ
from test_financial_import import source_fixture

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.environ.get("LMM_FINANCIAL_PG_TEST") == "1", "isolated PostgreSQL test not enabled")
class PostgreSQLKeyQuotaTests(unittest.TestCase):
    @classmethod
    def docker(cls, *args, input=None, check=True, timeout=90):
        result = subprocess.run(
            ["docker", "--host", "unix:///var/run/docker.sock", *args],
            input=input, text=True, capture_output=True, timeout=timeout, check=False
        )
        if check and result.returncode:
            raise AssertionError("Docker command failed: " + result.stderr[-4000:])
        return result

    @classmethod
    def setUpClass(cls):
        cls.name = "lmm-kq-" + uuid.uuid4().hex
        cls.label = "lmm.test.kq=" + cls.name
        for candidate in ("postgres:18-alpine", "postgres:17-alpine"):
            image = cls.docker("image", "inspect", candidate, "--format", "{{.Id}}", check=False)
            if image.returncode == 0:
                cls.image = image.stdout.strip()
                break
        else:
            raise AssertionError("PostgreSQL image not found")

        data_mount = "/var/lib/postgresql:rw,size=512m" if "18" in candidate else "/var/lib/postgresql/data:rw,size=512m"
        cls.docker(
            "run", "--detach", "--pull=never", "--network=none", "--name", cls.name,
            "--label", cls.label, "--memory=384m", "--cpus=1", "--pids-limit=128",
            "--tmpfs", data_mount, "--tmpfs", "/var/run/postgresql:rw,size=16m",
            "--env", "POSTGRES_HOST_AUTH_METHOD=trust", cls.image, "postgres",
            "-c", "listen_addresses=", "-c", "autovacuum=off", "-c", "max_connections=20",
            "-c", "shared_buffers=32MB", "-c", "lmm.migration_instance_nonce=" + cls.name
        )

        # Wait until PostgreSQL PID 1 is ready
        for _ in range(300):
            if cls.docker(
                "exec", cls.name, "sh", "-c",
                'test "$(cat /proc/1/comm)" = postgres && pg_isready -q -U postgres',
                check=False
            ).returncode == 0:
                break
            time.sleep(0.1)
        else:
            raise AssertionError("PostgreSQL fixture did not become ready")

        cls.db = "postgres"
        cls.sql(
            "CREATE ROLE a03_owner NOLOGIN; CREATE ROLE a03_operator LOGIN;"
            "GRANT a03_owner TO a03_operator; CREATE ROLE a03_runtime NOLOGIN;",
            admin=True
        )

    _all_passed = True

    def tearDown(self):
        for _, err in getattr(self._outcome, 'errors', []):
            if err is not None:
                type(self)._all_passed = False
        if hasattr(self._outcome, 'result') and not self._outcome.result.wasSuccessful():
            type(self)._all_passed = False

    @classmethod
    def tearDownClass(cls):
        # Retain failure container if any test fails
        if not getattr(cls, "_all_passed", False):
            sys.stderr.write(f"\n[RETAINED_FAILURE_SCENE] Test failed; container {cls.name} retained for review.\n")
            return

        try:
            val = cls.docker("inspect", cls.name, "--format", '{{index .Config.Labels "lmm.test.kq"}}', check=False)
            if val.returncode != 0 or val.stdout.strip() != cls.name:
                sys.stderr.write(f"\n[REFUSING_CLEANUP] Unowned container or mismatch: {cls.name}\n")
                return

            clients = cls.query(
                "SELECT count(*) FROM pg_stat_activity WHERE pid <> pg_backend_pid() AND backend_type = 'client backend';",
                admin=True
            )
            if clients > 0:
                sys.stderr.write(f"\n[RETAINED_ACTIVE_CLIENTS] Container {cls.name} has other clients; retaining.\n")
                return

            cls.docker("rm", "--force", "--volumes", cls.name, check=False)
        except Exception as e:
            sys.stderr.write(f"\n[CLEANUP_EXCEPTION] Could not clean up {cls.name}: {e}\n")

    @classmethod
    def command(cls, *, admin=False, application_name="kq-synthetic-test"):
        args = ["docker", "--host", "unix:///var/run/docker.sock", "exec", "-i",
                "--env", "PGAPPNAME=" + application_name]
        if not admin:
            args += ["--env", "PGOPTIONS=-c role=a03_owner"]
        return args + [cls.name, "psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1",
                       "-h", "/var/run/postgresql", "-U", "postgres" if admin else "a03_operator", "-d", cls.db]

    @classmethod
    def sql(cls, sql, *, admin=False, check=True):
        result = subprocess.run(
            cls.command(admin=admin), input=sql, text=True, capture_output=True, timeout=140, check=False
        )
        if check and result.returncode:
            raise AssertionError("PostgreSQL SQL failed: " + result.stderr[-6000:])
        return result

    @classmethod
    def query(cls, sql, admin=False):
        return json.loads(cls.sql("SET timezone='UTC'; " + sql, admin=admin).stdout)

    def get_sequence_states(self):
        seq_records = self.query("""
            SELECT coalesce(json_agg(jsonb_build_object(
                'relname', c.relname,
                'full_name', n.nspname || '.' || c.relname
            ) ORDER BY c.relname), '[]'::json)
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE c.relkind = 'S' AND n.nspname NOT IN ('pg_catalog', 'information_schema');
        """)
        if not seq_records:
            return {}
        queries = [
            f"SELECT {KQ.literal(item['relname'])} AS name, jsonb_build_object('last_value', s.last_value, 'is_called', s.is_called) AS val FROM {item['full_name']} s"
            for item in seq_records
        ]
        sql = f"SELECT jsonb_object_agg(name, val) FROM ({' UNION ALL '.join(queries)}) q;"
        return self.query(sql)

    def setUp(self):
        type(self).db = "postgres"
        db = "kq_" + uuid.uuid4().hex
        self.sql(
            f"CREATE DATABASE {db} OWNER a03_owner; "
            f"REVOKE CONNECT ON DATABASE {db} FROM PUBLIC; "
            f"GRANT CONNECT ON DATABASE {db} TO a03_operator;",
            admin=True
        )
        type(self).db = db
        self.sql("GRANT EXECUTE ON FUNCTION pg_control_system() TO a03_owner;", admin=True)

        contract = KQ.IDENTITY.current_contract()
        components = []
        for name in contract["component_fingerprints"]:
            path = ROOT / "services/core/schema" / (name.replace("-", "_") + ".sql")
            components.append(path.read_text())
        schema_source = (ROOT / "services/core/src/core_schema.rs").read_text()
        contracts = re.findall(r'const CONTRACT: &str = "(.*?)";', schema_source)
        self.assertEqual(len(contracts), 1)

        ddl = "BEGIN;\n" + "\n".join(components) + contracts[0]
        ddl += f"INSERT INTO core_meta.schema_contract(version,fingerprint) VALUES({contract['identity_version']},decode('{contract['identity_fingerprint_sha256']}','hex'));"
        for name, fingerprint in contract["component_fingerprints"].items():
            ddl += f"INSERT INTO core_meta.component_contract VALUES({KQ.literal(name)},decode({KQ.literal(fingerprint)},'hex'));"
        ddl += f"INSERT INTO core_meta.runtime_contract(catalog_fingerprint) SELECT decode(({KQ.IDENTITY.catalog_hash_sql(contract)}),'hex');COMMIT;"
        self.sql(ddl)

        # Base seed: 1 personal account, 1 user, 3 native credentials
        seed = """BEGIN;
INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE VALUES (101,'personal');
INSERT INTO core_identity.users(id,personal_account_id) VALUES (1,101);
INSERT INTO core_identity.credentials(id,digest,kind,user_id,user_version,owner_account_id,expires_at)
OVERRIDING SYSTEM VALUE VALUES
 (201, decode('1111111111111111111111111111111111111111111111111111111111111111','hex'), 'api_key', 1, 1, 101, clock_timestamp() + interval '100 days'),
 (202, decode('2222222222222222222222222222222222222222222222222222222222222222','hex'), 'api_key', 1, 1, 101, clock_timestamp() + interval '100 days'),
 (203, decode('3333333333333333333333333333333333333333333333333333333333333333','hex'), 'api_key', 1, 1, 101, clock_timestamp() + interval '100 days');
COMMIT;"""
        self.sql(seed)

        # Target receipt parameters
        ident_sql = """SELECT jsonb_build_object(
  'role', current_user,
  'session_user', session_user,
  'database', current_database(),
  'database_owner', (SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=current_database()),
  'effective_role_can_login', (SELECT rolcanlogin FROM pg_roles WHERE rolname=current_user),
  'database_oid', (SELECT oid FROM pg_database WHERE datname=current_database()),
  'port', current_setting('port')::int,
  'cluster_system_identifier', (SELECT system_identifier::text FROM pg_control_system()),
  'server_version_num', current_setting('server_version_num')::int,
  'postmaster_started_at', pg_postmaster_start_time()::text,
  'server_instance_nonce', current_setting('lmm.migration_instance_nonce', true)
);"""
        self.instance_ident = self.query(ident_sql)
        self.catalog_hash = self.sql(KQ.IDENTITY.catalog_hash_sql(contract)).stdout.strip()
        contract["catalog_fingerprint_sha256"] = self.catalog_hash
        self.target_receipt = {
            "connection": {
                "database": db,
                "role": "a03_owner",
                "set_role": "a03_owner",
                "login_role": "a03_operator",
                "port": self.instance_ident["port"],
            },
            "instance_identity": self.instance_ident,
            "contract": contract,
        }

    def test_key_quota_import_and_enforcement(self):
        """Verify finite, negative remaining, unlimited, and replay behavior."""
        plan = {
            "format_version": 1,
            "scope": KQ.SCOPE,
            "migration_id": "kq-batch-001",
            "source_archive_sha256": "a" * 64,
            "mapping_sha256": "b" * 64,
            "key_records": [
                # Key 201: Finite, used 500, remaining 1500 -> limit 2000
                {
                    "source_key_id": 11,
                    "target_credential_id": 201,
                    "target_user_id": 1,
                    "target_account_id": 101,
                    "used_units": 500,
                    "remaining_units": 1500,
                    "unlimited": False,
                    "source_secret_sha256": "1" * 64,
                },
                # Key 202: Negative remaining, used 1000, remaining -300 -> limit 700
                {
                    "source_key_id": 12,
                    "target_credential_id": 202,
                    "target_user_id": 1,
                    "target_account_id": 101,
                    "used_units": 1000,
                    "remaining_units": -300,
                    "unlimited": False,
                    "source_secret_sha256": "2" * 64,
                },
                # Key 203: Unlimited, used 250, remaining 750 -> limit 0, display 1000
                {
                    "source_key_id": 13,
                    "target_credential_id": 203,
                    "target_user_id": 1,
                    "target_account_id": 101,
                    "used_units": 250,
                    "remaining_units": 750,
                    "unlimited": True,
                    "source_secret_sha256": "3" * 64,
                },
            ]
        }
        plan["mapping_sha256"] = KQ.digest(plan["key_records"])
        apply_sql = KQ.build_apply_sql(plan, self.target_receipt)
        result = json.loads(self.sql(apply_sql).stdout)
        self.assertEqual(result["status"], "succeeded")
        self.assertEqual(result["keys_imported"], 3)

        # 1. Verify rows in core_charging.budgets
        budgets = self.query(
            "SELECT jsonb_agg(to_jsonb(b) ORDER BY b.key_id) FROM core_charging.budgets b;"
        )
        self.assertEqual(len(budgets), 3)
        self.assertEqual(budgets[0]["key_id"], 201)
        self.assertEqual(budgets[0]["limit_credits"], 2000)
        self.assertFalse(budgets[0]["unlimited"])
        self.assertIsNone(budgets[0]["display_limit_credits"])

        self.assertEqual(budgets[1]["key_id"], 202)
        self.assertEqual(budgets[1]["limit_credits"], 700)
        self.assertFalse(budgets[1]["unlimited"])

        self.assertEqual(budgets[2]["key_id"], 203)
        self.assertEqual(budgets[2]["limit_credits"], 0)
        self.assertTrue(budgets[2]["unlimited"])
        self.assertEqual(budgets[2]["display_limit_credits"], 1000)

        # 2. Verify rows in core_charging.budget_opening_consumption
        openings = self.query(
            "SELECT jsonb_agg(to_jsonb(o) ORDER BY o.key_id) FROM core_charging.budget_opening_consumption o;"
        )
        self.assertEqual(len(openings), 3)
        self.assertEqual(openings[0]["consumed_credits"], 500)
        self.assertEqual(openings[0]["start_at"], KQ.LIFETIME_START)
        self.assertEqual(openings[0]["end_at"], KQ.LIFETIME_END)
        self.assertEqual(openings[1]["consumed_credits"], 1000)
        self.assertEqual(openings[2]["consumed_credits"], 250)

        # 3. Verify core_migration.import_runs receipt
        run = self.query(
            "SELECT to_jsonb(r) FROM core_migration.import_runs r WHERE migration_id='kq-batch-001';"
        )
        self.assertEqual(run["migration_id"], "kq-batch-001")
        self.assertEqual(run["source_archive_sha256"], "a" * 64)

        # 4. Verify Core ChargingEngine admission evaluation logic:
        # Key 201: limit=2000, consumed=500 -> remaining=1500
        fits_1500 = self.query(
            """SELECT to_jsonb((SELECT consumed_credits FROM core_charging.budget_opening_consumption WHERE key_id=201) + 1500
               <= (SELECT limit_credits FROM core_charging.budgets WHERE key_id=201));"""
        )
        self.assertTrue(fits_1500)

        fits_1501 = self.query(
            """SELECT to_jsonb((SELECT consumed_credits FROM core_charging.budget_opening_consumption WHERE key_id=201) + 1501
               <= (SELECT limit_credits FROM core_charging.budgets WHERE key_id=201));"""
        )
        self.assertFalse(fits_1501)

        # Key 202: negative remaining (limit=700, consumed=1000). Any non-negative reserve fails!
        fits_zero = self.query(
            """SELECT to_jsonb((SELECT consumed_credits FROM core_charging.budget_opening_consumption WHERE key_id=202) + 0
               <= (SELECT limit_credits FROM core_charging.budgets WHERE key_id=202));"""
        )
        self.assertFalse(fits_zero)

        # Key 203: unlimited=true, display basis = 1000 - 250 = 750
        snapshot_203 = self.query(
            """SELECT jsonb_build_object(
               'used', (SELECT consumed_credits FROM core_charging.budget_opening_consumption WHERE key_id=203),
               'remaining', (SELECT display_limit_credits - consumed_credits FROM core_charging.budgets b
                             JOIN core_charging.budget_opening_consumption o ON o.key_id=b.key_id WHERE b.key_id=203),
               'unlimited', (SELECT unlimited FROM core_charging.budgets WHERE key_id=203)
            );"""
        )
        self.assertEqual(snapshot_203["used"], 250)
        self.assertEqual(snapshot_203["remaining"], 750)
        self.assertTrue(snapshot_203["unlimited"])

        # 5. Strict Replay Idempotency: Running the same plan again is a NO-OP
        seq_before = self.query("SELECT to_jsonb(last_value) FROM core_charging.budgets_id_seq;")
        replay_result = self.sql(apply_sql)
        self.assertEqual(replay_result.returncode, 0)
        seq_after = self.query("SELECT to_jsonb(last_value) FROM core_charging.budgets_id_seq;")
        self.assertEqual(seq_before, seq_after)
        budget_count = self.query("SELECT to_jsonb(count(*)) FROM core_charging.budgets;")
        self.assertEqual(budget_count, 3)

        # 6. Conflicting Replay (same migration_id, different mapping hash) MUST be rejected
        conflicting_plan = copy.deepcopy(plan)
        conflicting_plan["mapping_sha256"] = "c" * 64
        conflict_sql = KQ.build_apply_sql(conflicting_plan, self.target_receipt)
        conflict_res = self.sql(conflict_sql, check=False)
        self.assertNotEqual(conflict_res.returncode, 0)
        self.assertIn("key_quota_import_conflict", conflict_res.stderr)

        # 7. Key Overlap Rejection: Different migration_id with already-imported key
        overlap_plan = {
            "format_version": 1,
            "scope": KQ.SCOPE,
            "migration_id": "kq-batch-002",
            "source_archive_sha256": "a" * 64,
            "mapping_sha256": "d" * 64,
            "key_records": [
                {
                    "source_key_id": 99,
                    "target_credential_id": 201,  # Already imported!
                    "target_user_id": 1,
                    "target_account_id": 101,
                    "used_units": 10,
                    "remaining_units": 10,
                    "unlimited": False,
                    "source_secret_sha256": "1" * 64,
                }
            ]
        }
        overlap_plan["mapping_sha256"] = KQ.digest(overlap_plan["key_records"])
        overlap_sql = KQ.build_apply_sql(overlap_plan, self.target_receipt)
        overlap_res = self.sql(overlap_sql, check=False)
        self.assertNotEqual(overlap_res.returncode, 0)
        self.assertIn("key budget already exists", overlap_res.stderr)

    def test_zero_quota_and_boundary_limits(self):
        """Verify zero quota, maximum BIGINT limits, and overflow rejection."""
        # 1. Overflow plan validation must fail before SQL generation
        overflow_plan = {
            "format_version": 1,
            "scope": KQ.SCOPE,
            "migration_id": "kq-overflow",
            "source_archive_sha256": "a" * 64,
            "mapping_sha256": "b" * 64,
            "key_records": [
                {
                    "source_key_id": 1,
                    "target_credential_id": 201,
                    "target_user_id": 1,
                    "target_account_id": 101,
                    "used_units": KQ.BIGINT_MAX,
                    "remaining_units": 1,
                    "unlimited": False,
                    "source_secret_sha256": "0" * 64,
                }
            ]
        }
        with self.assertRaises(KQ.Error):
            KQ.build_apply_sql(overflow_plan, self.target_receipt)

        # 2. Valid zero quota and BIGINT_MAX quota import
        boundary_plan = {
            "format_version": 1,
            "scope": KQ.SCOPE,
            "migration_id": "kq-boundary",
            "source_archive_sha256": "a" * 64,
            "mapping_sha256": "b" * 64,
            "key_records": [
                # Key 201: Zero quota
                {
                    "source_key_id": 1,
                    "target_credential_id": 201,
                    "target_user_id": 1,
                    "target_account_id": 101,
                    "used_units": 0,
                    "remaining_units": 0,
                    "unlimited": False,
                    "source_secret_sha256": "1" * 64,
                },
                # Key 202: Maximum BIGINT limit
                {
                    "source_key_id": 2,
                    "target_credential_id": 202,
                    "target_user_id": 1,
                    "target_account_id": 101,
                    "used_units": 0,
                    "remaining_units": KQ.BIGINT_MAX,
                    "unlimited": False,
                    "source_secret_sha256": "2" * 64,
                },
            ]
        }
        boundary_plan["mapping_sha256"] = KQ.digest(boundary_plan["key_records"])
        apply_sql = KQ.build_apply_sql(boundary_plan, self.target_receipt)
        result = json.loads(self.sql(apply_sql).stdout)
        self.assertEqual(result["status"], "succeeded")

        # Zero quota: reserve 1 credit must fail
        zero_fits = self.query(
            """SELECT to_jsonb((SELECT consumed_credits FROM core_charging.budget_opening_consumption WHERE key_id=201) + 1
               <= (SELECT limit_credits FROM core_charging.budgets WHERE key_id=201));"""
        )
        self.assertFalse(zero_fits)

        # Max quota: reserve 1,000,000,000 credits fits
        max_fits = self.query(
            """SELECT to_jsonb((SELECT consumed_credits FROM core_charging.budget_opening_consumption WHERE key_id=202) + 1000000000
               <= (SELECT limit_credits FROM core_charging.budgets WHERE key_id=202));"""
        )
        self.assertTrue(max_fits)

    def test_invalid_credential_and_guard_rejections_preserve_state(self):
        """Verify invalid credentials or mismatched guard causes total rollback."""
        # 1. Non-existent credential
        invalid_cred_plan = {
            "format_version": 1,
            "scope": KQ.SCOPE,
            "migration_id": "kq-invalid-cred",
            "source_archive_sha256": "a" * 64,
            "mapping_sha256": "b" * 64,
            "key_records": [
                {
                    "source_key_id": 999,
                    "target_credential_id": 99999,  # Does not exist!
                    "target_user_id": 1,
                    "target_account_id": 101,
                    "used_units": 10,
                    "remaining_units": 10,
                    "unlimited": False,
                    "source_secret_sha256": "9" * 64,
                }
            ]
        }
        invalid_cred_plan["mapping_sha256"] = KQ.digest(invalid_cred_plan["key_records"])
        apply_sql = KQ.build_apply_sql(invalid_cred_plan, self.target_receipt)
        res = self.sql(apply_sql, check=False)
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("target credential 99999 invalid", res.stderr)
        # Verify 0 rows in core_charging.budgets
        self.assertEqual(self.query("SELECT to_jsonb(count(*)) FROM core_charging.budgets;"), 0)

        # 2. Mismatched database OID in target receipt
        bad_receipt = copy.deepcopy(self.target_receipt)
        bad_receipt["instance_identity"]["database_oid"] = 99999999
        valid_plan = {
            "format_version": 1,
            "scope": KQ.SCOPE,
            "migration_id": "kq-guard-fail",
            "source_archive_sha256": "a" * 64,
            "mapping_sha256": "b" * 64,
            "key_records": [
                {
                    "source_key_id": 1,
                    "target_credential_id": 201,
                    "target_user_id": 1,
                    "target_account_id": 101,
                    "used_units": 10,
                    "remaining_units": 10,
                    "unlimited": False,
                    "source_secret_sha256": "1" * 64,
                }
            ]
        }
        valid_plan["mapping_sha256"] = KQ.digest(valid_plan["key_records"])
        bad_guard_sql = KQ.build_apply_sql(valid_plan, bad_receipt)
        res2 = self.sql(bad_guard_sql, check=False)
        self.assertNotEqual(res2.returncode, 0)
        self.assertIn("opening target identity mismatch", res2.stderr)
        self.assertEqual(self.query("SELECT to_jsonb(count(*)) FROM core_charging.budgets;"), 0)

    def test_sequence_conservation_across_all_schemas(self):
        """Verify that ONLY core_charging.budgets_id_seq advances upon key quota import."""
        seqs_before = self.get_sequence_states()

        # Import 2 keys
        plan = {
            "format_version": 1,
            "scope": KQ.SCOPE,
            "migration_id": "kq-seq-test",
            "source_archive_sha256": "a" * 64,
            "mapping_sha256": "b" * 64,
            "key_records": [
                {
                    "source_key_id": 1,
                    "target_credential_id": 201,
                    "target_user_id": 1,
                    "target_account_id": 101,
                    "used_units": 100,
                    "remaining_units": 200,
                    "unlimited": False,
                    "source_secret_sha256": "1" * 64,
                },
                {
                    "source_key_id": 2,
                    "target_credential_id": 202,
                    "target_user_id": 1,
                    "target_account_id": 101,
                    "used_units": 50,
                    "remaining_units": 150,
                    "unlimited": False,
                    "source_secret_sha256": "2" * 64,
                },
            ]
        }
        plan["mapping_sha256"] = KQ.digest(plan["key_records"])
        apply_sql = KQ.build_apply_sql(plan, self.target_receipt)
        result = json.loads(self.sql(apply_sql).stdout)
        self.assertEqual(result["status"], "succeeded")

        seqs_after = self.get_sequence_states()

        # Verify only budgets_id_seq changed
        for seq_name, state_before in seqs_before.items():
            state_after = seqs_after[seq_name]
            if seq_name == "budgets_id_seq":
                # Sequence restarted with next available ID (3) and is_called=false
                self.assertEqual(state_after["last_value"], 3)
                self.assertFalse(state_after["is_called"])
            else:
                self.assertEqual(
                    state_before, state_after,
                    f"Unrelated sequence {seq_name} was mutated: before={state_before}, after={state_after}"
                )

        # Verify subsequent insert with default nextval uses allocated sequence ID without collision
        self.sql(
            "BEGIN TRANSACTION ISOLATION LEVEL SERIALIZABLE; "
            "INSERT INTO core_charging.budgets(scope,key_id,issuer_user_id,period,limit_credits,unlimited) "
            "VALUES ('key',203,1,'lifetime',0,TRUE); "
            "COMMIT;"
        )
        next_id = self.query("SELECT to_jsonb(id) FROM core_charging.budgets WHERE key_id = 203;")
        self.assertEqual(next_id, 3)


if __name__ == "__main__":
    unittest.main()

