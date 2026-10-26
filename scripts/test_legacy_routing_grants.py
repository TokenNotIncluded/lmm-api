"""Routing grant source conservation; actual PostgreSQL acceptance is separate."""
from copy import deepcopy
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import legacy_routing_grants as G


def fixture():
    users = [{"id": 7, "group": "原组/一", "status": 1, "deleted_at": None,
              "password": "synthetic-never-emit-password", "email": "private@example.invalid"},
             {"id": 29, "group": "inactive-group", "status": 2, "deleted_at": None},
             {"id": 41, "group": "deleted-group", "status": 1, "deleted_at": "2026-01-01T00:00:00+00:00"}]
    mapping = [{"source_user_id": row["id"], "target_user_id": row["id"], "target_personal_account_id": index}
               for index, row in enumerate(users, 1)]
    encode = lambda rows: ("\n".join(G.FIN.canonical(row) for row in rows) + "\n").encode()
    mapping_bytes = encode(mapping)
    export_bytes = encode([{"kind": "identity", "row": {}}, {"kind": "schema_identity", "row": {}},
        {"kind": "user_count", "row": len(users)}, *({"kind": "user", "row": row} for row in users)])
    manifest = {"format_version": 2, "scope": "identity-only", "source_archive_sha256": "a" * 64,
        "source_export_sha256": G.FIN.sha(export_bytes), "source_target_id_map_sha256": G.FIN.sha(mapping_bytes),
        "mapping_sha256": "e" * 64}
    plan = {"format_version": 1, "scope": "legacy-routing-offline-plan", "status": "blocked", "apply_supported": False,
        "bindings": {"source_archive_sha256": "a" * 64, "source_export_sha256": "b" * 64,
                     "identity_map_file_sha256": G.FIN.sha(mapping_bytes)},
        "user_group_plan": [{"user_id": row["id"], "group_name": row["group"],
            "enabled": row["status"] == 1 and row["deleted_at"] is None,
            "source_reference": "legacy.users/" + str(row["id"]), "source_row_sha256": G.FIN.digest(row)} for row in users]}
    plan["plan_sha256"] = G.FIN.digest(plan)
    tables = sorted(set(G.FIN.CHANGED_TABLES) | set(G.FIN.FRESH_EMPTY_TABLES) |
        {G.TABLE, "core_migration.import_runs", "core_identity.users", "core_identity.accounts", "core_trust_pricing.policies"})
    sequences = sorted(G.FIN.CHANGED_SEQUENCES)
    state = {"inventory": {"tables": tables, "sequences": sequences},
        "tables": {name: {"rows": 0, "sha256": G.FIN.sha(b"")} for name in tables},
        "sequences": {name: {"last_value": 201, "is_called": False, "start": 1, "increment": 1,
            "min": 1, "max": 9223372036854775807, "cache": 1, "cycle": False} for name in sequences}}
    contract = G.IDENTITY.current_contract()
    snapshot = {"instance": {"database": "synthetic_groups", "database_oid": "13", "role": "synthetic_owner",
        "session_user": "synthetic_installer", "database_owner": "synthetic_owner", "effective_role_can_login": False,
        "tcp_connection": False, "listen_addresses": "", "port": 5432, "server_version_num": "180000",
        "postmaster_started_at": "2026-10-26 00:00:00+00", "system_identifier": "123456789", "server_instance_nonce": "f" * 64},
        "catalog_fingerprint_sha256": "c" * 64, "ownership_sha256": "d" * 64,
        "component_fingerprints": contract["component_fingerprints"], "trust_policy_seed": {
            "singleton": True, "revision": 1, "spec": G.IDENTITY.DEFAULT_TRUST_POLICY,
            "source_digest": hashlib.sha256(b"core-native-trust-policy-v1").hexdigest(),
            "native_user_floor": 1, "source_cutoff": 1000, "native_since": "2026-10-26T00:00:00+00:00"},
        "state": state, "native_users": [{"id": row["source_user_id"], "personal_account_id": row["target_personal_account_id"],
            "active": False, "auth_version": 3, "platform_role": "user", "account_active": True, "account_kind": "personal"}
            for row in mapping], "identity_import": {"migration_id": "synthetic-identity-import", "source_archive_sha256": "a" * 64,
                "mapping_sha256": "e" * 64, "manifest": manifest}}
    data = {"routing_plan": (G.FIN.canonical(plan) + "\n").encode(), "identity_manifest": (G.FIN.canonical(manifest) + "\n").encode(),
            "identity_map": mapping_bytes, "source_identity_export": export_bytes}
    files = {name: {"path": "/home/lightjunction/Backups/synthetic-routing-grants/" + name, "sha256": G.FIN.sha(raw)}
             for name, raw in data.items()}
    return plan, manifest, mapping_bytes, export_bytes, snapshot, files, data


