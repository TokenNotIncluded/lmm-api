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


SYNTHETIC_ROOT = tempfile.TemporaryDirectory(prefix="routing-synthetic-")


def fixture():
    root = Path(tempfile.mkdtemp(dir=SYNTHETIC_ROOT.name))
    root.chmod(0o700)
    archive = root / "source.dump"
    archive.write_bytes(b"synthetic-source")
    archive.chmod(0o600)
    archive_hash = G.FIN.sha(archive.read_bytes())
    users = [{"id": 7, "group": "原组/一", "status": 1, "deleted_at": None,
              "password": "synthetic-never-emit-password", "email": "private@example.invalid"},
             {"id": 29, "group": "inactive-group", "status": 2, "deleted_at": None},
             {"id": 41, "group": "deleted-group", "status": 1, "deleted_at": "2026-01-01T00:00:00+00:00"}]
    for row in users:
        row.setdefault("password", "synthetic-password")
        row.setdefault("email", "synthetic@example.invalid")
    mapping = [{"source_user_id": row["id"], "target_user_id": row["id"], "target_personal_account_id": index}
               for index, row in enumerate(users, 1)]
    encode = lambda rows: ("\n".join(G.FIN.canonical(row) for row in rows) + "\n").encode()
    mapping_bytes = encode(mapping)
    export_bytes = encode([{"kind": "identity", "row": {}}, {"kind": "schema_identity", "row": {}},
        {"kind": "user_count", "row": len(users)}, *({"kind": "user", "row": row} for row in users)])
    for row in users:
        row.setdefault("password", "synthetic-password")
        row.setdefault("email", "synthetic@example.invalid")
    source_instance = {"database": "synthetic_source", "role": "synthetic_reader", "database_oid": "99",
        "postmaster_started_at": "2026-01-01T00:00:00+00:00", "server_version_num": "180000",
        "port": 5433, "tcp_connection": False, "read_only": "on"}
    financial_bytes = encode([{"kind": "identity", "row": source_instance},
        {"kind": "schema", "row": {"schema": "public", "schema_owner": "synthetic_owner"}},
        *({"kind": "row", "table": "users", "row": row} for row in users)])
    manifest = {"format_version": 2, "scope": "identity-only", "source_archive_sha256": archive_hash,
        "source_export_sha256": G.FIN.sha(export_bytes), "source_target_id_map_sha256": G.FIN.sha(mapping_bytes),
        "mapping_sha256": "e" * 64}
    plan = {"format_version": 1, "scope": "legacy-routing-offline-plan", "status": "blocked", "apply_supported": False,
        "bindings": {"source_archive_sha256": archive_hash, "source_export_sha256": G.FIN.sha(financial_bytes),
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
            for row in mapping], "identity_import": {"migration_id": "synthetic-identity-import", "source_archive_sha256": archive_hash,
                "mapping_sha256": "e" * 64, "manifest": manifest}}
    data = {"routing_plan": (G.FIN.canonical(plan) + "\n").encode(), "identity_manifest": (G.FIN.canonical(manifest) + "\n").encode(),
            "identity_map": mapping_bytes, "source_identity_export": export_bytes, "source_financial_export": financial_bytes}
    columns = [{"schema": "public", "table_name": "users", "column_name": key} for key in users[0]]
    catalog = root / "native-catalog-metadata.jsonl"
    catalog.write_bytes(encode([[], [], columns, [], [], [], []]))
    baseline = root / "native-table-row-count-baseline.jsonl"
    baseline.write_bytes(encode([{"table": "users", "row_count": len(users)}]))
    for path in (catalog, baseline):
        path.chmod(0o600)
    receipt = {"status": "snapshot-restored-baseline-verified-frozen-and-classified",
        "archive": {"path": str(archive), "sha256": archive_hash},
        "source": {"schema": "public"}, "private_metadata_files": [str(catalog), str(baseline)],
        "readonly_reader": {"database": "synthetic_source", "role": "synthetic_reader", "port": 5433},
        "source_instance_identity": {**source_instance, "listen_addresses": "", "schema_owner": "synthetic_owner"}}
    proof = {"source_archive_sha256": archive_hash, "source_export_sha256": G.FIN.sha(financial_bytes),
        "metadata_hashes": {path.name: G.FIN.sha(path.read_bytes()) for path in (catalog, baseline)},
        "source_column_coverage_exact": True, "source_row_baseline_exact": True,
        "selected_table_counts": {"users": len(users)}, "source_instance": source_instance}
    data.update(source_receipt=(G.FIN.canonical(receipt) + "\n").encode(),
                export_proof=(G.FIN.canonical(proof) + "\n").encode())
    filenames = {"source_financial_export": "source-financial-export.private.jsonl"}
    files = {}
    for name, raw in data.items():
        path = root / filenames.get(name, name)
        path.write_bytes(raw); path.chmod(0o600)
        files[name] = {"path": str(path), "sha256": G.FIN.sha(raw)}
    return plan, manifest, mapping_bytes, export_bytes, snapshot, files, data


