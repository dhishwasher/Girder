import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

from tools.plan_executor_oracle import (
    AUTHORING_POLICY,
    DEFAULT_POLICY,
    GRAPH_POLICY,
    atomic_write_json,
    authored_plan_envelope_error,
    authored_plan_shape_error,
    authoring_edits,
    authoring_plan,
    authoring_plan_json_schema,
    authoring_progress_header,
    authoring_prompt_context,
    authoring_target_node_source,
    create_graph_mutant_binary,
    create_mutant_binary,
    evaluate_graph_policy,
    evaluate_policy,
    graph_corpus,
    malformed_plan,
    observation_provenance,
    outcome_projection,
    parse_dry_report,
    rollback_steps,
    run,
    sha256_file,
    source_tree_digest,
    validate_graph_policy,
    validate_authoring_policy,
    validate_policy,
)


class PlanExecutorOracleUnitTests(unittest.TestCase):
    def setUp(self):
        self.policy = json.loads(DEFAULT_POLICY.read_text(encoding="utf-8"))

    def test_checked_policy_precommits_exact_zero_tolerance_corpus(self):
        policy = validate_policy(self.policy)

        self.assertEqual(policy["policy_id"], "plan-executor-v1")
        self.assertEqual(len(policy["corpus"]["plan_ids"]), 4)
        self.assertEqual(len(policy["corpus"]["fail_closed"]), 14)
        self.assertEqual(len(policy["corpus"]["rollback_fidelity"]), 4)
        self.assertEqual(len(policy["corpus"]["error_legibility"]), 11)
        for thresholds in policy["properties"].values():
            for field, value in thresholds.items():
                if not field.startswith("required_"):
                    self.assertEqual(value, 0, field)

    def test_policy_validation_rejects_threshold_weakening_and_corpus_drift(self):
        weakened = copy.deepcopy(self.policy)
        weakened["properties"]["P1_dry_equals_real"]["max_outcome_mismatches"] = 1
        with self.assertRaisesRegex(RuntimeError, "zero tolerance"):
            validate_policy(weakened)

        shortened = copy.deepcopy(self.policy)
        shortened["corpus"]["fail_closed"].pop()
        with self.assertRaisesRegex(RuntimeError, "14 unique ids"):
            validate_policy(shortened)

    def test_graph_policy_binds_exact_zero_tolerance_corpus_and_mutants(self):
        policy = validate_graph_policy(
            json.loads(GRAPH_POLICY.read_text(encoding="utf-8"))
        )

        self.assertEqual(policy["properties"]["P8_span_safety"]["required_case_count"], 6)
        self.assertEqual(policy["mutation_adequacy"]["required_property_count"], 4)
        self.assertEqual(policy["mutation_adequacy"]["max_surviving_mutants"], 0)
        for thresholds in policy["properties"].values():
            for field, value in thresholds.items():
                if not field.startswith("required_"):
                    self.assertEqual(value, 0, field)
        for case in graph_corpus()["span_safety"].values():
            self.assertIn("expected_files", case)
            if case["should_pass"]:
                self.assertTrue(case["expected_files"])

    def test_graph_policy_rejects_threshold_corpus_and_mutant_drift(self):
        policy = json.loads(GRAPH_POLICY.read_text(encoding="utf-8"))

        weakened = copy.deepcopy(policy)
        weakened["properties"]["P8_span_safety"]["max_corruptions"] = 1
        with self.assertRaisesRegex(RuntimeError, "zero tolerance"):
            validate_graph_policy(weakened)

        shortened = copy.deepcopy(policy)
        shortened["corpus"]["span_safety"].pop()
        with self.assertRaisesRegex(RuntimeError, "6 unique ids"):
            validate_graph_policy(shortened)

        changed_mutant = copy.deepcopy(policy)
        changed_mutant["mutation_adequacy"]["mutants"]["P8_span_safety"] = "noop"
        with self.assertRaisesRegex(RuntimeError, "mutation adequacy"):
            validate_graph_policy(changed_mutant)

    def test_authoring_tasks_are_real_repository_files_with_paired_targets(self):
        for language in ("rust", "python"):
            for operation in ("replace", "rename", "delete", "insert"):
                observed_language, graph_edit, text_edit = authoring_edits(
                    f"{language}-{operation}"
                )
                self.assertEqual(observed_language, language)
                projection = (Path(__file__).parents[1] / text_edit["path"]).read_text(
                    encoding="utf-8"
                )
                self.assertIn("node", graph_edit)
                self.assertNotIn("replace_node_body", graph_edit)
                self.assertIn("match", text_edit)
                self.assertEqual(
                    projection.count(text_edit["match"]),
                    text_edit.get("occurrences", 1),
                )

    def test_authoring_policy_rejects_corpus_and_threshold_weakening(self):
        policy = json.loads(AUTHORING_POLICY.read_text(encoding="utf-8"))
        validate_authoring_policy(policy)

        shortened = copy.deepcopy(policy)
        shortened["tasks"].pop()
        with self.assertRaisesRegex(RuntimeError, "exact eight-task"):
            validate_authoring_policy(shortened)

        weakened = copy.deepcopy(policy)
        weakened["success"]["minimum_graph_semantic_successes"] = 1
        with self.assertRaisesRegex(RuntimeError, "thresholds"):
            validate_authoring_policy(weakened)

        replacements = (
            ("model", "another-model"),
            (
                "task_intents",
                {**policy["task_intents"], "rust-delete": "Leak source"},
            ),
            (
                "prompt_protocol",
                {**policy["prompt_protocol"], "graph_context": "full source"},
            ),
            (
                "task_sources",
                {
                    **policy["task_sources"],
                    "rust": {"path": "other.rs", "sha256": "0" * 64},
                },
            ),
        )
        for field, replacement in replacements:
            changed = copy.deepcopy(policy)
            changed[field] = replacement
            with self.assertRaisesRegex(RuntimeError, "precommitment"):
                validate_authoring_policy(changed)

    def test_authoring_policy_pins_on_failure_and_criterion_rationale(self):
        policy = json.loads(AUTHORING_POLICY.read_text(encoding="utf-8"))

        missing = copy.deepcopy(policy)
        del missing["rationale"]
        with self.assertRaisesRegex(RuntimeError, "missing or unknown fields"):
            validate_authoring_policy(missing)

        reworded = copy.deepcopy(policy)
        reworded["rationale"]["success_criterion"] = "different reasoning"
        with self.assertRaisesRegex(RuntimeError, "rationale"):
            validate_authoring_policy(reworded)

        self.assertIn("on_failure", policy["rationale"])
        self.assertIn("success_criterion", policy["rationale"])
        self.assertEqual(policy["success"]["primary_criterion"], "graph_arm_semantic_successes")
        self.assertEqual(policy["success"]["minimum_graph_semantic_successes"], 4)
        self.assertEqual(policy["success"]["text_arm_reporting"], "secondary_hardware_bounded")

    def test_authoring_shape_rejects_text_graph_swaps_and_changed_semantic_check(self):
        canonical = authoring_plan("rust-replace", "abc", graph_addressed=True)
        self.assertIsNone(
            authored_plan_shape_error(copy.deepcopy(canonical), canonical, "graph")
        )

        text_edit = authoring_edits("rust-replace")[2]
        swapped = copy.deepcopy(canonical)
        swapped["steps"][0]["edits"] = [text_edit]
        self.assertIn(
            "unpermitted edit shape",
            authored_plan_shape_error(swapped, canonical, "graph"),
        )

        checked = copy.deepcopy(canonical)
        checked["steps"][0]["checks"] = [{"kind": "command", "run": "true"}]
        self.assertIn(
            "required task-specific semantic check",
            authored_plan_shape_error(checked, canonical, "graph"),
        )

    def test_authoring_json_schema_structurally_enforces_exact_envelope(self):
        canonical = authoring_plan("python-replace", "abc", graph_addressed=True)
        schema = authoring_plan_json_schema(canonical)

        self.assertFalse(schema["additionalProperties"])
        self.assertEqual(set(schema["required"]), set(canonical))
        steps = schema["properties"]["steps"]
        self.assertEqual((steps["minItems"], steps["maxItems"]), (1, 1))
        step = steps["items"]
        self.assertFalse(step["additionalProperties"])
        self.assertEqual(set(step["required"]), {"id", "edits", "checks"})
        edit = step["properties"]["edits"]["items"]
        self.assertEqual(set(edit["required"]), {"node", "replace_node"})
        self.assertFalse(edit["additionalProperties"])

    def test_authoring_json_schema_constrains_on_failure_enum(self):
        canonical = authoring_plan("python-replace", "abc", graph_addressed=True)
        schema = authoring_plan_json_schema(canonical)

        self.assertEqual(
            schema["properties"]["on_failure"],
            {"type": "string", "enum": ["rollback_plan", "rollback_step", "stop"]},
        )

    def test_authoring_envelope_violation_is_a_protocol_error_not_content_repair(self):
        canonical = authoring_plan("python-replace", "abc", graph_addressed=True)
        malformed = copy.deepcopy(canonical)
        malformed["unexpected"] = True

        self.assertEqual(
            authored_plan_envelope_error(malformed, canonical),
            "plan has missing or extra fields",
        )
        changed_content = copy.deepcopy(canonical)
        changed_content["base_commit"] = "wrong"
        self.assertIsNone(authored_plan_envelope_error(changed_content, canonical))

    def test_authoring_graph_context_matches_precommitted_protocol(self):
        graph_edit = authoring_edits("python-replace")[1]

        context = authoring_prompt_context(
            "graph",
            "python",
            graph_edit,
            Path(__file__).parents[1],
            case_id="python-replace",
            intent="uppercase the greeting",
        )

        self.assertEqual(
            context,
            {
                "node": graph_edit["node"],
                "language": "python",
                "intent": "uppercase the greeting",
                "source": "def greet(name):\n    return hello(name)",
            },
        )

    def test_authoring_graph_context_never_contains_full_target_projection(self):
        root = Path(__file__).parents[1]
        for language in ("rust", "python"):
            projection = (
                root / self._authoring_source_path(language)
            ).read_text(encoding="utf-8")
            for operation in ("replace", "rename", "delete", "insert"):
                case_id = f"{language}-{operation}"
                node_source = authoring_target_node_source(case_id, root)
                self.assertIn(node_source, projection)
                self.assertNotEqual(node_source, projection)

    @staticmethod
    def _authoring_source_path(language: str) -> str:
        return {
            "rust": "crates/aether-debugger/src/trace.rs",
            "python": "sample-project/calc.py",
        }[language]

    def test_authoring_progress_binds_binary_and_harnesses(self):
        binary = Path(sys.executable)

        header = authoring_progress_header(
            {"policy_sha256": "a" * 64, "source_commit": "b" * 40},
            "c" * 64,
            binary,
        )

        self.assertEqual(
            set(header),
            {
                "policy_sha256",
                "source_commit",
                "model_manifest_sha256",
                "bitcode_sha256",
                "harness_sha256",
                "harness_support_sha256",
                "authoring_task_check_sha256",
            },
        )
        self.assertEqual(header["bitcode_sha256"], sha256_file(binary))

    def test_dry_report_parser_ignores_trailing_human_output(self):
        report = parse_dry_report(
            "prefix\nreport (dry run, not written to disk):\n"
            '{"result":"passed","steps":[]}\nplan p passed\n'
        )

        self.assertEqual(report["result"], "passed")

    def test_outcome_projection_ignores_only_commit_specific_fields(self):
        dry = {
            "result": "passed",
            "failed_at": None,
            "steps": [
                {
                    "id": "s1",
                    "result": "passed",
                    "committed": False,
                    "files_changed": ["src/lib.rs"],
                    "checks": [{"kind": "command", "result": "passed", "detail": "ok"}],
                }
            ],
            "final_state": {"dry_run": True},
        }
        real = copy.deepcopy(dry)
        real["steps"][0]["committed"] = True
        real["final_state"] = {"dry_run": False}

        self.assertEqual(outcome_projection(dry), outcome_projection(real))

    def test_policy_evaluation_reports_exact_threshold_violation(self):
        results = {
            "P1_dry_equals_real": {"plan_count": 4, "outcome_mismatches": 1},
            "P2_no_vacuous_pass": {
                "mutation_count": 18,
                "validate_acceptances": 0,
                "run_passes": 0,
            },
            "P3_fail_closed": {"check_kind_count": 14, "unexpected_passes": 0},
            "P4_rollback_fidelity": {
                "plan_count": 4,
                "dirty_worktrees": 0,
                "tree_mismatches": 0,
            },
            "P5_error_legibility": {
                "case_count": 11,
                "missing_path_diagnostics": 0,
                "missing_step_diagnostics": 0,
            },
        }

        self.assertEqual(
            evaluate_policy(self.policy, results),
            [
                "P1_dry_equals_real.outcome_mismatches=1 violates "
                "max_outcome_mismatches=0"
            ],
        )

    def test_cross_step_rollback_case_deletes_then_recreates_before_failure(self):
        steps = rollback_steps("delete-recreate-across-steps-then-fail")

        self.assertEqual(
            [step["id"] for step in steps],
            [
                "delete-tracked",
                "intervening-step",
                "recreate-tracked",
                "trigger-rollback",
            ],
        )
        self.assertEqual(steps[0]["edits"], [{"path": "src/doomed.rs", "delete": True}])
        self.assertEqual(steps[2]["edits"][0]["path"], "src/doomed.rs")
        self.assertIn("create", steps[2]["edits"][0])
        self.assertEqual(steps[3]["checks"], [{"kind": "command", "run": "false"}])

    def test_every_legibility_case_declares_step_and_path_expectations(self):
        for case_id in self.policy["corpus"]["error_legibility"]:
            plan, step_id, path = malformed_plan(case_id, "abc")
            self.assertEqual(plan["steps"][0]["id"], step_id)
            self.assertTrue(path, case_id)

    def test_bounded_runner_can_observe_expected_nonzero_status(self):
        with tempfile.TemporaryDirectory() as directory:
            result = run(
                (sys.executable, "-c", "raise SystemExit(7)"),
                cwd=Path(directory),
                timeout_seconds=5,
                max_output_bytes=1024,
                check=False,
            )

        self.assertEqual(result.returncode, 7)

    def test_atomic_json_write_replaces_complete_document(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "observation.json"
            atomic_write_json(path, {"first": True})
            atomic_write_json(path, {"second": True})

            self.assertEqual(json.loads(path.read_text(encoding="utf-8")), {"second": True})

    def test_source_tree_digest_excludes_bitcode_runtime_metadata_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            (root / "src" / "lib.rs").write_text("fn main() {}\n", encoding="utf-8")
            (root / ".bitcode" / "reports").mkdir(parents=True)
            (root / ".bitcode" / "reports" / "one.json").write_text(
                '{"run":1}\n', encoding="utf-8"
            )
            for command in (
                ("git", "init", "-q"),
                ("git", "add", "src/lib.rs"),
            ):
                result = run(
                    command,
                    cwd=root,
                    timeout_seconds=5,
                    max_output_bytes=4096,
                    check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)

            baseline = source_tree_digest(root)
            (root / ".bitcode" / "reports" / "one.json").write_text(
                '{"run":2}\n', encoding="utf-8"
            )
            self.assertEqual(source_tree_digest(root), baseline)
            (root / "src" / "lib.rs").write_text("fn main() { panic!() }\n", encoding="utf-8")
            self.assertNotEqual(source_tree_digest(root), baseline)

    def test_vacuity_mutant_is_an_executable_broken_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            real = root / "real-bitcode"
            real.write_text("#!/bin/sh\nexit 99\n", encoding="utf-8")
            mutant = create_mutant_binary(root, real, "P2_no_vacuous_pass")
            result = run(
                (str(mutant), "plan", "validate", "unused.json"),
                cwd=root,
                timeout_seconds=5,
                max_output_bytes=1024,
                check=False,
            )

        self.assertEqual(result.returncode, 0)

    def test_rollback_mutant_removes_recreated_tracked_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            tracked = root / "src" / "doomed.rs"
            tracked.write_text("pub fn doomed() {}\n", encoding="utf-8")
            plan = root / "rollback.json"
            plan.write_text(
                json.dumps(
                    {
                        "plan_id": "delete-recreate-across-steps-then-fail",
                        "steps": [],
                    }
                ),
                encoding="utf-8",
            )
            real = root / "real-bitcode"
            real.write_text("#!/bin/sh\nexit 1\n", encoding="utf-8")
            real.chmod(0o700)
            mutant = create_mutant_binary(root, real, "P4_rollback_fidelity")

            result = run(
                (str(mutant), "plan", "run", str(plan)),
                cwd=root,
                timeout_seconds=5,
                max_output_bytes=1024,
                check=False,
            )

            self.assertEqual(result.returncode, 1)
            self.assertFalse(tracked.exists())

    def test_resolution_mutant_is_an_executable_broken_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            real = root / "real-bitcode"
            real.write_text("#!/bin/sh\nexit 99\n", encoding="utf-8")
            mutant = create_graph_mutant_binary(
                root, real, "accept-resolution-failure"
            )
            result = run(
                (str(mutant), "plan", "validate", "unused.json"),
                cwd=root,
                timeout_seconds=5,
                max_output_bytes=1024,
                check=False,
            )

        self.assertEqual(result.returncode, 0)

    def test_graph_policy_evaluation_reports_exact_threshold_violation(self):
        policy = json.loads(GRAPH_POLICY.read_text(encoding="utf-8"))
        results = {
            "P6_lowering_determinism": {
                "plan_count": 8,
                "runs_per_plan": 3,
                "fingerprint_mismatches": 1,
            },
            "P7_text_equivalence": {"pair_count": 8, "tree_mismatches": 0},
            "P8_span_safety": {
                "case_count": 6,
                "corruptions": 0,
                "rule_mismatches": 0,
            },
            "P9_resolution_fail_closed": {
                "case_count": 4,
                "unexpected_passes": 0,
                "missing_node_diagnostics": 0,
                "missing_step_diagnostics": 0,
                "missing_category_diagnostics": 0,
            },
        }

        self.assertEqual(
            evaluate_graph_policy(policy, results),
            [
                "P6_lowering_determinism.fingerprint_mismatches=1 violates "
                "max_fingerprint_mismatches=0"
            ],
        )

    def test_provenance_binds_policy_and_clean_source_commit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            policy = root / "policy.json"
            policy.write_text('{"precommitted":true}\n', encoding="utf-8")
            commands = (
                ("git", "init", "-q"),
                ("git", "config", "user.name", "Plan Oracle"),
                ("git", "config", "user.email", "oracle@example.invalid"),
                ("git", "add", "policy.json"),
                ("git", "commit", "-q", "-m", "Precommit policy"),
            )
            for command in commands:
                result = run(
                    command,
                    cwd=root,
                    timeout_seconds=5,
                    max_output_bytes=4096,
                    check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)

            provenance = observation_provenance(policy, root)
            head = run(
                ("git", "rev-parse", "HEAD"),
                cwd=root,
                timeout_seconds=5,
                max_output_bytes=4096,
                check=False,
            ).stdout.strip()

            self.assertEqual(provenance["source_commit"], head)
            self.assertEqual(len(provenance["policy_sha256"]), 64)
            policy.write_text('{"precommitted":false}\n', encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "not the recorded commit"):
                observation_provenance(policy, root)


if __name__ == "__main__":
    unittest.main()