def prepare(value=None):
    plan, manifest, mapping, exported, snapshot, files, _ = value or fixture()
    return G.prepare_material(plan, manifest, mapping, exported, migration_id="synthetic-routing-grants",
        snapshot=snapshot, runtime_roles=["synthetic_runtime"], source_files=files)


class SourceTests(unittest.TestCase):
    def test_original_groups_and_disabled_deleted_states_survive_without_user_activation(self):
        material = prepare()
        self.assertEqual([r["group_name"] for r in material["records"]], ["原组/一", "inactive-group", "deleted-group"])
        self.assertEqual([r["enabled"] for r in material["records"]], [True, False, False])
        self.assertTrue(all(not r["active"] and r["auth_version"] == 3 for r in material["native_users"]))
        self.assertNotIn("password", G.FIN.canonical(material))
        self.assertNotIn("private@example.invalid", G.FIN.canonical(material))

    def test_blocked_models_do_not_prevent_copying_proven_group_memberships(self):
        material = prepare()
        self.assertEqual(len(material["records"]), 3)
        self.assertNotIn("candidate", material)
        self.assertNotIn("routes", material)

    def test_plan_cannot_add_another_group_enable_disabled_or_omit_a_user(self):
        for mutation in (lambda p: p["user_group_plan"][0].update(group_name="other-group"),
                         lambda p: p["user_group_plan"][1].update(enabled=True),
                         lambda p: p["user_group_plan"][0].update(enabled=1),
                         lambda p: p["user_group_plan"].pop(),
                         lambda p: p["user_group_plan"].append(deepcopy(p["user_group_plan"][0]))):
            value = fixture(); mutation(value[0]); value[0].pop("plan_sha256")
            value[0]["plan_sha256"] = G.FIN.digest(value[0])
            with self.assertRaises(G.Error):
                prepare(value)

    def test_source_or_map_hash_change_rejects_even_when_plan_groups_still_look_correct(self):
        value = fixture()
        for index in (2, 3):
            changed = list(value); changed[index] += b"\n"
            with self.subTest(index=index), self.assertRaises(G.Error):
                prepare(changed)

    def test_unknown_source_state_auto_group_duplicate_and_missing_rows_reject(self):
        for mutate in (lambda rows: rows[3]["row"].update(status=3), lambda rows: rows[3]["row"].update(group="auto"),
                       lambda rows: rows.append(deepcopy(rows[3])), lambda rows: rows.pop()):
            value = list(fixture())
            tagged = [json.loads(line) for line in value[3].splitlines()]
            mutate(tagged)
            value[3] = ("\n".join(G.FIN.canonical(row) for row in tagged) + "\n").encode()
            value[1]["source_export_sha256"] = G.FIN.sha(value[3])
            with self.assertRaises(G.Error):
                prepare(value)

    def test_mapping_must_reuse_exact_source_id_and_unique_personal_account(self):
        for field, item in (("target_user_id", 999), ("target_personal_account_id", 2)):
            value = list(fixture())
            mapping = [json.loads(line) for line in value[2].splitlines()]
            mapping[0][field] = item
            value[2] = ("\n".join(G.FIN.canonical(row) for row in mapping) + "\n").encode()
            value[1]["source_target_id_map_sha256"] = G.FIN.sha(value[2])
            value[0]["bindings"]["identity_map_file_sha256"] = G.FIN.sha(value[2])
            value[0].pop("plan_sha256"); value[0]["plan_sha256"] = G.FIN.digest(value[0])
            with self.subTest(field=field), self.assertRaises(G.Error):
                prepare(value)

    def test_native_missing_extra_or_cross_account_user_and_wrong_identity_receipt_reject(self):
        for mutate in (lambda s: s["native_users"].pop(),
                       lambda s: s["native_users"][0].update(personal_account_id=2),
                       lambda s: s["native_users"][0].update(account_kind="team"),
                       lambda s: s["identity_import"].update(mapping_sha256="f" * 64)):
            value = fixture(); mutate(value[4])
            with self.assertRaises(G.Error):
                prepare(value)

    def test_nonempty_target_group_table_is_a_conflict_never_upserted(self):
        value = fixture(); value[4]["state"]["tables"][G.TABLE] = {"rows": 1, "sha256": "1" * 64}
        with self.assertRaisesRegex(G.Error, "routing_target_grants_not_empty"):
            prepare(value)

    def test_public_tcp_login_owner_or_source_contract_drift_rejects_preparation(self):
        for mutate in (lambda s: s["instance"].update(tcp_connection=True),
                       lambda s: s["instance"].update(effective_role_can_login=True),
                       lambda s: s["instance"].update(listen_addresses="127.0.0.1"),
                       lambda s: s["component_fingerprints"].update(identity="f" * 64)):
            value = fixture(); mutate(value[4])
            with self.assertRaises(G.Error):
                prepare(value)

    def test_json_round_trip_and_source_tool_drift_are_checked(self):
        material = prepare()
        loaded = json.loads(G.FIN.canonical(material))
        G.validate_material(loaded)
        self.assertEqual(G.FIN.digest(material), G.FIN.digest(loaded))
        self.assertEqual(G.build_apply_sql(material), G.build_apply_sql(loaded))
        material["tool_source_hashes"]["legacy_routing_grants.py"] = "f" * 64
        with self.assertRaisesRegex(G.Error, "routing_tool_source_changed"):
            G.build_apply_sql(material)

    def test_nested_input_shape_errors_are_fixed_refusals(self):
        for bindings in (None, [], {"unexpected": "a" * 64}):
            value = fixture(); value[0]["bindings"] = bindings
            value[0].pop("plan_sha256"); value[0]["plan_sha256"] = G.FIN.digest(value[0])
            with self.subTest(bindings=bindings), self.assertRaises(G.Error):
                prepare(value)
        for field in ("instance", "state", "identity_import"):
            value = fixture(); value[4][field] = []
            with self.subTest(field=field), self.assertRaises(G.Error):
                prepare(value)
        for mutate in (lambda m: m.update(identity_manifest=[]), lambda m: m.update(target=[]),
                       lambda m: m["target"].update(instance=None),
                       lambda m: m["before_state"].update(tables=[]),
                       lambda m: m["before_state"]["tables"].update({G.TABLE: []})):
            material = prepare(); mutate(material)
            with self.assertRaises(G.Error):
                G.validate_material(material)


