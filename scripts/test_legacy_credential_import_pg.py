"""Generated identity -> financial -> credential SQL on a new Unix-only PG cluster.

Run LMM_CREDENTIAL_IMPORT_PG_TEST=1 python3 -B
scripts/test_legacy_credential_import_pg.py -v. Retains private synthetic inputs,
SQL, command results and stopped database under ~/Backups/api-lmm-credential-import/local/.
Uses complete reviewed installer DDL, not compiled init-db. Financial source
metadata/review are synthetic: original archive/export validation, production
financial approval and runtime authentication are outside this test's scope.
"""
from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import selectors
import subprocess
import tempfile
import time
import unittest
import uuid

import legacy_credential_import as C
from test_financial_import import F, material_fixture, source_fixture, tables, user
from test_financial_import_pg import reviewed_fixture_ddl
from test_legacy_api_key_migration import inputs, token


def coherent_source():
    source = source_fixture()
    archive = source["plan"]["source_archive_sha256"]
    identity = F.IDENTITY.map_identity([
        {"id": uid, "username": "credential-chain-" + str(uid),
         "password": "$2b$10$" + "." * 53, "display_name": "Synthetic chain",
         "email": "", "setting": "{}", "role": 1, "status": 1,
         "auth_version": 1, "deleted_at": None, "trust_level_override": 1,
         "console_activated_at": 0} for uid in source["identities"]], [], source_sha256=archive)
    manifest = source["identity_run"]["manifest"]
    manifest.update(mapping_sha256=identity["mapping_sha256"], summary=identity["summary"],
                    source_export_sha256=F.sha(b"synthetic-frozen-export"))
    source["files"]["source_export"]["sha256"] = manifest["source_export_sha256"]
    provenance = F.PLANNER.bind_identity_provenance(manifest,
        map_file_sha256=manifest["source_target_id_map_sha256"], source_archive_sha256=archive)
    plan = F.PLANNER.plan_financial(tables(users=[user(71, quota=-120),
        user(72, quota=0, aff_quota=-5), user(73, quota=43, aff_quota=0)],
        tokens=[], user_subscriptions=[], subscription_plans=[]), source["identities"],
        source_sha256=archive, identity_provenance=provenance)
    source["identity_plan"] = identity
    source["identity_run"]["mapping_sha256"] = identity["mapping_sha256"]
    source["plan"] = plan
    source["review"]["mapping_sha256"] = plan["mapping_sha256"]
    source["reservations"]["mapping_sha256"] = plan["mapping_sha256"]
    return source


@unittest.skipUnless(os.environ.get("LMM_CREDENTIAL_IMPORT_PG_TEST") == "1",
                     "isolated credential chain PostgreSQL test not enabled")
