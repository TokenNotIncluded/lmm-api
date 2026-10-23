"""Synthetic contracts and opt-in owned-fixture transactions; no real snapshots."""

from copy import deepcopy
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("migrate-legacy-snapshot.py")
SPEC = importlib.util.spec_from_file_location("migrate_legacy_snapshot", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)

SOURCE_SHA256 = "a" * 64


def synthetic_bcrypt(prefix="2b", cost=10):
    """Shape-only verifier fixture; contains no person's password or verifier."""
    return f"${prefix}${cost:02d}$" + "." * 53


def user(identifier=37, **changes):
    result = {
        "id": identifier,
        "username": f"synthetic-{identifier}",
        "password": synthetic_bcrypt(),
        "display_name": "Synthetic fixture",
        "role": 1,
        "status": 1,
        "email": f"fixture-{identifier}@example.invalid",
        "setting": '{"language":"zh","unknown_future_option":true}',
        "deleted_at": None,
        "auth_version": 1,
        "trust_level_override": None,
        "console_activated_at": 0,
    }
    result.update(changes)
    return result


def mapped(rows, factors=None, source_sha256=SOURCE_SHA256, **options):
    return MODULE.map_identity(rows, factors or [], source_sha256=source_sha256, **options)


def target_contract(**options):
    return {**MODULE.current_contract(), "catalog_fingerprint_sha256": "c" * 64,
            "allow_initialized_core_seed": True, "initialized_core_seed_contract": MODULE.CORE_SEED_CONTRACT,
            "initialized_core_seed_sha256": "d" * 64,
            "trust_policy_seed": {
                "singleton": True, "revision": 1, "spec": deepcopy(MODULE.DEFAULT_TRUST_POLICY),
                "source_digest": hashlib.sha256(b"core-native-trust-policy-v1").hexdigest(),
                "source_cutoff": 1791763200, "native_user_floor": 1,
                "native_since": "2026-10-12T00:00:00+00:00"}, **options}


def manifest_for(plan, **changes):
    return {"format_version": 2, "scope": "identity-only", "source_archive_sha256": SOURCE_SHA256,
            "identity_contract": MODULE.identity_contract(), "developer_access_policy": MODULE.DEVELOPER_POLICY,
            "initialized_core_seed_contract": MODULE.CORE_SEED_CONTRACT, "source_export_sha256": "d" * 64,
            "import_tool_sha256": MODULE.sha256_file(SCRIPT),
            "import_contract_sha256": hashlib.sha256(MODULE.canonical(MODULE.current_contract()).encode()).hexdigest(),
            "source_baseline_proof_sha256": "e" * 64, "source_target_id_map_sha256": hashlib.sha256(MODULE.id_map_bytes(plan["records"]).encode()).hexdigest(),
            "mapping_sha256": plan["mapping_sha256"], "summary": plan["summary"], **changes}


def target_identity(**changes):
    return {"database": "synthetic_database", "role": "synthetic_owner", "database_owner": "synthetic_owner",
            "session_user": "synthetic_installer", "effective_role_can_login": False,
            "database_oid": 12345, "port": 55432, "server_version_num": "180006",
            "postmaster_started_at": "2026-10-12 00:00:00+00", "server_instance_nonce": "synthetic-identity-fixture", **changes}


def do_block(sql):
    """Separate the outer PostgreSQL dollar string without trusting its tag."""
    match = re.search(r"\bDO\s+(\$[a-zA-Z_][a-zA-Z_0-9]*\$)", sql)
    if match is None:
        raise AssertionError("SQL plan has no dollar-quoted DO block")
    delimiter = match.group(1)
    end = sql.find(delimiter, match.end())
    if end < 0:
        raise AssertionError("SQL plan has no closing dollar delimiter")
    return sql[:match.start()], sql[match.end():end], sql[end + len(delimiter):], delimiter


def single_quoted_literal(sql, marker):
    """Decode SQL doubled quotes; backslashes stay literal with standard strings."""
    position = sql.index(marker) + len(marker)
    while sql[position].isspace():
        position += 1
    if sql[position] != "'":
        raise AssertionError("expected a single-quoted SQL value")
    position += 1
    decoded = []
    while position < len(sql):
        char = sql[position]
        if char == "'":
            if position + 1 < len(sql) and sql[position + 1] == "'":
                decoded.append("'")
                position += 2
                continue
            return "".join(decoded), sql[position + 1:]
        decoded.append(char)
        position += 1
    raise AssertionError("unterminated SQL string literal")


def without_literals_and_comments(sql):
    """Keep SQL code positions while excluding data strings and SQL comments."""
    output = []
    index = 0
    while index < len(sql):
        if sql[index] == "'":
            output.append(" ")
            index += 1
            while index < len(sql):
                if sql[index] == "'":
                    if index + 1 < len(sql) and sql[index + 1] == "'":
                        index += 2
                        continue
                    index += 1
                    break
                index += 1
        elif sql.startswith("--", index):
            ending = sql.find("\n", index)
            index = len(sql) if ending < 0 else ending
            output.append(" ")
        else:
            output.append(sql[index])
            index += 1
    return "".join(output)


class IdentityMappingTests(unittest.TestCase):
    def test_preserves_user_ids_and_maps_accounts_independently_in_stable_order(self):
        result = mapped([user(91), user(7), user(300)])
        self.assertEqual([row["user_id"] for row in result["records"]], [7, 91, 300])
        self.assertEqual([row["account_id"] for row in result["records"]], [1, 2, 3])
        self.assertEqual(len({row["account_id"] for row in result["records"]}), 3)
        self.assertNotEqual(result["records"][0]["account_id"], result["records"][0]["user_id"])

    def test_exact_profile_and_auth_revision_are_preserved(self):
        original = user(
            username=" 测试 O'Reilly\\Name ",
            display_name="模型网络 🐦",
            email="CaseSensitive+fixture@example.invalid",
            setting='{"unrecognized":{"quoted":"\\\"; DROP TABLE users; --"}}',
            auth_version=19,
        )
        record = mapped([original])["records"][0]
        for field in ("username", "display_name", "email", "setting", "auth_version"):
            self.assertEqual(record[field], original[field])
        self.assertEqual(record["platform_role"], "user")
        self.assertTrue(record["active"])
        self.assertTrue(record["password_login_allowed"])
        self.assertFalse(record["email_login_allowed"])
        self.assertEqual(record["blocked_reasons"], [])

    def test_case_sensitive_usernames_and_duplicate_emails_are_not_collapsed(self):
        records = mapped([
            user(10, username="Fixture", email="shared@example.invalid"),
            user(20, username="fixture", email="shared@example.invalid"),
        ])["records"]
        self.assertEqual([row["username"] for row in records], ["Fixture", "fixture"])
        self.assertEqual([row["email"] for row in records], ["shared@example.invalid"] * 2)
        self.assertTrue(all(not row["email_login_allowed"] for row in records))

    def test_nullable_optional_profiles_have_explicit_normalizations(self):
        row = mapped([user(display_name=None, email=None, setting=None)])["records"][0]
        self.assertEqual((row["display_name"], row["email"], row["setting"]), ("", "", "{}"))
        self.assertEqual(len(row["normalizations"]), 3)

    def test_bcrypt_variants_and_supported_costs_keep_exact_verifier(self):
        for prefix in ("2a", "2b", "2y"):
            for cost in (4, 10, 16):
                with self.subTest(prefix=prefix, cost=cost):
                    verifier = synthetic_bcrypt(prefix, cost)
                    row = mapped([user(password=verifier)])["records"][0]
                    self.assertEqual(row["password_hash"], verifier)
                    self.assertTrue(row["legacy_bcrypt_allowed"])
                    self.assertTrue(row["password_login_allowed"])

    def test_unsupported_or_malformed_verifiers_never_become_credentials(self):
        verifiers = [
            synthetic_bcrypt(cost=3),
            synthetic_bcrypt(cost=17),
            synthetic_bcrypt(prefix="2x"),
            synthetic_bcrypt()[:-1],
            synthetic_bcrypt() + ".",
            synthetic_bcrypt()[:-1] + "!",
            "unknown-verifier-format",
        ]
        for verifier in verifiers:
            with self.subTest(verifier_shape=len(verifier)):
                row = mapped([user(password=verifier)])["records"][0]
                self.assertIsNone(row["password_hash"])
                self.assertFalse(row["legacy_bcrypt_allowed"])
                self.assertFalse(row["password_login_allowed"])
                self.assertTrue(row["blocked_reasons"])

    def test_empty_or_null_password_does_not_get_a_fabricated_verifier(self):
        for password in ("", None):
            with self.subTest(password_is_null=password is None):
                row = mapped([user(password=password)])["records"][0]
                self.assertIsNone(row["password_hash"])
                self.assertFalse(row["password_login_allowed"])
                self.assertFalse(row["legacy_bcrypt_allowed"])

    def test_privileged_roles_are_preserved_but_blocked_until_permissions_match(self):
        for role, platform_role in ((10, "admin"), (100, "superadmin")):
            with self.subTest(role=role):
                row = mapped([user(role=role)])["records"][0]
                self.assertEqual(row["platform_role"], platform_role)
                self.assertFalse(row["active"])
                self.assertFalse(row["password_login_allowed"])
                self.assertTrue(row["blocked_reasons"])

    def test_unknown_role_or_status_is_explicitly_blocked(self):
        for changes in ({"role": 0}, {"role": 999}, {"role": None}, {"status": 0}, {"status": 999}, {"status": None}):
            with self.subTest(changes=changes):
                row = mapped([user(**changes)])["records"][0]
                self.assertFalse(row["active"])
                self.assertFalse(row["password_login_allowed"])
                self.assertTrue(row["blocked_reasons"])

    def test_disabled_and_soft_deleted_users_are_not_reactivated(self):
        for changes in ({"status": 2}, {"deleted_at": "2020-01-01T00:00:00Z"}):
            with self.subTest(changes=changes):
                row = mapped([user(**changes)])["records"][0]
                self.assertFalse(row["active"])
                self.assertTrue(row["blocked_reasons"])

    def test_empty_deleted_marker_is_not_a_soft_delete(self):
        self.assertTrue(mapped([user(deleted_at="")])["records"][0]["active"])

    def test_enabled_second_factor_cannot_be_bypassed_by_password_import(self):
        result = mapped([user(37), user(38)], [{"user_id": 37, "enabled": True}])
        first, second = result["records"]
        self.assertTrue(first["second_factor_required"])
        self.assertFalse(first["password_login_allowed"])
        self.assertTrue(first["blocked_reasons"])
        self.assertFalse(second["second_factor_required"])
        self.assertTrue(second["password_login_allowed"])

    def test_disabled_second_factor_does_not_block_password_login(self):
        row = mapped([user()], [{"user_id": 37, "enabled": False}])["records"][0]
        self.assertFalse(row["second_factor_required"])
        self.assertTrue(row["password_login_allowed"])

    def test_global_password_disable_blocks_all_users_without_changing_profiles(self):
        records = mapped([user(37), user(38)], password_login_enabled=False)["records"]
        self.assertTrue(all(not row["password_login_allowed"] for row in records))
        self.assertTrue(all(row["blocked_reasons"] for row in records))
        self.assertEqual([row["username"] for row in records], ["synthetic-37", "synthetic-38"])

    def test_global_password_flag_must_be_boolean(self):
        for value in (None, 0, 1, "true", "false"):
            with self.subTest(value=value), self.assertRaises(MODULE.MappingError):
                mapped([user()], password_login_enabled=value)

    def test_unknown_second_factor_policy_cannot_allow_password_login(self):
        row = mapped([user()], [{"user_id": 37, "enabled": False, "policy_unknown": True}])["records"][0]
        self.assertTrue(row["second_factor_required"])
        self.assertFalse(row["password_login_allowed"])
        self.assertIn("second_factor_policy_unknown", row["blocked_reasons"])

    def test_duplicate_second_factor_rows_abort_instead_of_last_row_winning(self):
        for flags in ((True, False), (False, True), (True, True)):
            with self.subTest(flags=flags), self.assertRaises(MODULE.MappingError):
                mapped([user()], [{"user_id": 37, "enabled": enabled} for enabled in flags])

    def test_argon2_default_profile_preserves_verifier_without_legacy_bcrypt_flag(self):
        import base64

        salt = base64.b64encode(b"0123456789abcdef").decode().rstrip("=")
        output = base64.b64encode(b"x" * 32).decode().rstrip("=")
        verifier = f"$argon2id$v=19$m=19456,t=2,p=1${salt}${output}"
        row = mapped([user(password=verifier)])["records"][0]
        self.assertEqual(row["password_hash"], verifier)
        self.assertFalse(row["legacy_bcrypt_allowed"])
        self.assertTrue(row["password_login_allowed"])

    def test_argon2_unbounded_or_malformed_parameters_never_allow_login(self):
        import base64

        salt = base64.b64encode(b"0123456789abcdef").decode().rstrip("=")
        output = base64.b64encode(b"x" * 32).decode().rstrip("=")
        valid = f"$argon2id$v=19$m=19456,t=2,p=1${salt}${output}"
        variants = [
            valid.replace("m=19456", "m=999999999999"),
            valid.replace("m=19456", "m=65536"),
            valid.replace("t=2", "t=999999"),
            valid.replace("p=1", "p=32"),
            valid.replace("v=19", "v=16"),
            valid.replace(salt, "AA"),
            valid.replace(output, "AA"),
            valid + "=",
        ]
        for verifier in variants:
            with self.subTest(verifier_shape=len(verifier)):
                row = mapped([user(password=verifier)])["records"][0]
                self.assertIsNone(row["password_hash"])
                self.assertFalse(row["password_login_allowed"])

    def test_second_factor_must_reference_a_known_user_and_boolean_enabled(self):
        factors = [
            [{"user_id": 999, "enabled": True}],
            [{"enabled": True}],
            [{"user_id": 37}],
            [{"user_id": 37, "enabled": 1}],
            [{"user_id": 37, "enabled": "true"}],
            [{"user_id": True, "enabled": True}],
        ]
        for factor in factors:
            with self.subTest(factor=factor), self.assertRaises(MODULE.MappingError):
                mapped([user()], factor)

    def test_ids_are_strict_positive_integers(self):
        for identifier in (0, -1, True, False, None, "37", 37.0):
            with self.subTest(identifier=identifier), self.assertRaises(MODULE.MappingError):
                mapped([user(identifier)])

    def test_auth_revision_is_a_strict_positive_integer(self):
        for revision in (0, -1, True, False, None, "1", 1.0):
            with self.subTest(revision=revision), self.assertRaises(MODULE.MappingError):
                mapped([user(auth_version=revision)])

    def test_duplicate_ids_abort_mapping(self):
        with self.assertRaises(MODULE.MappingError):
            mapped([user(37), user(37, username="another-fixture")])

    def test_duplicate_usernames_abort_mapping(self):
        with self.assertRaises(MODULE.MappingError):
            mapped([user(37), user(38, username="synthetic-37")])

    def test_blank_or_overlong_username_aborts_mapping(self):
        for username in ("", " \t\n", "界" * 129, None):
            with self.subTest(shape=type(username).__name__), self.assertRaises(MODULE.MappingError):
                mapped([user(username=username)])

    def test_profile_limits_reject_data_instead_of_silently_truncating(self):
        for changes in ({"display_name": "x" * 257}, {"email": "x" * 321}, {"setting": "x" * 65537}):
            with self.subTest(field=next(iter(changes))), self.assertRaises(MODULE.MappingError):
                mapped([user(**changes)])

    def test_boundary_length_profiles_and_username_are_preserved(self):
        original = user(username="界" * 128, display_name="界" * 256, email="x" * 320, setting="x" * 65536)
        row = mapped([original])["records"][0]
        for field in ("username", "display_name", "email", "setting"):
            self.assertEqual(row[field], original[field])

    def test_digest_is_stable_under_input_permutation_and_bound_to_snapshot(self):
        rows = [user(91), user(7), user(300)]
        first = mapped(rows)
        reordered = mapped(list(reversed(rows)))
        different_source = mapped(rows, source_sha256="b" * 64)
        changed_profile = mapped([user(91), user(7, display_name="Different fixture"), user(300)])
        self.assertEqual(first["mapping_sha256"], reordered["mapping_sha256"])
        self.assertEqual(first["records"], reordered["records"])
        self.assertRegex(first["mapping_sha256"], r"^[0-9a-f]{64}$")
        self.assertNotEqual(first["mapping_sha256"], different_source["mapping_sha256"])
        self.assertNotEqual(first["mapping_sha256"], changed_profile["mapping_sha256"])

    def test_snapshot_digest_must_be_exact_lowercase_sha256(self):
        for digest in ("", "a" * 63, "a" * 65, "A" * 64, "g" * 64, None):
            with self.subTest(shape=type(digest).__name__), self.assertRaises(MODULE.MappingError):
                mapped([user()], source_sha256=digest)

    def test_source_records_are_not_mutated_even_when_a_later_row_fails(self):
        rows = [user(37, unknown_field={"nested": [1, 2, 3]}), user(38, username="")]
        before = deepcopy(rows)
        with self.assertRaises(MODULE.MappingError):
            mapped(rows)
        self.assertEqual(rows, before)
        valid = [user(37)]
        before = deepcopy(valid)
        mapped(valid)
        self.assertEqual(valid, before)

    def test_production_like_tokens_and_session_fields_never_enter_mapping(self):
        sentinel = "synthetic-unusable-token-marker"
        row = mapped([user(access_token=sentinel, sessions=[{"secret": sentinel}], unknown_future_field=sentinel)])["records"][0]
        self.assertNotIn(sentinel, json.dumps(row, ensure_ascii=False))
        self.assertNotIn("access_token", row)
        self.assertNotIn("sessions", row)

    def test_public_summary_has_no_profiles_verifiers_or_unknown_secrets(self):
        original = user(access_token="synthetic-summary-secret-marker")
        summary_text = json.dumps(mapped([original])["summary"], ensure_ascii=False)
        for sensitive in (original["username"], original["email"], original["password"], original["access_token"]):
            self.assertNotIn(sensitive, summary_text)

    def test_mapping_errors_do_not_echo_source_values(self):
        marker = "private-profile-marker-"
        original = user(username=marker * 10, password="private-verifier-marker")
        with self.assertRaises(MODULE.MappingError) as caught:
            mapped([original])
        self.assertNotIn(marker, str(caught.exception))
        self.assertNotIn(original["password"], str(caught.exception))