class SqlTests(unittest.TestCase):
    def test_apply_is_atomic_fenced_exactly_idempotent_and_preserves_every_other_table_and_sequence(self):
        sql = G.build_apply_sql(prepare())
        self.assertTrue(sql.startswith("\\set ON_ERROR_STOP on\n"))
        for required in ("pg_try_advisory_lock", "BEGIN ISOLATION LEVEL SERIALIZABLE", "synchronous_commit=on",
                         "ACCESS EXCLUSIVE", "routing_replay_conflict", "routing_target_state_changed",
                         "routing_exact_grants_readback_failed", "routing_unrelated_state_changed", "COMMIT;"):
            self.assertIn(required, sql)
        self.assertIn("INSERT INTO \"core_identity\".\"user_routing_groups\"", sql)
        self.assertIn("OR before_state->'sequences' IS DISTINCT FROM after_state->'sequences'", sql)
        self.assertNotIn("ON CONFLICT", sql)
        for forbidden in ("UPDATE core_identity.users", "DELETE FROM", "UPDATE routing_active_release", "setval(", "nextval("):
            self.assertNotIn(forbidden, sql)

    def test_resolve_is_read_only_and_distinguishes_commit_absence_drift_and_live_attempt(self):
        sql = G.build_resolve_sql(prepare(), "lmm-routing-" + "1" * 32)
        self.assertTrue(sql.startswith("\\set ON_ERROR_STOP on\n"))
        for required in ("BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY", "routing_original_attempt_active",
                         "routing_resolve_conflict", "routing_absence_not_proven", "'COMMITTED'", "'NOT_APPLIED'"):
            self.assertIn(required, sql)
        self.assertNotIn("INSERT INTO", sql)
        self.assertNotIn("DELETE FROM", sql)
        with self.assertRaisesRegex(G.Error, "routing_original_attempt_invalid"):
            G.build_resolve_sql(prepare(), "unbound-attempt")

    def test_material_cannot_change_a_grant_without_a_matching_mapping_hash(self):
        material = prepare(); material["records"][0]["group_name"] = "tampered"
        with self.assertRaisesRegex(G.Error, "routing_grant_rows_invalid"):
            G.build_apply_sql(material)

    def test_snapshot_sql_is_select_only_and_includes_full_target_and_identity_contract(self):
        material = prepare()
        sql = G.snapshot_sql(material["before_state"]["inventory"], material["migration_id"])
        self.assertIn("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY", sql)
        self.assertIn("'native_users'", sql)
        self.assertIn("'identity_import'", sql)
        self.assertIn("core_identity.accounts", sql)
        self.assertNotIn("INSERT", sql)