class CredentialImportPostgreSQLTests(unittest.TestCase):
    owner = "credential_owner"
    operator = "credential_installer"
    runtime = "credential_runtime"
    other = "credential_other"

    @classmethod
    def setUpClass(cls):
        root = C.IDENTITY.BACKUP_ROOT
        for part in ("api-lmm-credential-import", "local", time.strftime("%Y-%m-%d")):
            root = root / part
            root.mkdir(mode=0o700, exist_ok=True)
            C.IDENTITY.private_backup_path(root, directory=True)
        cls.output = Path(tempfile.mkdtemp(prefix="run-", dir=root))
        cls.socket = cls.output / "sock"
        cls.socket.mkdir(mode=0o700)
        cls.data = cls.output / "data"
        cls.env = {k: v for k, v in os.environ.items()
                   if not k.startswith("PG") and k not in ("DATABASE_URL", "SQL_DSN")}
        cls.env.update(PGPASSFILE="/dev/null", PGSERVICEFILE="/dev/null")
        cls.command_count = 0
        print("Synthetic credential chain evidence: " + str(cls.output), flush=True)
        cls.command(["initdb", "-D", str(cls.data), "--auth=trust",
                     "--username=credential_admin", "--no-locale", "--encoding=UTF8"])
        cls.command(["pg_ctl", "-D", str(cls.data), "-l", str(cls.output / "postgres.log"),
                     "-o", "-k " + str(cls.socket) + " -p 55474 -c listen_addresses=''"
                     " -c max_connections=10 -c shared_buffers=16MB -c autovacuum=off"
                     " -c lmm.migration_instance_nonce=" + uuid.uuid4().hex + uuid.uuid4().hex,
                     "-w", "start"])
        cls.addClassCleanup(cls.command, ["pg_ctl", "-D", str(cls.data), "-m", "fast", "-w", "stop"])
        cls.db = "postgres"
        cls.sql("CREATE ROLE credential_owner NOLOGIN; CREATE ROLE credential_installer LOGIN;"
                "GRANT credential_owner TO credential_installer; CREATE ROLE credential_runtime NOLOGIN;"
                "CREATE ROLE credential_other LOGIN;", admin=True)
        cls.input_hashes = cls.source_inputs()
        cls.write("inputs.json", F.canonical(cls.input_hashes) + "\n")
        cls.addClassCleanup(cls.check_inputs)

    @classmethod
    def source_inputs(cls):
        root = Path(__file__).resolve().parent.parent
        paths = set(root.joinpath("scripts").glob("*.py"))
        paths.update(root / name for name in C.IDENTITY.current_contract()["source_file_hashes"])
        hashes = {str(path.relative_to(root)): C.IDENTITY.sha256_file(path) for path in sorted(paths)}
        for name, raw in C.IDENTITY.component_schemas().items():
            hashes["schema:" + name] = F.sha(raw)
        return hashes

    @classmethod
    def check_inputs(cls):
        actual = cls.source_inputs()
        cls.write("inputs-after.json", F.canonical(actual) + "\n")
        if actual != cls.input_hashes:
            raise AssertionError("test inputs changed during execution")

    @classmethod
    def write(cls, name, content):
        path = cls.output / name
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(content.encode() if isinstance(content, str) else content)
        return path

    @classmethod
    def command(cls, argv, *, sql=None, check=True, env=None):
        cls.command_count += 1
        label = "command-" + str(cls.command_count)
        if sql is not None:
            cls.write(label + ".sql", sql)
        result = subprocess.run(argv, input=sql, text=True, capture_output=True,
                                env=env or cls.env, timeout=150, umask=0o077)
        cls.write(label + ".json", json.dumps({"argv": argv, "exit_code": result.returncode,
            "stdout": result.stdout, "stderr": result.stderr}, indent=2) + "\n")
        if check and result.returncode:
            raise AssertionError(label + " failed: " + result.stderr)
        return result

    @classmethod
    def argv(cls, *, admin=False):
        return ["psql", "-XqAt", "--no-password", "-v", "ON_ERROR_STOP=1", "-h", str(cls.socket),
                "-p", "55474", "-U", "credential_admin" if admin else cls.operator, "-d", cls.db]

    @classmethod
    def sql(cls, sql, *, admin=False, check=True):
        env = dict(cls.env)
        if not admin:
            env["PGOPTIONS"] = "-c role=" + cls.owner + " -c timezone=UTC"
        return cls.command(cls.argv(admin=admin), sql=sql, check=check, env=env)

    @classmethod
    def query(cls, sql):
        return json.loads(cls.sql(sql.rstrip() + ";\n").stdout)

    @classmethod
    def scalar(cls, sql):
        return cls.query("SELECT to_jsonb(value) FROM (" + sql.rstrip(";\n ") + ") scalar_value(value)")

    def setUp(self):
        type(self).db = "postgres"
        db = "credential_" + uuid.uuid4().hex
        self.sql("CREATE DATABASE " + db + " OWNER " + self.owner + ";", admin=True)
        type(self).db = db
        self.sql("REVOKE CONNECT ON DATABASE " + db + " FROM PUBLIC; GRANT CONNECT ON DATABASE " + db +
                 " TO " + self.operator + "; GRANT EXECUTE ON FUNCTION pg_control_system() TO " + self.owner + ";", admin=True)
        # Retain each owned database for failure evidence; the whole cluster is stopped.
        self.sql(reviewed_fixture_ddl(F.IDENTITY.current_contract()))
        source = coherent_source()
        contract = F.IDENTITY.current_contract()
        target = {**contract, "catalog_fingerprint_sha256": self.scalar(F.IDENTITY.catalog_hash_sql(contract)),
            "allow_initialized_core_seed": True, "initialized_core_seed_contract": F.IDENTITY.CORE_SEED_CONTRACT,
            "initialized_core_seed_sha256": self.scalar(F.IDENTITY.fresh_seed_hash_query()),
            "trust_policy_seed": self.query(F.IDENTITY.trust_policy_seed_query())}
        identity_sql = F.IDENTITY.build_import_sql(source["identity_plan"], migration_id="synthetic-identity",
            source_sha256=source["plan"]["source_archive_sha256"], target_contract=target,
            manifest=source["identity_run"]["manifest"], target_identity=self.query(F.IDENTITY.IDENTITY_SQL))
        self.sql(identity_sql)
        self.inventory = self.query(F.inventory_sql())
        financial_snapshot = self.query(F.snapshot_sql(self.inventory, "synthetic-financial"))
        financial = material_fixture(source, financial_snapshot, runtime_roles=[self.runtime])
        self.assertEqual(self.query(F.build_apply_sql(financial))["status"], "COMMITTED")
        self.sql("INSERT INTO routing_price_versions(version,document) VALUES(1,'{\"version\":1}');"
                 "INSERT INTO routing_config_versions(version,price_version,document) VALUES(1,1,'{\"version\":1}');"
                 "INSERT INTO routing_groups(version,name,enabled) VALUES(1,'paid',true),(1,'backup',true);"
                 "UPDATE routing_price_versions SET sealed=true WHERE version=1;"
                 "UPDATE routing_config_versions SET sealed=true WHERE version=1;"
                 "UPDATE routing_active_release SET config_version=1;"
                 "INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference)"
                 " SELECT u.id,g.name,true,'synthetic-chain' FROM core_identity.users u CROSS JOIN routing_groups g;")
        wire, projection = inputs([token(user_id=71), token(id=18, user_id=72,
            key="SyntheticRecoverableCredentialBytes1234567890", one_time_reveal=False)])
        wire.update(source_archive_sha256=source["plan"]["source_archive_sha256"],
                    source_export_sha256=source["files"]["source_export"]["sha256"])
        wire["users"] = [{"id": uid, "status": 1, "deleted_at": None, "auth_version": 1,
                          "group": "paid", "billing_preference": "wallet_first"} for uid in source["identities"]]
        snapshot = self.query(C.snapshot_sql(self.inventory, "synthetic-credentials"))
        projection.update(source_archive_sha256=wire["source_archive_sha256"],
            source_target_id_map=[{"source_user_id": uid, "target_user_id": uid,
                "target_personal_account_id": aid} for uid, aid in source["identities"].items()],
            source_target_id_map_sha256=source["identity_run"]["manifest"]["source_target_id_map_sha256"],
            users=snapshot["native_users"])
        proposal = C.PROPOSAL.plan_api_keys(wire, projection)
        economic = snapshot["financial_import"]["manifest"]["material"]["source_review"]
        review = {"format_version": 1, "scope": C.SCOPE,
            "decision": "approve-offline-native-equivalent-credentials", "source_input_sha256": C.PROPOSAL.sha256(wire),
            "target_input_sha256": C.PROPOSAL.sha256(projection), "proposal_sha256": proposal["proposal_sha256"],
            "source_frozen": True, "source_unit": "source_quota_integer", "native_unit": "credit_500k_usd",
            "numerator": 1, "denominator": 1, "evidence_sha256": economic["evidence_sha256"],
            "economic_review_sha256": F.digest(economic)}
        self.files = {}
        for name, value in (("source", wire), ("target_projection", projection),
                            ("proposal", proposal), ("review", review), ("encryption_key", bytes(range(32)))):
            raw = value if isinstance(value, bytes) else (F.canonical(value) + "\n").encode()
            path = self.write(db + "-" + name, raw)
            self.files[name] = {"path": str(path), "sha256": F.sha(raw)}
        self.material = C.prepare_material(wire, projection, proposal, review,
            migration_id="synthetic-credentials", snapshot=snapshot, runtime_roles=[self.runtime],
            source_files=self.files, encryption_key=bytes(range(32)))
        C.verify_source_files(self.material)
        self.apply = C.build_apply_sql(self.material)
        self.verify = C.build_verify_sql(self.material, "lmm-credential-" + uuid.uuid4().hex)
        self.before = self.full_state()
        self.write(db + "-material.json", F.canonical(self.material) + "\n")

    def full_state(self):
        return self.query(F.state_sql(self.inventory, "no-such-migration"))

    def rejected(self, sql, error):
        before = self.full_state()
        result = self.sql(sql, check=False)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(error, result.stderr)
        self.assertEqual(self.full_state(), before)

    def assert_identity_and_financial_rows(self):
        self.assertEqual(self.query("SELECT jsonb_agg(jsonb_build_array(id,personal_account_id,platform_role,active,auth_version) ORDER BY id) FROM core_identity.users"),
                         [[71, 1, "user", True, 1], [72, 2, "user", True, 1], [73, 3, "user", True, 1]])
        self.assertEqual(self.query("SELECT jsonb_agg(jsonb_build_array(id,kind,active) ORDER BY id) FROM core_identity.accounts"),
                         [[1, "personal", True], [2, "personal", True], [3, "personal", True]])
        self.assertEqual(self.query("SELECT jsonb_agg(jsonb_build_array(user_id,granted) ORDER BY user_id) FROM core_identity.developer_access"),
                         [[71, True], [72, True], [73, True]])
        self.assertEqual(self.query("SELECT count(*) FROM core_identity.teams"), 0)
        self.assertEqual(self.query("SELECT jsonb_agg(jsonb_build_array(a.owner_account_id,a.bucket,b.balance_units) ORDER BY a.owner_account_id NULLS FIRST,a.bucket)"
            " FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id"),
            [[None, "clearing", 37], [None, "revenue", 0],
             [1, "affiliate", 38], [1, "debt", -120], [1, "reserved", 7], [1, "wallet", 0],
             [2, "affiliate", -5], [2, "debt", 0], [2, "reserved", 0], [2, "wallet", 0],
             [3, "affiliate", 0], [3, "debt", 0], [3, "reserved", 0], [3, "wallet", 43]])
        self.assertEqual(self.query("SELECT sum(balance_units) FROM core_billing.balance_state"), 0)
        self.assertEqual(self.query("SELECT count(*) FROM core_billing.ledger_entries"), 8)
        self.assertEqual(self.query("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0 OR count(*)<2) bad"), 0)
        for table in ("ledger_journals", "ledger_operations", "opening_imports", "account_opening_counters"):
            self.assertEqual(self.query("SELECT count(*) FROM core_billing." + table), 3, table)
        self.assertEqual(self.query("SELECT jsonb_agg(jsonb_build_array(user_id,account_id,used_quota,request_count,aff_history) ORDER BY user_id) FROM core_billing.account_opening_counters"),
                         [[71, 1, 9001, 11, 103], [72, 2, 9001, 11, 103], [73, 3, 9001, 11, 103]])
        rows = self.query("SELECT jsonb_agg(to_jsonb(r) ORDER BY migration_id) FROM core_migration.import_runs r")
        receipts = {row["migration_id"]: row for row in rows}
        self.assertEqual(set(receipts), {"synthetic-identity", "synthetic-financial", "synthetic-credentials"})
        identity, financial, credentials = (receipts[key] for key in
            ("synthetic-identity", "synthetic-financial", "synthetic-credentials"))
        self.assertEqual(identity["manifest"]["scope"], "identity-only")
        self.assertEqual(financial["manifest"]["scope"], F.SCOPE)
        self.assertEqual(credentials["manifest"]["scope"], C.SCOPE)
        self.assertEqual(financial["manifest"]["material"]["identity_import"], identity)
        self.assertEqual(credentials["manifest"]["material"]["identity_import"], identity)
        self.assertEqual(credentials["manifest"]["material"]["financial_import"], financial)
        self.assertEqual(credentials["manifest"]["material"], self.material)
        for receipt in rows:
            self.assertEqual(receipt["source_archive_sha256"], self.material["source_archive_sha256"])
        self.assertEqual(self.material["source_review"]["economic_review_sha256"],
                         F.digest(financial["manifest"]["material"]["source_review"]))

    def test_commit_readonly_verify_exact_replay_and_business_rows(self):
        self.assertEqual(self.query(self.verify)["status"], "NOT_APPLIED")
        self.assertEqual(self.full_state(), self.before)
        committed = self.query(self.apply)
        self.assertEqual(committed["status"], "COMMITTED")
        after = self.full_state()
        self.assertEqual(self.query(self.verify)["status"], "COMMITTED")
        self.assertEqual(self.query(self.apply), committed)
        self.assertEqual(self.full_state(), after)
        self.assert_identity_and_financial_rows()
        self.assertEqual(self.query("SELECT jsonb_agg(jsonb_build_array(c.user_id,c.owner_account_id,c.user_version,"
            "s.enabled,s.one_time_reveal,b.limit_credits,o.consumed_credits) ORDER BY c.user_id)"
            " FROM core_identity.credentials c JOIN core_identity.api_key_settings s ON s.credential_id=c.id"
            " JOIN core_charging.budgets b ON b.key_id=c.id JOIN core_charging.budget_opening_consumption o ON o.key_id=c.id"),
            [[71, 1, 1, True, True, 100, 70], [72, 2, 1, True, False, 100, 70]])
        for name, value in self.before["tables"].items():
            if name not in (*C.CHANGED_TABLES, "core_migration.import_runs"):
                self.assertEqual(after["tables"][name], value, name)
        for name, value in self.before["sequences"].items():
            if name not in C.CHANGED_SEQUENCES:
                self.assertEqual(after["sequences"][name], value, name)
        for name in C.CHANGED_SEQUENCES:
            self.assertEqual(after["sequences"][name]["last_value"],
                C._next_id(self.material["before_state"], name, 2) + 2)
            self.assertFalse(after["sequences"][name]["is_called"])

    def test_changed_replay_and_committed_state_tamper(self):
        self.sql(self.apply)
        changed = copy.deepcopy(self.material)
        changed["source_files"]["review"]["sha256"] = "0" * 64
        self.rejected(C.build_apply_sql(changed), "credential_replay_conflict")
        self.sql("UPDATE core_identity.api_key_settings SET name='synthetic-tamper';")
        self.rejected(self.verify, "credential_verify_conflict")

    def authority_refused(self, mutation):
        # Revocation generations are monotonic; each case owns a fresh database.
        self.sql(mutation)
        self.rejected(self.apply, "credential_authority_or_financial_opening_changed")
        self.rejected(self.verify, "credential_authority_or_financial_opening_changed")

    def test_routing_grant_revocation_refused(self):
        self.authority_refused("UPDATE core_identity.user_routing_groups SET enabled=false")

    def test_developer_revocation_refused(self):
        self.authority_refused("UPDATE core_identity.developer_access SET granted=false")

    def test_auth_version_change_refused(self):
        self.authority_refused("UPDATE core_identity.users SET auth_version=2")

    def test_account_deactivation_refused(self):
        self.authority_refused("UPDATE core_identity.accounts SET active=false")

    def test_source_file_mutation_rejected_before_write(self):
        path = Path(self.files["source"]["path"])
        original = path.read_bytes()
        with path.open("ab") as stream:
            stream.write(b" ")
        try:
            with self.assertRaisesRegex(C.Error, "credential_source_file_changed"):
                C.verify_source_files(self.material)
        finally:
            path.write_bytes(original)
        self.assertEqual(self.full_state(), self.before)

    def test_instance_catalog_and_role_refusals(self):
        changed = copy.deepcopy(self.material)
        changed["target"]["instance"]["database_oid"] = "1"
        self.rejected(C.build_apply_sql(changed), "financial_target_instance_mismatch")
        changed = copy.deepcopy(self.material)
        changed["target"]["catalog_fingerprint_sha256"] = "0" * 64
        self.rejected(C.build_apply_sql(changed), "financial_target_catalog_mismatch")
        self.sql("ALTER ROLE " + self.runtime + " LOGIN", admin=True)
        try:
            self.rejected(self.apply, "financial_offline_roles_mismatch")
        finally:
            self.sql("ALTER ROLE " + self.runtime + " NOLOGIN", admin=True)
        self.sql("GRANT CONNECT ON DATABASE " + self.db + " TO " + self.other, admin=True)
        self.rejected(self.apply, "financial_offline_roles_mismatch")

    def test_failure_after_sequence_restart_rolls_back_and_resolves_absent(self):
        marker = "SET CONSTRAINTS ALL IMMEDIATE;"
        self.assertEqual(self.apply.count(marker), 1)
        broken = self.apply.replace(marker, "RAISE EXCEPTION 'synthetic-after-sequence';\n" + marker)
        self.rejected(broken, "synthetic-after-sequence")
        self.assertEqual(self.full_state(), self.before)
        self.assertEqual(self.query(self.verify)["status"], "NOT_APPLIED")
        self.assertEqual(self.query(self.apply)["status"], "COMMITTED")

    def test_backend_disconnect_after_sequence_restart_rolls_back(self):
        app = "lmm-credential-" + uuid.uuid4().hex
        marker = "SET CONSTRAINTS ALL IMMEDIATE;"
        self.assertEqual(self.apply.count(marker), 1)
        interrupted = self.apply.replace(marker,
            "RAISE NOTICE 'CREDENTIAL_SEQUENCE_PAUSED'; PERFORM pg_sleep(30);\n" + marker)
        self.write(self.db + "-disconnect.sql", interrupted)
        env = dict(self.env, PGOPTIONS="-c role=" + self.owner, PGAPPNAME=app)
        with subprocess.Popen(self.argv(), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True, env=env) as process:
            try:
                process.stdin.write(interrupted)
                process.stdin.close()
                process.stdin = None
                notice = b""
                deadline = time.monotonic() + 10
                with selectors.DefaultSelector() as selector:
                    selector.register(process.stderr, selectors.EVENT_READ)
                    while b"CREDENTIAL_SEQUENCE_PAUSED" not in notice and time.monotonic() < deadline:
                        if selector.select(timeout=0.2):
                            chunk = os.read(process.stderr.fileno(), 4096)
                            if not chunk:
                                break
                            notice += chunk
                self.assertIn(b"CREDENTIAL_SEQUENCE_PAUSED", notice)
                stopped = self.sql("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=" +
                    F.literal(self.db) + " AND application_name=" + F.literal(app) +
                    " AND pid<>pg_backend_pid();", admin=True)
                self.assertEqual(stopped.stdout.strip(), "t")
                stdout, stderr = process.communicate(timeout=10)
                self.write(self.db + "-disconnect-result.json", json.dumps({"exit_code": process.returncode,
                    "stdout": stdout, "stderr": notice.decode() + stderr}, indent=2) + "\n")
                self.assertNotEqual(process.returncode, 0)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.communicate(timeout=10)
        self.assertEqual(self.full_state(), self.before)
        self.assertEqual(self.query(C.build_verify_sql(self.material, app))["status"], "NOT_APPLIED")
        self.assertEqual(self.full_state(), self.before)
        self.assertEqual(self.query(self.apply)["status"], "COMMITTED")

    def test_lost_commit_ack_resolves_readonly_without_reapplying(self):
        app = "lmm-credential-" + uuid.uuid4().hex
        env = dict(self.env, PGOPTIONS="-c role=" + self.owner, PGAPPNAME=app)
        self.command(self.argv(), sql=self.apply, env=env)  # Deliberately discard client acknowledgement.
        after = self.full_state()
        self.assertEqual(self.query(C.build_verify_sql(self.material, app))["status"], "COMMITTED")
        self.assertEqual(self.full_state(), after)
        self.assertEqual(self.query(self.apply)["status"], "COMMITTED")
        self.assertEqual(self.full_state(), after)

    def test_verifier_transaction_is_readonly(self):
        self.assertEqual(self.verify.count("COMMIT;"), 1)
        probe = self.verify.replace("COMMIT;", "SELECT current_setting('transaction_read_only'); COMMIT;")
        lines = self.sql(probe).stdout.splitlines()
        self.assertEqual(json.loads(lines[0])["status"], "NOT_APPLIED")
        self.assertEqual(lines[1:], ["on"])
        self.assertEqual(self.full_state(), self.before)

    def test_unrelated_state_prevents_apply_and_absence_claim(self):
        self.sql("INSERT INTO core_telegram_auth.bindings(subject,user_id) VALUES('12345',71)")
        self.rejected(self.apply, "credential_target_state_changed")
        self.rejected(self.verify, "credential_absence_not_proven")

    def test_other_client_and_competing_fence(self):
        env = dict(self.env, PGOPTIONS="-c role=" + self.owner)
        with subprocess.Popen(self.argv(), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True, env=env) as process:
            try:
                process.stdin.write("SELECT 'READY';\n")
                process.stdin.flush()
                with selectors.DefaultSelector() as selector:
                    selector.register(process.stdout, selectors.EVENT_READ)
                    self.assertTrue(selector.select(timeout=10), "other client did not start")
                    self.assertEqual(process.stdout.readline().strip(), "READY")
                self.rejected(self.apply, "financial_target_has_other_clients")
                process.stdin.write("SELECT pg_advisory_lock(" + str(F.FENCE) + "); SELECT 'FENCED';\n")
                process.stdin.flush()
                # psql prints an empty void result before the explicit marker.
                ready = b""
                deadline = time.monotonic() + 10
                with selectors.DefaultSelector() as selector:
                    selector.register(process.stdout, selectors.EVENT_READ)
                    while b"FENCED\n" not in ready and time.monotonic() < deadline:
                        if selector.select(timeout=0.2):
                            chunk = os.read(process.stdout.fileno(), 4096)
                            if not chunk:
                                break
                            ready += chunk
                self.assertIn(b"FENCED\n", ready)
                self.rejected(self.apply, "financial_import_busy")
                self.rejected(self.verify, "financial_import_busy")
            finally:
                process.stdin.close()
                process.wait(timeout=10)
        self.assertEqual(self.full_state(), self.before)


if __name__ == "__main__":
    unittest.main()