class DeveloperAccessTests(unittest.TestCase):
    def decision(self, **changes):
        record = mapped([user(**changes)])["records"][0]
        return record["developer_access_granted"], record["developer_access_reason"]

    def test_explicit_override_zero_denies_even_after_manual_activation_or_paid_hints(self):
        self.assertEqual(self.decision(trust_level_override=0, console_activated_at=100,
            quota=999999999, paid_total=999999999), (False, "explicit_override_deny"))

    def test_each_supported_nonzero_override_grants_without_manual_activation(self):
        for level in range(1, 5):
            with self.subTest(level=level):
                self.assertEqual(self.decision(trust_level_override=level), (True, "explicit_override_allow"))

    def test_invalid_override_never_falls_back_to_manual_activation(self):
        for value in (-1, 5, 100, True, False, "1", 1.0, {}, []):
            with self.subTest(type=type(value).__name__, value=value):
                self.assertEqual(self.decision(trust_level_override=value, console_activated_at=100),
                    (False, "invalid_override_fail_closed"))

    def test_null_override_with_positive_manual_activation_grants(self):
        self.assertEqual(self.decision(console_activated_at=100), (True, "approved_console_activation"))

    def test_invalid_manual_activation_is_denied(self):
        for value in (-1, 9223372036854775808, True, False, None, "100", 100.0, {}, []):
            with self.subTest(type=type(value).__name__, value=value):
                self.assertEqual(self.decision(console_activated_at=value), (False, "invalid_manual_activation_fail_closed"))

    def test_missing_policy_fields_do_not_prove_null_override_or_manual_activation(self):
        for field, reason in (("trust_level_override", "override_source_missing"),
                              ("console_activated_at", "manual_activation_source_missing")):
            source = user(console_activated_at=100)
            del source[field]
            record = mapped([source])["records"][0]
            self.assertFalse(record["developer_access_granted"])
            self.assertEqual(record["developer_access_reason"], reason)

    def test_wallet_or_generic_success_hints_never_prove_paid_access(self):
        self.assertEqual(self.decision(quota=999999999, used_quota=0, top_up_status="success",
            setting='{"developer_access_granted":true,"paid":true}'), (False, "paid_activation_unproven"))

    def test_unknown_role_does_not_gain_access_from_override_or_manual_activation(self):
        for role in (0, 2, 6, None, True, "10"):
            with self.subTest(role=role):
                self.assertEqual(self.decision(role=role, trust_level_override=4, console_activated_at=100),
                    (False, "unknown_role_fail_closed"))

    def test_privileged_grants_remain_dormant_and_do_not_unlock_password_login(self):
        for role in (10, 100):
            record = mapped([user(role=role, trust_level_override=0)])["records"][0]
            self.assertTrue(record["developer_access_granted"])
            self.assertEqual(record["developer_access_reason"], "admin_role_dormant")
            self.assertFalse(record["active"])
            self.assertFalse(record["password_login_allowed"])
            self.assertIn("privileged_permissions_unmapped", record["blocked_reasons"])

    def test_proven_access_does_not_bypass_disabled_mfa_or_password_policy(self):
        disabled = mapped([user(status=2, trust_level_override=1)])["records"][0]
        mfa = mapped([user(trust_level_override=1)], [{"user_id": 37, "enabled": True}])["records"][0]
        global_disabled = mapped([user(trust_level_override=1)], password_login_enabled=False)["records"][0]
        unsupported = mapped([user(trust_level_override=1, password="unsupported")])["records"][0]
        for record in (disabled, mfa, global_disabled, unsupported):
            self.assertTrue(record["developer_access_granted"])
            self.assertFalse(record["password_login_allowed"])
        self.assertFalse(disabled["active"])
        self.assertTrue(mfa["second_factor_required"])

    def test_provenance_binds_archive_and_reason_with_positive_initial_revision(self):
        for source in (user(), user(trust_level_override=0), user(trust_level_override=4),
                       user(console_activated_at=100), user(role=100)):
            record = mapped([source])["records"][0]
            reference = record["developer_access_source_reference"]
            self.assertEqual(reference, f"legacy:{SOURCE_SHA256}:{record['developer_access_reason']}")
            self.assertLessEqual(len(reference), 200)
            self.assertEqual(record["developer_access_revision"], 1)
            self.assertIs(type(record["developer_access_granted"]), bool)

    def test_summary_separates_developer_denial_from_auth_blocking_and_reports_paid_gap(self):
        plan = mapped([user(37), user(38, trust_level_override=0), user(39, console_activated_at=100)])
        self.assertEqual(plan["summary"]["developer_access_granted_count"], 1)
        self.assertEqual(plan["summary"]["developer_access_denied_count"], 2)
        self.assertEqual(plan["summary"]["developer_access_reason_counts"], {
            "approved_console_activation": 1, "explicit_override_deny": 1, "paid_activation_unproven": 1})
        self.assertEqual(plan["summary"]["blocked_user_count"], 0)
        self.assertIn("legacy_paid_activation_unproven", plan["summary"]["functional_gaps"])

    def test_mapping_digest_changes_when_entitlement_changes_even_with_same_account(self):
        original = mapped([user()])
        changed = mapped([user(trust_level_override=1)])
        self.assertNotEqual(original["mapping_sha256"], changed["mapping_sha256"])
        self.assertEqual(original["records"][0]["account_id"], changed["records"][0]["account_id"])


class IdentityContractTests(unittest.TestCase):
    def test_core26_assembly_uses_exact_concat_lf_and_telegram_alias(self):
        schemas = MODULE.component_schemas()
        self.assertEqual(len(schemas), 26)
        root = MODULE.ROOT / "services/core/schema"
        self.assertEqual(schemas["trust-pricing"], (root / "trust_pricing.sql").read_bytes()
                         + b"\n" + (root / "image_trust_extension.sql").read_bytes())
        self.assertEqual(schemas["telegram-auth"], (root / "telegram_auth.sql").read_bytes())
        self.assertEqual(hashlib.sha256(MODULE.installer_metadata()).hexdigest(),
                         MODULE.current_contract()["installer_contract_sha256"])

    def test_unreviewed_component_bytes_and_installer_metadata_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shutil.copytree(MODULE.ROOT / "services/core/src", root / "services/core/src")
            shutil.copytree(MODULE.ROOT / "services/core/schema", root / "services/core/schema")
            for name in ("trust_pricing.sql", "image_trust_extension.sql", "telegram_auth.sql",
                         "core_upgrade.sql", "component_upgrade.sql", "events.sql"):
                path = root / "services/core/schema" / name
                original = path.read_bytes()
                with self.subTest(name=name):
                    path.write_bytes(original + b"\n")
                    with self.assertRaises(MODULE.MappingError):
                        MODULE.current_contract(root)
                path.write_bytes(original)

    def test_duplicate_unknown_modified_and_comment_shadowed_constants_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shutil.copytree(MODULE.ROOT / "services/core/src", root / "services/core/src")
            shutil.copytree(MODULE.ROOT / "services/core/schema", root / "services/core/schema")
            mutations = (
                ("core_schema.rs", '("trust-pricing", crate::billing::trust_pricing::SCHEMA),', ""),
                ("core_schema.rs", '("trust-pricing", crate::billing::trust_pricing::SCHEMA),',
                 '("trust-pricing", crate::billing::trust_pricing::SCHEMA),\n("trust-pricing", crate::billing::trust_pricing::SCHEMA),'),
                ("core_schema.rs", "crate::billing::trust_pricing::SCHEMA", "crate::billing::unknown::SCHEMA"),
                ("billing/trust_pricing.rs", '"\\n",', '"\\n ",'),
                ("billing/trust_pricing.rs", '"\\n",', '"\\r\\n",'),
                ("identity/mod.rs", "pub(crate) use telegram::SCHEMA as TELEGRAM_AUTH_SCHEMA;",
                 "/* pub(crate) use telegram::SCHEMA as TELEGRAM_AUTH_SCHEMA; */"),
                ("identity/mod.rs", "pub(crate) use telegram::SCHEMA as TELEGRAM_AUTH_SCHEMA;",
                 "pub(crate) use missing::SCHEMA as TELEGRAM_AUTH_SCHEMA;"),
                ("identity/telegram.rs", 'include_str!("../../schema/telegram_auth.sql")',
                 'include_str!("../../schema/telegram_auth.sql ")'),
            )
            for name, before, after in mutations:
                path = root / "services/core/src" / name
                original = path.read_text()
                self.assertIn(before, original)
                with self.subTest(file=name, replacement=after):
                    path.write_text(original.replace(before, after))
                    with self.assertRaises(MODULE.MappingError):
                        MODULE.current_contract(root)
                path.write_text(original)

    def test_constant_normalization_preserves_literal_whitespace(self):
        expected = 'concat!(include_str!("../schema/a.sql"),"\\n",include_str!("../schema/b.sql"))'
        MODULE._reviewed_constant('const SCHEMA: &str = ' + expected + ';', "SCHEMA", expected)
        for bad in (expected.replace('"\\n"', '"\\n "'), expected.replace('a.sql', 'a .sql'),
                    expected + ';\nconst SCHEMA: &str = ' + expected):
            with self.subTest(expression=bad), self.assertRaises(MODULE.MappingError):
                MODULE._reviewed_constant('const SCHEMA: &str = ' + bad + ';', "SCHEMA", expected)

    def test_oauth_catalog_query_changes_invalidate_frozen_contract(self):
        contract = MODULE.current_contract()
        original = Path.read_text
        grants = MODULE.ROOT / "services/core/src/identity/grants.rs"
        def changed(path, *args, **kwargs):
            source = original(path, *args, **kwargs)
            return source.replace('SELECT d FROM definitions', 'SELECT d||chr(32) AS d FROM definitions') if path == grants else source
        with patch.object(Path, "read_text", changed):
            with self.assertRaisesRegex(MODULE.MappingError, "^oauth_catalog_contract_changed$"):
                MODULE.current_contract()
            with self.assertRaisesRegex(MODULE.MappingError, "^oauth_catalog_contract_changed$"):
                MODULE.oauth_seed_guard_sql(contract)

    def test_assembled_contract_includes_commerce_abuse_and_history(self):
        contract = MODULE.current_contract()
        self.assertEqual(set(contract["component_fingerprints"]), MODULE.CORE_COMPONENTS)
        for name in ("commerce", "abuse", "history"):
            self.assertEqual(contract["component_fingerprints"][name],
                hashlib.sha256((MODULE.ROOT / "services/core/schema" / (name + ".sql")).read_bytes()).hexdigest())

    def test_missing_or_replaced_component_is_rejected(self):
        source = (MODULE.ROOT / "services/core/src/core_schema.rs").read_text()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shutil.copytree(MODULE.ROOT / "services/core/src", root / "services/core/src")
            shutil.copytree(MODULE.ROOT / "services/core/schema", root / "services/core/schema")
            for name in ("identity", "commerce", "abuse", "history"):
                self.assertIn('(\"' + name + '\", include_str!', source)
            for component in ("abuse", "history"):
                original = f'("{component}", include_str!("../schema/{component}.sql")),'
                self.assertEqual(source.count(original), 1)
                for replacement in ("", f'("unknown", include_str!("../schema/{component}.sql")),'):
                    changed = source.replace(original, replacement)
                    (root / "services/core/src/core_schema.rs").write_text(changed)
                    with patch.object(MODULE, "ROOT", root):
                        with self.assertRaisesRegex(MODULE.MappingError, "^component_rust_source_fingerprint_mismatch$"):
                            MODULE.current_contract()

    def test_current_source_raw_bytes_and_rust_version_match_pinned_review(self):
        contract = MODULE.identity_contract()
        self.assertEqual(contract, {"identity_version": 5,
            "identity_fingerprint_sha256": hashlib.sha256(MODULE.IDENTITY_SCHEMA.read_bytes()).hexdigest()})
        self.assertEqual(contract["identity_fingerprint_sha256"], MODULE.IDENTITY_FINGERPRINT)

    def test_changed_raw_sql_is_rejected_even_when_caller_supplies_its_hash(self):
        with tempfile.TemporaryDirectory() as temporary:
            sql = Path(temporary) / "identity.sql"
            sql.write_bytes(MODULE.IDENTITY_SCHEMA.read_bytes() + b"\n")
            with patch.object(MODULE, "IDENTITY_SCHEMA", sql):
                with self.assertRaisesRegex(MODULE.MappingError, "^identity_source_fingerprint_mismatch$"):
                    MODULE.identity_contract()

    def test_old_future_missing_or_duplicate_source_versions_are_rejected(self):
        for contents in ("const VERSION: i32 = 2;", "const VERSION: i32 = 4;", "const VERSION: i32 = 6;", "",
                         "const VERSION: i32 = 5;\nconst VERSION: i32 = 5;"):
            with self.subTest(contents=contents), tempfile.TemporaryDirectory() as temporary:
                rust = Path(temporary) / "schema.rs"
                rust.write_text(contents)
                with patch.object(MODULE, "IDENTITY_SCHEMA_RUST", rust):
                    with self.assertRaisesRegex(MODULE.MappingError, "^identity_source_version_mismatch$"):
                        MODULE.identity_contract()


