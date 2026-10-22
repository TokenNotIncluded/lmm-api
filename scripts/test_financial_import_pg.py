"""Real PostgreSQL opening/replay/rollback tests; no images or tools downloaded.

LMM_FINANCIAL_PG_TEST=1 uses a network-isolated Docker PostgreSQL18 and reviewed
Core26 DDL (synthetic SQL fixture, NOT compiled init-db acceptance).
LMM_FINANCIAL_PG_FIXTURE_RECEIPT selects a resource-owner Unix-only PG18 receipt
with an already init-db-created empty Core template and matching verified binary
and schema-manifest. That mode clones only into its random owned test databases.
Neither mode connects to a legacy snapshot or any production database.
"""
import copy
import json
import os
from pathlib import Path
import re
import selectors
import subprocess
import sys
import time
import tempfile
import unittest
from unittest.mock import patch
import uuid

_SCRIPTS_DIR = str(Path(__file__).parent)
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

import financial_import as F
from test_financial_import import source_fixture, material_fixture

ROOT = Path(__file__).resolve().parents[1]
# CI service images are pulled by digest and need not have a floating tag.
POSTGRES_IMAGES = (
    ("postgres:18-alpine@sha256:77f585114c32fbca283dc835b0596f4e52b51b4c6662d7810b2f4084f60a1873", 18),
    ("postgres:18-alpine", 18),
)


def installed_postgres_image(docker):
    for reference, major in POSTGRES_IMAGES:
        image = docker("image", "inspect", reference, "--format", "{{.Id}}", check=False)
        if image.returncode == 0:
            identity = image.stdout.strip()
            if not re.fullmatch(r"sha256:[0-9a-f]{64}", identity):
                raise AssertionError("preinstalled PostgreSQL image identity invalid")
            return reference, identity, major
    raise AssertionError("preinstalled PostgreSQL image identity missing")


def expected_schema_manifest(contract):
    return {"format_version": 1, "scope": "compiled-core-schema-contract",
        "components": [{"name": name, "raw_schema_sha256": value}
                       for name, value in contract["component_fingerprints"].items()],
        "identity": {"version": contract["identity_version"], "fingerprint_sha256": contract["identity_fingerprint_sha256"]},
        "installer_contract_sql_sha256": contract["installer_contract_sha256"],
        "catalog": {"query_utf8": contract["catalog_query"], "query_sha256": contract["catalog_query_sha256"],
            "query_hash_domain": "sha256-raw-utf8-v1",
            "fingerprint_hash_domain": "sha256-u64be-byte-length-prefixed-utf8-v1",
            "definition_order": "postgresql-c-collation"}}


def verify_compiled_fixture(receipt, *, expected_status="ready-isolated-synthetic-fixture"):
    """Check owner-supplied files and actual binary output; never init a database."""
    F.require(receipt.get("status") == expected_status
              and receipt.get("isolation", {}).get("synthetic_only", receipt.get("synthetic_only")) is True, "synthetic_fixture_required")
    binary = receipt.get("verified_binary", {})
    manifest = receipt.get("schema_manifest", {})
    binary_path = F.IDENTITY.owned_path(Path(binary.get("path", "")), private=False)
    manifest_path = F.IDENTITY.private_backup_path(Path(manifest.get("path", "")))
    F.require(binary_path.is_absolute() and binary_path.is_file() and os.access(binary_path, os.X_OK)
              and F.IDENTITY.sha256_file(binary_path) == binary.get("sha256"), "fixture_binary_mismatch")
    F.require(F.IDENTITY.sha256_file(manifest_path) == manifest.get("sha256"), "fixture_manifest_file_mismatch")
    contract = F.IDENTITY.current_contract()
    expected = expected_schema_manifest(contract)
    F.require(json.loads(manifest_path.read_text()) == expected, "fixture_manifest_contract_mismatch")
    result = subprocess.run([str(binary_path), "schema-manifest"], text=True, capture_output=True,
                            timeout=30, check=False, env={key: os.environ[key] for key in ("PATH", "LANG", "LC_ALL", "TZ") if key in os.environ})
    F.require(result.returncode == 0 and json.loads(result.stdout) == expected, "fixture_binary_manifest_mismatch")
    F.require(F.IDENTITY.sha256_file(binary_path) == binary["sha256"], "fixture_binary_changed")
    return contract


def reviewed_fixture_ddl(contract):
    """Synthetic fixture only: mirror exact raw installer order, including receipts."""
    parts = ["BEGIN;", *(raw.decode("utf-8") for raw in F.IDENTITY.component_schemas().values()),
             F.IDENTITY.installer_metadata().decode("utf-8")]
    parts.append("INSERT INTO core_meta.schema_contract(version,fingerprint) VALUES(" +
                 str(contract["identity_version"]) + ",decode(" + F.literal(contract["identity_fingerprint_sha256"]) + ",'hex'));")
    oauth = F.IDENTITY.catalog_hash_sql({"catalog_query": "SELECT d AS definition FROM (" + F.IDENTITY.oauth_catalog_query() + ") oauth_definitions"})
    parts.append("INSERT INTO core_oauth.contract(version,ddl_fingerprint,catalog_fingerprint) SELECT 1,decode(" +
                 F.literal(contract["component_fingerprints"]["oauth-grants"]) + ",'hex'),decode((" + oauth + "),'hex');")
    for name, value in contract["component_fingerprints"].items():
        parts.append("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES(" + F.literal(name) + ",decode(" + F.literal(value) + ",'hex'));")
    parts.append("INSERT INTO core_meta.runtime_contract(catalog_fingerprint) SELECT decode((" + F.IDENTITY.catalog_hash_sql(contract) + "),'hex');COMMIT;")
    return "\n".join(parts)


def fixture_connection(receipt, fixture, output):
    """Reuse the existing resource-owner fixture receipt and private socket gate."""
    owner, login = fixture["owner"], fixture["login_role"]
    F.require(owner != login, "fixture_distinct_installer_required")
    role = receipt["roles"][login]
    password = F.IDENTITY.private_backup_path(Path(role["password_file"])).read_text().strip()
    escape = lambda value: str(value).replace("\\", "\\\\").replace(":", "\\:")
    pgpass = F.write_private(output, "pgpass.private", ":".join(map(escape,
        (role["socket_directory"], role["port"], "*", role["role"], password))) + "\n")
    connection = {"socket_directory": role["socket_directory"], "port": role["port"],
        "database": fixture["database"], "role": owner, "login_role": login,
        "set_role": owner, "pgpass_file": str(pgpass)}
    F.IDENTITY.socket_connection(connection)
    return connection


