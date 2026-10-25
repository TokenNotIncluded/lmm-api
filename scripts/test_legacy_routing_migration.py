"""Offline routing behavior checks; native/production acceptance is separate."""
import copy
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import legacy_routing_migration as R


def fixture(weights=(0, 0), priorities=(10, 10), memory=True):
    bindings = {"source_archive_sha256": "a" * 64, "source_export_sha256": "b" * 64,
                "identity_map_file_sha256": "c" * 64}
    tables = {"users": [{"id": 71, "group": "原组/一", "status": 1, "deleted_at": None,
                         "email": "synthetic-private@example.invalid"}], "channels": [], "abilities": [], "options": []}
    for cid, (weight, priority) in enumerate(zip(weights, priorities), 1):
        tables["channels"].append({"id": cid, "type": 1, "key": "synthetic-secret-" + str(cid),
            "base_url": "https://provider.example", "status": 1, "group": "原组/一", "models": "example-model",
            "weight": weight, "priority": priority})
        tables["abilities"].append({"group": "原组/一", "model": "example-model", "channel_id": cid,
                                    "enabled": True, "weight": weight, "priority": priority})
    runtime = {"format_version": 1, "scope": "legacy-routing-runtime-evidence", "bindings": bindings,
               "source_revision": "d" * 40, "evidence_reference": "synthetic-source-contract",
               "memory_cache_enabled": memory, "streaming_enabled": False, "model_matching": "exact", "max_attempts": 1,
               "limits": {"input_token_ceiling": 4096, "lease_seconds": 90, "timeout_seconds": 60, "max_output_tokens": 2048}}
    providers = {"format_version": 1, "scope": "legacy-routing-provider-contracts", "bindings": bindings, "channels": []}
    credentials = {"format_version": 1, "scope": "legacy-routing-credential-references", "bindings": bindings, "credentials": []}
    for channel in tables["channels"]:
        providers["channels"].append({"channel_id": channel["id"], "source_channel_sha256": R.F.digest(channel),
            "protocol": "openai_chat", "endpoint": channel["base_url"] + "/v1", "legacy_path_prefix": "/v1",
            "contract_revision": "synthetic-only-provider-contract", "models": [{"source_model": "example-model",
                "upstream_model": "example-model", "source_operations": ["chat.completions"], "max_input_tokens": 4096, "max_output_tokens": 2048,
                "missing_cached_tokens_is_zero": True, "missing_cache_creation_tokens_is_zero": False}],
            "prices": [{"group": "原组/一", "model": "example-model", "kind": "flat_tokens",
                "source_options_sha256": R.F.digest(tables["options"]), "contract_revision": "synthetic-price-contract",
                "rates": {"input": 2, "output": 4, "cache_read": 0, "cache_write": 0, "request": 0},
                "group_multiplier_ppm": 1_000_000, "speed_multiplier_ppm": 1_000_000}]})
        credentials["credentials"].append({"channel_id": channel["id"], "source_channel_sha256": R.F.digest(channel),
            "source_key_sha256": hashlib.sha256(channel["key"].encode()).hexdigest(),
            "reference": {"id": channel["id"], "revision": 1}, "secret_file": "/private/install/channel-" + str(channel["id"])})
    return tables, {71: 19}, bindings, {"runtime": runtime, "providers": providers, "credentials": credentials,
                                          "source_counts": {"public_relay_preferences": 0}}


def run_fixture(value):
    tables, identities, bindings, contracts = value
    return R.plan(tables, identities, bindings, **contracts)


def reasons(result):
    return {entry["reason"] for entry in result["blockers"]}


def rebind_channel(value, cid=1):
    tables, _, _, contracts = value
    channel = next(c for c in tables["channels"] if c["id"] == cid)
    for document, field in ((contracts["providers"], "channels"), (contracts["credentials"], "credentials")):
        next(c for c in document[field] if c["channel_id"] == cid)["source_channel_sha256"] = R.F.digest(channel)
    next(c for c in contracts["credentials"]["credentials"] if c["channel_id"] == cid)["source_key_sha256"] = hashlib.sha256(channel["key"].encode()).hexdigest()