class PrivateIoTests(unittest.TestCase):
    def test_frozen_files_are_rebuilt_and_tampered_grant_or_changed_source_refused(self):
        value = fixture()
        with tempfile.TemporaryDirectory() as directory, patch.object(G.IDENTITY, "BACKUP_ROOT", Path(directory)):
            root = Path(directory) / "inputs"; root.mkdir(mode=0o700)
            files = {}
            for name, data in value[-1].items():
                path = root / name; path.write_bytes(data); path.chmod(0o600)
                files[name] = {"path": str(path), "sha256": G.FIN.sha(data)}
            mutable = list(value); mutable[5] = files
            material = prepare(mutable)
            G.verify_source_files(material)
            material["records"][0]["group_name"] = "other"
            material["mapping_sha256"] = G.FIN.digest(material["records"])
            with self.assertRaisesRegex(G.Error, "routing_frozen_inputs_mismatch"):
                G.verify_source_files(material)
            material = prepare(mutable)
            Path(files["source_identity_export"]["path"]).write_bytes(value[3] + b"\n")
            with self.assertRaisesRegex(G.Error, "routing_source_file_changed"):
                G.verify_source_files(material)

    def test_cli_has_no_apply_or_connection_mode_and_refuses_without_leaking_source_data(self):
        with patch.object(sys, "argv", ["legacy_routing_grants.py", "--output", "/invalid"]), \
                patch("sys.stdout", new_callable=io.StringIO) as out, patch("sys.stderr", new_callable=io.StringIO) as err:
            self.assertEqual(G.main(), 1)
            self.assertEqual(out.getvalue(), "")
            self.assertEqual(json.loads(err.getvalue()), {"status": "REFUSED", "error_code": "routing_grant_inputs_or_private_output_invalid"})


if __name__ == "__main__":
    unittest.main()
