import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

from tools.plan_executor_oracle import (
    DEFAULT_POLICY,
    atomic_write_json,
    create_mutant_binary,
    evaluate_policy,
    malformed_plan,
    measure,
    observation_provenance,
    outcome_projection,
    parse_dry_report,
    run,
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
        self.assertEqual(len(policy["corpus"]["rollback_fidelity"]), 3)
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
                "plan_count": 3,
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

    def test_measurement_refuses_incomplete_provenance_before_running(self):
        with self.assertRaisesRegex(RuntimeError, "complete provenance"):
            measure(self.policy, Path("unused-bitcode"), {})

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