def prepare(value=None):
    plan, manifest, mapping, exported, snapshot, files, data = value or fixture()
    # Persist only this synthetic fixture's supplied bytes; real file parsing,
    # instance checks, catalog and baseline verification remain enabled.
    data = {**data, "routing_plan": (G.FIN.canonical(plan) + "\n").encode(),
            "identity_manifest": (G.FIN.canonical(manifest) + "\n").encode(),
            "identity_map": mapping, "source_identity_export": exported}
    for name, raw in data.items():
        Path(files[name]["path"]).write_bytes(raw)
        files[name]["sha256"] = G.FIN.sha(raw)
    with patch.object(G.IDENTITY, "BACKUP_ROOT", Path(tempfile.gettempdir())), \
                patch.object(G.ROUTING.F.IDENTITY, "BACKUP_ROOT", Path(tempfile.gettempdir())):
        return G.prepare_material(plan, manifest, mapping, exported, data["source_financial_export"], migration_id="synthetic-routing-grants",
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

    def test_plan_source_row_hash_must_match_frozen_identity_user(self):
        value = fixture()
        value[0]["user_group_plan"][0]["source_row_sha256"] = "f" * 64
        value[0].pop("plan_sha256")
        value[0]["plan_sha256"] = G.FIN.digest(value[0])
        with self.assertRaisesRegex(G.Error, "routing_financial_user_hash_disagrees"):
            prepare(value)

    def test_utc_binding_accepts_same_instant_but_preserves_all_other_fields(self):
        value = fixture()
        rows = [json.loads(line) for line in value[3].splitlines()]
        original = rows[-1]["row"]
        financial = {**original, "deleted_at": "2026-01-01T08:00:00+08:00"}
        financial_rows = [json.loads(line) for line in value[-1]["source_financial_export"].splitlines()]
        financial_rows[-1]["row"] = financial
        value[-1]["source_financial_export"] = ("\n".join(G.FIN.canonical(row) for row in financial_rows) + "\n").encode()
        value[0]["bindings"]["source_export_sha256"] = G.FIN.sha(value[-1]["source_financial_export"])
        proof = json.loads(value[-1]["export_proof"])
        proof["source_export_sha256"] = value[0]["bindings"]["source_export_sha256"]
        value[-1]["export_proof"] = (G.FIN.canonical(proof) + "\n").encode()
        planned = value[0]["user_group_plan"][-1]
        planned.update(source_row_sha256=G.FIN.digest(financial),
                       source_user_utc_sha256=G.ROUTING.user_binding_hash(financial))
        def seal():
            value[0].pop("plan_sha256", None)
            value[0]["plan_sha256"] = G.FIN.digest(value[0])
        seal()
        prepare(value)
        for field, changed in (("deleted_at", "2026-01-01T08:00:01+08:00"),
                               ("id", 42), ("group", "other-group"), ("status", 2),
                               ("email", "different@example.invalid")):
            with self.subTest(field=field):
                planned["source_user_utc_sha256"] = G.ROUTING.user_binding_hash({**financial, field: changed})
                seal()
                with self.assertRaisesRegex(G.Error, "routing_user_group_source_hash_disagrees"):
                    prepare(value)
        planned.pop("source_user_utc_sha256")
        seal()
        with self.assertRaisesRegex(G.Error, "routing_legacy_plan_requires_frozen_replan"):
            prepare(value)

    def test_self_resealed_plan_cannot_claim_unrelated_financial_export_or_raw_row(self):
        for mutation in (
                lambda p: p["bindings"].update(source_export_sha256="f" * 64),
                lambda p: p["user_group_plan"][0].update(source_row_sha256="f" * 64,
                    source_user_utc_sha256=G.ROUTING.user_binding_hash(json.loads(fixture()[3].splitlines()[3])["row"]))):
            value = fixture()
            mutation(value[0])
            value[0].pop("plan_sha256")
            value[0]["plan_sha256"] = G.FIN.digest(value[0])
            with self.assertRaisesRegex(G.Error, "routing_financial_(export_hash_mismatch|user_hash_disagrees)"):
                prepare(value)

    def test_changed_financial_row_cannot_be_laundered_through_resealed_plan(self):
        # Even if the export hash and both plan row hashes are recomputed, the
        # independent identity source must still match every source user field.
        for field, changed in (("email", "changed@example.invalid"), ("status", 2),
                               ("group", "other"), ("deleted_at", "2026-01-01T00:00:00Z")):
            with self.subTest(field=field):
                value = fixture()
                rows = [json.loads(line) for line in value[-1]["source_financial_export"].splitlines()]
                rows[2]["row"][field] = changed
                raw = ("\n".join(G.FIN.canonical(row) for row in rows) + "\n").encode()
                value[-1]["source_financial_export"] = raw
                value[0]["bindings"]["source_export_sha256"] = G.FIN.sha(raw)
                value[0]["user_group_plan"][0].update(source_row_sha256=G.FIN.digest(rows[2]["row"]),
                    source_user_utc_sha256=G.ROUTING.user_binding_hash(rows[2]["row"]))
                value[0].pop("plan_sha256")
                value[0]["plan_sha256"] = G.FIN.digest(value[0])
                with self.assertRaisesRegex(G.Error, "routing_financial_identity_user_disagrees"):
                    prepare(value)

    def test_financial_user_coverage_and_framing_are_required(self):
        for mutate in (lambda rows: rows.pop(), lambda rows: rows.append(deepcopy(rows[-1])),
                       lambda rows: rows.pop(0), lambda rows: rows.append(deepcopy(rows[0]))):
            value = fixture()
            rows = [json.loads(line) for line in value[-1]["source_financial_export"].splitlines()]
            mutate(rows)
            raw = ("\n".join(G.FIN.canonical(row) for row in rows) + "\n").encode()
            value[-1]["source_financial_export"] = raw
            value[0]["bindings"]["source_export_sha256"] = G.FIN.sha(raw)
            value[0].pop("plan_sha256")
            value[0]["plan_sha256"] = G.FIN.digest(value[0])
            with self.assertRaises(G.Error):
                prepare(value)

    def test_resealed_export_and_plan_cannot_replace_source_framing(self):
        for kind in ("identity", "schema"):
            value = fixture()
            rows = [json.loads(line) for line in value[-1]["source_financial_export"].splitlines()]
            row = next(tag["row"] for tag in rows if tag["kind"] == kind)
            row["database" if kind == "identity" else "schema_owner"] = "unrelated_source"
            raw = ("\n".join(G.FIN.canonical(tag) for tag in rows) + "\n").encode()
            value[-1]["source_financial_export"] = raw
            value[0]["bindings"]["source_export_sha256"] = G.FIN.sha(raw)
            value[0].pop("plan_sha256")
            value[0]["plan_sha256"] = G.FIN.digest(value[0])
            proof = json.loads(value[-1]["export_proof"])
            proof["source_export_sha256"] = G.FIN.sha(raw)
            if kind == "identity":
                proof["source_instance"] = row
            value[-1]["export_proof"] = (G.FIN.canonical(proof) + "\n").encode()
            with self.subTest(kind=kind), self.assertRaises(G.Error):
                prepare(value)

    def test_receipt_archive_must_match_independently_bound_identity_archive(self):
        value = fixture()
        receipt = json.loads(value[-1]["source_receipt"])
        receipt["archive"]["sha256"] = "f" * 64
        value[-1]["source_receipt"] = (G.FIN.canonical(receipt) + "\n").encode()
        with self.assertRaisesRegex(G.Error, "routing_financial_receipt_binding_mismatch"):
            prepare(value)

    def test_financial_parser_validates_each_row_before_decoding_the_next(self):
        value = fixture()
        raw = b'{"kind":"unknown","row":{}}\nnot-json\n'
        value[-1]["source_financial_export"] = raw
        value[0]["bindings"]["source_export_sha256"] = G.FIN.sha(raw)
        value[0].pop("plan_sha256")
        value[0]["plan_sha256"] = G.FIN.digest(value[0])
        with self.assertRaisesRegex(G.Error, "routing_financial_export_shape_invalid"):
            prepare(value)

    def test_old_material_without_financial_input_is_refused(self):
        material = prepare()
        del material["source_files"]["source_financial_export"]
        with self.assertRaisesRegex(G.Error, "routing_source_files_incomplete"):
            G.verify_source_files(material)

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
        with patch.object(G.IDENTITY, "BACKUP_ROOT", Path(tempfile.gettempdir())), \
                patch.object(G.ROUTING.F.IDENTITY, "BACKUP_ROOT", Path(tempfile.gettempdir())):
            files = value[5]
            mutable = list(value)
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