def synthetic_upgrade_receipt_sql(table):
    """Create a constraint-valid forbidden receipt in the owned failure fixture."""
    common_columns = "upgrade_id,procedure,request_sha256,predecessor_manifest_sha256,predecessor_catalog_sha256,successor_manifest_sha256,successor_catalog_sha256"
    zero = "decode(repeat('0',64),'hex')"
    suffix_columns = "database_system_identifier,database_oid,database_name,installer_role,owner_role,runtime_role,extensions_role"
    suffix_values = "(SELECT system_identifier::text FROM pg_control_system()),(SELECT oid FROM pg_database WHERE datname=current_database()),current_database(),session_user,current_user,'synthetic_runtime','synthetic_extensions'"
    if table == "core_meta.schema_upgrades":
        columns = common_columns + ",image_definition_changes," + suffix_columns
        values = "'synthetic-forbidden','core-24-to-25-trust-pricing-v1'," + ",".join([zero] * 5) + ",'[{},{}]'::jsonb," + suffix_values
    elif table == "core_meta.component_upgrades":
        columns = common_columns + ",predecessor_components,successor_components,delta_components,added_catalog_entries,preserved_data_sha256,preserved_permissions_sha256," + suffix_columns
        values = "'synthetic-forbidden','core-additive-components-v1'," + ",".join([zero] * 5) + ",25,26,'[\"telegram-auth\"]'::jsonb,'[\"synthetic\"]'::jsonb," + zero + "," + zero + "," + suffix_values
    else:
        raise AssertionError("unexpected test upgrade table")
    return "INSERT INTO " + table + "(" + columns + ") VALUES(" + values + ");"


class FixtureContractTests(unittest.TestCase):
    def test_reviewed_ddl_includes_current26_metadata_and_oauth_contract(self):
        contract = F.IDENTITY.current_contract()
        ddl = reviewed_fixture_ddl(contract)
        self.assertEqual(len(contract["component_fingerprints"]), 26)
        self.assertIn("VALUES(5,decode(", ddl)
        self.assertIn("CREATE TABLE core_meta.schema_upgrades", ddl)
        self.assertIn("CREATE TABLE core_meta.component_upgrades", ddl)
        self.assertIn("INSERT INTO core_oauth.contract(version,ddl_fingerprint,catalog_fingerprint)", ddl)
        self.assertIn(F.IDENTITY.component_schemas()["trust-pricing"].decode(), ddl)
        self.assertIn(F.IDENTITY.component_schemas()["telegram-auth"].decode(), ddl)

    def test_manifest_binds_component_order_identity_catalog_and_installer(self):
        contract = F.IDENTITY.current_contract()
        expected = expected_schema_manifest(contract)
        self.assertEqual(expected["components"][24]["name"], "trust-pricing")
        self.assertEqual(expected["components"][25]["name"], "telegram-auth")
        for key in ("identity_version", "installer_contract_sha256", "catalog_query_sha256"):
            changed = copy.deepcopy(contract)
            changed[key] = 0 if key == "identity_version" else "0" * 64
            self.assertNotEqual(expected_schema_manifest(changed), expected)


