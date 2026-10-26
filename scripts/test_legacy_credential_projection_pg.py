"""Real PostgreSQL tests of credential authority projection, using synthetic rows only.

Run with LMM_CREDENTIAL_PROJECTION_PG_TEST=1 python3 -B
scripts/test_legacy_credential_projection_pg.py -v. Creates its own Unix-only
cluster under ~/Backups and retains SQL, logs and data after stopping it.
This tests the projection against selected native table DDL, not installation,
financial approval, credential import or production acceptance.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

import legacy_credential_import as IMPORT

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.environ.get("LMM_CREDENTIAL_PROJECTION_PG_TEST") == "1",
                     "isolated credential projection PostgreSQL test not enabled")
class CredentialProjectionPostgreSQLTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        evidence_root = IMPORT.IDENTITY.BACKUP_ROOT / "api-lmm" / "local" / "credential-projection"
        evidence_root.mkdir(mode=0o700, parents=True, exist_ok=True)
        cls.output = Path(tempfile.mkdtemp(prefix="run-", dir=evidence_root))
        cls.env = {k: v for k, v in os.environ.items()
                   if not k.startswith("PG") and k not in ("DATABASE_URL", "SQL_DSN")}
        cls.env.update(PGPASSFILE="/dev/null", PGSERVICEFILE="/dev/null")
        cls.socket = cls.output / "sock"
        cls.socket.mkdir(mode=0o700)
        cls.data = cls.output / "data"
        cls.command_count = 0
        print("Synthetic PostgreSQL evidence: " + str(cls.output), flush=True)
        cls.command(["initdb", "-D", str(cls.data), "--auth=trust",
                     "--username=projection_admin", "--no-locale", "--encoding=UTF8"])
        cls.addClassCleanup(cls.command, ["pg_ctl", "-D", str(cls.data), "-m", "fast", "-w", "stop"])
        cls.command(["pg_ctl", "-D", str(cls.data), "-l", str(cls.output / "postgres.log"),
                     "-o", "-k " + str(cls.socket) + " -p 55473 -c listen_addresses='' -c max_connections=10",
                     "-w", "start"])
        # Use the actual native table definitions, including keys and constraints.
        schemas = {name: (ROOT / "services/core/schema" / (name + ".sql")).read_text()
                   for name in ("identity", "routing")}
        ddl = ["CREATE SCHEMA core_identity;"]
        for schema, tables in (
            ("identity", ("core_identity.accounts", "core_identity.users",
                          "core_identity.user_routing_groups", "core_identity.developer_access")),
            ("routing", ("routing_price_versions", "routing_config_versions",
                         "routing_active_release", "routing_groups")),
        ):
            for table in tables:
                match = re.search(r"CREATE TABLE " + re.escape(table) + r" \(.*?\n\);", schemas[schema], re.S)
                if match is None:
                    raise AssertionError("native table definition missing: " + table)
                ddl.append(match.group())
        cls.sql("\n".join(ddl))
        cls.sql("""
INSERT INTO core_identity.accounts(id,kind,active) OVERRIDING SYSTEM VALUE
 VALUES (8,'personal',true),(9,'personal',false);
INSERT INTO core_identity.users(id,personal_account_id,auth_version) VALUES (37,8,5),(38,9,7);
INSERT INTO core_identity.developer_access(user_id,granted,source_reference) VALUES (37,true,'synthetic');
INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES
 (37,'z-active',true,'synthetic'),(37,'a-active',true,'synthetic'),
 (37,'disabled-catalog',true,'synthetic'),(37,'disabled-grant',false,'synthetic'),
 (37,'old-only',true,'synthetic'),(37,'missing',true,'synthetic');
INSERT INTO routing_price_versions(version,document) VALUES (1,'{"version":1}');
INSERT INTO routing_config_versions(version,price_version,document)
 VALUES (1,1,'{"version":1}'),(2,1,'{"version":2}');
INSERT INTO routing_groups(version,name,enabled) VALUES
 (1,'old-only',true),(1,'disabled-catalog',true),
 (2,'z-active',true),(2,'a-active',true),
 (2,'disabled-catalog',false),(2,'disabled-grant',true),(2,'ungranted',true);
""")

    @classmethod
    def write(cls, name, value):
        with (cls.output / name).open("x") as stream:
            os.chmod(stream.name, 0o600)
            stream.write(value)

    @classmethod
    def command(cls, argv, *, sql=None):
        cls.command_count += 1
        label = "command-" + str(cls.command_count)
        if sql is not None:
            cls.write(label + ".sql", sql)
        result = subprocess.run(argv, input=sql, text=True, capture_output=True,
                                env=cls.env, timeout=60, umask=0o077)
        cls.write(label + ".json", json.dumps({"argv": argv, "exit_code": result.returncode,
                                               "stdout": result.stdout, "stderr": result.stderr}, indent=2) + "\n")
        if result.returncode:
            raise AssertionError("command failed: " + str(argv) + "\n" + result.stderr)
        return result.stdout.strip()

    @classmethod
    def sql(cls, sql):
        return cls.command(["psql", "-XqAt", "-v", "ON_ERROR_STOP=1", "-h", str(cls.socket),
                            "-p", "55473", "-U", "projection_admin", "-d", "postgres"], sql=sql)

    def test_only_enabled_grants_in_active_catalog_are_projected(self):
        cases = (
            ("missing-release", "", []),
            ("null-active-version", "INSERT INTO routing_active_release(singleton) VALUES(true);", []),
            ("active-catalog", "INSERT INTO routing_active_release VALUES(true,2);", ["a-active", "z-active"]),
            ("previous-version", "INSERT INTO routing_active_release VALUES(true,1);", ["disabled-catalog", "old-only"]),
            ("empty-catalog", "INSERT INTO routing_active_release VALUES(true,2); DELETE FROM routing_groups WHERE version=2;", []),
            ("catalog-disabled", "INSERT INTO routing_active_release VALUES(true,2); UPDATE routing_groups SET enabled=false WHERE version=2;", []),
            ("grant-disabled", "INSERT INTO routing_active_release VALUES(true,2); UPDATE core_identity.user_routing_groups SET enabled=false;", []),
        )
        for label, setup, expected in cases:
            with self.subTest(case=label):
                rows = json.loads(self.sql("BEGIN;\n" + setup + "\n" + IMPORT.native_users_sql() + ";\nROLLBACK;"))
                self.assertEqual(rows, [
                    {"user_id": 37, "personal_account_id": 8, "auth_version": 5, "active": True,
                     "developer_access_granted": True, "routing_groups": expected},
                    {"user_id": 38, "personal_account_id": 9, "auth_version": 7, "active": False,
                     "developer_access_granted": False, "routing_groups": []},
                ])

    def test_empty_users_remain_empty(self):
        rows = json.loads(self.sql("BEGIN; DELETE FROM core_identity.user_routing_groups; "
                                  "DELETE FROM core_identity.developer_access; DELETE FROM core_identity.users;\n" +
                                  IMPORT.native_users_sql() + "; ROLLBACK;"))
        self.assertEqual(rows, [])


if __name__ == "__main__":
    unittest.main()