class ImportSqlTests(unittest.TestCase):
    def sql(self, rows=None, **changes):
        plan = mapped(rows if rows is not None else [user()])
        options = {
            "migration_id": "synthetic-identity-import",
            "source_sha256": SOURCE_SHA256,
            "target_contract": target_contract(),
            "manifest": manifest_for(plan),
            "target_identity": target_identity(),
        }
        if "manifest" in changes:
            changes["manifest"] = {**options["manifest"], **changes["manifest"]}
        options.update(changes)
        return MODULE.build_import_sql(plan, **options), plan

    def test_owner_login_and_missing_installer_are_rejected_before_sql(self):
        for login in (None, "synthetic_owner", ""):
            with self.subTest(login=login), self.assertRaisesRegex(MODULE.MappingError, "target_independent_installer_required"):
                self.sql(target_identity=target_identity(session_user=login))
        sql, _ = self.sql()
        for privilege in ("rolcanlogin", "rolsuper", "rolcreaterole", "rolcreatedb", "rolreplication", "rolbypassrls"):
            self.assertIn(privilege, sql)
        self.assertIn("pg_has_role(session_user,current_user,'MEMBER')", sql)

    def test_fresh26_trust_seed_all_fields_and_dynamic_snapshot_are_bound(self):
        contract = target_contract(allow_initialized_core_seed=True, initialized_core_seed_contract=MODULE.CORE_SEED_CONTRACT)
        for field, value in (("singleton", False), ("revision", 2), ("source_digest", "f" * 64),
                             ("source_cutoff", False), ("native_user_floor", 2), ("native_since", "2026-10-12")):
            changed = deepcopy(contract)
            changed["trust_policy_seed"][field] = value
            with self.subTest(field=field), self.assertRaises(MODULE.MappingError):
                self.sql(target_contract=changed)
        for mutate in (
            lambda spec: spec.update(paid_activation_enabled=False),
            lambda spec: spec.update(decay_period_days=89),
            lambda spec: spec["tiers"][2]["discount"].update(numerator=96),
            lambda spec: spec["tiers"][4].update(min_paid_credits=99999999),
            lambda spec: spec["admin_discount"].update(denominator=11),
            lambda spec: spec["superadmin_discount"].update(numerator=8),
            lambda spec: spec.update(extra_field=True),
        ):
            changed = deepcopy(contract)
            mutate(changed["trust_policy_seed"]["spec"])
            with self.assertRaisesRegex(MODULE.MappingError, "trust_seed_receipt_not_default"):
                self.sql(target_contract=changed)
        missing = deepcopy(contract)
        del missing["initialized_core_seed_sha256"]
        with self.assertRaisesRegex(MODULE.MappingError, "target_seed_hash_required"):
            self.sql(target_contract=missing)
        sql, _ = self.sql(target_contract=contract)
        self.assertIn("non-canonical initialized trust seed", sql)
        self.assertIn("initialized seed receipt mismatch", sql)
        self.assertIn("initialized seed receipt changed", sql)
        self.assertIn("seed_before->'trust_policy' IS DISTINCT FROM", sql)
        code = without_literals_and_comments(do_block(sql)[1])
        self.assertNotRegex(code, r"(?i)\b(?:INSERT\s+INTO|UPDATE|DELETE\s+FROM)\s+core_trust_pricing")
        self.assertNotIn("AND NOT (n.nspname='core_telegram_auth'", sql)
        self.assertNotIn("c.relname IN ('user_bases'", sql)

    def test_upgrade_receipts_all_sequences_and_end_catalog_are_checked(self):
        sql, _ = self.sql()
        for marker in ("fresh target has upgrade receipts", "fresh target upgrade receipts changed",
                       "non-canonical sequence definition", "non-canonical unused sequence state",
                       "non-import sequence changed", "identity sequence readback mismatch", "target catalog changed"):
            self.assertIn(marker, sql)
        self.assertIn("core_meta.schema_upgrades,core_meta.component_upgrades", sql)

    def test_sequence_rows_are_serialized_explicitly_in_seed_and_guards(self):
        seed_query = MODULE.fresh_seed_hash_query()
        self.assertIn("jsonb_build_object('last_value',t.last_value,'log_cnt',t.log_cnt,'is_called',t.is_called)", seed_query)
        self.assertNotIn("jsonb_agg(to_jsonb(t) ORDER BY last_value)", seed_query)
        sql, _ = self.sql()
        self.assertNotIn("SELECT to_jsonb(s) FROM core_billing.ledger_accounts_id_seq", sql)
        self.assertNotIn("SELECT to_jsonb(s) FROM %I.%I s", sql)
        self.assertEqual(sql.count("SELECT jsonb_build_object('last_value',s.last_value,'log_cnt',s.log_cnt,'is_called',s.is_called) FROM core_billing.ledger_accounts_id_seq"), 2)
        self.assertEqual(sql.count("SELECT jsonb_build_object(''last_value'',s.last_value,''log_cnt'',s.log_cnt,''is_called'',s.is_called) FROM %I.%I s"), 2)
        # Data and definition defenses remain on both sides of the import.
        for marker in ("non-canonical sequence definition", "non-canonical unused sequence state",
                       "non-import sequence changed", "identity sequence readback mismatch", "target catalog changed"):
            self.assertIn(marker, sql)

    def test_seed_capture_and_import_share_explicit_utc_contract(self):
        sql, _ = self.sql()
        self.assertIn("SET LOCAL timezone='UTC';", do_block(sql)[0])
        seed_query, trust_query = MODULE.fresh_seed_hash_query(), MODULE.trust_policy_seed_query()
        self.assertIn("t.created_at AT TIME ZONE 'UTC'", seed_query)
        self.assertIn("t.native_since AT TIME ZONE 'UTC'", seed_query)
        self.assertIn("p.native_since AT TIME ZONE 'UTC'", trust_query)
        for query in (seed_query, trust_query):
            self.assertIn("#>> '{}') || '+00:00'", query)
            self.assertNotIn("set_config", query)
            self.assertNotIn("date_trunc", query)

    def test_old_receipt_cannot_be_reused_by_only_changing_identity_version(self):
        plan = mapped([user()])
        old = manifest_for(plan)
        for field in ("import_tool_sha256", "import_contract_sha256"):
            old.pop(field)
        with self.assertRaisesRegex(MODULE.MappingError, "identity_manifest_mismatch"):
            MODULE.validate_import_material(plan, SOURCE_SHA256, old)
        for field in ("import_tool_sha256", "import_contract_sha256"):
            with self.subTest(field=field), self.assertRaisesRegex(MODULE.MappingError, "identity_manifest_mismatch"):
                MODULE.validate_import_material(plan, SOURCE_SHA256, manifest_for(plan, **{field: "f" * 64}))

    def test_quotes_backslashes_controls_and_unicode_roundtrip_as_data(self):
        original = user(username="模型 O'Reilly\\fixture", setting="quote ' \\ newline\n tab\t 雪")
        sql, plan = self.sql([original])
        before, body, after, delimiter = do_block(sql)
        payload, tail = single_quoted_literal(body, "DECLARE payload JSONB :=")
        self.assertEqual(json.loads(payload), plan["records"])
        self.assertTrue(tail.startswith("::jsonb;"))
        self.assertIn("standard_conforming_strings=on", before)
        self.assertNotIn(delimiter, body)
        self.assertTrue(after.rstrip().endswith("COMMIT;"))

    def test_source_dollar_delimiter_cannot_terminate_do_block(self):
        injection = "$lmm_identity_import$; DROP SCHEMA core_billing CASCADE; --"
        sql, plan = self.sql([user(username=injection, setting="END $lmm_identity_import$;")])
        before, body, after, delimiter = do_block(sql)
        payload, _ = single_quoted_literal(body, "DECLARE payload JSONB :=")
        self.assertEqual(json.loads(payload), plan["records"])
        self.assertIn(injection, payload)
        self.assertNotIn("DROP SCHEMA", without_literals_and_comments(body))
        self.assertNotIn("DROP SCHEMA", after)
        self.assertNotEqual(delimiter, "$lmm_identity_import$")
        self.assertEqual(sql.count(delimiter), 2)

    def test_manifest_quotes_and_dollar_tags_cannot_inject_statements(self):
        hostile = {"synthetic": "'); COMMIT; $lmm_identity_import$ DROP SCHEMA core_identity CASCADE; --"}
        sql, _ = self.sql(manifest=hostile)
        before, body, after, delimiter = do_block(sql)
        self.assertNotIn("DROP SCHEMA", without_literals_and_comments(body))
        self.assertNotIn("DROP SCHEMA", after)
        self.assertEqual(sql.count(delimiter), 2)
        self.assertTrue(before.startswith("BEGIN ISOLATION LEVEL SERIALIZABLE;"))

    def test_mutation_scope_is_identity_policy_and_unique_import_receipt_only(self):
        sql, _ = self.sql()
        _, body, _, _ = do_block(sql)
        code = without_literals_and_comments(body)
        tables = set(re.findall(r"\bINSERT\s+INTO\s+([a-z_]+\.[a-z_]+)", code, re.IGNORECASE))
        self.assertEqual(tables, {
            "core_identity.accounts", "core_identity.users", "core_identity.user_profiles",
            "core_identity.password_logins", "core_identity.developer_access",
            "core_identity.privileged_import_facts",
            "core_billing.account_policies", "core_migration.import_runs",
        })
        self.assertNotRegex(code, r"(?i)\b(?:TRUNCATE|DELETE\s+FROM|UPDATE\s+[a-z_]+\.|DROP|CREATE)\b")
        self.assertNotRegex(code, r"(?i)\bINSERT\s+INTO\s+core_identity\.(?:credentials|web_sessions|sessions|oauth_links|passkeys)\b")

    def test_transaction_locks_and_readback_precede_commit_without_setval(self):
        sql, _ = self.sql()
        before, body, after, _ = do_block(sql)
        self.assertIn("synchronous_commit=on", before)
        self.assertIn("pg_advisory_xact_lock", before)
        code = without_literals_and_comments(body)
        self.assertLess(code.index("FOR table_row"), code.index("INSERT INTO core_identity.accounts"))
        self.assertLess(code.index("SELECT * INTO previous"), code.index("INSERT INTO core_identity.accounts"))
        self.assertIn("IS DISTINCT FROM", code)
        self.assertNotRegex(code, r"(?i)\bEXCEPTION\s+WHEN\b")
        self.assertNotRegex(sql, r"(?i)\bsetval\s*\(")
        self.assertEqual(len(re.findall(r"ALTER SEQUENCE %s RESTART WITH %s", sql)), 2)
        self.assertTrue(after.rstrip().endswith("COMMIT;"))

    def test_invalid_migration_metadata_is_rejected_before_sql_is_returned(self):
        variants = [
            {"migration_id": "injection'; DROP TABLE users; --"},
            {"migration_id": ""},
            {"source_sha256": "a" * 63},
            {"target_contract": {"identity_version": 1, "identity_fingerprint_sha256": "b" * 64}},
            {"target_contract": {"identity_version": 2, "identity_fingerprint_sha256": "invalid"}},
            {"target_contract": target_contract(identity_version=2)},
            {"target_contract": target_contract(identity_version="4")},
            {"target_contract": target_contract(identity_version=4.0)},
            {"target_contract": target_contract(identity_fingerprint_sha256="b" * 64)},
        ]
        for changes in variants:
            with self.subTest(fields=list(changes)), self.assertRaises(MODULE.MappingError):
                self.sql(**changes)

    def test_core_seed_exception_requires_literal_boolean_opt_in(self):
        base = target_contract()
        for value in (False, None, 0, 1, "true"):
            with self.subTest(value=value), self.assertRaisesRegex(MODULE.MappingError, "target_fresh26_seed_contract_required"):
                self.sql(target_contract={**base, "allow_initialized_core_seed": value})
        del base["allow_initialized_core_seed"]
        with self.assertRaisesRegex(MODULE.MappingError, "target_fresh26_seed_contract_required"):
            self.sql(target_contract=base)

    def test_core_seed_opt_in_requires_the_explicit_reviewed_seed_contract(self):
        for seed in (None, "", True, "ledger-zero-events-empty-routing-unset-v0", "ledger-zero-events-empty-routing-unset-v1", "ledger-zero-events-empty-routing-unset-abuse-default-v2"):
            with self.subTest(seed=seed), self.assertRaisesRegex(MODULE.MappingError, "^target_seed_contract_invalid$"):
                self.sql(target_contract=target_contract(allow_initialized_core_seed=True,
                    initialized_core_seed_contract=seed))

    def test_version_five_and_exact_fingerprint_are_bound_to_sql(self):
        sql, _ = self.sql()
        self.assertIn("singleton AND version=5 AND encode(fingerprint,'hex')='" + MODULE.IDENTITY_FINGERPRINT + "'", sql)
        self.assertNotIn("version=2", sql)

    def test_developer_rows_are_atomic_counted_and_exactly_read_back(self):
        sql, plan = self.sql([user(trust_level_override=0, console_activated_at=100)])
        _, body, _, _ = do_block(sql)
        payload, _ = single_quoted_literal(body, "DECLARE payload JSONB :=")
        self.assertEqual(json.loads(payload)[0]["developer_access_granted"], False)
        self.assertEqual(json.loads(payload)[0]["developer_access_source_reference"],
                         plan["records"][0]["developer_access_source_reference"])
        self.assertIn("INSERT INTO core_identity.developer_access(user_id,granted,source_reference,revision)", body)
        self.assertIn("(SELECT count(*) FROM core_identity.developer_access)<>expected_count", body)
        self.assertIn("LEFT JOIN core_identity.developer_access d ON d.user_id=u.id", body)
        self.assertIn("d.granted,d.source_reference,d.revision) IS DISTINCT FROM", body)
        self.assertLess(body.index("INSERT INTO core_identity.developer_access"), body.index("INSERT INTO core_migration.import_runs"))
        self.assertLess(body.index("d.granted,d.source_reference,d.revision) IS DISTINCT FROM"), body.index("non-import target table is not empty"))
        self.assertIn("'password_logins','developer_access'", body)

    def test_opted_in_core_seed_is_validated_and_read_back_without_writes(self):
        contract = target_contract(allow_initialized_core_seed=True, initialized_core_seed_contract=MODULE.CORE_SEED_CONTRACT)
        sql, _ = self.sql(target_contract=contract)
        _, body, _, _ = do_block(sql)
        code = without_literals_and_comments(body)
        self.assertLess(body.index("non-canonical initialized ledger seed"), body.index("INSERT INTO core_identity.accounts"))
        self.assertLess(body.index("non-canonical initialized core seed"), body.index("INSERT INTO core_identity.accounts"))
        self.assertGreater(body.index("initialized ledger seed changed"), body.index("INSERT INTO core_migration.import_runs"))
        self.assertGreater(body.index("initialized core seed changed"), body.index("INSERT INTO core_migration.import_runs"))
        self.assertIn("balance_units<>0 OR b.revision<>0", body)
        self.assertIn("pending_events=0 AND pending_bytes=0", body)
        self.assertIn("singleton AND config_version IS NULL", body)
        self.assertIn("singleton AND revision=0 AND document='{\"version\":1,\"policies\":[]}'::jsonb", body)
        self.assertIn("seed_before->'abuse_settings' IS DISTINCT FROM", body)
        self.assertIn("seed_before->'oauth_contract' IS DISTINCT FROM", body)
        for table, field in (("core_humangrant.human_grant_gate", "human_grant_gate"),
                             ("core_gift.risk_key", "gift_risk_key"),
                             ("core_gift.policy", "gift_policy"),
                             ("core_tool_market.credit_policy", "credit_policy")):
            self.assertIn("(SELECT count(*) FROM " + table + ")<>1", body)
            self.assertIn("seed_before->'" + field + "' IS DISTINCT FROM", body)
            self.assertNotRegex(code, r"(?i)\b(?:INSERT\s+INTO|UPDATE|DELETE\s+FROM)\s+" + re.escape(table) + r"\b")
        self.assertIn("octet_length(secret)=32", body)
        self.assertIn("id=1 AND revision=1", body)
        self.assertIn("NOT enabled AND exclusive_cap=0", body)
        self.assertIn("platform_account_id IS NULL AND platform_user_id IS NULL AND NOT enabled", body)
        self.assertIn("FROM core_billing.ledger_accounts_id_seq WHERE last_value=2 AND is_called", body)
        self.assertIn("non-canonical initialized OAuth seed", body)
        self.assertIn("encode(ddl_fingerprint,'hex')='" + contract['component_fingerprints']['oauth-grants'] + "'", body)
        self.assertIn("jsonb_agg(to_jsonb(a) ORDER BY id)", body)
        self.assertIn("jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id)", body)
        self.assertIn("to_jsonb(c) FROM core_events.capacity", body)
        self.assertIn("to_jsonb(r) FROM public.routing_active_release", body)
        self.assertNotRegex(code, r"(?i)\bINSERT\s+INTO\s+(?:core_billing\.(?:ledger_accounts|balance_state)|core_events\.capacity|public\.routing_active_release)\b")

    def test_ledger_only_seed_permission_does_not_allow_event_or_routing_seed(self):
        contract = target_contract(allow_initialized_zero_ledger_seed=True)
        with self.assertRaisesRegex(MODULE.MappingError, "target_fresh26_seed_contract_required"):
            self.sql(target_contract=contract)

    def test_target_emptiness_scope_includes_public_and_other_application_schemas(self):
        sql, _ = self.sql()
        _, body, _, _ = do_block(sql)
        scope = "n.nspname NOT IN ('pg_catalog','information_schema','core_meta','core_migration')"
        self.assertEqual(body.count(scope), 3)
        self.assertNotIn("n.nspname LIKE 'core", body)
        self.assertNotIn("n.nspname IN ('core", body)

    def test_mapping_payload_hash_must_be_recomputed(self):
        plan = mapped([user()])
        manifest = manifest_for(plan)
        plan["records"][0]["email"] = "other@example.invalid"
        with self.assertRaisesRegex(MODULE.MappingError, "^mapping_hash_mismatch$"):
            MODULE.build_import_sql(plan, migration_id="synthetic", source_sha256=SOURCE_SHA256,
                target_contract=target_contract(), manifest=manifest, target_identity=target_identity())

    def test_v3_manifest_is_rejected_without_probing_target(self):
        old = {"identity_version": 3,
               "identity_fingerprint_sha256": "9f3c43d55fe0115920ff16018343f17fc59f20746ba097732accbd465c415f64"}
        with patch.object(MODULE, "psql") as runner:
            with self.assertRaisesRegex(MODULE.MappingError, "^identity_manifest_mismatch$"):
                self.sql(manifest={"identity_contract": old})
            runner.assert_not_called()

    def test_manifest_map_source_and_proof_are_bound(self):
        for field, value in (("source_target_id_map_sha256", "f" * 64), ("mapping_sha256", "f" * 64),
                             ("source_archive_sha256", "f" * 64), ("format_version", 1),
                             ("summary", {}), ("source_baseline_proof_sha256", "invalid")):
            with self.subTest(field=field), self.assertRaises(MODULE.MappingError):
                self.sql(manifest={field: value})

    def test_in_transaction_instance_and_catalog_are_bound(self):
        sql, _ = self.sql()
        self.assertIn("current_database()<>'synthetic_database'", sql)
        self.assertIn("target instance identity mismatch", sql)
        self.assertIn("target has other database clients", sql)
        self.assertIn("current_setting('listen_addresses') NOT IN ('','localhost','127.0.0.1')", sql)
        self.assertIn("installed core contract or actual catalog mismatch", sql)
        self.assertIn("int8send(octet_length(convert_to(definition,'UTF8'))", sql)
        self.assertIn("LOCK TABLE core_meta.schema_contract", sql)

    def test_postgres_json_oid_string_is_normalized_without_mutating_receipt(self):
        identity = target_identity(database_oid="12345")
        sql, _ = self.sql(target_identity=identity)
        self.assertIn("datname=current_database())<>12345", sql)
        self.assertEqual(identity["database_oid"], "12345")

    def test_missing_instance_and_assembled_contract_are_rejected(self):
        for changes in ({"target_identity": {}}, {"target_identity": target_identity(database_oid=True)},
                        {"target_identity": target_identity(database_owner="other")},
                        {"target_contract": target_contract(component_fingerprints={})},
                        {"target_contract": target_contract(catalog_query_sha256="f" * 64)},
                        {"target_contract": target_contract(catalog_fingerprint_sha256="invalid")}):
            with self.subTest(fields=list(changes)), self.assertRaises(MODULE.MappingError):
                self.sql(**changes)