class RoutingSemanticsTests(unittest.TestCase):
    def test_high_old_priority_maps_to_low_native_priority(self):
        result = run_fixture(fixture(priorities=(-5, 100)))
        self.assertEqual(result["status"], "candidate-requires-native-validation")
        self.assertEqual([(a["channel_id"], a["native_priority"]) for a in result["ability_assessments"]], [(1, 1), (2, 0)])

    def test_cached_zero_weights_are_equal_nonzero_and_mixed_zero_remains_unselected(self):
        for old, expected in (((0, 0), [100, 100]), ((0, 5), [0, 5])):
            result = run_fixture(fixture(weights=old))
            self.assertEqual(result["status"], "candidate-requires-native-validation")
            self.assertEqual([a["native_weight"] for a in result["ability_assessments"]], expected)

    def test_database_all_zero_tie_is_explicitly_blocked_and_no_candidate_written(self):
        result = run_fixture(fixture(memory=False))
        self.assertIn("legacy_database_tie_order_not_defined", reasons(result))
        self.assertNotIn("candidate", result)

    def test_database_distinct_weights_preserve_real_boundary_bias(self):
        result = run_fixture(fixture(weights=(0, 20), memory=False))
        self.assertEqual([a["native_weight"] for a in result["ability_assessments"]], [9, 31])
        self.assertEqual(result["status"], "candidate-requires-native-validation")
        old = [0, 20]
        counts = [0, 0]
        for ticket in range(sum(w + 10 for w in old)):
            remaining = ticket
            for index in (1, 0):
                remaining -= old[index] + 10
                if remaining <= 0:
                    counts[index] += 1
                    break
        self.assertEqual(counts, [9, 31])

    def test_actual_user_group_is_preserved_without_granting_another_route_group(self):
        value = fixture()
        value[0]["users"][0]["group"] = "另一个组"
        result = run_fixture(value)
        self.assertEqual(result["user_group_plan"][0]["group_name"], "另一个组")
        self.assertNotEqual(result["user_group_plan"][0]["group_name"], result["candidate"]["routing"]["groups"][0]["name"])
        self.assertEqual(result["candidate"]["key_groups"], [])

    def test_nonempty_or_unproven_user_channel_preferences_cannot_be_ignored(self):
        for counts in (None, {}, {"public_relay_preferences": False}, {"public_relay_preferences": 1}):
            value = fixture(); value[3]["source_counts"] = counts
            self.assertIn("legacy_user_channel_preferences_not_mapped", reasons(run_fixture(value)))
        value = fixture(); value[0]["channels"][0]["public_relay_contribution_id"] = 0; rebind_channel(value)
        self.assertEqual(run_fixture(value)["status"], "candidate-requires-native-validation")

    def test_missing_or_orphan_user_and_duplicate_channel_or_ability_are_rejected(self):
        for mutation in (lambda v: v[1].clear(), lambda v: v[1].update({99: 20}),
                         lambda v: v[0]["channels"].append(copy.deepcopy(v[0]["channels"][0])),
                         lambda v: v[0]["abilities"].append(copy.deepcopy(v[0]["abilities"][0]))):
            value = fixture(); mutation(value)
            with self.assertRaises(R.MappingError):
                run_fixture(value)

    def test_disabled_and_deleted_users_receive_only_disabled_grant_plans(self):
        for change in ({"status": 2}, {"deleted_at": "2026-01-01T00:00:00Z"}):
            value = fixture(); value[0]["users"][0].update(change)
            self.assertFalse(run_fixture(value)["user_group_plan"][0]["enabled"])

    def test_cache_projection_or_weight_disagreement_cannot_silently_use_abilities(self):
        for field, item, reason in (("models", "other-model", "cache_channel_ability_projection_disagrees"),
                                     ("weight", 11, "cache_channel_priority_or_weight_disagrees")):
            value = fixture(); value[0]["channels"][0][field] = item; rebind_channel(value)
            self.assertIn(reason, reasons(run_fixture(value)))

    def test_special_channel_credentials_headers_and_expression_prices_are_not_dropped(self):
        cases = (("header_override", '{"x-private":"synthetic-value"}', "special_channel_behavior_not_mapped"),
                 ("key", "first-key\nsecond-key", "special_credential_not_supported"),
                 ("key", '{"refresh_token":"synthetic"}', "special_credential_not_supported"),
                 ("channel_info", {"is_multi_key": True}, "multi_key_or_channel_state_not_mapped"),
                 ("type", 58, "channel_type_not_supported"))
        for field, item, reason in cases:
            value = fixture(); value[0]["channels"][0][field] = item; rebind_channel(value)
            with self.subTest(field=field, item=item):
                result = run_fixture(value)
                self.assertIn(reason, reasons(result)); self.assertNotIn("candidate", result)
        value = fixture(); value[0]["options"].append({"key": "ModelBillingExpression", "value": "input * 0.1"})
        for provider in value[3]["providers"]["channels"]:
            provider["prices"][0]["source_options_sha256"] = R.F.digest(value[0]["options"])
        self.assertIn("legacy_expression_pricing_not_compiled", reasons(run_fixture(value)))

    def test_endpoint_prefix_is_explicit_and_preserves_actual_operation_path(self):
        value = fixture()
        result = run_fixture(value)
        self.assertEqual(result["candidate"]["routing"]["upstreams"][0]["endpoint"] + "/chat/completions",
                         value[0]["channels"][0]["base_url"] + "/v1/chat/completions")
        value[3]["providers"]["channels"][0]["endpoint"] = value[0]["channels"][0]["base_url"]
        self.assertIn("provider_endpoint_not_bound_to_source", reasons(run_fixture(value)))

    def test_runtime_retry_normalization_and_absent_contracts_are_blocked(self):
        for field, item, reason in (("max_attempts", 3, "legacy_retry_semantics_not_mapped"),
                                    ("model_matching", "legacy-normalized-fallback", "legacy_model_normalization_not_mapped")):
            value = fixture(); value[3]["runtime"][field] = item
            self.assertIn(reason, reasons(run_fixture(value)))
        tables, identities, bindings, _ = fixture()
        result = R.plan(tables, identities, bindings)
        self.assertIn("runtime_evidence_missing", reasons(result))
        self.assertIn("provider_contract_missing", reasons(result))
        self.assertIn("credential_reference_missing", reasons(result))
        self.assertNotIn("candidate", result)

    def test_partial_observed_runtime_retains_cache_weights_without_inventing_limits(self):
        value = fixture()
        value[3]["runtime"].update(limits=None, max_attempts=None, streaming_enabled=None)
        result = run_fixture(value)
        self.assertEqual([a["native_weight"] for a in result["ability_assessments"]], [100, 100])
        self.assertIn("runtime_native_limits_not_supplied", reasons(result))
        self.assertIn("runtime_attempts_evidence_missing", reasons(result))
        self.assertNotIn("candidate", result)

    def test_unknown_user_state_and_unproven_nontext_operations_are_not_silently_converted(self):
        for field, item in (("status", 3), ("deleted_at", 0)):
            value = fixture(); value[0]["users"][0][field] = item
            with self.subTest(field=field), self.assertRaises(R.MappingError):
                run_fixture(value)
        value = fixture(); value[3]["providers"]["channels"][0]["models"][0]["source_operations"] = ["images.generations"]
        result = run_fixture(value)
        self.assertIn("legacy_operation_surface_not_supported_or_unproven", reasons(result))
        self.assertNotIn("candidate", result)

    def test_hash_drift_in_all_contracts_source_channel_key_and_options_is_rejected(self):
        for scope in ("runtime", "providers", "credentials"):
            value = fixture(); value[3][scope] = copy.deepcopy(value[3][scope])
            value[3][scope]["bindings"]["source_export_sha256"] = "f" * 64
            with self.subTest(scope=scope), self.assertRaises(R.MappingError):
                run_fixture(value)
        for mutation in (lambda v: v[0]["channels"][0].update(name="changed"),
                         lambda v: v[3]["credentials"]["credentials"][0].update(source_key_sha256="f" * 64),
                         lambda v: v[0]["options"].append({"key": "ModelRatio", "value": "changed"})):
            value = fixture(); mutation(value)
            with self.assertRaises(R.MappingError):
                run_fixture(value)

    def test_different_keys_cannot_share_an_immutable_ref_or_secret_destination(self):
        for changes in ({"reference": {"id": 1, "revision": 1}, "secret_file": "/private/install/channel-1"},
                        {"secret_file": "/private/install/channel-1"}):
            value = fixture(); value[3]["credentials"]["credentials"][1].update(changes)
            with self.assertRaisesRegex(R.MappingError, "credential_reference_or_destination_key_conflict"):
                run_fixture(value)

    def test_same_channel_in_multiple_models_has_globally_unique_target_ids(self):
        value = fixture(weights=(0,), priorities=(10,))
        value[0]["channels"][0]["models"] += ",second-model"
        second = {**value[0]["abilities"][0], "model": "second-model"}
        value[0]["abilities"].append(second); rebind_channel(value)
        provider = value[3]["providers"]["channels"][0]
        provider["models"].append({**provider["models"][0], "source_model": "second-model", "upstream_model": "second-model"})
        provider["prices"].append({**provider["prices"][0], "model": "second-model"})
        routes = run_fixture(value)["candidate"]["routing"]["routes"]
        target_ids = [target["id"] for route in routes for target in route["targets"]]
        self.assertEqual(len(set(target_ids)), 2)

    def test_native_toml_round_trip_is_not_reported_as_approval_or_native_acceptance(self):
        result = run_fixture(fixture())
        candidate = result["candidate"]
        encoded = "\n".join(json.dumps(k) + " = " + R.toml_value(v) for k, v in candidate.items())
        self.assertEqual(tomllib.loads(encoded), candidate)
        self.assertFalse(result["apply_supported"]); self.assertFalse(result["approval"])
        self.assertIn("price-equivalence", result["unverified"])
        self.assertTrue(all(route["speeds"] == ["standard"] for route in candidate["routing"]["routes"]))
        summary = R.F.canonical(R.public_summary(result))
        private_plan = R.F.canonical(result)
        for channel in fixture()[0]["channels"]:
            self.assertNotIn(channel["key"], private_plan)
        self.assertNotIn("synthetic-private@example.invalid", private_plan)
        self.assertNotIn("71", summary); self.assertNotIn("原组", summary)

    def test_typed_numbers_group_ascii_and_native_price_bounds_are_strict(self):
        self.assertFalse(R.label("group+$")); self.assertTrue(R.label("官key/一"))
        for mutation in (lambda v: v[3]["runtime"].update(memory_cache_enabled=1),
                         lambda v: v[3]["providers"]["channels"][0]["prices"][0].update(speed_multiplier_ppm=0),
                         lambda v: v[3]["providers"]["channels"][0]["prices"][0]["rates"].update(input=2**63)):
            value = fixture(); mutation(value)
            with self.assertRaises(R.MappingError):
                run_fixture(value)
        value = fixture()
        price = value[3]["providers"]["channels"][0]["prices"][0]
        price.update(group_multiplier_ppm=1_000_000_000, speed_multiplier_ppm=1_000_000_000)
        price["rates"]["input"] = 2_000_000_000_000_000_000
        self.assertIn("price_not_exact_integer_credits", reasons(run_fixture(value)))


