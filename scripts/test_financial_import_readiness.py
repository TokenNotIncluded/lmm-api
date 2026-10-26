"""Offline prerequisite diagnostics and unchanged formal gates, synthetic files."""

import copy
from contextlib import redirect_stderr, redirect_stdout
import io
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))
import financial_import as F
import test_financial_import_files as files_tests


# @mark migration.financial-readiness role=test
class ReadinessTests(unittest.TestCase):
    def setUp(self):
        self.fixture = files_tests.PrepareFilesTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.number = 0

    def invoke(self, *, mode="--check-readiness", extra=(), omit=()):
        f = self.fixture
        output = f.input / ("readiness-" + str(self.number))
        self.number += 1
        argv = ["financial_import.py", mode, "--output", str(output)]
        for name in F.PREPARE_ARGUMENTS:
            if name in omit:
                continue
            value = getattr(f.args, name)
            if name == "runtime_role":
                for role in value:
                    argv += ["--runtime-role", role]
            else:
                argv += ["--" + name.replace("_", "-"), str(value)]
        if getattr(f.args, "include_legacy_escrow", False):
            argv += ["--include-legacy-escrow"]
            for name in sorted(F.ESCROW_SOURCE_FILES):
                argv += ["--" + name.replace("_", "-"), str(getattr(f.args, name))]
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(sys, "argv", [*argv, *extra]), redirect_stdout(stdout), redirect_stderr(stderr), \
             patch.object(F, "run_sql", side_effect=AssertionError("no database access")), \
             patch.object(F.IDENTITY, "psql", side_effect=AssertionError("no source database access")):
            code = F.main()
        return code, output, stdout.getvalue(), stderr.getvalue()

    def update(self, name, change):
        f = self.fixture
        path = Path(getattr(f.args, name))
        value = F.PLANNER.parse_json(F.read_private(path))
        change(value)
        path.write_text(F.canonical(value) + "\n")

    def report(self, **kwargs):
        code, output, stdout, stderr = self.invoke(**kwargs)
        self.assertEqual(stderr, "")
        public = F.PLANNER.parse_json(stdout)
        self.assertEqual(public, F.PLANNER.parse_json(F.read_private(output / "readiness.private.json")))
        self.assertEqual(sorted(path.name for path in output.iterdir()), ["readiness.private.json"])
        self.assertFalse(public["can_apply"])
        self.assertFalse(public["material_created"])
        self.assertFalse(public["approval_created"])
        self.assertFalse(public["sql_created"])
        self.assertEqual((public["source_queries"], public["target_queries"], public["target_writes"]), (0, 0, 0))
        self.assertEqual((public["target_state"], public["runtime_provision"]), ("NOT_CHECKED_OFFLINE", "NOT_CHECKED_OFFLINE"))
        self.assertNotIn(str(self.fixture.root), stdout)
        return code, public

    def test_valid_opening_inputs_pass_only_the_offline_subset(self):
        with patch.object(F.PLANNER, "reuse_frozen_export", side_effect=AssertionError("no large DTO rebuild")), \
             patch.object(F, "prepare_material", side_effect=AssertionError("no formal material")):
            code, report = self.report()
        self.assertEqual((code, report["status"]), (0, "OFFLINE_CHECK_PASSED"))
        self.assertFalse(report["source_plan_rebuilt"])
        self.assertFalse(report["source_mapper_rebuilt"])
        self.assertFalse(report["economic_authority_verified"])

    def test_unit_binding_can_pass_while_economic_and_drain_gates_still_block(self):
        self.fixture.composed_inputs()
        self.update("source_review", lambda value: value.update(decision="REVIEW_REQUIRED"))
        self.update("inflight_evidence", lambda value: value.update(basis="online-db-snapshot-without-drain", snapshot_relation="unknown"))
        code, report = self.report()
        self.assertEqual((code, report["status"]), (1, "OFFLINE_CHECK_BLOCKED"))
        checks = report["checks"]
        self.assertEqual(checks["economic_review"]["error_code"], "source_review_required")
        self.assertEqual(checks["inflight_source_evidence"]["error_code"], "inflight_source_drain_required")
        self.assertEqual(checks["escrow_unit_and_candidate_bindings"]["status"], "PASS")
        self.assertEqual(checks["target_and_role_declarations"]["status"], "PASS")

    def test_source_unit_binding_survives_missing_or_invalid_economic_inputs(self):
        self.fixture.composed_inputs()
        f = self.fixture
        names = ("source_review", "inflight_evidence")
        paths = {name: Path(getattr(f.args, name)) for name in names}
        originals = {name: path.read_bytes() for name, path in paths.items()}
        for case in ("missing", "null", "malformed"):
            try:
                for name, path in paths.items():
                    if case == "missing":
                        setattr(f.args, name, f.input / ("MISSING-" + name))
                    else:
                        path.write_text("null\n" if case == "null" else '{"incomplete":')
                with self.subTest(case=case):
                    code, report = self.report()
                    self.assertEqual((code, report["status"]), (1, "OFFLINE_CHECK_BLOCKED"))
                    checks = report["checks"]
                    self.assertEqual(checks["source_file_bindings"]["status"], "PASS")
                    self.assertEqual(checks["escrow_unit_and_candidate_bindings"]["status"], "PASS")
                    for name in names:
                        self.assertEqual(checks["input_" + name]["status"], "BLOCKED")
                    for name in ("economic_review", "inflight_source_evidence", "economic_and_policy_evidence_files"):
                        self.assertEqual(checks[name]["status"], "BLOCKED")
            finally:
                for name, path in paths.items():
                    setattr(f.args, name, path)
                    path.write_bytes(originals[name])

    def test_source_unit_binding_still_rejects_bad_candidates_without_economic_inputs(self):
        self.fixture.composed_inputs()
        f = self.fixture
        for name in ("source_review", "inflight_evidence"):
            setattr(f.args, name, f.input / ("MISSING-" + name))
        mutations = (("escrow_unit_evidence", {"credits_per_usd": 500001}, "escrow_unit_evidence_policy_invalid"),
                     ("escrow_unit_evidence", {"source_policy_evidence_sha256": "0" * 64}, "escrow_candidate_binding_mismatch"),
                     ("escrow_candidate", {"blockers": ["unverified-source"]}, "escrow_candidate_unresolved"),
                     ("escrow_candidate", {"source_unit_evidence_bound": False}, "escrow_candidate_binding_mismatch"))
        for name, change, error in mutations:
            path = Path(getattr(f.args, name))
            before = path.read_bytes()
            try:
                self.update(name, lambda value: value.update(change))
                with self.subTest(name=name, change=change):
                    code, report = self.report()
                    self.assertEqual(code, 1)
                    self.assertEqual(report["checks"]["escrow_unit_and_candidate_bindings"],
                                     {"status": "BLOCKED", "error_code": error})
            finally:
                path.write_bytes(before)

    def test_source_unit_binding_still_requires_the_frozen_source_file_binding(self):
        self.fixture.composed_inputs()
        for name in ("source_review", "inflight_evidence"):
            setattr(self.fixture.args, name, self.fixture.input / ("MISSING-" + name))
        raw = self.fixture.input / "source-financial-export.private.jsonl"
        raw.write_text(raw.read_text() + "PRIVATE-SOURCE-MARKER\n")
        code, report = self.report()
        self.assertEqual(code, 1)
        self.assertEqual(report["checks"]["source_file_bindings"]["error_code"], "source_export_file_mismatch")
        self.assertEqual(report["checks"]["escrow_unit_and_candidate_bindings"],
                         {"status": "BLOCKED", "error_code": "readiness_dependency_unavailable"})

    def test_composed_prepare_still_requires_review_and_inflight_files_before_rebuild(self):
        self.fixture.composed_inputs()
        f = self.fixture
        for name in ("source_review", "inflight_evidence"):
            original = getattr(f.args, name)
            try:
                setattr(f.args, name, f.input / ("MISSING-" + name))
                with self.subTest(name=name), patch.object(F.PLANNER, "reuse_frozen_export",
                        side_effect=AssertionError("missing authority must precede source rebuild")):
                    code, output, stdout, stderr = self.invoke(mode="--prepare")
                self.assertEqual(code, 1)
                self.assertEqual(stdout, "")
                self.assertEqual(F.PLANNER.parse_json(stderr)["status"], "REFUSED")
                self.assertTrue((output / "refusal.private.json").exists())
                self.assertFalse((output / "material.private.json").exists())
                self.assertFalse((output / "opening.private.sql").exists())
            finally:
                setattr(f.args, name, original)

    def test_source_only_readiness_cannot_replace_apply_approval_or_writer_hash(self):
        material = self.fixture.composed_inputs()
        for name in ("source_review", "inflight_evidence"):
            setattr(self.fixture.args, name, self.fixture.input / ("MISSING-" + name))
        _, report = self.report()
        self.assertEqual(report["checks"]["escrow_unit_and_candidate_bindings"]["status"], "PASS")
        sql = F.build_apply_sql(material)
        approval = {"format_version": 2, "scope": material["scope"], "decision": "REVIEW_REQUIRED",
                    "material_sha256": F.digest(material), "sql_sha256": F.sha(sql.encode()),
                    "source_review_sha256": F.digest(material["source_review"])}
        target = F.PLANNER.parse_json(Path(self.fixture.args.target_receipt).read_text())
        with patch.object(F, "run_sql", side_effect=AssertionError("no apply without approval")), \
             patch.object(F, "verify_source_files", side_effect=AssertionError("approval must be checked first")):
            with self.assertRaisesRegex(F.Error, "offline_approval_mismatch"):
                F.execute(material, approval=approval, original_sql=sql,
                          connection=target["connection"], output=self.fixture.input)
            changed = copy.deepcopy(material)
            changed["tool_source_hashes"]["financial_writer_sha256"] = "0" * 64
            with self.assertRaisesRegex(F.Error, "financial_tool_source_changed"):
                F.execute(changed, approval=approval, original_sql=sql,
                          connection=target["connection"], output=self.fixture.input)

    def test_unknown_reservations_are_not_replaced_with_zero(self):
        self.fixture.composed_inputs()
        self.update("reservations", lambda value: value["positions"][0].update(reserved_units=None))
        self.update("inflight_evidence", lambda value: value["positions"][0].update(reserved_units=None))
        before = Path(self.fixture.args.reservations).read_bytes()
        code, report = self.report()
        self.assertEqual(code, 1)
        self.assertEqual(report["checks"]["reservation_evidence"]["error_code"], "reservation_units_invalid")
        self.assertEqual(report["checks"]["inflight_source_evidence"]["error_code"], "inflight_nonzero_or_unknown_reserved")
        self.assertEqual(Path(self.fixture.args.reservations).read_bytes(), before)

    def test_explicit_source_unit_errors_are_not_reported_as_pending_review(self):
        for value in (1, True, None):
            self.update("source_review", lambda review: review.update(credits_per_usd=value))
            code, report = self.report()
            self.assertEqual(code, 1)
            self.assertEqual(report["checks"]["economic_review"]["error_code"], "source_unit_invalid")

    def test_invalid_role_declaration_does_not_claim_runtime_was_inspected(self):
        self.fixture.args.runtime_role = ["a03_owner"]
        code, report = self.report()
        self.assertEqual(code, 1)
        self.assertEqual(report["checks"]["target_and_role_declarations"]["error_code"], "runtime_roles_invalid")

    def test_tampered_raw_export_or_metadata_reports_fixed_binding_error(self):
        raw = self.fixture.input / "source-financial-export.private.jsonl"
        raw.write_text(raw.read_text() + "PRIVATE-SOURCE-MARKER\n")
        code, report = self.report()
        self.assertEqual(code, 1)
        self.assertEqual(report["checks"]["source_file_bindings"]["error_code"], "source_export_file_mismatch")
        self.assertNotIn("PRIVATE-SOURCE-MARKER", F.canonical(report))

    def test_missing_inputs_and_execution_argument_mixing_refuse_before_output(self):
        for omitted in F.PREPARE_ARGUMENTS:
            code, output, stdout, stderr = self.invoke(omit=(omitted,))
            self.assertEqual(code, 1)
            self.assertEqual(F.PLANNER.parse_json(stderr)["error_code"], "prepare_arguments_missing")
            self.assertEqual(stdout, "")
            self.assertFalse(output.exists())
        for flag in ("--material", "--approval", "--sql"):
            code, output, _, stderr = self.invoke(extra=(flag, "/private/credential-marker"))
            self.assertEqual(code, 1)
            self.assertIn("execution_arguments_on_prepare", stderr)
            self.assertNotIn("credential-marker", stderr)
            self.assertFalse(output.exists())

    def test_null_array_and_scalar_json_inputs_are_fixed_diagnostics(self):
        for name in ("plan", "source_receipt", "export_proof", "identity_manifest", "source_review", "reservations", "target_receipt"):
            path = Path(getattr(self.fixture.args, name))
            original = path.read_bytes()
            for value in (None, [], "PRIVATE-SOURCE-MARKER"):
                path.write_text(F.canonical(value))
                code, report = self.report()
                self.assertEqual(code, 1)
                self.assertEqual(report["checks"]["input_" + name]["error_code"], "readiness_input_shape_invalid")
                self.assertNotIn("PRIVATE-SOURCE-MARKER", F.canonical(report))
            path.write_bytes(original)

    def test_malformed_nested_plan_containers_are_rejected_at_the_boundary(self):
        path = Path(self.fixture.args.plan)
        original = F.PLANNER.parse_json(path.read_text())
        for field, value in (("public_summary", []), ("source_evidence", None), ("identity_provenance", []),
                             ("source_evidence", {"users": []}), ("wallet_positions", [None]), ("wallet_transfers", [None])):
            plan = copy.deepcopy(original); plan[field] = value
            path.write_text(F.canonical(plan))
            code, report = self.report()
            self.assertEqual(code, 1)
            self.assertEqual(report["checks"]["input_plan"]["error_code"], "readiness_input_shape_invalid")
        path.write_text(F.canonical(original))

    def test_missing_private_file_is_blocked_without_a_path_or_traceback(self):
        self.fixture.args.source_review = self.fixture.input / "PRIVATE-SOURCE-MARKER"
        code, report = self.report()
        self.assertEqual(code, 1)
        self.assertEqual(report["checks"]["input_source_review"]["status"], "BLOCKED")
        self.assertNotIn("PRIVATE-SOURCE-MARKER", F.canonical(report))

    def test_prepare_review_refusal_precedes_export_rebuild_and_database_access(self):
        self.update("source_review", lambda value: value.update(decision="REVIEW_REQUIRED"))
        with patch.object(F.PLANNER, "reuse_frozen_export", side_effect=AssertionError("review must be checked first")):
            code, output, stdout, stderr = self.invoke(mode="--prepare")
        self.assertEqual(code, 1)
        self.assertEqual(stdout, "")
        self.assertEqual(F.PLANNER.parse_json(stderr)["error_code"], "source_review_required")
        self.assertEqual(F.PLANNER.parse_json(F.read_private(output / "refusal.private.json"))["error_code"], "source_review_required")
        self.assertFalse((output / "material.private.json").exists())
        self.assertFalse((output / "opening.private.sql").exists())

    def test_ledger_upgrade_receipt_still_forbids_formal_opening(self):
        material = copy.deepcopy(self.fixture.material)
        material["before_state"]["tables"]["core_meta.ledger_revisions"] = {"rows": 1, "sha256": "a" * 64}
        with self.assertRaisesRegex(F.Error, "fresh26_business_or_upgrade_records_not_empty"):
            F.build_apply_sql(material)

    def test_policy_evidence_file_tampering_is_an_independent_blocker(self):
        self.fixture.composed_inputs()
        Path(self.fixture.args.escrow_policy_evidence).write_text("PRIVATE-SOURCE-MARKER")
        code, report = self.report()
        self.assertEqual(code, 1)
        self.assertEqual(report["checks"]["economic_and_policy_evidence_files"]["error_code"], "escrow_evidence_file_hash_mismatch")
        self.assertNotIn("PRIVATE-SOURCE-MARKER", F.canonical(report))

    def test_unit_and_review_validator_split_preserves_other_gates(self):
        plan, review = self.fixture.source["plan"], copy.deepcopy(self.fixture.source["review"])
        for field, value, code in (("decision", "REVIEW_REQUIRED", "source_review_required"),
                                   ("format_version", True, "source_review_profile_invalid"),
                                   ("credits_per_usd", True, "source_unit_invalid"),
                                   ("other_rights_policy", "activate-all", "source_other_rights_policy_invalid")):
            changed = {**review, field: value}
            with self.subTest(field=field), self.assertRaisesRegex(F.Error, "^" + code + "$"):
                F.validate_source_review(changed, plan)


if __name__ == "__main__":
    unittest.main()