class InstanceGuardTests(unittest.TestCase):
    def fixture(self, *, source=False):
        role = "synthetic_reader" if source else "synthetic_owner"
        connection = {"database": "synthetic_database", "role": role, "port": 55432}
        identity = {
            "database": connection["database"], "role": role, "port": connection["port"],
            "database_oid": 12345, "database_owner": "synthetic_owner",
            "server_version_num": "180006", "postmaster_started_at": "2026-10-12 00:00:00+00",
            "tcp_connection": False, "read_only": "on",
        }
        expected = {
            "database_oid": 12345, "database_owner": "synthetic_owner",
            "server_version_num": 180006, "postmaster_started_at": "2026-10-12T08:00:00+08:00",
            "listen_addresses": "",
        }
        return identity, connection, expected

    def test_source_accepts_equivalent_timestamp_formats_and_reader_role(self):
        identity, connection, expected = self.fixture(source=True)
        MODULE.verify_instance(identity, connection, expected, source=True)
        self.assertNotEqual(identity["database_owner"], connection["role"])

    def test_legacy_postgres_start_time_receipt_key_is_supported(self):
        identity, connection, expected = self.fixture(source=True)
        expected["postgres_start_time"] = expected.pop("postmaster_started_at")
        MODULE.verify_instance(identity, connection, expected, source=True)

    def test_target_accepts_offline_or_loopback_listener_only_with_unix_connection(self):
        for listener in ("", "127.0.0.1"):
            with self.subTest(listener=listener):
                identity, connection, expected = self.fixture()
                expected["listen_addresses"] = listener
                MODULE.verify_instance(identity, connection, expected, source=False)

    def test_actual_tcp_connection_is_rejected_for_both_source_and_target(self):
        for source in (False, True):
            with self.subTest(source=source):
                identity, connection, expected = self.fixture(source=source)
                identity["tcp_connection"] = True
                with self.assertRaisesRegex(MODULE.MappingError, "^database_not_offline$"):
                    MODULE.verify_instance(identity, connection, expected, source=source)

    def test_unproven_or_nonlocal_listeners_are_rejected(self):
        for source, listener in ((True, "127.0.0.1"), (True, None), (False, "*"), (False, "0.0.0.0"), (False, "127.0.0.1,::1")):
            with self.subTest(source=source, listener=listener):
                identity, connection, expected = self.fixture(source=source)
                expected["listen_addresses"] = listener
                with self.assertRaises(MODULE.MappingError):
                    MODULE.verify_instance(identity, connection, expected, source=source)

    def test_source_must_actually_be_readonly(self):
        identity, connection, expected = self.fixture(source=True)
        identity["read_only"] = "off"
        with self.assertRaisesRegex(MODULE.MappingError, "^source_not_readonly$"):
            MODULE.verify_instance(identity, connection, expected, source=True)

    def test_target_database_owner_must_match_receipt_and_connection_role(self):
        for actual, expected_owner in (("other_owner", "synthetic_owner"), ("synthetic_owner", "other_owner"), ("other_owner", "other_owner"), ("synthetic_owner", None)):
            with self.subTest(actual=actual, expected=expected_owner):
                identity, connection, expected = self.fixture()
                identity["database_owner"], expected["database_owner"] = actual, expected_owner
                with self.assertRaisesRegex(MODULE.MappingError, "^target_owner_mismatch$"):
                    MODULE.verify_instance(identity, connection, expected, source=False)

    def test_changed_instance_or_connection_identity_is_rejected(self):
        changes = {
            "database_oid": 54321, "server_version_num": "180005", "postmaster_started_at": "2026-10-12T00:00:01+00:00",
            "database": "other_database", "role": "other_role", "port": 55433,
        }
        for field, value in changes.items():
            with self.subTest(field=field):
                identity, connection, expected = self.fixture()
                identity[field] = value
                with self.assertRaises(MODULE.MappingError):
                    MODULE.verify_instance(identity, connection, expected, source=False)

    def test_matching_target_nonce_keeps_owner_and_instance_guards_required(self):
        identity, connection, expected = self.fixture()
        identity["server_instance_nonce"] = expected["server_instance_nonce"] = "synthetic-instance-marker"
        MODULE.verify_instance(identity, connection, expected, source=False)
        for changes in ({"database_owner": "other_owner"}, {"database_oid": 54321}):
            with self.subTest(field=next(iter(changes))), self.assertRaises(MODULE.MappingError):
                MODULE.verify_instance({**identity, **changes}, connection, expected, source=False)

    def test_expected_target_nonce_rejects_mismatch_empty_or_missing_actual_marker(self):
        for actual_nonce in ("other-instance-marker", "", None):
            with self.subTest(marker_missing=actual_nonce is None):
                identity, connection, expected = self.fixture()
                expected["server_instance_nonce"] = "synthetic-instance-marker"
                if actual_nonce is not None:
                    identity["server_instance_nonce"] = actual_nonce
                with self.assertRaisesRegex(MODULE.MappingError, "^target_instance_nonce_mismatch$"):
                    MODULE.verify_instance(identity, connection, expected, source=False)


class SourceExportGuardTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.backups = Path(self.temporary.name)
        self.snapshot = self.backups / "synthetic-snapshot"
        self.snapshot.mkdir(mode=0o700)
        self.archive = self.snapshot / "source.dump"
        self.archive.write_bytes(b"synthetic immutable archive")
        self.archive.chmod(0o600)
        self.baseline = self.snapshot / "native-table-row-count-baseline.jsonl"
        self.baseline.write_text(json.dumps({"table": "synthetic_legacy.users", "row_count": 1}) + "\n")
        self.baseline.chmod(0o600)
        identity, connection, expected = InstanceGuardTests().fixture(source=True)
        self.identity = identity
        expected["postgres_start_time"] = expected.pop("postmaster_started_at")
        expected["schema_owner"] = "synthetic_schema_owner"
        self.receipt = {
            "status": "snapshot-restored-baseline-verified-frozen-and-classified",
            "archive": {"path": str(self.archive), "sha256": hashlib.sha256(self.archive.read_bytes()).hexdigest()},
            "source": {"schema": "synthetic_legacy"}, "source_instance_identity": expected,
            "private_metadata_files": [str(self.baseline)],
            "readonly_reader": {**connection, "verified_no_application_write_privileges": True,
                "verified_no_security_definer_application_function_execute": True, "row_security_off_guard_verified": True},
        }
        self.output_number = 0
        self.backup_patch = patch.object(MODULE, "BACKUP_ROOT", self.backups)
        self.backup_patch.start()

    def tearDown(self):
        self.backup_patch.stop()
        self.temporary.cleanup()

    def rows(self):
        return [
            {"kind": "identity", "row": self.identity},
            {"kind": "schema_identity", "row": {"schema": "synthetic_legacy", "schema_owner": "synthetic_schema_owner"}},
            {"kind": "user_count", "row": 1},
            {"kind": "user", "row": user()},
        ]

    def export(self, rows):
        self.output_number += 1
        output = self.snapshot / f"preview-{self.output_number}"
        output.mkdir(mode=0o700)
        raw = "".join(json.dumps(row) + "\n" for row in rows)
        with patch.object(MODULE, "psql", side_effect=[json.dumps(self.identity), raw]) as runner:
            result = MODULE.export_source(self.receipt, output)
        self.assertTrue(all(call.kwargs["readonly"] for call in runner.call_args_list))
        return result, output

    def test_complete_export_preserves_original_user_fields_in_private_artifact(self):
        rows = self.rows()
        rows[-1]["row"]["unknown_field"] = {"synthetic_secret": "unusable-private-marker"}
        result, output = self.export(rows)
        self.assertEqual(result[1][0], rows[-1]["row"])
        self.assertTrue(result[3])
        original = output / "source-identity-export.private.jsonl"
        self.assertIn("unusable-private-marker", original.read_text())
        self.assertEqual(stat.S_IMODE(original.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o700)

    def test_missing_duplicate_or_inconsistent_count_markers_abort_export(self):
        cases = []
        baseline = self.rows()
        cases.extend([[row for row in baseline if row["kind"] != marker] for marker in ("identity", "schema_identity", "user_count")])
        cases.append(baseline + [baseline[0]])
        cases.append(baseline + [baseline[2]])
        cases.append([*baseline[:2], {"kind": "user_count", "row": 2}, baseline[3]])
        for rows in cases:
            with self.subTest(kinds=[row["kind"] for row in rows]), self.assertRaises(MODULE.MappingError):
                self.export(rows)

    def test_schema_owner_or_same_transaction_instance_drift_aborts_export(self):
        rows = self.rows()
        rows[1]["row"]["schema_owner"] = "other_owner"
        with self.assertRaisesRegex(MODULE.MappingError, "^source_schema_owner_mismatch$"):
            self.export(rows)
        rows = self.rows()
        rows[0]["row"] = {**self.identity, "read_only": "off"}
        with self.assertRaises(MODULE.MappingError):
            self.export(rows)

    def test_password_policy_unknown_is_disabled_and_deleted_mfa_is_ignored(self):
        rows = self.rows() + [
            {"kind": "password_policy", "row": {"key": "PasswordLoginEnabled", "value": None}},
            {"kind": "second_factor", "row": {"user_id": 37, "is_enabled": True, "deleted_at": "2020-01-01T00:00:00Z"}},
            {"kind": "second_factor", "row": {"user_id": 37, "is_enabled": None, "deleted_at": None}},
        ]
        result, _ = self.export(rows)
        self.assertFalse(result[3])
        self.assertEqual(result[2], [{"user_id": 37, "enabled": True, "policy_unknown": True}])

    def test_archive_hash_or_reader_attestation_failure_prevents_database_query(self):
        for failure in ("hash", "reader"):
            with self.subTest(failure=failure):
                receipt = deepcopy(self.receipt)
                if failure == "hash":
                    receipt["archive"]["sha256"] = "f" * 64
                else:
                    receipt["readonly_reader"]["verified_no_application_write_privileges"] = False
                with patch.object(MODULE, "psql") as runner, self.assertRaises(MODULE.MappingError):
                    MODULE.export_source(receipt, self.snapshot)
                runner.assert_not_called()

    def test_snapshot_baseline_mismatch_rejects_even_self_consistent_empty_export(self):
        rows = [row for row in self.rows() if row["kind"] != "user"]
        next(row for row in rows if row["kind"] == "user_count")["row"] = 0
        with self.assertRaisesRegex(MODULE.MappingError, "^source_snapshot_baseline_mismatch$"):
            self.export(rows)
        self.baseline.write_text(json.dumps({"table": "users", "row_count": 2}) + "\n")
        with self.assertRaisesRegex(MODULE.MappingError, "^source_snapshot_baseline_mismatch$"):
            self.export(self.rows())

    def test_missing_snapshot_row_baseline_rejects_before_database_query(self):
        self.receipt["private_metadata_files"] = []
        with patch.object(MODULE, "psql") as runner, self.assertRaisesRegex(MODULE.MappingError, "^source_row_baseline_missing$"):
            MODULE.export_source(self.receipt, self.snapshot)
        runner.assert_not_called()

    def test_duplicate_snapshot_baseline_paths_or_user_rows_are_rejected(self):
        self.receipt["private_metadata_files"] = [str(self.baseline)] * 2
        with patch.object(MODULE, "psql") as runner, self.assertRaisesRegex(MODULE.MappingError, "^source_row_baseline_missing$"):
            MODULE.export_source(self.receipt, self.snapshot)
        runner.assert_not_called()
        self.receipt["private_metadata_files"] = [str(self.baseline)]
        self.baseline.write_text("\n".join(json.dumps({"table": table, "row_count": 1}) for table in ("users", "synthetic_legacy.users")) + "\n")
        with patch.object(MODULE, "psql") as runner, self.assertRaisesRegex(MODULE.MappingError, "^source_row_baseline_invalid$"):
            MODULE.export_source(self.receipt, self.snapshot)
        runner.assert_not_called()

    def test_snapshot_baseline_count_requires_nonnegative_integer_not_bool(self):
        for count in (True, False, None, "1", 1.0, -1):
            with self.subTest(type=type(count).__name__):
                self.baseline.write_text(json.dumps({"table": "users", "row_count": count}) + "\n")
                with patch.object(MODULE, "psql") as runner, self.assertRaisesRegex(MODULE.MappingError, "^source_row_baseline_invalid$"):
                    MODULE.export_source(self.receipt, self.snapshot)
                runner.assert_not_called()

    def test_mapping_failure_keeps_source_private_and_prints_only_safe_error(self):
        marker = "synthetic-private-profile-marker-"
        rows = self.rows()
        rows[-1]["row"]["username"] = marker * 10
        raw = "".join(json.dumps(row) + "\n" for row in rows)
        receipt_path = self.snapshot / "source-receipt.json"
        receipt_path.write_text(json.dumps(self.receipt))
        receipt_path.chmod(0o600)
        output = self.snapshot / "failed-preview"
        argv = [str(SCRIPT), "--source-receipt", str(receipt_path), "--output", str(output), "--migration-id", "synthetic-failure"]
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(sys, "argv", argv), patch.object(MODULE, "psql", side_effect=[json.dumps(self.identity), raw]), contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = MODULE.main()
        self.assertEqual(result, 1)
        self.assertEqual(stdout.getvalue(), "")
        self.assertEqual(json.loads(stderr.getvalue()), {"status": "aborted", "error_code": "username_invalid"})
        self.assertNotIn(marker, stderr.getvalue())
        self.assertIn(marker, (output / "source-identity-export.private.jsonl").read_text())
        self.assertTrue(all(stat.S_IMODE(path.stat().st_mode) == 0o600 for path in output.iterdir()))

    def test_source_only_dry_run_binds_v3_policy_and_never_prints_private_fields(self):
        rows = self.rows()
        rows[-1]["row"].update(trust_level_override=1, username="private-synthetic-user-marker")
        raw = "".join(json.dumps(row) + "\n" for row in rows)
        receipt_path = self.snapshot / "source-receipt.json"
        receipt_path.write_text(json.dumps(self.receipt))
        receipt_path.chmod(0o600)
        output = self.snapshot / "successful-preview"
        argv = [str(SCRIPT), "--source-receipt", str(receipt_path), "--output", str(output),
                "--migration-id", "synthetic-v3-preview"]
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(sys, "argv", argv), patch.object(MODULE, "psql", side_effect=[json.dumps(self.identity), raw]) as runner, contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = MODULE.main()
        self.assertEqual(result, 0)
        self.assertEqual(stderr.getvalue(), "")
        self.assertTrue(all(call.kwargs["readonly"] for call in runner.call_args_list))
        public = json.loads(stdout.getvalue())
        self.assertFalse(public["apply"])
        self.assertEqual(public["developer_access_granted_count"], 1)
        self.assertEqual(public["identity_contract"], MODULE.identity_contract())
        for private_value in (rows[-1]["row"][field] for field in ("username", "password", "email", "setting")):
            self.assertNotIn(private_value, stdout.getvalue())
        manifest = json.loads((output / "manifest.private.json").read_text())
        self.assertEqual(manifest["format_version"], 2)
        self.assertEqual(manifest["scope"], "identity-only")
        self.assertEqual(manifest["identity_contract"], MODULE.identity_contract())
        self.assertEqual(manifest["developer_access_policy"], MODULE.DEVELOPER_POLICY)
        self.assertEqual(manifest["initialized_core_seed_contract"], MODULE.CORE_SEED_CONTRACT)
        self.assertEqual(manifest["source_export_sha256"], MODULE.sha256_file(output / "source-identity-export.private.jsonl"))
        self.assertFalse((output / "identity-import.private.sql").exists())
        self.assertTrue(all(stat.S_IMODE(path.stat().st_mode) == 0o600 for path in output.iterdir()))

    def test_old_target_contract_fails_before_any_target_probe(self):
        receipt_path = self.snapshot / "source-receipt.json"
        receipt_path.write_text(json.dumps(self.receipt))
        receipt_path.chmod(0o600)
        target_path = self.snapshot / "target-receipt.json"
        target_path.write_text(json.dumps({
            "status": "initialized-empty-identity-target", "registry_provisioned": True,
            "application_or_worker_started": False, "migration_id": "synthetic-stale-target",
            "source_archive_sha256": self.receipt["archive"]["sha256"],
            "instance_identity": {"server_instance_nonce": "synthetic-auxiliary-nonce"},
            "contract": target_contract(identity_version=2),
        }))
        target_path.chmod(0o600)
        raw = "".join(json.dumps(row) + "\n" for row in self.rows())
        argv = [str(SCRIPT), "--source-receipt", str(receipt_path), "--target-receipt", str(target_path),
                "--output", str(self.snapshot / "stale-target-preview"), "--migration-id", "synthetic-stale-target"]
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(sys, "argv", argv), patch.object(MODULE, "psql", side_effect=[json.dumps(self.identity), raw]) as runner, contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = MODULE.main()
        self.assertEqual(result, 1)
        self.assertEqual(json.loads(stderr.getvalue())["error_code"], "target_contract_invalid")
        self.assertEqual(runner.call_count, 2)
        self.assertTrue(all(call.kwargs["readonly"] for call in runner.call_args_list))
        self.assertEqual(stdout.getvalue(), "")

    def test_v3_target_contract_fails_before_any_target_probe(self):
        receipt_path = self.snapshot / "source-receipt.json"
        receipt_path.write_text(json.dumps(self.receipt))
        receipt_path.chmod(0o600)
        target_path = self.snapshot / "target-receipt.json"
        target_path.write_text(json.dumps({
            "status": "initialized-empty-identity-target", "registry_provisioned": True,
            "application_or_worker_started": False, "migration_id": "synthetic-stale-target",
            "source_archive_sha256": self.receipt["archive"]["sha256"],
            "instance_identity": {"server_instance_nonce": "synthetic-auxiliary-nonce"},
            "contract": target_contract(identity_version=3),
        }))
        target_path.chmod(0o600)
        raw = "".join(json.dumps(row) + "\n" for row in self.rows())
        argv = [str(SCRIPT), "--source-receipt", str(receipt_path), "--target-receipt", str(target_path),
                "--output", str(self.snapshot / "stale-target-preview"), "--migration-id", "synthetic-stale-target"]
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(sys, "argv", argv), patch.object(MODULE, "psql", side_effect=[json.dumps(self.identity), raw]) as runner, contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = MODULE.main()
        self.assertEqual(result, 1)
        self.assertEqual(json.loads(stderr.getvalue())["error_code"], "target_contract_invalid")
        self.assertEqual(runner.call_count, 2)
        self.assertTrue(all(call.kwargs["readonly"] for call in runner.call_args_list))
        self.assertEqual(stdout.getvalue(), "")

    def test_unconfirmed_apply_keeps_original_manifest_plan_and_run_id(self):
        receipt_path = self.snapshot / "source-receipt.json"
        receipt_path.write_text(json.dumps(self.receipt))
        receipt_path.chmod(0o600)
        target_path = self.snapshot / "target-receipt.json"
        target = target_identity(database_oid=12346, tcp_connection=False)
        target_path.write_text(json.dumps({
            "status": "initialized-empty-identity-target", "registry_provisioned": True,
            "application_or_worker_started": False, "migration_id": "synthetic-unknown-commit",
            "source_archive_sha256": self.receipt["archive"]["sha256"],
            "connection": {"database": target["database"], "role": target["role"], "port": target["port"],
                           "login_role": target["session_user"], "set_role": target["role"]},
            "instance_identity": {**target, "listen_addresses": ""}, "contract": target_contract(),
        }))
        target_path.chmod(0o600)
        raw = "".join(json.dumps(row) + "\n" for row in self.rows())
        output = self.snapshot / "unknown-commit-preview"
        argv = [str(SCRIPT), "--source-receipt", str(receipt_path), "--target-receipt", str(target_path),
                "--output", str(output), "--migration-id", "synthetic-unknown-commit", "--apply",
                "--confirm-source-sha256", self.receipt["archive"]["sha256"]]
        stdout, stderr = io.StringIO(), io.StringIO()
        error = MODULE.MappingError("synthetic_acknowledgement_lost")
        with patch.object(sys, "argv", argv), patch.object(MODULE, "psql", side_effect=[json.dumps(self.identity), raw, json.dumps(target), error]) as runner, contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            self.assertEqual(MODULE.main(), 1)
        self.assertEqual(json.loads(stderr.getvalue()), {"status": "apply-outcome-unconfirmed", "error_code": "synthetic_acknowledgement_lost"})
        self.assertEqual(runner.call_count, 4)
        manifest = json.loads((output / "manifest.private.json").read_text())
        plan = json.loads((output / "identity-mapping.private.json").read_text())
        MODULE.validate_import_material(plan, self.receipt["archive"]["sha256"], manifest)
        self.assertIn("synthetic-unknown-commit", (output / "identity-import.private.sql").read_text())
        self.assertTrue((output / "source-target-id-map.private.jsonl").exists())
        self.assertEqual(stdout.getvalue(), "")

    def test_unreviewed_schema_aborts_before_source_query_or_output_creation(self):
        output = self.snapshot / "unreviewed-source-preview"
        argv = [str(SCRIPT), "--source-receipt", str(self.snapshot / "not-needed.json"),
                "--output", str(output), "--migration-id", "synthetic-unreviewed-schema"]
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(sys, "argv", argv), patch.object(MODULE, "IDENTITY_FINGERPRINT", "f" * 64), patch.object(MODULE, "psql") as runner, contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = MODULE.main()
        self.assertEqual(result, 1)
        self.assertEqual(json.loads(stderr.getvalue())["error_code"], "identity_source_fingerprint_mismatch")
        runner.assert_not_called()
        self.assertFalse(output.exists())


class PsqlSafetyTests(unittest.TestCase):
    def test_explicit_login_and_effective_owner_pair_is_validated(self):
        connection = {"role": "synthetic_owner", "login_role": "synthetic_login", "set_role": "synthetic_owner"}
        self.assertEqual(MODULE.connection_roles(connection), ("synthetic_owner", "synthetic_login"))
        self.assertEqual(MODULE.psql_options(connection, readonly=True), "-c default_transaction_read_only=on" + MODULE.READONLY_PSQL_TIMEOUT_OPTIONS + " -c role=synthetic_owner")
        for change in ({"set_role": None}, {"set_role": "other"}, {"login_role": "synthetic_owner"}, {"login_role": "bad; role"}):
            with self.subTest(change=change), self.assertRaises(MODULE.MappingError):
                MODULE.connection_roles({**connection, **change})
        for connection in ({"role": "owner", "login_role": "login"}, {"role": "owner", "set_role": "owner"}):
            with self.assertRaises(MODULE.MappingError):
                MODULE.connection_roles(connection)

    def test_source_role_switch_is_rejected_before_connection(self):
        for field in ("login_role", "set_role"):
            with self.subTest(field=field), patch.object(MODULE, "psql") as runner, self.assertRaisesRegex(MODULE.MappingError, "^source_role_switch_forbidden$"):
                identity, connection, expected = InstanceGuardTests().fixture(source=True)
                MODULE.verify_instance(identity, {**connection, field: "owner"}, expected, source=True)
            runner.assert_not_called()

    def test_target_requires_actual_login_session_and_nologin_owner(self):
        identity, connection, expected = InstanceGuardTests().fixture()
        connection.update(login_role="synthetic_login", set_role="synthetic_owner")
        identity.update(session_user="synthetic_login", effective_role_can_login=False)
        MODULE.verify_instance(identity, connection, expected, source=False)
        for changes in ({"session_user": "other"}, {"role": "synthetic_login"}, {"effective_role_can_login": True}):
            with self.subTest(changes=changes), self.assertRaises(MODULE.MappingError):
                MODULE.verify_instance({**identity, **changes}, connection, expected, source=False)

    def test_failed_database_command_keeps_stderr_private_and_reports_only_safe_code(self):
        marker = "synthetic-private-stderr-marker"
        completed = SimpleNamespace(returncode=1, stderr=marker, stdout="")
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            connection = {"PGHOST": str(directory), "PGDATABASE": "synthetic", "PGUSER": "synthetic_reader"}
            with patch.object(MODULE, "socket_connection", return_value=connection), patch.object(MODULE.subprocess, "run", return_value=completed) as runner:
                with self.assertRaises(MODULE.MappingError) as caught:
                    MODULE.psql({}, "BEGIN READ ONLY; SELECT 1; COMMIT;", readonly=True, private_dir=directory, label="synthetic")
            self.assertEqual(str(caught.exception), "synthetic_database_command_failed")
            self.assertNotIn(marker, str(caught.exception))
            path = directory / "synthetic.stderr.private.txt"
            self.assertEqual(path.read_text(), marker)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            command = runner.call_args.args[0]
            self.assertIn("-X", command)
            self.assertIn("ON_ERROR_STOP=1", command)
            self.assertIn("--no-password", command)
            self.assertEqual(runner.call_args.kwargs["timeout"], MODULE.PSQL_CLIENT_TIMEOUT_SECONDS)

    def test_database_timeout_is_private_bounded_and_never_retried(self):
        for readonly, suffix in ((True, "_database_command_timed_out"),
                                 (False, "_database_command_timed_out_result_unconfirmed")):
            for stdout, stderr in ((b"synthetic-private-output", b"synthetic-private-error"),
                                   ("synthetic-private-output", "synthetic-private-error"), (None, None)):
                with self.subTest(readonly=readonly, partial_type=type(stdout).__name__), tempfile.TemporaryDirectory() as temporary:
                    directory = Path(temporary)
                    error = subprocess.TimeoutExpired(["psql", "synthetic-private-command"],
                                                       MODULE.PSQL_CLIENT_TIMEOUT_SECONDS, output=stdout, stderr=stderr)
                    with patch.object(MODULE, "socket_connection", return_value={}), patch.object(MODULE.subprocess, "run", side_effect=error) as runner:
                        with self.assertRaises(MODULE.MappingError) as caught:
                            MODULE.psql({}, "synthetic-private-sql", readonly=readonly, private_dir=directory, label="synthetic")
                    self.assertEqual(str(caught.exception), "synthetic" + suffix)
                    self.assertIsNone(caught.exception.__cause__)
                    runner.assert_called_once()
                    self.assertEqual(runner.call_args.kwargs["timeout"], MODULE.PSQL_CLIENT_TIMEOUT_SECONDS)
                    for name, value in (("stdout", stdout), ("stderr", stderr)):
                        path = directory / ("synthetic." + name + ".private.txt")
                        expected = value.decode() if isinstance(value, bytes) else value or ""
                        self.assertEqual(path.read_text(), expected)
                        self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)

    def test_source_connection_does_not_inherit_ambient_production_pg_settings(self):
        completed = SimpleNamespace(returncode=0, stderr="", stdout="synthetic-ok")
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            connection = {"PGHOST": str(directory), "PGDATABASE": "synthetic", "PGUSER": "synthetic_reader"}
            ambient = {"PGSERVICE": "synthetic-production-service", "PGOPTIONS": "synthetic-unsafe-options", "PGPASSWORD": "synthetic-secret"}
            with patch.dict(os.environ, ambient), patch.object(MODULE, "socket_connection", return_value=connection), patch.object(MODULE.subprocess, "run", return_value=completed) as runner:
                result = MODULE.psql({}, "SELECT 1;", readonly=True, private_dir=directory, label="synthetic")
            self.assertEqual(result, "synthetic-ok")
            environment = runner.call_args.kwargs["env"]
            self.assertEqual(environment["PGHOST"], str(directory))
            self.assertEqual(environment["PGDATABASE"], "synthetic")
            self.assertEqual(environment["PGOPTIONS"], "-c default_transaction_read_only=on" + MODULE.READONLY_PSQL_TIMEOUT_OPTIONS)
            self.assertNotIn("PGSERVICE", environment)
            self.assertNotIn("PGPASSWORD", environment)

    def test_explicit_owner_role_options_replace_ambient_pg_authority(self):
        completed = SimpleNamespace(returncode=0, stderr="", stdout="synthetic-ok")
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            connection = {"role": "synthetic_owner", "login_role": "synthetic_login", "set_role": "synthetic_owner"}
            variables = {"PGHOST": str(directory), "PGDATABASE": "synthetic", "PGUSER": "synthetic_login"}
            with patch.dict(os.environ, {"PGOPTIONS": "-c role=untrusted", "PGUSER": "untrusted", "PGSERVICE": "untrusted"}), patch.object(MODULE, "socket_connection", return_value=variables), patch.object(MODULE.subprocess, "run", return_value=completed) as runner:
                MODULE.psql(connection, "SELECT 1;", readonly=False, private_dir=directory, label="synthetic")
            environment = runner.call_args.kwargs["env"]
            self.assertEqual(environment["PGUSER"], "synthetic_login")
            self.assertEqual(environment["PGOPTIONS"], "-c default_transaction_read_only=off -c role=synthetic_owner")
            self.assertNotIn("PGSERVICE", environment)