class FileSafetyTests(unittest.TestCase):
    def test_streamed_reuse_validates_unretained_rows_and_supports_shared_credential_tables(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "source.dump"; archive.write_bytes(b"synthetic-archive")
            raw = root / "source-financial-export.private.jsonl"
            proof_path = root / "source-export-proof.private.json"
            source = {"users": [{"id": 71}], "channels": [{"id": 1}], "abilities": [], "options": [],
                      "tokens": [{"id": 8, "key": "synthetic-token-never-print"}]}
            columns = {table: set(rows[0]) if rows else {"synthetic_empty_column"} for table, rows in source.items()}
            baseline = {table: len(rows) for table, rows in source.items()}
            identity = {"synthetic": "instance"}
            tags = [{"kind": "identity", "row": identity}, {"kind": "schema", "row": {"schema": "legacy", "schema_owner": "owner"}}]
            tags.extend({"kind": "row", "table": table, "row": row} for table, rows in source.items() for row in rows)
            receipt = {"status": "snapshot-restored-baseline-verified-frozen-and-classified",
                       "archive": {"path": str(archive), "sha256": R.F.IDENTITY.sha256_file(archive)},
                       "source": {"schema": "legacy"}, "source_instance_identity": {"schema_owner": "owner"}, "readonly_reader": {}}

            def write(rows):
                raw.write_text("\n".join(R.F.canonical(row) for row in rows) + "\n")
                proof_path.write_text(R.F.canonical({"source_archive_sha256": receipt["archive"]["sha256"],
                    "source_export_sha256": R.F.IDENTITY.sha256_file(raw), "metadata_hashes": {},
                    "source_column_coverage_exact": True, "source_row_baseline_exact": True,
                    "selected_table_counts": baseline, "source_instance": identity}))

            write(tags)
            with patch.object(R.F.IDENTITY, "private_backup_path", side_effect=lambda path: path), \
                 patch.object(R.F.IDENTITY, "verify_instance") as verify, \
                 patch.object(R.F, "metadata", return_value=(columns, baseline, {}, {}, {})), \
                 patch.object(R.F.subprocess, "run", side_effect=AssertionError("must-not-query")) as query:
                tables, _, counts = R.load_routing_export(receipt, proof_path, {})
                self.assertNotIn("tokens", tables); self.assertEqual(counts["tokens"], 1)
                credential_tables, _, _ = R.load_routing_export(receipt, proof_path, {}, retained_tables=("users", "tokens", "options"))
                self.assertEqual(credential_tables["tokens"], source["tokens"])
                self.assertEqual(verify.call_count, 2)
                for changed in (tags[:-1], tags + [tags[-1]], [*tags[:-1], {"kind": "row", "table": "tokens", "row": {"id": 8}}]):
                    write(changed)
                    with self.assertRaises(R.MappingError):
                        R.load_routing_export(receipt, proof_path, {})
                query.assert_not_called()

    def test_runtime_process_observation_binds_source_revision_cache_and_file_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runtime.private.json"
            _, _, bindings, contracts = fixture()
            runtime = contracts["runtime"]
            observed = {"source_archive_sha256": bindings["source_archive_sha256"], "source_revision": runtime["source_revision"],
                        "memory_cache_enabled": True}
            path.write_text(R.F.canonical(observed))
            runtime.update(evidence_file=str(path), evidence_file_sha256=R.F.IDENTITY.sha256_file(path))
            hashes = {}
            with patch.object(R.F.IDENTITY, "private_backup_path", side_effect=lambda path: path):
                R.bind_runtime_observation(runtime, bindings, hashes)
                self.assertIn(path, hashes)
                observed["memory_cache_enabled"] = False; path.write_text(R.F.canonical(observed))
                with self.assertRaises(R.MappingError):
                    R.bind_runtime_observation(runtime, bindings, hashes)
                runtime["evidence_file_sha256"] = R.F.IDENTITY.sha256_file(path)
                with self.assertRaises(R.MappingError):
                    R.bind_runtime_observation(runtime, bindings, hashes)

    def test_running_file_digest_drift_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "private.json"
            path.write_text("original")
            hashes = {path: R.F.IDENTITY.sha256_file(path)}
            R.verify_files(hashes)
            path.write_text("changed")
            with self.assertRaisesRegex(R.MappingError, "routing_input_changed_during_plan"):
                R.verify_files(hashes)

    def test_invalid_input_exception_never_prints_raw_secret_or_user(self):
        secret = "synthetic-do-not-print-this"
        arguments = ["planner", "--source-receipt", "/absent", "--export-proof", "/absent",
                     "--identity-map", "/absent", "--identity-manifest", "/absent", "--output", "/absent"]
        output = io.StringIO()
        with patch("sys.argv", arguments), patch("sys.stderr", output), \
             patch.object(R.F.IDENTITY, "private_backup_path", side_effect=ValueError(secret)):
            self.assertEqual(R.main(), 1)
        self.assertNotIn(secret, output.getvalue())
        self.assertEqual(json.loads(output.getvalue())["reason"], "routing_inputs_or_private_output_invalid")

    def test_no_apply_flag_and_bad_inputs_never_start_export_or_database_query(self):
        arguments = ["planner", "--source-receipt", "/absent", "--export-proof", "/absent",
                     "--identity-map", "/absent", "--identity-manifest", "/absent", "--output", "/absent", "--apply"]
        with patch("sys.argv", arguments), patch("sys.stderr", io.StringIO()), patch.object(R.F, "reuse_frozen_export") as read:
            with self.assertRaises(SystemExit):
                R.main()
        read.assert_not_called()


if __name__ == "__main__":
    unittest.main()