@unittest.skipUnless(os.environ.get("LMM_FINANCIAL_PG_TEST") == "1", "isolated PostgreSQL test not enabled")
class PostgreSQLTests(unittest.TestCase):
    owner_role, installer_role, runtime_role, other_role = "a03_owner", "a03_operator", "a03_runtime", "a03_other"
    @classmethod
    def docker(cls, *args, input=None, check=True, timeout=90):
        result = subprocess.run(["docker", "--host", "unix:///var/run/docker.sock", *args],
            input=input, text=True, capture_output=True, timeout=timeout, check=False)
        if check and result.returncode:
            raise AssertionError("synthetic Docker fixture failed: " + result.stderr[-4000:])
        return result

    @classmethod
    def setUpClass(cls):
        cls.name = "lmm-a03-" + uuid.uuid4().hex
        cls.label = "lmm.test.a03=" + cls.name
        cls.nonce = uuid.uuid4().hex + uuid.uuid4().hex
        reference, cls.image, major = installed_postgres_image(cls.docker)
        print("A03_POSTGRES_REFERENCE=" + reference, flush=True)
        print("A03_POSTGRES_IMAGE=" + cls.image, flush=True)
        data_mount = "/var/lib/postgresql:rw,size=512m" if major >= 18 else "/var/lib/postgresql/data:rw,size=512m"
        cls.docker("run", "--detach", "--pull=never", "--network=none", "--name", cls.name,
            "--label", cls.label, "--memory=384m", "--cpus=1", "--pids-limit=128",
            "--tmpfs", data_mount, "--tmpfs", "/var/run/postgresql:rw,size=16m",
            "--env", "POSTGRES_HOST_AUTH_METHOD=trust", cls.image, "postgres",
            "-c", "listen_addresses=", "-c", "autovacuum=off", "-c", "max_connections=20",
            "-c", "shared_buffers=32MB", "-c", "lmm.migration_instance_nonce=" + cls.nonce)
        cls.addClassCleanup(cls.cleanup)
        # The image starts a temporary bootstrap server first. Wait until PID 1
        # has execed the final server, not merely for that temporary socket.
        for _ in range(300):
            if cls.docker("exec", cls.name, "sh", "-c",
                'test "$(cat /proc/1/comm)" = postgres && pg_isready -q -U postgres',
                check=False).returncode == 0:
                break
            time.sleep(0.1)
        else:
            raise AssertionError("owned PostgreSQL fixture did not become ready")
        cls.db = "postgres"
        cls.sql("CREATE ROLE a03_owner NOLOGIN; CREATE ROLE a03_operator LOGIN;"
                "GRANT a03_owner TO a03_operator; CREATE ROLE a03_runtime NOLOGIN;"
                "CREATE ROLE a03_other LOGIN;", admin=True)

    @classmethod
    def cleanup(cls):
        # Never remove a container unless the exact random ownership label matches.
        value = cls.docker("inspect", cls.name, "--format", '{{index .Config.Labels "lmm.test.a03"}}').stdout.strip()
        if value != cls.name:
            raise AssertionError("refusing to remove an unowned test container")
        cls.docker("rm", "--force", "--volumes", cls.name)

    @classmethod
    def command(cls, *, admin=False, application_name="a03-synthetic-test"):
        args = ["docker", "--host", "unix:///var/run/docker.sock", "exec", "-i",
                "--env", "PGAPPNAME=" + application_name]
        if not admin:
            args += ["--env", "PGOPTIONS=-c role=a03_owner"]
        return args + [cls.name, "psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1",
                       "-h", "/var/run/postgresql", "-U", "postgres" if admin else "a03_operator", "-d", cls.db]

    @classmethod
    def sql(cls, sql, *, admin=False, check=True, application_name="a03-synthetic-test"):
        result = subprocess.run(cls.command(admin=admin, application_name=application_name), input=sql,
            text=True, capture_output=True, timeout=140, check=False)
        if check and result.returncode:
            raise AssertionError("synthetic PostgreSQL SQL failed: " + result.stderr[-6000:])
        return result

    @classmethod
    def query(cls, sql):
        return json.loads(cls.sql("SET timezone='UTC'; SELECT to_jsonb(value) FROM (" + sql.rstrip(";\n ") + ") fixture_value(value);").stdout)

    def create_test_database(self):
        type(self).db = "postgres"
        db = "a03_" + uuid.uuid4().hex
        self.sql("CREATE DATABASE " + db + " OWNER " + self.owner_role + ";"
                 "REVOKE CONNECT ON DATABASE " + db + " FROM PUBLIC;"
                 "GRANT CONNECT ON DATABASE " + db + " TO " + self.installer_role + ";", admin=True)
        type(self).db = db
        self.sql("GRANT EXECUTE ON FUNCTION pg_control_system() TO " + self.owner_role + ";", admin=True)

    def install_fresh_core(self):
        self.sql(reviewed_fixture_ddl(F.IDENTITY.current_contract()))

    def setUp(self):
        self.create_test_database()
        self.install_fresh_core()
        self.source = source_fixture()
        contract = F.IDENTITY.current_contract()
        identity = self.query(F.IDENTITY.IDENTITY_SQL)
        target = {**contract, "catalog_fingerprint_sha256": self.query(F.IDENTITY.catalog_hash_sql(contract)),
            "allow_initialized_core_seed": True, "initialized_core_seed_contract": F.IDENTITY.CORE_SEED_CONTRACT,
            "initialized_core_seed_sha256": self.query(F.IDENTITY.fresh_seed_hash_query()),
            "trust_policy_seed": self.query(F.IDENTITY.trust_policy_seed_query())}
        sql = F.IDENTITY.build_import_sql(self.source["identity_plan"], migration_id="synthetic-identity",
            source_sha256=self.source["plan"]["source_archive_sha256"], target_contract=target,
            manifest=self.source["identity_run"]["manifest"], target_identity=identity)
        self.sql(sql)
        self.inventory = self.query(F.inventory_sql())
        self.material = self.capture_material()
        self.apply_sql = F.build_apply_sql(self.material)
        self.before = self.full_state()

    def capture_material(self):
        snapshot = json.loads(self.sql(F.snapshot_sql(self.inventory, "synthetic-financial")).stdout)
        return material_fixture(self.source, snapshot, runtime_roles=[self.runtime_role])

    def full_state(self):
        # Include the receipt for independent before/after comparisons, unlike
        # the receipt's self-excluding hash used in the writer transaction.
        return self.query(F.state_sql(self.inventory, "no-such-migration"))

    def rejected_unchanged(self, sql, message):
        before = self.full_state()
        result = self.sql(sql, check=False)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(message, result.stderr)
        self.assertEqual(self.full_state(), before)

    def test_opening_exact_balanced_and_strict_replay(self):
        result = json.loads(self.sql(self.apply_sql).stdout)
        self.assertEqual(result["status"], "COMMITTED")
        after = self.full_state()
        self.assertNotEqual(after, self.before)
        self.assertEqual(json.loads(self.sql(self.apply_sql).stdout), result)
        self.assertEqual(self.full_state(), after)
        balances = self.query("SELECT jsonb_object_agg(bucket,balance_units) FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id WHERE a.owner_account_id=1")
        self.assertEqual(balances, {"wallet": 0, "debt": -120, "reserved": 7, "affiliate": 38})
        self.assertEqual(self.query("SELECT coalesce(sum(balance_units),0) FROM core_billing.balance_state"), 0)

    def test_same_id_changed_mapping_and_different_id_are_readonly_conflicts(self):
        self.sql(self.apply_sql)
        # A different source plan must be completely rehashed, not just edit one
        # request. Even independently approved SQL must not replace this receipt.
        other = copy.deepcopy(self.material)
        other["source_review"]["evidence_sha256"] = "0" * 64
        self.rejected_unchanged(F.build_apply_sql(other), "financial_replay_conflict")
        other = copy.deepcopy(self.material)
        other["migration_id"] = "different-financial-id"
        for row in other["opening"] + other["counters"]:
            row["migration_id"] = other["migration_id"]
        self.rejected_unchanged(F.build_apply_sql(other), "financial_other_import_conflict")

    def test_wrong_target_oid_boot_nonce_and_catalog_are_rejected(self):
        for field, value in (("database_oid", "1"), ("system_identifier", "1"),
                             ("postmaster_started_at", "2000-01-01 00:00:00+00"),
                             ("server_instance_nonce", "other-instance")):
            material = copy.deepcopy(self.material)
            material["target"]["instance"][field] = value
            self.rejected_unchanged(F.build_apply_sql(material), "financial_target_instance_mismatch")
        material = copy.deepcopy(self.material)
        material["target"]["catalog_fingerprint_sha256"] = "0" * 64
        self.rejected_unchanged(F.build_apply_sql(material), "financial_target_catalog_mismatch")

    def test_actual_catalog_mutation_and_business_fact_cas_are_rejected(self):
        self.sql("UPDATE core_identity.users SET auth_version=2 WHERE id=71;")
        self.rejected_unchanged(self.apply_sql, "financial_target_state_changed")
        self.sql("ALTER TABLE core_identity.users ADD COLUMN unexpected_test_column text;")
        self.rejected_unchanged(self.apply_sql, "financial_target_catalog_mismatch")

    def test_runtime_login_or_unlisted_connect_role_is_rejected(self):
        self.sql("ALTER ROLE " + self.runtime_role + " LOGIN;", admin=True)
        try:
            self.rejected_unchanged(self.apply_sql, "financial_offline_roles_mismatch")
        finally:
            self.sql("ALTER ROLE " + self.runtime_role + " NOLOGIN;", admin=True)
        self.sql("GRANT CONNECT ON DATABASE " + self.db + " TO " + self.other_role + ";", admin=True)
        self.rejected_unchanged(self.apply_sql, "financial_offline_roles_mismatch")

    def test_default_trust_policy_tamper_rejected_without_other_writes(self):
        self.sql("UPDATE core_trust_pricing.policies SET spec=jsonb_set(spec,'{tiers,2,discount,numerator}','99'::jsonb);")
        self.rejected_unchanged(self.apply_sql, "financial_trust_policy_seed_mismatch")

    def test_each_nonempty_upgrade_receipt_rejected(self):
        for table in ("core_meta.schema_upgrades", "core_meta.component_upgrades"):
            with self.subTest(table=table):
                self.sql(synthetic_upgrade_receipt_sql(table))
                self.rejected_unchanged(self.apply_sql, "financial_upgrade_receipts_not_empty")
                self.sql("DELETE FROM " + table + ";")
        self.assertEqual(self.full_state(), self.before)

    def test_telegram_business_rows_are_not_exempt_from_state_guard(self):
        self.sql("INSERT INTO core_telegram_auth.bindings(subject,user_id) VALUES('12345',71);")
        self.rejected_unchanged(self.apply_sql, "financial_target_state_changed")

    def test_partial_sql_failure_rolls_back_every_table_and_sequence(self):
        # Inject a failure AFTER a completed opening. Test-only altered SQL is
        # sent directly; the normal CLI rejects it because its hash differs.
        broken = self.apply_sql.replace("PERFORM core_billing.import_opening_balance(v_payload);",
            "PERFORM core_billing.import_opening_balance(v_payload); RAISE EXCEPTION 'synthetic-mid-import';", 1)
        self.rejected_unchanged(broken, "synthetic-mid-import")
        self.assertEqual(self.full_state(), self.before)
        self.assertEqual(json.loads(self.sql(self.apply_sql).stdout)["status"], "COMMITTED")

    def test_terminated_backend_mid_opening_rolls_back_then_resolves(self):
        app = "lmm-financial-" + uuid.uuid4().hex
        call = "PERFORM core_billing.import_opening_balance(v_payload);"
        self.assertIn(call, self.apply_sql)
        # A NOTICE is delivered before COMMIT, unlike NOTIFY. Wait until the
        # first opening has changed rows and sequences, then end only that
        # uniquely named test backend. The normal CLI rejects this altered SQL.
        interrupted = self.apply_sql.replace(call, call +
            " RAISE NOTICE 'A03_OPENING_PAUSED'; PERFORM pg_sleep(30);", 1)
        with subprocess.Popen(self.command(application_name=app), stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as process:
            self.addCleanup(lambda: process.kill() if process.poll() is None else None)
            process.stdin.write(interrupted)
            process.stdin.close()
            process.stdin = None
            notice = b""
            deadline = time.monotonic() + 10
            with selectors.DefaultSelector() as selector:
                selector.register(process.stderr, selectors.EVENT_READ)
                while b"A03_OPENING_PAUSED" not in notice and time.monotonic() < deadline:
                    if selector.select(timeout=0.2):
                        chunk = os.read(process.stderr.fileno(), 4096)
                        if not chunk:
                            break
                        notice += chunk
            self.assertIn(b"A03_OPENING_PAUSED", notice)
            stopped = self.sql("SELECT pg_terminate_backend(pid) FROM pg_stat_activity "
                "WHERE datname=" + F.literal(self.db) + " AND application_name=" + F.literal(app) +
                " AND pid<>pg_backend_pid();", admin=True)
            self.assertEqual(stopped.stdout.strip(), "t")
            process.communicate(timeout=10)
            self.assertNotEqual(process.returncode, 0)
        self.assertEqual(self.full_state(), self.before)
        resolved = json.loads(self.sql(F.build_resolve_sql(self.material, app)).stdout)
        self.assertEqual(resolved["status"], "NOT_APPLIED")
        self.assertEqual(self.full_state(), self.before)
        self.assertEqual(json.loads(self.sql(self.apply_sql).stdout)["status"], "COMMITTED")

    def test_lost_commit_ack_is_resolved_without_writing_again(self):
        app = "lmm-financial-" + uuid.uuid4().hex
        self.sql(self.apply_sql, application_name=app)  # Intentionally discard commit output.
        after = self.full_state()
        result = json.loads(self.sql(F.build_resolve_sql(self.material, app)).stdout)
        self.assertEqual(result["status"], "COMMITTED")
        self.assertEqual(self.full_state(), after)

    def test_absent_receipt_requires_exact_pre_state(self):
        app = "lmm-financial-" + uuid.uuid4().hex
        resolver = F.build_resolve_sql(self.material, app)
        self.assertEqual(json.loads(self.sql(resolver).stdout)["status"], "NOT_APPLIED")
        self.assertEqual(self.full_state(), self.before)
        self.sql("UPDATE core_identity.users SET auth_version=2 WHERE id=71;")
        self.rejected_unchanged(resolver, "financial_absence_not_proven")

    def test_competing_session_fence_and_unknown_original_session(self):
        app = "lmm-financial-" + uuid.uuid4().hex
        process = subprocess.Popen(self.command(application_name=app), stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
        self.addCleanup(lambda: process.kill() if process.poll() is None else None)
        process.stdin.write("SELECT pg_advisory_lock(" + str(F.FENCE) + "); SELECT 'FENCED';\n")
        process.stdin.flush()
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
        # No state probe while this session is alive: it is the sentinel.
        result = self.sql(self.apply_sql, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("financial_import_busy", result.stderr)
        result = self.sql(F.build_resolve_sql(self.material, app), check=False)
        self.assertIn("financial_import_busy", result.stderr)
        process.stdin.close()
        self.assertEqual(process.wait(timeout=10), 0)
        process.stdout.close()
        process.stderr.close()
        self.assertEqual(self.full_state(), self.before)
        self.assertEqual(json.loads(self.sql(F.build_resolve_sql(self.material, app)).stdout)["status"], "NOT_APPLIED")

    def test_future_ledger_transaction_prevents_strict_opening_replay(self):
        self.sql(self.apply_sql)
        # This is the existing ledger function, not a new arbitrary runtime API.
        request = {"scope": "a03-synthetic", "operation_key": "after-opening", "actor_user_id": 71,
            "reason": "synthetic debt payment", "action": {"kind": "credit", "account_id": 1,
            "amount_units": 150, "payment_reference": "a03-synthetic:payment"}}
        # Match the existing native JSON encoding; verified against ledger types.
        response = json.loads(self.sql("BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on;"
            "SELECT core_billing.post_ledger(" + F.literal(request) + "::jsonb); COMMIT;").stdout)
        self.assertEqual(response["status"], "posted", response)
        balances = self.query("SELECT jsonb_object_agg(bucket,balance_units) FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id WHERE a.owner_account_id=1")
        self.assertEqual(balances, {"wallet": 30, "debt": 0, "reserved": 7, "affiliate": 38})
        self.rejected_unchanged(self.apply_sql, "financial_replay_conflict")


@unittest.skipUnless(os.environ.get("LMM_FINANCIAL_PG_FIXTURE_RECEIPT"), "matched Core init-db Unix PG18 fixture not requested")
class UnixPostgreSQLTests(PostgreSQLTests):
    """The same assertions against a locally owned compiled-Core PG18 template."""
    __unittest_skip__ = not bool(os.environ.get("LMM_FINANCIAL_PG_FIXTURE_RECEIPT"))
    __unittest_skip_why__ = "matched Core init-db Unix PG18 fixture not requested"

    @classmethod
    def setUpClass(cls):
        receipt_path = F.IDENTITY.private_backup_path(Path(os.environ["LMM_FINANCIAL_PG_FIXTURE_RECEIPT"]))
        receipt = F.IDENTITY.load_receipt(receipt_path, private=True)
        cls.receipt = receipt
        cls.contract = verify_compiled_fixture(receipt)
        fixture = receipt["fixtures"]["financial"]
        cls.fixture = fixture
        F.require(fixture.get("initialized_by") == "core-init-db"
            and fixture.get("init_binary_sha256") == receipt["verified_binary"]["sha256"]
            and fixture.get("schema_manifest_sha256") == receipt["schema_manifest"]["sha256"],
            "fixture_core_init_proof_missing")
        cls.artifacts = F.IDENTITY.BACKUP_ROOT / ("api-lmm-financial-pg18-" + uuid.uuid4().hex)
        cls.artifacts.mkdir(mode=0o700)
        cls.connection = fixture_connection(receipt, fixture, cls.artifacts)
        cls.admin_connection = dict(fixture["admin_connection"])
        F.IDENTITY.socket_connection(cls.admin_connection)
        cls.owner_role, cls.installer_role = cls.connection["role"], cls.connection["login_role"]
        cls.runtime_role = receipt["roles"][fixture["runtime"]]["role"]
        cls.other_role = receipt["roles"][fixture["other_login"]]["role"]
        F.require(all(F.IDENTITY.IDENTIFIER.fullmatch(role) for role in
                      (cls.owner_role, cls.installer_role, cls.runtime_role, cls.other_role)), "fixture_role_invalid")
        F.require(cls.admin_connection["role"] not in (cls.owner_role, cls.installer_role)
            and cls.admin_connection["socket_directory"] == cls.connection["socket_directory"]
            and cls.admin_connection["port"] == cls.connection["port"], "fixture_admin_connection_mismatch")
        cls.template = cls.connection["database"]
        cls.db = cls.template
        cls.owned = set()
        cls.counter = 0
        cls.known_backends = {}
        cls.backend_history = []
        instance = cls.query(F.instance_sql())
        F.require(180000 <= int(instance["server_version_num"]) < 190000
            and instance["listen_addresses"] == "" and instance["tcp_connection"] is False
            and instance["effective_role_can_login"] is False
            and instance["role"] == instance["database_owner"] != instance["session_user"], "fixture_unix_pg18_owner_required")
        F.IDENTITY.verify_instance(instance, cls.connection,
            {**receipt["instance_identity"], **fixture["empty_owner_baseline"]}, source=False)
        for field in ("system_identifier", "postmaster_started_at", "server_instance_nonce", "port", "server_version_num"):
            F.require(str(instance[field]) == str(receipt["instance_identity"][field]), "fixture_instance_binding_mismatch")
        admin_instance = json.loads(cls.sql(F.instance_sql(), admin=True).stdout)
        F.require(all(str(admin_instance[key]) == str(instance[key]) for key in
            ("system_identifier", "postmaster_started_at", "server_instance_nonce", "port", "server_version_num")), "fixture_admin_instance_mismatch")
        cls.template_catalog = cls.query(F.IDENTITY.catalog_hash_sql(cls.contract))
        F.require(cls.template_catalog == fixture["catalog_fingerprint_sha256"], "fixture_template_catalog_mismatch")
        F.require(cls.query("SELECT count(*) FROM core_identity.users") == 0
            and cls.query("SELECT count(*) FROM core_migration.import_runs") == 0, "fixture_template_not_empty")
        F.IDENTITY.validate_trust_policy_seed(cls.query(F.IDENTITY.trust_policy_seed_query()))
        F.write_private(cls.artifacts, "compiled-contract.private.json", F.canonical(cls.contract) + "\n")
        F.write_private(cls.artifacts, "instance.private.json", F.canonical(instance) + "\n")

    @classmethod
    def command(cls, *, admin=False, application_name="a03-synthetic-test", as_login=False):
        connection = dict(cls.admin_connection if admin else cls.connection)
        if not admin:
            connection["database"] = cls.db
        F.IDENTITY.socket_connection(connection)
        login = connection.get("login_role", connection["role"])
        options = F.IDENTITY.psql_options(connection, readonly=False)
        if as_login:
            # Only the instrumented sql() path delays SET ROLE. Direct Popen
            # callers retain their existing owner execution context.
            options = F.IDENTITY.psql_options({"role": login}, readonly=False) + " -c role=none"
        env = ["env", "-i", "PATH=" + os.environ.get("PATH", "/usr/bin:/bin"), "LC_ALL=C.UTF-8",
               "PGAPPNAME=" + application_name, "PGPASSFILE=" + connection["pgpass_file"],
               "PGOPTIONS=" + options]
        return env + ["psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1",
                      "-h", connection["socket_directory"], "-p", str(connection["port"]),
                      "-U", login, "-d", connection["database"]]

    @classmethod
    def idle_sql(cls, database):
        """Wait only for exact prior fixture sessions whose psql client returned.

        Untracked/live clients are never waited out, terminated or relabeled;
        the importer must still reject them through its original session guards.
        """
        F.require(F.IDENTITY.IDENTIFIER.fullmatch(database) is not None, "fixture_database_invalid")
        known = cls.known_backends.get(database, [])
        if not known:
            return ""
        return """DO $fixture_previous_client$ DECLARE deadline timestamptz := clock_timestamp()+interval '5 seconds'; BEGIN
 LOOP
  PERFORM pg_stat_clear_snapshot();
  IF NOT EXISTS(SELECT 1 FROM pg_stat_activity a JOIN jsonb_to_recordset(""" + F.literal(known) + """::jsonb)
   AS previous(pid integer,backend_start timestamptz)
   ON a.pid=previous.pid AND a.backend_start=previous.backend_start
   WHERE a.datname=""" + F.literal(database) + """ AND a.pid<>pg_backend_pid()) THEN RETURN; END IF;
  IF clock_timestamp()>=deadline THEN RAISE EXCEPTION 'fixture previous client backend did not leave'; END IF;
  PERFORM pg_sleep(least(0.02,greatest(0,extract(epoch FROM deadline-clock_timestamp()))));
 END LOOP;
END $fixture_previous_client$;
"""

    @classmethod
    def sql(cls, sql, *, admin=False, check=True, application_name="a03-synthetic-test"):
        cls.counter += 1
        label = "pg-" + str(cls.counter)
        database = cls.admin_connection["database"] if admin else cls.db
        connection = cls.admin_connection if admin else cls.connection
        owner, login = F.IDENTITY.connection_roles(connection)
        F.write_private(cls.artifacts, label + ".input.private.sql", sql)
        pid_file = F.write_private(cls.artifacts, label + ".pid.private.json", "")
        output_path = str(pid_file).replace("\\", "\\\\").replace("'", "\\'")
        capture = "(:'fixture_identity'::jsonb)"
        same_backend = ("pg_backend_pid()=(" + capture + "->>'pid')::integer AND current_database()=" +
            F.literal(database) + " AND " + capture + "->>'database'=" + F.literal(database) +
            " AND session_user=" + F.literal(login) + " AND " + capture + "->>'session_user'=" + F.literal(login))
        login_identity = (same_backend + " AND current_user=" + F.literal(login) +
            " AND " + capture + "->>'capture_role'=" + F.literal(login) +
            " AND (" + capture + "->>'backend_start')::timestamptz="
            "(SELECT backend_start FROM pg_stat_activity WHERE pid=pg_backend_pid())")
        owner_identity = same_backend + " AND current_user=" + F.literal(owner)

        def guard(predicate, message):
            return ("SELECT coalesce((" + predicate + "),false) AS fixture_identity_valid \\gset\n"
                "\\if :fixture_identity_valid\n\\else\nDO $fixture_identity$ BEGIN RAISE EXCEPTION " +
                F.literal(message) + "; END $fixture_identity$;\n\\endif\n")

        assume_owner = 'SET ROLE "' + owner + '";\n'
        # PostgreSQL exposes backend_start to the actual LOGIN, but can hide it
        # after SET ROLE to a restricted NOLOGIN owner. Capture and wait under
        # LOGIN, verify that the same backend survives the role transition, and
        # restore the owner before executing any business SQL. No role gains
        # statistics privileges. SET ROLE NONE returns to session_user even if
        # a role/database default would make RESET ROLE assume another role.
        wrapped = ("SET ROLE NONE;\nSELECT jsonb_build_object('pid',pg_backend_pid(),"
            "'backend_start',(SELECT backend_start::text FROM pg_stat_activity WHERE pid=pg_backend_pid()),"
            "'database',current_database(),'capture_role',current_user,'session_user',session_user) AS identity \\gset fixture_\n"
            "\\o '" + output_path + "'\nSELECT :'fixture_identity';\n\\o\n" +
            guard(login_identity, "fixture actual login backend identity mismatch") + cls.idle_sql(database) +
            assume_owner + guard(owner_identity, "fixture effective owner identity mismatch") +
            "SET ROLE NONE;\n" + guard(login_identity, "fixture backend start changed across role switch") +
            "SELECT backend_start::text AS fixture_verified_start FROM pg_stat_activity WHERE pid=pg_backend_pid() \\gset\n" +
            assume_owner + guard(owner_identity, "fixture effective owner identity mismatch") +
            "\\o '" + output_path + "'\nSELECT " + capture +
            "||jsonb_build_object('effective_role',current_user,'verified_backend_start',:'fixture_verified_start');\n\\o\n" + sql)
        F.write_private(cls.artifacts, label + ".executed.private.sql", wrapped)
        result = None
        try:
            result = subprocess.run(cls.command(admin=admin, application_name=application_name, as_login=True), input=wrapped,
                text=True, capture_output=True, timeout=140, check=False)
        except subprocess.TimeoutExpired as error:
            def partial(value):
                return value.decode("utf-8", errors="replace") if isinstance(value, bytes) else value or ""
            F.write_private(cls.artifacts, label + ".stdout.private.txt", partial(error.stdout))
            F.write_private(cls.artifacts, label + ".stderr.private.txt", partial(error.stderr))
            F.write_private(cls.artifacts, label + ".status.private.json", F.canonical({"status": "TIMEOUT", "timeout_seconds": 140}) + "\n")
            raise
        except OSError as error:
            F.write_private(cls.artifacts, label + ".stdout.private.txt", "")
            F.write_private(cls.artifacts, label + ".stderr.private.txt", str(error))
            F.write_private(cls.artifacts, label + ".status.private.json", F.canonical({"status": "SPAWN_ERROR", "error_type": type(error).__name__}) + "\n")
            raise
        else:
            F.write_private(cls.artifacts, label + ".stdout.private.txt", result.stdout)
            F.write_private(cls.artifacts, label + ".stderr.private.txt", result.stderr)
            F.write_private(cls.artifacts, label + ".status.private.json", F.canonical({"status": "EXITED", "returncode": result.returncode}) + "\n")
        finally:
            raw = pid_file.read_text().strip()
            if result is not None and result.returncode == 0:
                F.require(bool(raw), "fixture_backend_identity_missing")
            if raw:
                session = json.loads(raw)
                F.require(type(session.get("pid")) is int and session["pid"] > 0
                    and type(session.get("backend_start")) is str and session["backend_start"]
                    and session.get("database") == database
                    and session.get("capture_role") == session.get("session_user") == login,
                    "fixture_backend_identity_invalid")
                if "verified_backend_start" in session or "effective_role" in session:
                    F.require(session.get("verified_backend_start") == session["backend_start"]
                        and session.get("effective_role") == owner, "fixture_backend_role_transition_invalid")
                if result is not None and result.returncode == 0:
                    F.require("verified_backend_start" in session and "effective_role" in session,
                              "fixture_backend_role_transition_missing")
                cls.known_backends.setdefault(database, []).append({key: session[key] for key in ("pid", "backend_start")})
                cls.backend_history.append({"label": label, **session})
        if check and result.returncode:
            raise AssertionError("synthetic PostgreSQL SQL failed; complete private evidence: " + str(cls.artifacts / label) + "\n" + result.stderr[-6000:])
        return result

    def create_test_database(self):
        type(self).db = self.template
        self.assertEqual(self.query(F.IDENTITY.catalog_hash_sql(self.contract)), self.template_catalog)
        database = "lmm_financial_p01_" + uuid.uuid4().hex
        self.assertRegex(database, r"^lmm_financial_p01_[a-f0-9]{32}$")
        type(self).db = self.admin_connection["database"]
        self.sql(self.idle_sql(self.template) + 'CREATE DATABASE "' + database + '" WITH TEMPLATE "' + self.template + '" OWNER "' + self.owner_role + '";', admin=True)
        self.owned.add(database)
        self.addCleanup(self.cleanup_database, database)
        type(self).db = database
        self.sql('REVOKE CONNECT ON DATABASE "' + database + '" FROM PUBLIC; GRANT CONNECT ON DATABASE "' +
                 database + '" TO "' + self.installer_role + '";', admin=True)

    def install_fresh_core(self):
        # The external resource owner ran the verified binary's init-db once in
        # the immutable empty template. Never patch or initialize that database.
        self.assertEqual(self.query(F.IDENTITY.catalog_hash_sql(self.contract)), self.template_catalog)

    def test_backend_identity_is_captured_as_login_without_stats_grants(self):
        result = self.sql("SELECT jsonb_build_object('pid',pg_backend_pid(),'owner',current_user,"
            "'login',session_user,'owner_can_login',(SELECT rolcanlogin FROM pg_roles WHERE rolname=current_user),"
            "'runtime_can_login',(SELECT rolcanlogin FROM pg_roles WHERE rolname=" + F.literal(self.runtime_role) + "),"
            "'owner_superuser',(SELECT rolsuper FROM pg_roles WHERE rolname=current_user),"
            "'login_superuser',(SELECT rolsuper FROM pg_roles WHERE rolname=session_user),"
            "'owner_reads_all_stats',pg_has_role(current_user,'pg_read_all_stats','MEMBER'),"
            "'login_reads_all_stats',pg_has_role(session_user,'pg_read_all_stats','MEMBER'),"
            "'owner_backend_start',(SELECT backend_start::text FROM pg_stat_activity WHERE pid=pg_backend_pid()));")
        facts = json.loads(result.stdout)
        captured = self.backend_history[-1]
        self.assertEqual(facts, {"pid": captured["pid"], "owner": self.owner_role,
            "login": self.installer_role, "owner_can_login": False, "runtime_can_login": False,
            "owner_superuser": False, "login_superuser": False, "owner_reads_all_stats": False,
            "login_reads_all_stats": False, "owner_backend_start": None})
        self.assertEqual(captured["capture_role"], self.installer_role)
        self.assertEqual(captured["session_user"], self.installer_role)
        self.assertEqual(captured["effective_role"], self.owner_role)
        self.assertTrue(captured["backend_start"])
        self.assertEqual(captured["backend_start"], captured["verified_backend_start"])

    def cleanup_database(self, database):
        self.assertIn(database, self.owned)
        self.assertRegex(database, r"^lmm_financial_p01_[a-f0-9]{32}$")
        type(self).db = self.admin_connection["database"]
        self.sql(self.idle_sql(database) + 'DROP DATABASE "' + database + '";', admin=True)
        self.owned.remove(database)

    @classmethod
    def tearDownClass(cls):
        cls.db = cls.template
        F.require(not cls.owned, "fixture_owned_databases_remain")
        F.require(cls.query(F.IDENTITY.catalog_hash_sql(cls.contract)) == cls.template_catalog,
                  "fixture_template_changed")
        F.require(verify_compiled_fixture(cls.receipt) == cls.contract, "fixture_source_changed")
        F.write_private(cls.artifacts, "result.private.json", F.canonical({"owned_databases_remaining": [],
            "fixture": "matched-core-init-db-unix-pg18", "production_access": False,
            "backend_history": cls.backend_history}) + "\n")


class UnixFixtureEvidenceTests(unittest.TestCase):
    def fixture(self, directory):
        class Fixture(UnixPostgreSQLTests):
            pass
        Fixture.artifacts = Path(directory)
        Fixture.db = "synthetic_evidence"
        Fixture.connection = {"database": Fixture.db, "role": "synthetic_owner",
            "login_role": "synthetic_installer", "set_role": "synthetic_owner",
            "socket_directory": "/synthetic/owned/socket", "port": 55432,
            "pgpass_file": str(Path(directory) / "synthetic.pgpass")}
        Fixture.admin_connection = {**Fixture.connection, "database": "synthetic_control",
            "role": "synthetic_admin"}
        Fixture.admin_connection.pop("login_role")
        Fixture.admin_connection.pop("set_role")
        Fixture.counter = 0
        Fixture.known_backends = {}
        Fixture.backend_history = []
        return Fixture

    def test_failed_process_preserves_complete_sql_stdout_stderr(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            result = subprocess.CompletedProcess(["synthetic-only"], 1, "partial output\n", "failure detail\n" * 2000)
            original = "SELECT 1/0;\n"
            with patch.object(fixture, "command", return_value=["synthetic-only"]), \
                    patch.object(subprocess, "run", return_value=result), \
                    self.assertRaisesRegex(AssertionError, "complete private evidence"):
                fixture.sql(original)
            self.assertEqual((Path(directory) / "pg-1.input.private.sql").read_text(), original)
            self.assertEqual((Path(directory) / "pg-1.stdout.private.txt").read_text(), result.stdout)
            self.assertEqual((Path(directory) / "pg-1.stderr.private.txt").read_text(), result.stderr)
            self.assertEqual(json.loads((Path(directory) / "pg-1.status.private.json").read_text())["returncode"], 1)

    def test_timeout_preserves_partial_binary_output_and_never_claims_rollback(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            timeout = subprocess.TimeoutExpired(["synthetic-only"], 140, output=b"partial stdout", stderr=b"partial error")
            with patch.object(fixture, "command", return_value=["synthetic-only"]), \
                    patch.object(subprocess, "run", side_effect=timeout), self.assertRaises(subprocess.TimeoutExpired):
                fixture.sql("SELECT synthetic_timeout;\n")
            self.assertEqual((Path(directory) / "pg-1.stdout.private.txt").read_text(), "partial stdout")
            self.assertEqual((Path(directory) / "pg-1.stderr.private.txt").read_text(), "partial error")
            self.assertEqual(json.loads((Path(directory) / "pg-1.status.private.json").read_text())["status"], "TIMEOUT")

    def test_idle_gate_only_tracks_prior_exact_sessions(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            self.assertEqual(fixture.idle_sql(fixture.db), "")
            fixture.known_backends[fixture.db] = [{"pid": 123, "backend_start": "2026-10-22 00:00:00+00"}]
            gate = fixture.idle_sql(fixture.db)
            self.assertIn("a.pid=previous.pid AND a.backend_start=previous.backend_start", gate)
            self.assertIn("interval '5 seconds'", gate)
            self.assertNotIn("pg_terminate_backend", gate)
            self.assertNotIn("pg_cancel_backend", gate)

    def test_login_transport_is_opt_in_and_direct_owner_commands_are_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            with patch.object(F.IDENTITY, "socket_connection"):
                owner = fixture.command()
                login = fixture.command(as_login=True)
            self.assertIn(" -c role=synthetic_owner", next(x for x in owner if x.startswith("PGOPTIONS=")))
            self.assertNotIn("role=synthetic_owner", next(x for x in login if x.startswith("PGOPTIONS=")))
            self.assertIn(" -c role=none", next(x for x in login if x.startswith("PGOPTIONS=")))
            self.assertEqual(login[login.index("-U") + 1], "synthetic_installer")

    def test_capture_and_known_client_wait_precede_owner_and_start_is_rechecked(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            fixture.known_backends[fixture.db] = [{"pid": 122, "backend_start": "2026-10-22 00:00:00+00"}]
            failed = subprocess.CompletedProcess(["synthetic-only"], 1, "", "synthetic SQL failure\n")
            with patch.object(fixture, "command", return_value=["synthetic-only"]) as command, \
                    patch.object(subprocess, "run", return_value=failed), \
                    self.assertRaisesRegex(AssertionError, "complete private evidence"):
                fixture.sql("SELECT 1/0;\n")
            command.assert_called_once_with(admin=False, application_name="a03-synthetic-test", as_login=True)
            script = (Path(directory) / "pg-1.executed.private.sql").read_text()
            first_owner = script.index('SET ROLE "synthetic_owner";')
            self.assertLess(script.index("AS identity \\gset fixture_"), first_owner)
            self.assertLess(script.index("DO $fixture_previous_client$"), first_owner)
            self.assertGreater(script.index("fixture backend start changed across role switch"), first_owner)
            self.assertEqual(script.count("SET ROLE NONE;"), 2)
            self.assertEqual(script.count('SET ROLE "synthetic_owner";'), 2)
            self.assertLess(script.rindex('SET ROLE "synthetic_owner";'), script.index("SELECT 1/0;"))
            self.assertIn("current_user='synthetic_installer'", script)
            self.assertIn("current_user='synthetic_owner'", script)
            self.assertNotIn("GRANT", script)

    def test_null_backend_start_and_role_transition_mismatch_fail_closed(self):
        base = {"pid": 123, "backend_start": "2026-10-22 00:00:00+00", "database": "synthetic_evidence",
            "capture_role": "synthetic_installer", "session_user": "synthetic_installer",
            "effective_role": "synthetic_owner", "verified_backend_start": "2026-10-22 00:00:00+00"}
        cases = (({"backend_start": None}, "fixture_backend_identity_invalid"),
                 ({"capture_role": "synthetic_owner"}, "fixture_backend_identity_invalid"),
                 ({"verified_backend_start": "2026-10-22 00:00:01+00"}, "fixture_backend_role_transition_invalid"),
                 ({"effective_role": "synthetic_installer"}, "fixture_backend_role_transition_invalid"))
        for changes, message in cases:
            with self.subTest(changes=changes), tempfile.TemporaryDirectory() as directory:
                fixture = self.fixture(directory)
                def failure(*args, **kwargs):
                    (Path(directory) / "pg-1.pid.private.json").write_text(json.dumps({**base, **changes}) + "\n")
                    return subprocess.CompletedProcess(["synthetic-only"], 1, "", "identity visibility or role failure\n")
                with patch.object(fixture, "command", return_value=["synthetic-only"]), \
                        patch.object(subprocess, "run", side_effect=failure), self.assertRaisesRegex(F.Error, message):
                    fixture.sql("SELECT 1/0;\n")
                self.assertEqual(fixture.known_backends, {})
                self.assertEqual(fixture.backend_history, [])
                self.assertEqual((Path(directory) / "pg-1.stderr.private.txt").read_text(), "identity visibility or role failure\n")

    def test_zero_exit_without_backend_proof_is_not_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            result = subprocess.CompletedProcess(["synthetic-only"], 0, "", "")
            with patch.object(fixture, "command", return_value=["synthetic-only"]), \
                    patch.object(subprocess, "run", return_value=result), \
                    self.assertRaisesRegex(F.Error, "fixture_backend_identity_missing"):
                fixture.sql("SELECT 1;\n")
            self.assertEqual(fixture.known_backends, {})

    def test_failed_business_sql_keeps_verified_exact_session_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.fixture(directory)
            captured = {"pid": 123, "backend_start": "2026-10-22 00:00:00+00", "database": fixture.db,
                "capture_role": "synthetic_installer", "session_user": "synthetic_installer",
                "effective_role": "synthetic_owner", "verified_backend_start": "2026-10-22 00:00:00+00"}
            def failed_business_sql(*args, **kwargs):
                (Path(directory) / "pg-1.pid.private.json").write_text(json.dumps(captured) + "\n")
                return subprocess.CompletedProcess(["synthetic-only"], 1, "", "division by zero\n")
            with patch.object(fixture, "command", return_value=["synthetic-only"]), \
                    patch.object(subprocess, "run", side_effect=failed_business_sql), \
                    self.assertRaisesRegex(AssertionError, "complete private evidence"):
                fixture.sql("SELECT 1/0;\n")
            self.assertEqual(fixture.known_backends, {fixture.db: [{"pid": 123, "backend_start": captured["backend_start"]}]})
            self.assertEqual(fixture.backend_history, [{"label": "pg-1", **captured}])


if __name__ == "__main__":
    unittest.main(verbosity=2)