@unittest.skipUnless(os.environ.get("LMM_IDENTITY_IMPORT_FIXTURE_RECEIPT"), "explicit owned PostgreSQL fixture receipt required")
class RealIdentityTransactionTests(unittest.TestCase):
    """Opt-in only: leaves three synthetic users in one dedicated fresh fixture.

    The private receipt must attest status=initialized-empty-identity-fixture,
    synthetic_only=true, registry_provisioned=true, and no app/worker started.
    It supplies connection, instance_identity, and the reviewed schema-5 Core26 contract
    with allow_initialized_core_seed=true and initialized_core_seed_contract.
    Never use an application database here.
    Every database response and error stays in a new private Backups directory.
    """

    def test_atomic_rollback_replay_and_conflicting_receipts(self):
        receipt_path = MODULE.private_backup_path(Path(os.environ["LMM_IDENTITY_IMPORT_FIXTURE_RECEIPT"]))
        receipt = MODULE.load_receipt(receipt_path, private=True)
        self.assertTrue(receipt.get("status") == "initialized-empty-identity-fixture", "fixture_status_invalid")
        self.assertTrue(receipt.get("synthetic_only") is True, "fixture_synthetic_attestation_missing")
        self.assertTrue(receipt.get("registry_provisioned") is True, "fixture_registry_missing")
        self.assertTrue(receipt.get("application_or_worker_started") is False, "fixture_application_started")
        from test_financial_import_pg import verify_compiled_fixture
        verify_compiled_fixture(receipt, expected_status="initialized-empty-identity-fixture")
        self.assertTrue(receipt.get("initialized_by") == "core-init-db"
                        and receipt.get("init_binary_sha256") == receipt["verified_binary"]["sha256"]
                        and receipt.get("schema_manifest_sha256") == receipt["schema_manifest"]["sha256"],
                        "fixture_core_init_proof_missing")
        contract = receipt.get("contract", {})
        self.assertTrue(contract.get("allow_initialized_core_seed") is True, "fixture_seed_permission_missing")
        self.assertTrue(contract.get("initialized_core_seed_contract") == MODULE.CORE_SEED_CONTRACT, "fixture_seed_contract_mismatch")
        self.assertTrue(all(contract.get(key) == value for key, value in MODULE.identity_contract().items()), "fixture_identity_contract_mismatch")
        output_parent = MODULE.private_backup_path(Path(receipt.get("private_output_parent", "")), directory=True)
        private_dir = Path(tempfile.mkdtemp(prefix="identity-transaction-test-", dir=output_parent))
        MODULE.private_backup_path(private_dir, directory=True)
        connection = dict(receipt.get("connection", {}))
        self.assertTrue(connection.get("login_role") != connection.get("role")
                        and connection.get("set_role") == connection.get("role"), "fixture_independent_installer_required")
        if not connection.get("pgpass_file"):
            connection["pgpass_file"] = str(MODULE.write_private(private_dir, "fixture-empty.pgpass", ""))

        def query(sql, label, *, readonly=True):
            response = MODULE.psql(connection, sql, readonly=readonly, private_dir=private_dir, label=label)
            MODULE.write_private(private_dir, label + ".response.private.jsonl", response)
            return response

        identity = json.loads(query("BEGIN READ ONLY; SET LOCAL timezone='UTC';\n" + MODULE.IDENTITY_SQL + "\nCOMMIT;", "fixture-identity"))
        self.assertTrue(180000 <= int(identity["server_version_num"]) < 190000, "fixture_pg18_required")
        MODULE.verify_instance(identity, connection, receipt.get("instance_identity", {}), source=False)
        # This owned fixture deliberately uses a non-UTC database default.
        # A new raw connection must observe it; generated A2 SQL pins its own
        # transaction, while standalone seed captures normalize exact instants.
        original_timezone = json.loads(query("""SELECT jsonb_build_object('timezone',(
 SELECT substr(setting,strpos(setting,'=')+1) FROM pg_db_role_setting r,
 LATERAL unnest(r.setconfig) setting WHERE r.setdatabase=(SELECT oid FROM pg_database WHERE datname=current_database())
 AND r.setrole=0 AND lower(split_part(setting,'=',1))='timezone'));""", "fixture-original-database-timezone"))["timezone"]
        database_name = '"' + connection["database"] + '"'
        restore_timezone = "ALTER DATABASE " + database_name + (" RESET timezone;" if original_timezone is None
            else " SET timezone TO " + MODULE.sql_literal(original_timezone) + ";")
        self.addCleanup(query, restore_timezone, "fixture-restore-database-timezone", readonly=False)
        query("ALTER DATABASE " + database_name + " SET timezone TO 'Asia/Shanghai';", "fixture-nonutc-database-default", readonly=False)
        self.assertEqual(query("SHOW timezone;", "fixture-raw-nonutc-default").strip(), "Asia/Shanghai")
        capture_sql = "SELECT jsonb_build_object('seed_sha256',(" + MODULE.fresh_seed_hash_query() + "),'trust_policy',(" + MODULE.trust_policy_seed_query() + "));"
        captured_default = json.loads(query(capture_sql, "fixture-seed-default-timezone"))
        captured_utc = json.loads(query("BEGIN READ ONLY; SET LOCAL timezone='UTC';\n" + capture_sql + "\nROLLBACK;", "fixture-seed-utc"))
        self.assertEqual(captured_default, captured_utc, "standalone_seed_capture_depends_on_session_timezone")
        self.assertEqual(captured_utc["seed_sha256"], contract["initialized_core_seed_sha256"])
        self.assertEqual(captured_utc["trust_policy"], contract["trust_policy_seed"])
        self.assertTrue(captured_utc["trust_policy"]["native_since"].endswith("+00:00"))
        timestamp_sql = "SELECT jsonb_agg(jsonb_build_object('canonical'," + MODULE.utc_timestamp_text_sql("sample.instant") + ", 'raw',to_jsonb(sample.instant)) ORDER BY sample.instant) FROM (VALUES ('2026-10-22 08:09:10.123456+08'::timestamptz),('2026-10-22 08:09:10.123+08'::timestamptz),('2026-10-22 08:09:10+08'::timestamptz)) sample(instant);"
        timestamps_default = json.loads(query(timestamp_sql, "fixture-canonical-nonutc-timestamps"))
        timestamps_utc = json.loads(query("BEGIN READ ONLY; SET LOCAL timezone='UTC';\n" + timestamp_sql + "\nROLLBACK;", "fixture-canonical-utc-timestamps"))
        self.assertEqual([row["canonical"] for row in timestamps_default], [row["raw"] for row in timestamps_utc])
        self.assertTrue(all(row["canonical"] == row["raw"] for row in timestamps_utc))
        self.assertNotEqual([row["raw"] for row in timestamps_default], [row["raw"] for row in timestamps_utc])
        self.assertEqual([row["canonical"] for row in timestamps_utc], ["2026-10-22T00:09:10+00:00", "2026-10-22T00:09:10.123+00:00", "2026-10-22T00:09:10.123456+00:00"])
        names = json.loads(query("""BEGIN READ ONLY;
SELECT json_build_object(
 'tables',(SELECT json_agg(c.relname ORDER BY c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='core_identity' AND c.relkind IN ('r','p')),
 'all_tables',(SELECT json_agg(json_build_object('schema',n.nspname,'name',c.relname) ORDER BY n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p') AND n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'),
 'all_sequences',(SELECT json_agg(n.nspname||'.'||c.relname ORDER BY n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='S' AND n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_'),
 'accounts_sequence',pg_get_serial_sequence('core_identity.accounts','id'),
 'users_sequence',pg_get_serial_sequence('core_identity.users','id'));
COMMIT;
""", "fixture-catalog"))
        table_names = names.get("tables")
        self.assertTrue(isinstance(table_names, list) and bool(table_names), "fixture_identity_tables_missing")
        self.assertTrue(all(isinstance(name, str) and MODULE.IDENTIFIER.fullmatch(name) for name in table_names), "fixture_identifier_invalid")
        mfa_tables = {"mfa_factors", "mfa_enrollments", "mfa_backup_codes", "mfa_challenges", "session_authentication"}
        self.assertTrue(mfa_tables <= set(table_names), "fixture_mfa_tables_missing")
        mfa_sequences = {"core_identity.mfa_enrollments_id_seq", "core_identity.mfa_backup_codes_id_seq"}
        self.assertTrue(mfa_sequences <= set(names["all_sequences"]), "fixture_mfa_sequences_missing")
        table_states = []
        for name in table_names:
            relation = 'core_identity."' + name + '"'
            table_states.extend([
                MODULE.sql_literal(name),
                "jsonb_build_object('count',(SELECT count(*) FROM " + relation + "),'rows',COALESCE((SELECT jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text) FROM " + relation + " t),'[]'::jsonb))",
            ])

        def sequence_state(name):
            self.assertTrue(isinstance(name, str), "fixture_sequence_missing")
            parts = name.split(".")
            self.assertTrue(len(parts) == 2 and all(MODULE.IDENTIFIER.fullmatch(part) for part in parts), "fixture_sequence_identifier_invalid")
            relation = ".".join('"' + part + '"' for part in parts)
            return "(SELECT jsonb_build_object('last_value',last_value,'log_cnt',log_cnt,'is_called',is_called,'start_value',(SELECT seqstart FROM pg_sequence WHERE seqrelid=" + MODULE.sql_literal(name) + "::regclass),'definition',(SELECT to_jsonb(s) FROM pg_sequence s WHERE seqrelid=" + MODULE.sql_literal(name) + "::regclass)) FROM " + relation + ")"

        global_tables = []
        for entry in names["all_tables"]:
            self.assertTrue(all(MODULE.IDENTIFIER.fullmatch(entry[key]) for key in ("schema", "name")), "fixture_table_identifier_invalid")
            relation = '"' + entry["schema"] + '"."' + entry["name"] + '"'
            global_tables.extend([MODULE.sql_literal(entry["schema"] + "." + entry["name"]),
                "(SELECT jsonb_build_object('count',count(*),'sha256',encode(sha256(coalesce(string_agg(int8send(octet_length(convert_to(to_jsonb(t)::text,'UTF8'))::bigint)||convert_to(to_jsonb(t)::text,'UTF8'),''::bytea ORDER BY to_jsonb(t)::text COLLATE \"C\"),''::bytea)),'hex')) FROM " + relation + " t)"])
        all_sequences = []
        for name in names["all_sequences"]:
            all_sequences.extend([MODULE.sql_literal(name), sequence_state(name)])

        def object_sql(arguments):
            # PostgreSQL caps function calls at 100 arguments.
            return "(" + " || ".join("jsonb_build_object(" + ",".join(arguments[start:start + 80]) + ")" for start in range(0, len(arguments), 80)) + ")"

        state_sql = """BEGIN READ ONLY;
SET LOCAL timezone='UTC';
SELECT jsonb_build_object(
 'all_tables',""" + object_sql(global_tables) + """,
 'all_sequences',""" + object_sql(all_sequences) + """,
 'identity',jsonb_build_object(""" + ",".join(table_states) + """),
 'policies',COALESCE((SELECT jsonb_agg(to_jsonb(p) ORDER BY account_id) FROM core_billing.account_policies p),'[]'::jsonb),
 'registry',COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY migration_id) FROM core_migration.import_runs r),'[]'::jsonb),
 'seed',jsonb_build_object(
  'ledger_accounts',(SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM core_billing.ledger_accounts a),
  'balance_state',(SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b),
  'event_capacity',(SELECT jsonb_agg(to_jsonb(c)) FROM core_events.capacity c),
  'routing_release',(SELECT jsonb_agg(to_jsonb(r)) FROM public.routing_active_release r)),
 'sequences',jsonb_build_object('accounts',""" + sequence_state(names["accounts_sequence"]) + ", 'users'," + sequence_state(names["users_sequence"]) + "));\nCOMMIT;\n"

        def state(label):
            return json.loads(query(state_sql, label))

        before = state("state-before")
        self.assertTrue(all(table["count"] == 0 for table in before["identity"].values()), "fixture_identity_not_empty")
        self.assertTrue(before["policies"] == [] and before["registry"] == [], "fixture_policy_or_registry_not_empty")
        self.assertTrue(all(sequence["is_called"] is False and sequence["last_value"] == sequence["start_value"] for sequence in before["sequences"].values()), "fixture_sequences_not_fresh")
        source_users = [
            user(37, username="quote ' $lmm_identity_import$ \\ fixture", trust_level_override=0, console_activated_at=100),
            user(91, console_activated_at=100),
            user(300, role=10),
        ]
        plan = mapped(source_users, [{"user_id": 91, "enabled": True}])
        migration_id = "synthetic-identity-transaction-test"

        def import_sql(value, run_id=migration_id):
            return MODULE.build_import_sql(value, migration_id=run_id, source_sha256=SOURCE_SHA256,
                target_contract=contract, manifest=manifest_for(value, synthetic=True), target_identity=identity)

        sql = import_sql(plan)
        def rejected(value, label, message, expected_state):
            with self.assertRaises(MODULE.MappingError):
                query(value, label, readonly=False)
            self.assertTrue(message in (private_dir / (label + ".stderr.private.txt")).read_text(), "failure_did_not_reach_expected_guard")
            self.assertTrue(state(label + "-state") == expected_state, "rejected_transaction_changed_state")

        if "login_role" in connection:
            self.assertTrue(identity.get("session_user") == connection["login_role"] and identity.get("effective_role_can_login") is False, "nologin_fixture_contract_not_real")
            wrong_session = MODULE.build_import_sql(plan, migration_id=migration_id, source_sha256=SOURCE_SHA256,
                target_contract=contract, manifest=manifest_for(plan, synthetic=True), target_identity={**identity, "session_user": "wrong_synthetic_login"})
            rejected(wrong_session, "wrong-session-user", "target session or effective owner mismatch", before)
            wrong_owner = MODULE.build_import_sql(plan, migration_id=migration_id, source_sha256=SOURCE_SHA256,
                target_contract=contract, manifest=manifest_for(plan, synthetic=True), target_identity={**identity, "role": "wrong_synthetic_owner", "database_owner": "wrong_synthetic_owner"})
            rejected(wrong_owner, "wrong-effective-owner", "target session or effective owner mismatch", before)
            unauthorized = receipt.get("unauthorized_connection", {})
            self.assertTrue(unauthorized.get("role") == connection["role"] and unauthorized.get("set_role") == connection["role"]
                            and unauthorized.get("database") == connection["database"] and unauthorized.get("login_role") != connection["login_role"], "nonmember_fixture_contract_missing")
            nonmember = unauthorized["login_role"]
            self.assertRegex(nonmember, MODULE.IDENTIFIER)
            # Grant only CONNECT for this negative role-switch check, then
            # restore the offline fence before any import can be attempted.
            query('GRANT CONNECT ON DATABASE "' + connection["database"] + '" TO "' + nonmember + '";', "nonmember-connect", readonly=False)
            try:
                with self.assertRaises(MODULE.MappingError):
                    MODULE.psql(unauthorized, sql, readonly=False, private_dir=private_dir, label="nonmember-role-startup")
            finally:
                query('REVOKE CONNECT ON DATABASE "' + connection["database"] + '" FROM "' + nonmember + '";', "nonmember-disconnect", readonly=False)
            self.assertTrue("permission denied to set role" in (private_dir / "nonmember-role-startup.stderr.private.txt").read_text(), "nonmember_not_rejected_at_role_authority")
            self.assertTrue(state("state-after-nonmember") == before, "nonmember_changed_state")

        client_env = {key: value for key, value in os.environ.items() if not key.startswith("PG")}
        client_env.update(MODULE.socket_connection(connection))
        client_env.update(PGCONNECT_TIMEOUT="5", PGAPPNAME="lmm-identity-fixture-competing", PGOPTIONS=MODULE.psql_options(connection, readonly=True))
        competitor = subprocess.Popen(["psql", "-X", "--no-password", "-qAt", "-v", "ON_ERROR_STOP=1"],
                                      env=client_env, stdin=subprocess.PIPE, text=True,
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            competitor.stdin.write("BEGIN READ ONLY; SELECT 1;\n")
            competitor.stdin.flush()
            for attempt in range(30):
                count = query("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND application_name='lmm-identity-fixture-competing';", "competing-client-probe-" + str(attempt)).strip()
                if count == "1":
                    break
                time.sleep(0.05)
            self.assertTrue(count == "1", "competing_synthetic_client_did_not_connect")
            rejected(sql, "other-client", "target has other database clients", before)
        finally:
            competitor.terminate()
            competitor.wait(timeout=5)
            competitor.stdin.close()
            # psql termination closes its socket; wait for its backend to exit.
            for attempt in range(30):
                remaining = query("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND application_name='lmm-identity-fixture-competing';", "competing-client-exit-" + str(attempt)).strip()
                if remaining == "0":
                    break
                time.sleep(0.05)
            self.assertTrue(remaining == "0", "competing_synthetic_client_did_not_exit")

        for field, value in (("database_oid", int(identity["database_oid"]) + 1), ("server_instance_nonce", "wrong-synthetic-nonce"),
                             ("postmaster_started_at", "2000-01-01 00:00:00+00"), ("port", identity["port"] - 1),
                             ("server_version_num", "180005"), ("database", "wrong_synthetic_database")):
            wrong = MODULE.build_import_sql(plan, migration_id=migration_id, source_sha256=SOURCE_SHA256,
                target_contract=contract, manifest=manifest_for(plan, synthetic=True), target_identity={**identity, field: value})
            rejected(wrong, "wrong-instance-" + field, "target instance identity mismatch", before)

        for label, mutation, message in (
            ("wrong-identity-version", "UPDATE core_meta.schema_contract SET version=2;", "identity contract mismatch"),
            ("wrong-component", "UPDATE core_meta.component_contract SET fingerprint=decode(repeat('aa',32),'hex') WHERE name='commerce';", "installed core contract or actual catalog mismatch"),
            ("wrong-runtime-marker", "UPDATE core_meta.runtime_contract SET catalog_fingerprint=decode(repeat('aa',32),'hex');", "installed core contract or actual catalog mismatch"),
            ("catalog-drift", "ALTER TABLE core_identity.user_profiles ADD COLUMN synthetic_unreviewed TEXT;", "installed core contract or actual catalog mismatch"),
            ("nonempty-target", "INSERT INTO core_identity.accounts(id,kind) OVERRIDING SYSTEM VALUE VALUES(7,'personal');", "target business table is not empty"),
            ("noncanonical-seed", "UPDATE core_events.capacity SET max_events=max_events+1;", "non-canonical initialized core seed"),
            ("changed-abuse-revision", "UPDATE core_abuse.settings SET revision=1;", "non-canonical initialized core seed"),
            ("changed-abuse-document", "UPDATE core_abuse.settings SET document='{}'::jsonb;", "non-canonical initialized core seed"),
            ("missing-abuse-seed", "DELETE FROM core_abuse.settings;", "non-canonical initialized core seed"),
            ("wrong-oauth-ddl", "UPDATE core_oauth.contract SET ddl_fingerprint=decode(repeat('aa',32),'hex');", "non-canonical initialized OAuth seed"),
            ("wrong-oauth-catalog", "UPDATE core_oauth.contract SET catalog_fingerprint=decode(repeat('aa',32),'hex');", "non-canonical initialized OAuth seed"),
            ("missing-oauth-seed", "DELETE FROM core_oauth.contract;", "non-canonical initialized OAuth seed"),
            ("trust-seed-revision", "UPDATE core_trust_pricing.policies SET revision=2;", "non-canonical initialized trust seed"),
            ("trust-seed-discount", "UPDATE core_trust_pricing.policies SET spec=jsonb_set(spec,'{tiers,2,discount,numerator}','96');", "non-canonical initialized trust seed"),
            ("trust-seed-fence", "UPDATE core_trust_pricing.policies SET native_user_floor=2;", "non-canonical initialized trust seed"),
            ("trust-seed-time", "UPDATE core_trust_pricing.policies SET native_since=native_since-interval '1 second';", "non-canonical initialized trust seed"),
            ("trust-seed-cutoff", "UPDATE core_trust_pricing.policies SET source_cutoff=source_cutoff-1;", "non-canonical initialized trust seed"),
            ("trust-seed-digest", "UPDATE core_trust_pricing.policies SET source_digest=decode(repeat('ab',32),'hex');", "non-canonical initialized trust seed"),
            ("trust-seed-missing", "DELETE FROM core_trust_pricing.policies;", "non-canonical initialized trust seed"),
            ("gift-seed-randomized", "UPDATE core_gift.risk_key SET secret=decode(repeat('ab',32),'hex');", "initialized seed receipt mismatch"),
            ("telegram-not-empty", "INSERT INTO core_telegram_auth.assertions(digest) VALUES(decode(repeat('ab',32),'hex'));", "target business table is not empty"),
            ("sequence-not-fresh", "ALTER SEQUENCE core_events.outbox_id_seq RESTART WITH 9;", "non-canonical unused sequence state"),
            ("sequence-definition", "ALTER SEQUENCE core_events.outbox_id_seq CACHE 2;", "non-canonical sequence definition"),
            ("identity-sequence-not-fresh", "ALTER SEQUENCE core_identity.users_id_seq RESTART WITH 9;", "non-canonical identity sequence state"),
            ("unexpected-public-grant", "GRANT SELECT ON core_telegram_auth.assertions TO PUBLIC;", "target schema ownership or public privileges mismatch"),
        ):
            rejected(sql.replace("SELECT pg_advisory_xact_lock", mutation + "\nSELECT pg_advisory_xact_lock", 1), label, message, before)

        # Insert structurally valid old-upgrade receipts; a catalog match alone
        # must not misclassify an upgraded/history-bearing target as fresh.
        base_upgrade = {
            "upgrade_id": "synthetic-old-upgrade", "request_sha256": "\\x" + "ab" * 32,
            "predecessor_manifest_sha256": "\\x" + "ab" * 32,
            "predecessor_catalog_sha256": "\\x" + "ab" * 32,
            "successor_manifest_sha256": "\\x" + "ab" * 32,
            "successor_catalog_sha256": "\\x" + "ab" * 32,
            "database_system_identifier": "123", "database_oid": int(identity["database_oid"]),
            "database_name": identity["database"], "installer_role": identity["session_user"],
            "owner_role": identity["role"], "runtime_role": "synthetic_runtime",
            "extensions_role": "synthetic_extensions", "recorded_at": "2026-10-12T00:00:00+00:00",
        }
        for table, extra in (
            ("schema_upgrades", {"procedure": "core-24-to-25-trust-pricing-v1", "image_definition_changes": [{}, {}]}),
            ("component_upgrades", {"procedure": "core-additive-components-v1", "predecessor_components": 25,
                "successor_components": 26, "delta_components": ["telegram-auth"], "added_catalog_entries": ["synthetic"],
                "preserved_data_sha256": "\\x" + "ab" * 32, "preserved_permissions_sha256": "\\x" + "ab" * 32}),
        ):
            relation = "core_meta." + table
            mutation = "INSERT INTO " + relation + " SELECT * FROM jsonb_populate_record(NULL::" + relation + "," + MODULE.sql_literal(MODULE.canonical({**base_upgrade, **extra})) + "::jsonb);"
            rejected(sql.replace("SELECT pg_advisory_xact_lock", mutation + "\nSELECT pg_advisory_xact_lock", 1),
                     "nonempty-" + table, "fresh target has upgrade receipts", before)

        preceding, commit, trailing = sql.rpartition("COMMIT;")
        self.assertTrue(bool(commit) and not trailing.strip(), "fixture_plan_commit_boundary_invalid")
        forced_failure = "\\set VERBOSITY sqlstate\n" + preceding + "SELECT 1/0;\nCOMMIT;\n"
        MODULE.write_private(private_dir, "forced-rollback.private.sql", forced_failure)
        with self.assertRaises(MODULE.MappingError):
            query(forced_failure, "forced-rollback", readonly=False)
        failure_text = (private_dir / "forced-rollback.stderr.private.txt").read_text()
        self.assertTrue("22012" in failure_text, "forced_failure_did_not_reach_final_statement")
        self.assertTrue(state("state-after-forced-rollback") == before, "forced_rollback_changed_rows_seed_or_sequences")

        query(sql, "successful-import", readonly=False)
        imported = state("state-after-import")
        self.assertTrue(imported["seed"] == before["seed"], "successful_import_changed_seed")
        permitted_tables = {"core_identity." + name for name in MODULE.TABLES} | {"core_billing.account_policies", "core_migration.import_runs", "core_identity.privileged_import_facts"}
        self.assertTrue({key: value for key, value in imported["all_tables"].items() if key not in permitted_tables}
                        == {key: value for key, value in before["all_tables"].items() if key not in permitted_tables}, "successful_import_changed_other_business_tables")
        permitted_sequences = {names["accounts_sequence"], names["users_sequence"]}
        self.assertTrue({key: value for key, value in imported["all_sequences"].items() if key not in permitted_sequences}
                        == {key: value for key, value in before["all_sequences"].items() if key not in permitted_sequences}, "successful_import_changed_other_sequences")
        self.assertTrue(len(imported["registry"]) == 1, "successful_import_receipt_count_wrong")
        expected_counts = {"accounts": 3, "users": 3, "user_profiles": 3, "password_logins": 3, "developer_access": 3, "privileged_import_facts": 1}
        self.assertTrue(all(table["count"] == expected_counts.get(name, 0) for name, table in imported["identity"].items()), "successful_import_identity_counts_wrong")
        self.assertTrue(len(imported["policies"]) == 3, "successful_import_policy_count_wrong")
        users_by_id = {row["id"]: row for row in imported["identity"]["users"]["rows"]}
        accounts_by_id = {row["id"]: row for row in imported["identity"]["accounts"]["rows"]}
        profiles_by_id = {row["user_id"]: row for row in imported["identity"]["user_profiles"]["rows"]}
        logins_by_id = {row["user_id"]: row for row in imported["identity"]["password_logins"]["rows"]}
        developers_by_id = {row["user_id"]: row for row in imported["identity"]["developer_access"]["rows"]}
        policies_by_id = {row["account_id"]: row for row in imported["policies"]}
        profile_fields = ("username", "display_name", "email", "email_login_allowed", "setting",
                          "password_login_allowed", "second_factor_required", "legacy_bcrypt_allowed")
        for expected in plan["records"]:
            user_id, account_id = expected["user_id"], expected["account_id"]
            actual_user, actual_account = users_by_id[user_id], accounts_by_id[account_id]
            self.assertTrue(all(actual_user[field] == value for field, value in {
                "personal_account_id": account_id, "account_kind": "personal", "platform_role": expected["platform_role"],
                "active": expected["active"], "auth_version": expected["auth_version"],
            }.items()), "successful_import_user_exact_readback_failed")
            self.assertTrue(all(actual_account[field] == value for field, value in {
                "kind": "personal", "service_level": 0, "active": expected["active"],
            }.items()), "successful_import_account_exact_readback_failed")
            self.assertTrue(all(profiles_by_id[user_id][field] == expected[field] for field in profile_fields), "successful_import_profile_exact_readback_failed")
            self.assertTrue(logins_by_id[user_id]["login_name"] == f"legacy-{user_id}" and logins_by_id[user_id]["password_hash"] == expected["password_hash"], "successful_import_verifier_exact_readback_failed")
            self.assertTrue(policies_by_id[account_id]["preference"] == "wallet_only" and policies_by_id[account_id]["revision"] == 1, "successful_import_policy_exact_readback_failed")
            self.assertTrue(all(developers_by_id[user_id][field] == value for field, value in {
                "granted": expected["developer_access_granted"], "source_reference": expected["developer_access_source_reference"],
                "revision": expected["developer_access_revision"],
            }.items()), "successful_import_developer_exact_readback_failed")
        # Read privileged provenance independently, rather than relying only on
        # the importer's internal assertions or the broad write-scope allowlist.
        privileged = imported["identity"]["privileged_import_facts"]["rows"]
        self.assertEqual(len(privileged), 1)
        expected = next(row for row in plan["records"] if row["user_id"] == 300)
        digest_match = re.search(r"encode\(f\.a2_sql_sha256,'hex'\)='([0-9a-f]{64})'", sql)
        delimiter_match = re.search(r"DO (\$lmm_import_[a-z0-9]+\$)", sql)
        self.assertIsNotNone(digest_match)
        self.assertIsNotNone(delimiter_match)
        body_digest = digest_match.group(1)
        delimiter = delimiter_match.group(1)
        unsealed = sql.replace("DO " + delimiter, "DO $lmm_identity_import$", 1).replace(
            "END " + delimiter + ";", "END $lmm_identity_import$;", 1)
        self.assertEqual(unsealed.count(body_digest), 2)
        self.assertEqual(hashlib.sha256(unsealed.replace(body_digest, "0" * 64).encode()).hexdigest(), body_digest)
        bytea = lambda value: "\\x" + value
        system_id = query("SELECT encode(int8send(system_identifier)||decode(repeat('00',24),'hex'),'hex') FROM pg_control_system();",
                          "privileged-expected-system-id").strip()
        expected_facts = {
            "user_id": 300, "archive_sha256": bytea(SOURCE_SHA256),
            "a2_semantic_sha256": bytea(plan["mapping_sha256"]),
            "a2_file_sha256": bytea(hashlib.sha256((MODULE.canonical(plan) + "\n").encode()).hexdigest()),
            "a2_sql_sha256": bytea(body_digest), "migration_id": migration_id,
            "system_id": bytea(system_id), "database_oid": int(identity["database_oid"]),
            "instance_nonce": bytea(identity["server_instance_nonce"]),
            "import_run": bytea(hashlib.sha256(migration_id.encode()).hexdigest()),
            "source_enabled": expected["source_enabled"], "source_deleted": expected["source_deleted"],
            "password_verifier_recognized": expected["password_hash"] is not None,
            "password_login_enabled": expected["source_password_login_enabled"],
            "password_hash_sha256": bytea(hashlib.sha256(expected["password_hash"].encode()).hexdigest()),
            "login_name_sha256": bytea(hashlib.sha256(b"legacy-300").hexdigest()),
            "source_role": expected["platform_role"],
            "source_second_factor_required": expected["second_factor_required"],
            "source_auth_version": expected["auth_version"],
        }
        self.assertEqual({key: privileged[0][key] for key in expected_facts}, expected_facts,
                         "successful_import_privileged_provenance_exact_readback_failed")
        actual_receipt = imported["registry"][0]
        self.assertTrue(actual_receipt["migration_id"] == migration_id and actual_receipt["source_archive_sha256"] == SOURCE_SHA256 and actual_receipt["mapping_sha256"] == plan["mapping_sha256"], "successful_import_receipt_exact_readback_failed")
        self.assertTrue(imported["sequences"]["accounts"]["last_value"] == 4 and imported["sequences"]["users"]["last_value"] == 301, "successful_import_sequence_restart_wrong")
        self.assertTrue(all(sequence["is_called"] is False for sequence in imported["sequences"].values()), "successful_import_sequence_called_unexpectedly")

        query(sql, "replay-same-import", readonly=False)
        self.assertTrue(state("state-after-replay") == imported, "identical_replay_changed_state")
        rejected(sql.replace("SELECT pg_advisory_xact_lock", "UPDATE core_identity.user_profiles SET display_name='tampered synthetic' WHERE user_id=37;\nSELECT pg_advisory_xact_lock", 1),
                 "field-proof-conflict", "identity exact readback mismatch", imported)

        # A disabled native factor is still non-import state; replay must not
        # invent assurance or erase it. This fixture uses no sequence values.
        rejected(sql.replace("SELECT pg_advisory_xact_lock",
                 "INSERT INTO core_identity.mfa_factors(user_id,enabled) VALUES(37,false);\nSELECT pg_advisory_xact_lock", 1),
                 "native-mfa-replay-conflict", "non-import target table is not empty", imported)

        changed_users = deepcopy(source_users)
        changed_users[0]["display_name"] = "Changed synthetic profile"
        conflicting_plan = mapped(changed_users, [{"user_id": 91, "enabled": True}])
        with self.assertRaises(MODULE.MappingError):
            query(import_sql(conflicting_plan), "conflicting-mapping", readonly=False)
        self.assertTrue(state("state-after-mapping-conflict") == imported, "mapping_conflict_changed_state")

        with self.assertRaises(MODULE.MappingError):
            query(import_sql(plan, run_id="another-synthetic-migration"), "conflicting-migration-id", readonly=False)
        self.assertTrue(state("state-after-migration-id-conflict") == imported, "migration_id_conflict_changed_state")
        MODULE.write_private(private_dir, "integration-result.private.json", json.dumps({
            "status": "passed", "scope": "synthetic-owned-fixture-only", "rollback_rows_seed_and_sequences_verified": True,
            "successful_import_and_exact_readback_verified": True, "identical_replay_verified": True,
            "conflicting_mapping_and_migration_id_verified": True, "source_users": 3,
            "transaction_bound_instance_negative_cases": 6, "schema_catalog_seed_empty_negative_cases": 6,
            "target_field_tampering_detected_and_rolled_back": True,
            "all_application_tables_snapshotted": len(names["all_tables"]), "all_sequences_snapshotted": len(names["all_sequences"]),
            "other_business_rows_and_sequences_unchanged": True,
            "explicit_nologin_owner_login_member_contract_verified": "login_role" in connection,
            "wrong_session_owner_and_no_membership_rejected": "login_role" in connection,
            "nonutc_database_default": "Asia/Shanghai", "standalone_seed_capture_timezone_invariant": True,
            "utc_serialization_preserves_full_timestamp_precision": True,
        }) + "\n")


if __name__ == "__main__":
    unittest.main()
