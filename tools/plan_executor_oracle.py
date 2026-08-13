#!/usr/bin/env python3
"""Measure Bit Code plan-executor correctness against its precommitted policy."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import stat
import sys
import tempfile
from pathlib import Path
from typing import Any, Mapping, Sequence

try:
    from tools.harness_support import BoundedProcessResult, run_bounded
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from harness_support import BoundedProcessResult, run_bounded


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_POLICY = REPO_ROOT / "docs" / "plan-executor-policy.json"
DEFAULT_OBSERVATION = REPO_ROOT / "docs" / "plan-executor-observation.json"
DRY_REPORT_MARKER = "report (dry run, not written to disk):\n"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def observation_provenance(
    policy_path: Path, source_root: Path = REPO_ROOT
) -> dict[str, str]:
    policy_sha256 = sha256_file(policy_path)
    if len(policy_sha256) != 64:
        raise RuntimeError("refusing to emit observation without policy SHA-256")

    source_commit = require_success(
        run(
            ("git", "rev-parse", "--verify", "HEAD^{commit}"),
            cwd=source_root,
            timeout_seconds=30,
            max_output_bytes=64 * 1024,
        ),
        "resolve source commit",
    ).stdout.strip()
    if len(source_commit) != 40 or any(
        character not in "0123456789abcdef" for character in source_commit
    ):
        raise RuntimeError("refusing to emit observation without source commit")

    status = require_success(
        run(
            ("git", "status", "--porcelain=v1", "--untracked-files=all"),
            cwd=source_root,
            timeout_seconds=30,
            max_output_bytes=1024 * 1024,
        ),
        "verify source worktree",
    )
    if status.stdout:
        raise RuntimeError(
            "refusing to emit observation because the source tree is not the recorded commit"
        )

    return {"policy_sha256": policy_sha256, "source_commit": source_commit}


def validate_policy(data: Mapping[str, Any]) -> Mapping[str, Any]:
    if data.get("schema_version") != 1 or type(data.get("schema_version")) is not int:
        raise RuntimeError("plan executor policy schema_version must equal 1")
    if data.get("policy_id") != "plan-executor-v1":
        raise RuntimeError("plan executor policy_id must equal plan-executor-v1")
    if set(data) != {"schema_version", "policy_id", "execution", "corpus", "properties"}:
        raise RuntimeError("plan executor policy has missing or unknown top-level fields")
    execution = data.get("execution")
    corpus = data.get("corpus")
    properties = data.get("properties")
    if not all(isinstance(section, dict) for section in (execution, corpus, properties)):
        raise RuntimeError("plan executor policy sections must be objects")
    for field in ("command_timeout_seconds", "max_command_output_bytes"):
        value = execution.get(field)
        if type(value) is not int or value <= 0:
            raise RuntimeError(f"plan executor execution {field} must be positive")
    for field in ("require_clean_fixture_before_each_case", "require_fresh_repository_per_run"):
        if execution.get(field) is not True:
            raise RuntimeError(f"plan executor execution {field} must be true")

    expected_properties = {
        "P1_dry_equals_real",
        "P2_no_vacuous_pass",
        "P3_fail_closed",
        "P4_rollback_fidelity",
        "P5_error_legibility",
    }
    if set(properties) != expected_properties:
        raise RuntimeError("plan executor policy must define exactly P1 through P5")
    for name, thresholds in properties.items():
        if not isinstance(thresholds, dict):
            raise RuntimeError(f"{name} thresholds must be an object")
        for field, value in thresholds.items():
            if type(value) is not int or value < 0:
                raise RuntimeError(f"{name}.{field} must be a non-negative integer")

    list_fields = {
        "plan_ids": 4,
        "fail_closed": 14,
        "rollback_fidelity": 3,
        "error_legibility": 11,
    }
    for field, expected_count in list_fields.items():
        values = corpus.get(field)
        if (
            not isinstance(values, list)
            or len(values) != expected_count
            or len(set(values)) != len(values)
            or any(not isinstance(value, str) or not value for value in values)
        ):
            raise RuntimeError(f"plan executor corpus {field} must contain {expected_count} unique ids")
    vacuous = corpus.get("no_vacuous_pass")
    if not isinstance(vacuous, dict):
        raise RuntimeError("plan executor vacuous corpus must be an object")
    expected_vacuous = {
        "set_check_kinds": ["graph.callers_of", "graph.callees_of", "graph.tests_for"],
        "modes": ["superset", "absent"],
        "forms": ["missing-expect", "empty-expect", "misspelled-expect"],
    }
    if vacuous != expected_vacuous:
        raise RuntimeError("plan executor vacuous corpus matrix changed from the precommitted shape")

    required_counts = {
        "P1_dry_equals_real": ("required_plan_count", len(corpus["plan_ids"])),
        "P2_no_vacuous_pass": (
            "required_mutation_count",
            len(vacuous["set_check_kinds"]) * len(vacuous["modes"]) * len(vacuous["forms"]),
        ),
        "P3_fail_closed": ("required_check_kind_count", len(corpus["fail_closed"])),
        "P4_rollback_fidelity": ("required_plan_count", len(corpus["rollback_fidelity"])),
        "P5_error_legibility": ("required_case_count", len(corpus["error_legibility"])),
    }
    for property_name, (field, count) in required_counts.items():
        if properties[property_name].get(field) != count:
            raise RuntimeError(f"{property_name}.{field} must equal its corpus count {count}")
    for property_name, thresholds in properties.items():
        for field, value in thresholds.items():
            if not field.startswith("required_") and value != 0:
                raise RuntimeError(f"{property_name}.{field} must remain zero tolerance")
    return data


def run(
    command: Sequence[str],
    *,
    cwd: Path,
    timeout_seconds: float,
    max_output_bytes: int,
    check: bool = False,
) -> BoundedProcessResult:
    return run_bounded(
        command,
        cwd=cwd,
        timeout_seconds=timeout_seconds,
        max_output_bytes=max_output_bytes,
        check=check,
    )


def require_success(result: BoundedProcessResult, label: str) -> BoundedProcessResult:
    if result.returncode != 0:
        raise RuntimeError(
            f"{label} failed ({result.returncode})\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result


def initialize_repository(root: Path, options: Mapping[str, Any]) -> str:
    root.mkdir(parents=True)
    (root / "src").mkdir()
    (root / ".gitignore").write_text(".bitcode/\n", encoding="utf-8")
    (root / "src" / "lib.rs").write_text(
        "// MODULES\n"
        "pub fn target() -> i64 { 1 }\n"
        "pub fn caller() -> i64 { 0 }\n"
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    use super::*;\n"
        "    #[test]\n"
        "    fn known_test() { assert_eq!(target(), 1); }\n"
        "}\n",
        encoding="utf-8",
    )
    (root / "src" / "doomed.rs").write_text("pub fn doomed() {}\n", encoding="utf-8")
    (root / "bitcode.toml").write_text(
        "version = 1\n"
        "[source]\n"
        'roots = ["src"]\n'
        "[tests]\n"
        'rust = ["sh", "-c", "exit 1", "--", "{test}"]\n'
        'python = ["sh", "-c", "exit 1 # {filter}"]\n'
        "run_timeout_seconds = 5\n"
        "run_max_output_bytes = 65536\n",
        encoding="utf-8",
    )
    commands = (
        ("git", "init", "--quiet"),
        ("git", "config", "user.email", "plan-oracle@bitcode.invalid"),
        ("git", "config", "user.name", "Bit Code Plan Oracle"),
        ("git", "add", "."),
        ("git", "commit", "--quiet", "-m", "baseline"),
    )
    for command in commands:
        require_success(run(command, cwd=root, check=False, **options), " ".join(command))
    result = require_success(
        run(("git", "rev-parse", "HEAD"), cwd=root, check=False, **options),
        "resolve baseline commit",
    )
    return result.stdout.strip()


def write_plan(work_root: Path, case_id: str, plan: Mapping[str, Any]) -> Path:
    path = work_root / f"{case_id}.plan.json"
    path.write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")
    return path


def base_plan(case_id: str, base_commit: str, steps: list[Mapping[str, Any]]) -> dict[str, Any]:
    return {
        "plan_version": 1,
        "plan_id": case_id,
        "intent": f"measure {case_id}",
        "base_commit": base_commit,
        "on_failure": "rollback_plan",
        "steps": steps,
    }


def parse_dry_report(stdout: str, stderr: str = "") -> Mapping[str, Any]:
    if DRY_REPORT_MARKER not in stdout:
        raise RuntimeError(
            f"dry run omitted its JSON report\nstdout:\n{stdout}\nstderr:\n{stderr}"
        )
    tail = stdout.split(DRY_REPORT_MARKER, 1)[1].lstrip()
    report, _ = json.JSONDecoder().raw_decode(tail)
    if not isinstance(report, dict):
        raise RuntimeError("dry report must be a JSON object")
    return report


def read_real_report(repository: Path) -> Mapping[str, Any]:
    reports = sorted((repository / ".bitcode" / "reports").glob("*.json"))
    if len(reports) != 1:
        raise RuntimeError(f"expected one real-run report, found {len(reports)}")
    report = json.loads(reports[0].read_text(encoding="utf-8"))
    if not isinstance(report, dict):
        raise RuntimeError("real report must be a JSON object")
    return report


def outcome_projection(report: Mapping[str, Any]) -> dict[str, Any]:
    steps = []
    for step in report.get("steps", []):
        steps.append(
            {
                "id": step.get("id"),
                "result": step.get("result"),
                "files_changed": step.get("files_changed"),
                "checks": step.get("checks"),
            }
        )
    return {
        "result": report.get("result"),
        "failed_at": report.get("failed_at"),
        "steps": steps,
    }


def p1_plan(case_id: str, base_commit: str) -> dict[str, Any]:
    if case_id == "chained-substitution":
        steps = [
            {
                "id": "introduce-mid",
                "edits": [
                    {
                        "path": "src/lib.rs",
                        "match": "pub fn caller() -> i64 { 0 }",
                        "replace": "pub fn caller() -> i64 { 1 }",
                    }
                ],
            },
            {
                "id": "consume-mid",
                "edits": [
                    {
                        "path": "src/lib.rs",
                        "match": "pub fn caller() -> i64 { 1 }",
                        "replace": "pub fn caller() -> i64 { target() }",
                    }
                ],
                "checks": [{"kind": "command", "run": "grep -q 'target()' src/lib.rs"}],
            },
        ]
    elif case_id == "created-node":
        steps = [
            {
                "id": "create-source",
                "edits": [{"path": "src/new_module.rs", "create": "pub fn fresh() {}\n"}],
            },
            {
                "id": "observe-created-node",
                "checks": [{"kind": "graph.node_exists", "node": "crate::new_module::fresh"}],
            },
        ]
    elif case_id == "cross-file-call":
        steps = [
            {
                "id": "create-dependency",
                "edits": [{"path": "src/dep.rs", "create": "pub fn added_target() -> i64 { 3 }\n"}],
            },
            {
                "id": "declare-dependency",
                "edits": [
                    {
                        "path": "src/lib.rs",
                        "match": "// MODULES",
                        "replace": "pub mod dep;",
                    }
                ],
            },
            {
                "id": "wire-cross-file-call",
                "edits": [
                    {
                        "path": "src/lib.rs",
                        "match": "pub fn caller() -> i64 { 0 }",
                        "replace": "pub fn caller() -> i64 { dep::added_target() }",
                    }
                ],
                "checks": [
                    {
                        "kind": "graph.callees_of",
                        "node": "crate::lib::caller",
                        "expect": ["crate::dep::added_target"],
                        "mode": "superset",
                    }
                ],
            },
        ]
    elif case_id == "rollback-after-composed-step":
        steps = [
            {
                "id": "first-success",
                "edits": [
                    {
                        "path": "src/lib.rs",
                        "match": "pub fn caller() -> i64 { 0 }",
                        "replace": "pub fn caller() -> i64 { 1 }",
                    }
                ],
            },
            {
                "id": "composed-failure",
                "edits": [
                    {
                        "path": "src/lib.rs",
                        "match": "pub fn caller() -> i64 { 1 }",
                        "replace": "pub fn caller() -> i64 { 2 }",
                    }
                ],
                "checks": [{"kind": "command", "run": "exit 9"}],
            },
        ]
    else:
        raise RuntimeError(f"unknown P1 plan id: {case_id}")
    return base_plan(case_id, base_commit, steps)


def measure_p1(
    policy: Mapping[str, Any], work_root: Path, bitcode: Path, options: Mapping[str, Any]
) -> dict[str, Any]:
    cases = []
    mismatches = 0
    for case_id in policy["corpus"]["plan_ids"]:
        reports = {}
        returncodes = {}
        for mode in ("dry", "real"):
            repository = work_root / f"{case_id}-{mode}"
            base_commit = initialize_repository(repository, options)
            plan_path = write_plan(work_root, f"{case_id}-{mode}", p1_plan(case_id, base_commit))
            command = [str(bitcode), "plan", "run", str(plan_path)]
            if mode == "dry":
                command.append("--dry")
            result = run(command, cwd=repository, check=False, **options)
            returncodes[mode] = result.returncode
            reports[mode] = (
                parse_dry_report(result.stdout, result.stderr)
                if mode == "dry"
                else read_real_report(repository)
            )
        dry_projection = outcome_projection(reports["dry"])
        real_projection = outcome_projection(reports["real"])
        matched = dry_projection == real_projection and (
            (returncodes["dry"] == 0) == (returncodes["real"] == 0)
        )
        mismatches += int(not matched)
        cases.append(
            {
                "id": case_id,
                "matched": matched,
                "dry_returncode": returncodes["dry"],
                "real_returncode": returncodes["real"],
                "dry": dry_projection,
                "real": real_projection,
            }
        )
    return {"plan_count": len(cases), "outcome_mismatches": mismatches, "cases": cases}


def vacuous_check(kind: str, mode: str, form: str) -> dict[str, Any]:
    check: dict[str, Any] = {"kind": kind, "node": "crate::lib::target", "mode": mode}
    if form == "empty-expect":
        check["expect"] = []
    elif form == "misspelled-expect":
        check["expct"] = ["crate::lib::caller"]
    elif form != "missing-expect":
        raise RuntimeError(f"unknown vacuous form: {form}")
    return check


def measure_p2(
    policy: Mapping[str, Any], work_root: Path, bitcode: Path, options: Mapping[str, Any]
) -> dict[str, Any]:
    matrix = policy["corpus"]["no_vacuous_pass"]
    cases = []
    validate_acceptances = 0
    run_passes = 0
    for kind in matrix["set_check_kinds"]:
        for mode in matrix["modes"]:
            for form in matrix["forms"]:
                case_id = f"{kind}-{mode}-{form}"
                repository = work_root / case_id.replace(".", "-")
                base_commit = initialize_repository(repository, options)
                plan = base_plan(
                    case_id,
                    base_commit,
                    [{"id": "reject-vacuity", "checks": [vacuous_check(kind, mode, form)]}],
                )
                plan_path = write_plan(work_root, case_id.replace(".", "-"), plan)
                validated = run(
                    (str(bitcode), "plan", "validate", str(plan_path)),
                    cwd=repository,
                    check=False,
                    **options,
                )
                executed = run(
                    (str(bitcode), "plan", "run", str(plan_path), "--dry"),
                    cwd=repository,
                    check=False,
                    **options,
                )
                validate_acceptances += int(validated.returncode == 0)
                run_passes += int(executed.returncode == 0)
                cases.append(
                    {
                        "id": case_id,
                        "validate_rejected": validated.returncode != 0,
                        "run_rejected": executed.returncode != 0,
                    }
                )
    return {
        "mutation_count": len(cases),
        "validate_acceptances": validate_acceptances,
        "run_passes": run_passes,
        "cases": cases,
    }


def fail_closed_step(case_id: str) -> dict[str, Any]:
    unknown = "crate::does_not_exist::missing"
    checks: dict[str, Mapping[str, Any]] = {
        "graph-callers-of-unknown-node": {
            "kind": "graph.callers_of",
            "node": unknown,
            "expect": [],
        },
        "graph-callees-of-unknown-node": {
            "kind": "graph.callees_of",
            "node": unknown,
            "expect": [],
        },
        "graph-tests-for-unknown-node": {
            "kind": "graph.tests_for",
            "node": unknown,
            "expect": [],
        },
        "graph-node-exists-unknown-node": {"kind": "graph.node_exists", "node": unknown},
        "graph-node-absent-existing-node": {
            "kind": "graph.node_absent",
            "node": "crate::lib::target",
        },
        "graph-no-new-edges-unknown-node": {
            "kind": "graph.no_new_edges_into",
            "node": unknown,
        },
        "graph-edge-delta-exceeded": {
            "kind": "graph.edge_delta",
            "max_added": 0,
            "max_removed": 0,
        },
        "graph-unresolved-unknown-from": {
            "kind": "graph.unresolved",
            "from": unknown,
            "node": "crate::lib::target",
        },
        "tests-impacted-failing-runner": {"kind": "tests.impacted"},
        "tests-named-unknown-test": {"kind": "tests.named", "tests": [unknown]},
        "tests-full-failing-runner": {"kind": "tests.full"},
        "command-unknown-program": {
            "kind": "command",
            "run": "bitcode-plan-oracle-command-does-not-exist",
        },
        "oracle-not-implemented": {"kind": "oracle", "min_precision": 1.0},
        "benchmark-not-implemented": {"kind": "benchmark", "policy": "missing.json"},
    }
    edits = []
    if case_id == "graph-edge-delta-exceeded":
        edits = [
            {
                "path": "src/lib.rs",
                "match": "pub fn caller() -> i64 { 0 }",
                "replace": "pub fn caller() -> i64 { target() }",
            }
        ]
    elif case_id == "tests-impacted-failing-runner":
        edits = [
            {
                "path": "src/lib.rs",
                "match": "pub fn target() -> i64 { 1 }",
                "replace": "pub fn target() -> i64 { 2 }",
            }
        ]
    if case_id not in checks:
        raise RuntimeError(f"unknown P3 case id: {case_id}")
    return {"id": "fail-closed", "edits": edits, "checks": [checks[case_id]]}


def measure_p3(
    policy: Mapping[str, Any], work_root: Path, bitcode: Path, options: Mapping[str, Any]
) -> dict[str, Any]:
    cases = []
    unexpected_passes = 0
    observed_kinds = set()
    for case_id in policy["corpus"]["fail_closed"]:
        repository = work_root / case_id
        base_commit = initialize_repository(repository, options)
        plan_path = write_plan(
            work_root,
            case_id,
            base_plan(case_id, base_commit, [fail_closed_step(case_id)]),
        )
        result = run(
            (str(bitcode), "plan", "run", str(plan_path)),
            cwd=repository,
            check=False,
            **options,
        )
        report = read_real_report(repository)
        checks = report.get("steps", [{}])[-1].get("checks", [])
        observed = checks[-1] if checks else {}
        kind = observed.get("kind")
        passed = result.returncode == 0 or observed.get("result") != "failed"
        unexpected_passes += int(passed)
        if isinstance(kind, str):
            observed_kinds.add(kind)
        cases.append(
            {
                "id": case_id,
                "returncode": result.returncode,
                "kind": kind,
                "failed_closed": not passed,
                "detail": observed.get("detail"),
            }
        )
    return {
        "check_kind_count": len(observed_kinds),
        "unexpected_passes": unexpected_passes,
        "cases": cases,
    }


def rollback_steps(case_id: str) -> list[Mapping[str, Any]]:
    if case_id == "substitute-then-fail":
        edits = [
            {
                "path": "src/lib.rs",
                "match": "pub fn caller() -> i64 { 0 }",
                "replace": "pub fn caller() -> i64 { 9 }",
            }
        ]
    elif case_id == "create-then-fail":
        edits = [{"path": "src/created.rs", "create": "pub fn created() {}\n"}]
    elif case_id == "delete-then-fail":
        edits = [{"path": "src/doomed.rs", "delete": True}]
    else:
        raise RuntimeError(f"unknown P4 case id: {case_id}")
    return [
        {"id": "commit-first-step", "edits": edits, "checks": [{"kind": "command", "run": "true"}]},
        {"id": "trigger-rollback", "checks": [{"kind": "command", "run": "false"}]},
    ]


def git_output(repository: Path, options: Mapping[str, Any], *args: str) -> str:
    result = require_success(
        run(("git", *args), cwd=repository, check=False, **options),
        f"git {' '.join(args)}",
    )
    return result.stdout.strip()


def measure_p4(
    policy: Mapping[str, Any], work_root: Path, bitcode: Path, options: Mapping[str, Any]
) -> dict[str, Any]:
    cases = []
    dirty_worktrees = 0
    tree_mismatches = 0
    for case_id in policy["corpus"]["rollback_fidelity"]:
        repository = work_root / case_id
        base_commit = initialize_repository(repository, options)
        base_tree = git_output(repository, options, "rev-parse", f"{base_commit}^{{tree}}")
        plan_path = write_plan(
            work_root,
            case_id,
            base_plan(case_id, base_commit, rollback_steps(case_id)),
        )
        result = run(
            (str(bitcode), "plan", "run", str(plan_path)),
            cwd=repository,
            check=False,
            **options,
        )
        status = git_output(repository, options, "status", "--porcelain")
        actual_tree = git_output(repository, options, "write-tree")
        clean = status == ""
        tree_matches = actual_tree == base_tree
        dirty_worktrees += int(not clean)
        tree_mismatches += int(not tree_matches)
        cases.append(
            {
                "id": case_id,
                "run_failed": result.returncode != 0,
                "status_porcelain": status,
                "base_tree": base_tree,
                "actual_tree": actual_tree,
                "tree_matches": tree_matches,
            }
        )
    return {
        "plan_count": len(cases),
        "dirty_worktrees": dirty_worktrees,
        "tree_mismatches": tree_mismatches,
        "cases": cases,
    }


def malformed_plan(case_id: str, base_commit: str) -> tuple[dict[str, Any], str, str]:
    step_id = "diagnose-malformed"
    edit_path = "src/lib.rs"
    step: dict[str, Any] = {"id": step_id}
    expected_path = ""
    if case_id == "edit-missing-path":
        step["edits"] = [{"match": "old", "replace": "new"}]
        expected_path = "edits[0]"
    elif case_id == "edit-unknown-field":
        step["edits"] = [{"path": edit_path, "match": "old", "replace": "new", "replce": "new"}]
        expected_path = edit_path
    elif case_id == "edit-multiple-discriminators":
        step["edits"] = [{"path": edit_path, "match": "old", "replace": "new", "create": "x"}]
        expected_path = edit_path
    elif case_id == "edit-substitute-missing-replace":
        step["edits"] = [{"path": edit_path, "match": "old"}]
        expected_path = edit_path
    elif case_id == "edit-create-wrong-type":
        step["edits"] = [{"path": edit_path, "create": 7}]
        expected_path = edit_path
    elif case_id == "edit-delete-wrong-type":
        step["edits"] = [{"path": edit_path, "delete": "yes"}]
        expected_path = edit_path
    elif case_id == "check-missing-kind":
        step["checks"] = [{"node": "crate::lib::target"}]
        expected_path = "checks[0]"
    elif case_id == "check-unknown-kind":
        step["checks"] = [{"kind": "graph.typo", "node": "crate::lib::target"}]
        expected_path = "checks[0]"
    elif case_id == "check-unknown-field":
        step["checks"] = [{"kind": "graph.node_exists", "node": "crate::lib::target", "nod": "x"}]
        expected_path = "nod"
    elif case_id == "step-unknown-field":
        step["unknown_step_field"] = True
        expected_path = "unknown_step_field"
    elif case_id == "plan-unknown-field":
        expected_path = "unexpected_plan_field"
    else:
        raise RuntimeError(f"unknown P5 case id: {case_id}")
    plan = base_plan(case_id, base_commit, [step])
    if case_id == "plan-unknown-field":
        plan["unexpected_plan_field"] = True
    return plan, step_id, expected_path


def measure_p5(
    policy: Mapping[str, Any], work_root: Path, bitcode: Path, options: Mapping[str, Any]
) -> dict[str, Any]:
    cases = []
    missing_steps = 0
    missing_paths = 0
    for case_id in policy["corpus"]["error_legibility"]:
        repository = work_root / case_id
        base_commit = initialize_repository(repository, options)
        plan, step_id, expected_path = malformed_plan(case_id, base_commit)
        plan_path = write_plan(work_root, case_id, plan)
        result = run(
            (str(bitcode), "plan", "validate", str(plan_path)),
            cwd=repository,
            check=False,
            **options,
        )
        diagnostic = f"{result.stdout}\n{result.stderr}"
        has_step = step_id in diagnostic
        has_path = expected_path in diagnostic
        missing_steps += int(not has_step)
        missing_paths += int(not has_path)
        cases.append(
            {
                "id": case_id,
                "rejected": result.returncode != 0,
                "named_step": has_step,
                "named_path": has_path,
                "expected_step": step_id,
                "expected_path": expected_path,
            }
        )
    return {
        "case_count": len(cases),
        "missing_step_diagnostics": missing_steps,
        "missing_path_diagnostics": missing_paths,
        "cases": cases,
    }


def create_mutant_binary(root: Path, bitcode: Path, property_name: str) -> Path:
    mutant = root / f"bitcode-mutant-{property_name.lower()}"
    script = f'''#!/usr/bin/env python3
import subprocess
import sys
from pathlib import Path

MUTATION = {property_name!r}
REAL_BITCODE = {str(bitcode)!r}
ARGS = sys.argv[1:]

is_plan_validate = len(ARGS) >= 2 and ARGS[0:2] == ["plan", "validate"]
is_plan_run = len(ARGS) >= 2 and ARGS[0:2] == ["plan", "run"]
is_dry_run = is_plan_run and "--dry" in ARGS

if MUTATION == "P2_no_vacuous_pass" and (is_plan_validate or is_dry_run):
    raise SystemExit(0)
if MUTATION == "P5_error_legibility" and is_plan_validate:
    print("invalid plan", file=sys.stderr)
    raise SystemExit(1)

completed = subprocess.run([REAL_BITCODE, *ARGS], check=False)
returncode = completed.returncode
if MUTATION == "P1_dry_equals_real" and is_dry_run:
    returncode = 1 if returncode == 0 else 0
elif MUTATION == "P3_fail_closed" and is_plan_run:
    returncode = 0
elif MUTATION == "P4_rollback_fidelity" and is_plan_run:
    target = Path.cwd() / "src" / "lib.rs"
    target.write_text(target.read_text(encoding="utf-8") + "// rollback mutant\\n", encoding="utf-8")

raise SystemExit(returncode)
'''
    mutant.write_text(script, encoding="utf-8")
    mutant.chmod(mutant.stat().st_mode | stat.S_IXUSR)
    return mutant


def measure_mutation_adequacy(
    policy: Mapping[str, Any],
    work_root: Path,
    bitcode: Path,
    options: Mapping[str, Any],
    baseline_results: Mapping[str, Any],
) -> dict[str, Any]:
    work_root.mkdir(parents=True)
    measurements = {
        "P1_dry_equals_real": measure_p1,
        "P2_no_vacuous_pass": measure_p2,
        "P3_fail_closed": measure_p3,
        "P4_rollback_fidelity": measure_p4,
        "P5_error_legibility": measure_p5,
    }
    baseline_violations = set(evaluate_policy(policy, baseline_results))
    cases = []
    survivors = []
    for property_name, measurement in measurements.items():
        mutant = create_mutant_binary(work_root, bitcode, property_name)
        observed = measurement(
            policy,
            work_root / property_name.lower(),
            mutant,
            options,
        )
        mutant_results = dict(baseline_results)
        mutant_results[property_name] = observed
        introduced = sorted(
            violation
            for violation in set(evaluate_policy(policy, mutant_results)) - baseline_violations
            if violation.startswith(f"{property_name}.")
        )
        killed = bool(introduced)
        if not killed:
            survivors.append(property_name)
        cases.append(
            {
                "property": property_name,
                "mutant_sha256": sha256_file(mutant),
                "killed": killed,
                "violations": introduced,
                "observed": {key: value for key, value in observed.items() if key != "cases"},
            }
        )
    if survivors:
        raise RuntimeError(
            "refusing to emit observation because property mutants survived: "
            + ", ".join(survivors)
        )
    return {"required_property_count": len(measurements), "survivors": 0, "cases": cases}


def evaluate_policy(policy: Mapping[str, Any], results: Mapping[str, Any]) -> list[str]:
    comparisons = {
        "P1_dry_equals_real": {
            "required_plan_count": ("plan_count", "equal"),
            "max_outcome_mismatches": ("outcome_mismatches", "maximum"),
        },
        "P2_no_vacuous_pass": {
            "required_mutation_count": ("mutation_count", "equal"),
            "max_validate_acceptances": ("validate_acceptances", "maximum"),
            "max_run_passes": ("run_passes", "maximum"),
        },
        "P3_fail_closed": {
            "required_check_kind_count": ("check_kind_count", "equal"),
            "max_unexpected_passes": ("unexpected_passes", "maximum"),
        },
        "P4_rollback_fidelity": {
            "required_plan_count": ("plan_count", "equal"),
            "max_dirty_worktrees": ("dirty_worktrees", "maximum"),
            "max_tree_mismatches": ("tree_mismatches", "maximum"),
        },
        "P5_error_legibility": {
            "required_case_count": ("case_count", "equal"),
            "max_missing_path_diagnostics": ("missing_path_diagnostics", "maximum"),
            "max_missing_step_diagnostics": ("missing_step_diagnostics", "maximum"),
        },
    }
    violations = []
    for property_name, fields in comparisons.items():
        thresholds = policy["properties"][property_name]
        observed = results[property_name]
        for threshold_name, (result_name, comparison) in fields.items():
            expected = thresholds[threshold_name]
            actual = observed[result_name]
            passed = actual == expected if comparison == "equal" else actual <= expected
            if not passed:
                violations.append(
                    f"{property_name}.{result_name}={actual} violates {threshold_name}={expected}"
                )
    return violations


def atomic_write_json(path: Path, document: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    rendered = (json.dumps(document, indent=2, sort_keys=True) + "\n").encode()
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=f".{path.name}.", delete=False) as output:
        temporary = Path(output.name)
        output.write(rendered)
        output.flush()
    temporary.replace(path)


def measure(
    policy: Mapping[str, Any],
    bitcode: Path,
    provenance: Mapping[str, str],
) -> dict[str, Any]:
    policy_sha256 = provenance.get("policy_sha256", "")
    source_commit = provenance.get("source_commit", "")
    if len(policy_sha256) != 64 or len(source_commit) != 40:
        raise RuntimeError("refusing to emit observation without complete provenance")
    execution = policy["execution"]
    options = {
        "timeout_seconds": float(execution["command_timeout_seconds"]),
        "max_output_bytes": int(execution["max_command_output_bytes"]),
    }
    original_sha = sha256_file(bitcode)
    with tempfile.TemporaryDirectory(prefix="bitcode-plan-executor-") as directory:
        root = Path(directory)
        measured_bitcode = root / "bitcode-under-test"
        shutil.copy2(bitcode, measured_bitcode)
        measured_bitcode.chmod(0o500)
        if sha256_file(measured_bitcode) != original_sha:
            raise RuntimeError("private Bit Code copy differs from the supplied binary")
        results = {
            "P1_dry_equals_real": measure_p1(policy, root / "p1", measured_bitcode, options),
            "P2_no_vacuous_pass": measure_p2(policy, root / "p2", measured_bitcode, options),
            "P3_fail_closed": measure_p3(policy, root / "p3", measured_bitcode, options),
            "P4_rollback_fidelity": measure_p4(policy, root / "p4", measured_bitcode, options),
            "P5_error_legibility": measure_p5(policy, root / "p5", measured_bitcode, options),
        }
        mutation_adequacy = measure_mutation_adequacy(
            policy,
            root / "mutation-adequacy",
            measured_bitcode,
            options,
            results,
        )
        if sha256_file(measured_bitcode) != original_sha:
            raise RuntimeError("private Bit Code copy changed during measurement")
    if sha256_file(bitcode) != original_sha:
        raise RuntimeError("supplied Bit Code binary changed during measurement")
    violations = evaluate_policy(policy, results)
    return {
        "schema_version": 1,
        "policy_id": policy["policy_id"],
        "policy_sha256": policy_sha256,
        "bitcode_sha256": original_sha,
        "tool": {
            "bitcode_sha256": original_sha,
            "source_commit": source_commit,
        },
        "results": results,
        "mutation_adequacy": mutation_adequacy,
        "policy": {"passed": not violations, "violations": violations},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bitcode", type=Path, required=True, help="exact Bit Code binary to measure")
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY, help="precommitted policy JSON")
    parser.add_argument("--output", type=Path, default=DEFAULT_OBSERVATION, help="observation JSON path")
    parser.add_argument("--json", action="store_true", help="also print the full observation")
    args = parser.parse_args()
    bitcode = args.bitcode.resolve()
    if not bitcode.is_file():
        parser.error(f"Bit Code binary does not exist: {bitcode}")
    policy_path = args.policy.resolve()
    policy = validate_policy(json.loads(policy_path.read_text(encoding="utf-8")))
    provenance = observation_provenance(policy_path)
    observation = measure(policy, bitcode, provenance)
    if not observation.get("policy_sha256") or not observation.get("tool", {}).get(
        "source_commit"
    ):
        raise RuntimeError("refusing to write observation without complete provenance")
    atomic_write_json(args.output.resolve(), observation)
    if args.json:
        print(json.dumps(observation, indent=2, sort_keys=True))
    print(
        f"plan executor policy: {'PASS' if observation['policy']['passed'] else 'FAIL'}; "
        f"observation: {args.output.resolve()}"
    )
    for violation in observation["policy"]["violations"]:
        print(f"  - {violation}", file=sys.stderr)
    return 0 if observation["policy"]["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
