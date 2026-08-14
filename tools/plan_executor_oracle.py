#!/usr/bin/env python3
"""Measure Bit Code plan-executor correctness against its precommitted policy."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import socket
import shutil
import stat
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Mapping, Sequence

try:
    from tools.harness_support import BoundedProcessResult, run_bounded
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from harness_support import BoundedProcessResult, run_bounded


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_POLICY = REPO_ROOT / "docs" / "plan-executor-policy.json"
DEFAULT_OBSERVATION = REPO_ROOT / "docs" / "plan-executor-observation.json"
GRAPH_POLICY = REPO_ROOT / "docs" / "graph-edit-policy.json"
GRAPH_OBSERVATION = REPO_ROOT / "docs" / "graph-edit-observation.json"
GRAPH_CORPUS = REPO_ROOT / "docs" / "graph-edit-corpus.json"
AUTHORING_POLICY = REPO_ROOT / "docs" / "authoring-cost-policy.json"
DRY_REPORT_MARKER = "report (dry run, not written to disk):\n"
OBSERVATION_OUTPUTS = {
    "docs/plan-executor-observation.json",
    "docs/graph-edit-observation.json",
    "docs/authoring-cost-observation.json",
}
AUTHORING_MODEL = "qwen2.5-coder:1.5b"
AUTHORING_PROMPT_PROTOCOL = {
    "initial_template": (
        "Author a Bit Code plan for task {task}. Task: {intent}. Use plan_version {version}. "
        "Context: {context}. Permitted edit schema: {schema}. base_commit is {base_commit}. "
        "Return JSON only."
    ),
    "repair_suffix": (
        " Previous output failed: {diagnostic}. Return only the corrected plan object."
    ),
    "transport": "ollama /api/chat",
    "response_format": "task-specific plan JSON schema",
    "envelope_shape": "structurally enforced; repair loop evaluates content errors only",
    "text_context": "complete target-file projection",
    "graph_context": "node path, language, and bounded current Node.source only",
    "graph_full_projection": "reject",
}
AUTHORING_SEMANTIC_VERIFICATION = {
    "plan_validate": "required",
    "plan_run": "required",
    "declared_checks": "required task-specific semantic check and must pass",
    "impacted_tests": "bitcode test-impact . --run --quiet must pass",
    "reference_tree_comparison": False,
}
AUTHORING_TASK_INTENTS = {
    "rust-replace": "Change Trace::final_env to use map_or_else(Env::new, |step| step.env.clone()).",
    "rust-rename": "Rename Trace::final_env to completed_env.",
    "rust-delete": "Delete Trace::final_env.",
    "rust-insert": "Insert a module-level pub fn trace_fixture_marker() -> usize that returns 1.",
    "python-replace": "Change greet so it returns hello(name).upper().",
    "python-rename": "Rename greet to welcome_greeting.",
    "python-delete": "Delete greet.",
    "python-insert": "Insert a module-level sample_fixture_marker function that returns 1.",
}
AUTHORING_TASK_SOURCES = {
    "rust": {
        "path": "crates/aether-debugger/src/trace.rs",
        "sha256": "089a12acd5983b1fc3d36422faa34516a9ad65aea340a595de6113da8a9f30bb",
    },
    "python": {
        "path": "sample-project/calc.py",
        "sha256": "77e4ee13f9c82fe115c0c24515116459c8a260e4b28668c615612cfe4674d345",
    },
}


def graph_corpus() -> Mapping[str, Any]:
    corpus = json.loads(GRAPH_CORPUS.read_bytes())
    if corpus.get("schema_version") != 1 or corpus.get("corpus_id") != "graph-edit-v2":
        raise RuntimeError("unsupported graph-edit corpus manifest")
    return corpus


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


def source_snapshot_sha256(root: Path) -> str:
    listed = require_success(
        run_bounded(
            ("git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"),
            cwd=root,
            timeout_seconds=30,
            max_output_bytes=16 * 1024 * 1024,
        ),
        "list measured source snapshot",
    )
    digest = hashlib.sha256()
    for relative in sorted(value for value in listed.stdout.split("\0") if value):
        if relative in OBSERVATION_OUTPUTS:
            continue
        path = root / relative
        if not path.is_file():
            continue
        encoded = relative.encode()
        contents = path.read_bytes()
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
        digest.update(len(contents).to_bytes(8, "big"))
        digest.update(contents)
    return digest.hexdigest()


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
        "rollback_fidelity": 4,
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


def validate_graph_policy(data: Mapping[str, Any]) -> Mapping[str, Any]:
    if set(data) != {
        "schema_version",
        "policy_id",
        "execution",
        "corpus",
        "properties",
        "mutation_adequacy",
    }:
        raise RuntimeError("graph edit policy has missing or unknown top-level fields")
    if data.get("schema_version") != 1 or data.get("policy_id") != "graph-edit-v2":
        raise RuntimeError("graph edit policy must be schema_version 1 and graph-edit-v2")
    execution = data.get("execution")
    corpus = data.get("corpus")
    properties = data.get("properties")
    if not all(isinstance(section, dict) for section in (execution, corpus, properties)):
        raise RuntimeError("graph edit policy sections must be objects")
    if execution.get("repeated_runs") != 3:
        raise RuntimeError("graph edit policy repeated_runs must remain precommitted at 3")
    for field in ("command_timeout_seconds", "max_command_output_bytes"):
        if type(execution.get(field)) is not int or execution[field] <= 0:
            raise RuntimeError(f"graph edit execution {field} must be positive")
    for field in ("require_clean_fixture_before_each_case", "require_fresh_repository_per_run"):
        if execution.get(field) is not True:
            raise RuntimeError(f"graph edit execution {field} must be true")
    expected_counts = {
        "lowering_determinism": 8,
        "text_equivalence": 8,
        "span_safety": 6,
        "resolution_fail_closed": 4,
    }
    for field, count in expected_counts.items():
        values = corpus.get(field)
        if not isinstance(values, list) or len(values) != count or len(set(values)) != count:
            raise RuntimeError(f"graph edit corpus {field} must contain {count} unique ids")
    expected_properties = {
        "P6_lowering_determinism",
        "P7_text_equivalence",
        "P8_span_safety",
        "P9_resolution_fail_closed",
    }
    if set(properties) != expected_properties:
        raise RuntimeError("graph edit policy must define exactly P6 through P9")
    for name, thresholds in properties.items():
        if not isinstance(thresholds, dict):
            raise RuntimeError(f"{name} thresholds must be an object")
        for field, value in thresholds.items():
            if type(value) is not int or value < 0:
                raise RuntimeError(f"{name}.{field} must be a non-negative integer")
            if not field.startswith("required_") and value != 0:
                raise RuntimeError(f"{name}.{field} must remain zero tolerance")
    required = {
        "P6_lowering_determinism": {
            "required_plan_count": 8,
            "required_runs_per_plan": 3,
        },
        "P7_text_equivalence": {"required_pair_count": 8},
        "P8_span_safety": {"required_case_count": 6},
        "P9_resolution_fail_closed": {"required_case_count": 4},
    }
    for property_name, fields in required.items():
        for field, expected in fields.items():
            if properties[property_name].get(field) != expected:
                raise RuntimeError(f"{property_name}.{field} must remain {expected}")
    manifest = graph_corpus()
    if manifest.get("corpus_id") != data["policy_id"]:
        raise RuntimeError("graph corpus manifest must identify the measured policy")
    manifest_cases = {
        "lowering_determinism": list(manifest.get("paired_cases", {})),
        "text_equivalence": list(manifest.get("paired_cases", {})),
        "span_safety": list(manifest.get("span_safety", {})),
        "resolution_fail_closed": list(manifest.get("resolution_fail_closed", {})),
    }
    if any(set(manifest_cases[field]) != set(corpus[field]) for field in manifest_cases):
        raise RuntimeError("graph policy corpus ids differ from graph-edit-corpus.json")
    expected_mutation_adequacy = {
        "required_property_count": 4,
        "max_surviving_mutants": 0,
        "mutants": {
            "P6_lowering_determinism": "drop-write-fingerprints",
            "P7_text_equivalence": "corrupt-graph-result",
            "P8_span_safety": "skip-second-graph-edit",
            "P9_resolution_fail_closed": "accept-resolution-failure",
        },
    }
    if data.get("mutation_adequacy") != expected_mutation_adequacy:
        raise RuntimeError("graph edit mutation adequacy differs from the precommitment")
    return data


def validate_selected_policy(data: Mapping[str, Any]) -> Mapping[str, Any]:
    policy_id = data.get("policy_id")
    if policy_id == "plan-executor-v1":
        return validate_policy(data)
    if policy_id == "graph-edit-v2":
        return validate_graph_policy(data)
    raise RuntimeError(f"unsupported policy_id: {policy_id!r}")


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
    elif case_id == "delete-recreate-across-steps-then-fail":
        return [
            {
                "id": "delete-tracked",
                "edits": [{"path": "src/doomed.rs", "delete": True}],
                "checks": [{"kind": "command", "run": "true"}],
            },
            {"id": "intervening-step"},
            {
                "id": "recreate-tracked",
                "edits": [
                    {
                        "path": "src/doomed.rs",
                        "create": "pub fn recreated_doomed() {}\n",
                    }
                ],
                "checks": [{"kind": "command", "run": "true"}],
            },
            {
                "id": "trigger-rollback",
                "checks": [{"kind": "command", "run": "false"}],
            },
        ]
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
import json
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
    plan = json.loads(Path(ARGS[2]).read_text(encoding="utf-8"))
    if plan.get("plan_id") == "delete-recreate-across-steps-then-fail":
        (Path.cwd() / "src" / "doomed.rs").unlink(missing_ok=True)

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
        observed = measurement(policy, work_root / property_name.lower(), mutant, options)
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
        cases.append({
            "property": property_name,
            "mutant_sha256": sha256_file(mutant),
            "killed": killed,
            "violations": introduced,
            "observed": {key: value for key, value in observed.items() if key != "cases"},
        })
    if survivors:
        raise RuntimeError(
            "refusing to emit observation because property mutants survived: "
            + ", ".join(survivors)
        )
    return {"required_property_count": len(measurements), "survivors": 0, "cases": cases}


def initialize_graph_repository(root: Path, language: str, options: Mapping[str, Any]) -> str:
    root.mkdir(parents=True)
    (root / "src").mkdir()
    (root / ".gitignore").write_text(".bitcode/\n", encoding="utf-8")
    fixtures = graph_corpus()["fixture"]
    if language not in fixtures:
        raise RuntimeError(f"unknown graph fixture language: {language}")
    extension = "rs" if language == "rust" else "py"
    (root / "src" / f"lib.{extension}").write_text(fixtures[language]["lib"], encoding="utf-8")
    (root / "src" / f"doomed.{extension}").write_text(
        fixtures[language]["doomed"], encoding="utf-8"
    )
    (root / "bitcode.toml").write_text(
        "version = 1\n[source]\nroots = [\"src\"]\n", encoding="utf-8"
    )
    for command in (
        ("git", "init", "--quiet"),
        ("git", "config", "user.email", "graph-oracle@bitcode.invalid"),
        ("git", "config", "user.name", "Bit Code Graph Oracle"),
        ("git", "add", "."),
        ("git", "commit", "--quiet", "-m", "baseline"),
    ):
        require_success(run(command, cwd=root, check=False, **options), " ".join(command))
    return require_success(
        run(("git", "rev-parse", "HEAD"), cwd=root, check=False, **options),
        "resolve graph baseline",
    ).stdout.strip()


def graph_and_text_edits(case_id: str) -> tuple[str, Mapping[str, Any], Mapping[str, Any]]:
    try:
        case = graph_corpus()["paired_cases"][case_id]
    except KeyError as error:
        raise RuntimeError(f"unknown graph edit corpus case: {case_id}") from error
    return case["language"], case["graph_edit"], case["text_edit"]


def paired_plan(
    case_id: str, base_commit: str, *, graph_addressed: bool
) -> tuple[str, dict[str, Any]]:
    language, graph_edit, text_edit = graph_and_text_edits(case_id)
    edit = graph_edit if graph_addressed else text_edit
    plan = {
        "plan_version": 2 if graph_addressed else 1,
        "plan_id": f"{case_id}-{'graph' if graph_addressed else 'text'}",
        "intent": f"measure paired {case_id}",
        "base_commit": base_commit,
        "on_failure": "rollback_plan",
        "steps": [{"id": "apply-change", "edits": [edit]}],
    }
    return language, plan


def source_tree_digest(repository: Path) -> str:
    digest = hashlib.sha256()
    listed = require_success(
        run_bounded(
            ("git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"),
            cwd=repository,
            timeout_seconds=30,
            max_output_bytes=1024 * 1024,
        ),
        "list tracked tree",
    )
    paths = sorted(
        repository / value
        for value in listed.stdout.split("\0")
        if value and not value.startswith(".bitcode/")
    )
    for path in paths:
        relative = path.relative_to(repository).as_posix().encode()
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        if not path.is_file():
            digest.update(b"\x00")
            continue
        digest.update(b"\x01")
        contents = path.read_bytes()
        digest.update(len(contents).to_bytes(8, "big"))
        digest.update(contents)
    return digest.hexdigest()


def valid_fingerprint_side(byte_count: Any, digest: Any) -> bool:
    return (byte_count is None and digest is None) or (
        type(byte_count) is int
        and byte_count >= 0
        and isinstance(digest, str)
        and len(digest) == 64
        and all(character in "0123456789abcdef" for character in digest)
    )


def measure_p6(
    policy: Mapping[str, Any], work_root: Path, bitcode: Path, options: Mapping[str, Any]
) -> dict[str, Any]:
    cases = []
    mismatches = 0
    runs_per_plan = policy["execution"]["repeated_runs"]
    for case_id in policy["corpus"]["lowering_determinism"]:
        fingerprints = []
        for ordinal in range(1, runs_per_plan + 1):
            language, _ = graph_and_text_edits(case_id)[:2]
            repository = work_root / f"{case_id}-{ordinal}"
            base_commit = initialize_graph_repository(repository, language, options)
            _, plan = paired_plan(case_id, base_commit, graph_addressed=True)
            if case_id == "rust-replace":
                plan["steps"][0]["edits"].append(
                    graph_corpus()["lowering_determinism"]["rust-replace-extra-edit"]
                )
            plan_path = write_plan(work_root, f"{case_id}-{ordinal}", plan)
            result = require_success(
                run((str(bitcode), "plan", "run", str(plan_path), "--dry"),
                    cwd=repository, check=False, **options),
                f"P6 {case_id} run {ordinal}",
            )
            report = parse_dry_report(result.stdout, result.stderr)
            writes = report["steps"][0].get("writes")
            if not isinstance(writes, list) or not writes:
                raise RuntimeError(f"P6 {case_id} omitted non-empty write fingerprints")
            for write in writes:
                if (
                    not isinstance(write, dict)
                    or set(write) != {
                        "path", "before_bytes", "before_sha256", "after_bytes", "after_sha256"
                    }
                    or not isinstance(write["path"], str)
                    or not valid_fingerprint_side(
                        write["before_bytes"], write["before_sha256"]
                    )
                    or not valid_fingerprint_side(
                        write["after_bytes"], write["after_sha256"]
                    )
                ):
                    raise RuntimeError(f"P6 {case_id} emitted malformed write fingerprint")
            if case_id == "rust-replace" and len(writes) < 2:
                raise RuntimeError("P6 cross-file case emitted fewer than two writes")
            fingerprints.append(writes)
        matched = all(value == fingerprints[0] for value in fingerprints[1:])
        mismatches += int(not matched)
        cases.append({"id": case_id, "matched": matched, "fingerprints": fingerprints})
    return {
        "plan_count": len(cases),
        "runs_per_plan": runs_per_plan,
        "fingerprint_mismatches": mismatches,
        "cases": cases,
    }


def measure_p7(
    policy: Mapping[str, Any], work_root: Path, bitcode: Path, options: Mapping[str, Any]
) -> dict[str, Any]:
    cases = []
    mismatches = 0
    for case_id in policy["corpus"]["text_equivalence"]:
        digests = {}
        for mode in ("text", "graph"):
            language, _ = graph_and_text_edits(case_id)[:2]
            repository = work_root / f"{case_id}-{mode}"
            base_commit = initialize_graph_repository(repository, language, options)
            _, plan = paired_plan(case_id, base_commit, graph_addressed=mode == "graph")
            plan_path = write_plan(work_root, f"{case_id}-{mode}", plan)
            require_success(
                run((str(bitcode), "plan", "run", str(plan_path)),
                    cwd=repository, check=False, **options),
                f"P7 {case_id} {mode}",
            )
            digests[mode] = source_tree_digest(repository)
        matched = digests["text"] == digests["graph"]
        mismatches += int(not matched)
        cases.append({"id": case_id, "matched": matched, "tree_sha256": digests})
    return {"pair_count": len(cases), "tree_mismatches": mismatches, "cases": cases}


def span_safety_plan(case_id: str, base_commit: str) -> tuple[str, dict[str, Any], bool]:
    try:
        case = graph_corpus()["span_safety"][case_id]
    except KeyError as error:
        raise RuntimeError(f"unknown span-safety corpus case: {case_id}") from error
    language = case["language"]
    steps = case["steps"]
    should_pass = case["should_pass"]
    return language, {
        "plan_version": 2,
        "plan_id": case_id,
        "intent": "measure span safety",
        "base_commit": base_commit,
        "on_failure": "rollback_plan",
        "steps": steps,
    }, should_pass


def measure_p8(
    policy: Mapping[str, Any], work_root: Path, bitcode: Path, options: Mapping[str, Any]
) -> dict[str, Any]:
    cases = []
    corruptions = 0
    rule_mismatches = 0
    for case_id in policy["corpus"]["span_safety"]:
        language = "python" if case_id.startswith("python") else "rust"
        case = graph_corpus()["span_safety"][case_id]
        outcomes = {}
        corrupted = False
        should_pass = case["should_pass"]
        expected_digest = None
        if should_pass:
            expected_repository = work_root / f"{case_id}-expected"
            initialize_graph_repository(expected_repository, language, options)
            for relative, contents in case["expected_files"].items():
                (expected_repository / relative).write_text(contents, encoding="utf-8")
            expected_digest = source_tree_digest(expected_repository)
        for mode in ("dry", "real"):
            repository = work_root / f"{case_id}-{mode}"
            base_commit = initialize_graph_repository(repository, language, options)
            fixture = case.get("fixture")
            if fixture is not None:
                source_path = repository / "src/lib.rs"
                source_path.write_text(fixture, encoding="utf-8")
                require_success(run(("git", "add", "."), cwd=repository,
                                    check=False, **options), f"stage {case_id} fixture")
                require_success(run(("git", "commit", "--quiet", "-m", case_id),
                                    cwd=repository, check=False, **options),
                                f"commit {case_id} fixture")
                base_commit = git_output(repository, options, "rev-parse", "HEAD")
            _, plan, should_pass = span_safety_plan(case_id, base_commit)
            plan_path = write_plan(work_root, f"{case_id}-{mode}", plan)
            before_digest = source_tree_digest(repository)
            command = [str(bitcode), "plan", "run", str(plan_path)]
            if mode == "dry":
                command.append("--dry")
            result = run(command, cwd=repository, check=False, **options)
            outcomes[mode] = result.returncode == 0
            after_digest = source_tree_digest(repository)
            if mode == "dry" or not should_pass:
                corrupted |= after_digest != before_digest
            else:
                corrupted |= expected_digest is None or after_digest != expected_digest
        matched = outcomes["dry"] == outcomes["real"] == should_pass
        rule_mismatches += int(not matched)
        corruptions += int(corrupted)
        cases.append({"id": case_id, "expected_pass": should_pass, "passed": outcomes,
                      "rule_matched": matched, "corrupted": corrupted})
    return {"case_count": len(cases), "corruptions": corruptions,
            "rule_mismatches": rule_mismatches, "cases": cases}


def resolution_failure_fixture(
    case_id: str, repository: Path, options: Mapping[str, Any]
) -> tuple[str, str, dict[str, Any], str]:
    case = graph_corpus()["resolution_fail_closed"][case_id]
    if case_id == "ambiguous-node":
        base_commit = initialize_graph_repository(repository, "python", options)
        (repository / "src/lib.py").write_text(case["fixture"], encoding="utf-8")
        require_success(run(("git", "add", "."), cwd=repository, check=False, **options), "stage")
        require_success(run(("git", "commit", "--quiet", "-m", "ambiguous"),
                            cwd=repository, check=False, **options), "commit ambiguous")
        base_commit = git_output(repository, options, "rev-parse", "HEAD")
    elif case_id == "spanless-node":
        base_commit = initialize_graph_repository(repository, "rust", options)
        (repository / case["fixture_path"]).write_text(case["fixture"], encoding="utf-8")
        require_success(run(("git", "add", "."), cwd=repository, check=False, **options), "stage")
        require_success(run(("git", "commit", "--quiet", "-m", "empty"),
                            cwd=repository, check=False, **options), "commit empty")
        base_commit = git_output(repository, options, "rev-parse", "HEAD")
    elif case_id == "unsupported-language":
        base_commit = initialize_graph_repository(repository, "rust", options)
        (repository / case["fixture_path"]).write_text(case["fixture"], encoding="utf-8")
        require_success(run(("git", "add", "."), cwd=repository, check=False, **options), "stage")
        require_success(run(("git", "commit", "--quiet", "-m", "unsupported"),
                            cwd=repository, check=False, **options), "commit unsupported")
        base_commit = git_output(repository, options, "rev-parse", "HEAD")
    else:
        base_commit = initialize_graph_repository(repository, "rust", options)
    node = case["node"]
    plan = {
        "plan_version": 2, "plan_id": case_id, "intent": "reject resolution",
        "base_commit": base_commit, "steps": [{"id": "reject-resolution", "edits": [{
            "node": node,
            "insert_into_module" if case_id == "spanless-node" else "delete_node":
                "fn inserted() {}\n" if case_id == "spanless-node" else True,
        }]}],
    }
    return base_commit, node, plan, case["category"]


def measure_p9(
    policy: Mapping[str, Any], work_root: Path, bitcode: Path, options: Mapping[str, Any]
) -> dict[str, Any]:
    cases = []
    unexpected_passes = missing_nodes = missing_steps = missing_categories = 0
    for case_id in policy["corpus"]["resolution_fail_closed"]:
        repository = work_root / case_id
        _, node, plan, expected_category = resolution_failure_fixture(case_id, repository, options)
        plan_path = write_plan(work_root, case_id, plan)
        result = run((str(bitcode), "plan", "validate", str(plan_path)),
                     cwd=repository, check=False, **options)
        diagnostic = f"{result.stdout}\n{result.stderr}"
        rejected = result.returncode != 0
        named_node = node in diagnostic
        named_step = "reject-resolution" in diagnostic
        named_category = expected_category in diagnostic
        unexpected_passes += int(not rejected)
        missing_nodes += int(not named_node)
        missing_steps += int(not named_step)
        missing_categories += int(not named_category)
        cases.append({"id": case_id, "rejected": rejected, "named_node": named_node,
                      "named_step": named_step, "named_category": named_category})
    return {"case_count": len(cases), "unexpected_passes": unexpected_passes,
            "missing_node_diagnostics": missing_nodes,
            "missing_step_diagnostics": missing_steps,
            "missing_category_diagnostics": missing_categories, "cases": cases}


def create_graph_mutant_binary(root: Path, bitcode: Path, mutant_id: str) -> Path:
    mutant = root / f"bitcode-mutant-{mutant_id}"
    script = f'''#!/usr/bin/env python3
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

MUTATION = {mutant_id!r}
REAL_BITCODE = {str(bitcode)!r}
ARGS = sys.argv[1:]
MARKER = "report (dry run, not written to disk):\\n"

is_validate = len(ARGS) >= 2 and ARGS[:2] == ["plan", "validate"]
is_run = len(ARGS) >= 3 and ARGS[:2] == ["plan", "run"]
is_dry = is_run and "--dry" in ARGS
temporary_plan = None

if MUTATION == "accept-resolution-failure" and is_validate:
    raise SystemExit(0)

if MUTATION == "skip-second-graph-edit" and is_run:
    plan_path = Path(ARGS[2])
    plan = json.loads(plan_path.read_text(encoding="utf-8"))
    changed = False
    for step in plan.get("steps", []):
        edits = step.get("edits", [])
        if len(edits) > 1:
            step["edits"] = edits[:-1]
            changed = True
            break
    if changed:
        handle = tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", encoding="utf-8", delete=False
        )
        json.dump(plan, handle)
        handle.close()
        temporary_plan = handle.name
        ARGS[2] = temporary_plan

completed = subprocess.run(
    [REAL_BITCODE, *ARGS], check=False, capture_output=True, text=True
)
stdout = completed.stdout
stderr = completed.stderr

if MUTATION == "drop-write-fingerprints" and is_dry and completed.returncode == 0:
    before, tail = stdout.split(MARKER, 1)
    report, offset = json.JSONDecoder().raw_decode(tail.lstrip())
    for step in report.get("steps", []):
        step.pop("writes", None)
    stdout = before + MARKER + json.dumps(report) + tail.lstrip()[offset:]

if MUTATION == "corrupt-graph-result" and is_run and not is_dry and completed.returncode == 0:
    plan = json.loads(Path(ARGS[2]).read_text(encoding="utf-8"))
    if plan.get("plan_version") == 2:
        for relative in ("src/lib.rs", "src/lib.py"):
            target = Path.cwd() / relative
            if target.is_file():
                target.write_text(
                    target.read_text(encoding="utf-8") + "\\n# graph result mutant\\n",
                    encoding="utf-8",
                )
                break

sys.stdout.write(stdout)
sys.stderr.write(stderr)
if temporary_plan is not None:
    os.unlink(temporary_plan)
raise SystemExit(completed.returncode)
'''
    mutant.write_text(script, encoding="utf-8")
    mutant.chmod(mutant.stat().st_mode | stat.S_IXUSR)
    return mutant


def measure_graph_mutation_adequacy(
    policy: Mapping[str, Any],
    work_root: Path,
    bitcode: Path,
    options: Mapping[str, Any],
    baseline_results: Mapping[str, Any],
) -> dict[str, Any]:
    work_root.mkdir(parents=True)
    measurements = {
        "P6_lowering_determinism": measure_p6,
        "P7_text_equivalence": measure_p7,
        "P8_span_safety": measure_p8,
        "P9_resolution_fail_closed": measure_p9,
    }
    baseline_violations = set(evaluate_graph_policy(policy, baseline_results))
    cases = []
    survivors = []
    for property_name, measurement in measurements.items():
        mutant_id = policy["mutation_adequacy"]["mutants"][property_name]
        mutant = create_graph_mutant_binary(work_root, bitcode, mutant_id)
        try:
            observed = measurement(
                policy,
                work_root / property_name.lower(),
                mutant,
                options,
            )
        except RuntimeError as error:
            observed = {"measurement_error": str(error)}
            introduced = [f"{property_name} rejected mutant: {error}"]
        else:
            mutant_results = dict(baseline_results)
            mutant_results[property_name] = observed
            introduced = sorted(
                violation
                for violation in set(evaluate_graph_policy(policy, mutant_results))
                - baseline_violations
                if violation.startswith(f"{property_name}.")
            )
        killed = bool(introduced)
        if not killed:
            survivors.append(property_name)
        cases.append({
            "property": property_name,
            "mutant": mutant_id,
            "mutant_sha256": sha256_file(mutant),
            "killed": killed,
            "violations": introduced,
            "observed": {key: value for key, value in observed.items() if key != "cases"},
        })
    required = policy["mutation_adequacy"]["required_property_count"]
    maximum = policy["mutation_adequacy"]["max_surviving_mutants"]
    if len(cases) != required or len(survivors) > maximum:
        raise RuntimeError(
            "refusing to emit observation because graph property mutants survived: "
            + ", ".join(survivors)
        )
    return {"required_property_count": required, "survivors": len(survivors), "cases": cases}


def evaluate_graph_policy(policy: Mapping[str, Any], results: Mapping[str, Any]) -> list[str]:
    comparisons = {
        "P6_lowering_determinism": {
            "required_plan_count": ("plan_count", "equal"),
            "required_runs_per_plan": ("runs_per_plan", "equal"),
            "max_fingerprint_mismatches": ("fingerprint_mismatches", "maximum"),
        },
        "P7_text_equivalence": {
            "required_pair_count": ("pair_count", "equal"),
            "max_tree_mismatches": ("tree_mismatches", "maximum"),
        },
        "P8_span_safety": {
            "required_case_count": ("case_count", "equal"),
            "max_corruptions": ("corruptions", "maximum"),
            "max_rule_mismatches": ("rule_mismatches", "maximum"),
        },
        "P9_resolution_fail_closed": {
            "required_case_count": ("case_count", "equal"),
            "max_unexpected_passes": ("unexpected_passes", "maximum"),
            "max_missing_node_diagnostics": ("missing_node_diagnostics", "maximum"),
            "max_missing_step_diagnostics": ("missing_step_diagnostics", "maximum"),
            "max_missing_category_diagnostics": (
                "missing_category_diagnostics", "maximum"
            ),
        },
    }
    violations = []
    for property_name, fields in comparisons.items():
        for threshold_name, (result_name, comparison) in fields.items():
            expected = policy["properties"][property_name][threshold_name]
            actual = results[property_name][result_name]
            passed = actual == expected if comparison == "equal" else actual <= expected
            if not passed:
                violations.append(
                    f"{property_name}.{result_name}={actual} violates {threshold_name}={expected}"
                )
    return violations


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


def observation_metadata(policy: Mapping[str, Any], policy_bytes: bytes, bitcode_sha: str) -> dict[str, Any]:
    source = run_bounded(
        ("git", "rev-parse", "HEAD"),
        cwd=REPO_ROOT,
        timeout_seconds=30,
        max_output_bytes=64 * 1024,
    )
    source_commit = source.stdout.strip()
    if (
        source.returncode != 0
        or len(source_commit) != 40
        or any(character not in "0123456789abcdef" for character in source_commit)
    ):
        raise RuntimeError("refusing to write observation without source commit")
    status = run_bounded(
        ("git", "status", "--porcelain=v1", "--untracked-files=all"),
        cwd=REPO_ROOT,
        timeout_seconds=30,
        max_output_bytes=1024 * 1024,
    )
    if status.returncode != 0:
        raise RuntimeError("refusing to write observation without a successful git status")
    if status.stdout:
        raise RuntimeError(
            "refusing to write observation because the source tree is not the recorded commit"
        )
    policy_sha = hashlib.sha256(policy_bytes).hexdigest()
    if len(policy_sha) != 64:
        raise RuntimeError("refusing to write observation without policy SHA-256")
    corpus_bytes = (
        GRAPH_CORPUS.read_bytes()
        if policy["policy_id"] == "graph-edit-v2"
        else json.dumps(policy["corpus"], sort_keys=True, separators=(",", ":")).encode()
    )
    return {
        "policy_sha256": policy_sha,
        "corpus_manifest_sha256": hashlib.sha256(corpus_bytes).hexdigest(),
        "host": {
            "hostname": socket.gethostname(),
            "platform": platform.platform(),
            "python": platform.python_version(),
            "cpu_count": os.cpu_count(),
        },
        "run_definition": "fresh-repository-per-case-private-binary-bounded-subprocesses",
        "tool": {
            "bitcode_sha256": bitcode_sha,
            "harness_sha256": sha256_file(Path(__file__)),
            "harness_support_sha256": sha256_file(REPO_ROOT / "tools" / "harness_support.py"),
            "source_commit": source_commit,
            "source_worktree_clean": True,
            "source_diff_sha256": hashlib.sha256(
                require_success(
                    run_bounded(
                        ("git", "diff", "--binary", "HEAD"),
                        cwd=REPO_ROOT,
                        timeout_seconds=30,
                        max_output_bytes=16 * 1024 * 1024,
                    ),
                    "capture measured source diff",
                ).stdout.encode()
            ).hexdigest(),
            "source_snapshot_sha256": source_snapshot_sha256(REPO_ROOT),
            "argv": ["bitcode", "plan", "validate|run", "<plan>", "[--dry]"],
        },
    }


def ollama_chat_json(
    host: str, payload: Mapping[str, Any], timeout: int
) -> Mapping[str, Any]:
    request = urllib.request.Request(
        f"{host.rstrip('/')}/api/chat",
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            parsed = json.loads(response.read())
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
        raise RuntimeError(f"Ollama authoring request failed: {error}") from error
    if not isinstance(parsed, dict):
        raise RuntimeError("Ollama authoring response must be an object")
    return parsed


def recover_ollama_prompt_tokens(
    host: str,
    model: str,
    prompt: str,
    response_schema: Mapping[str, Any],
    options: Mapping[str, Any],
) -> int:
    stopped = run_bounded(
        ("ollama", "stop", model),
        cwd=REPO_ROOT,
        timeout_seconds=30,
        max_output_bytes=64 * 1024,
    )
    if stopped.returncode != 0:
        raise RuntimeError(
            "could not stop the timed-out local model before recovering prompt tokens"
        )
    response = ollama_chat_json(
        host,
        {
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "stream": False,
            "format": response_schema,
            "keep_alive": 0,
            "options": {
                "temperature": options["temperature"],
                "seed": options["seed"],
                "num_predict": 1,
            },
        },
        options["timeout_seconds"],
    )
    tokens = response.get("prompt_eval_count")
    if type(tokens) is not int or tokens < 0:
        raise RuntimeError("Ollama token-recovery response omitted prompt_eval_count")
    return tokens


def initialize_authoring_repository(root: Path, options: Mapping[str, Any]) -> str:
    require_success(
        run(("git", "clone", "--quiet", "--no-hardlinks", str(REPO_ROOT), str(root)),
            cwd=REPO_ROOT, check=False, **options),
        "clone Bit Code authoring fixture",
    )
    return git_output(root, options, "rev-parse", "HEAD")


def authoring_edits(case_id: str) -> tuple[str, Mapping[str, Any], Mapping[str, Any]]:
    language, operation = case_id.split("-", 1)
    if language == "rust":
        path = "crates/aether-debugger/src/trace.rs"
        module = "crate::crates::aether-debugger::src::trace"
        node = f"{module}::Trace::final_env"
        old = (
            "pub fn final_env(&self) -> Env {\n"
            "        self.steps.last().map(|s| s.env.clone()).unwrap_or_default()\n"
            "    }"
        )
        replacement = (
            "pub fn final_env(&self) -> Env {\n"
            "        self.steps.last().map_or_else(Env::new, |step| step.env.clone())\n"
            "    }"
        )
        terminal = (
            "    pub fn is_empty(&self) -> bool {\n"
            "        self.steps.is_empty()\n"
            "    }\n"
            "}\n"
        )
        insertion = "\npub fn trace_fixture_marker() -> usize { 1 }\n"
        rename_old = "final_env"
        rename_new = "completed_env"
    elif language == "python":
        path = "sample-project/calc.py"
        module = "crate::sample-project::calc"
        node = f"{module}::greet"
        old = "def greet(name):\n    return hello(name)"
        replacement = "def greet(name):\n    return hello(name).upper()"
        terminal = (
            "class ScientificCalculator(Calculator):\n"
            "    def square(self, value):\n"
            "        return value * value\n"
        )
        insertion = "\n\ndef sample_fixture_marker():\n    return 1\n"
        rename_old = "greet"
        rename_new = "welcome_greeting"
    else:
        raise RuntimeError(f"unknown authoring task language: {language}")

    if operation == "replace":
        return language, {"node": node, "replace_node": replacement}, {
            "path": path, "match": old, "replace": replacement
        }
    if operation == "rename":
        rename_node = (
            f"{module}::Trace::{rename_old}" if language == "rust" else f"{module}::{rename_old}"
        )
        occurrences = 1
        return language, {"node": rename_node, "rename_node": rename_new}, {
            "path": path, "match": rename_old, "replace": rename_new,
            "occurrences": occurrences,
        }
    if operation == "delete":
        return language, {"node": node, "delete_node": True}, {
            "path": path, "match": old, "replace": ""
        }
    if operation == "insert":
        return language, {"node": module, "insert_into_module": insertion}, {
            "path": path, "match": terminal, "replace": terminal + insertion
        }
    raise RuntimeError(f"unknown authoring task operation: {operation}")


def authoring_plan(case_id: str, base_commit: str, *, graph_addressed: bool) -> dict[str, Any]:
    _, graph_edit, text_edit = authoring_edits(case_id)
    return {
        "plan_version": 2 if graph_addressed else 1,
        "plan_id": f"authoring-{case_id}-{'graph' if graph_addressed else 'text'}",
        "intent": f"measure authoring cost for {case_id}",
        "base_commit": base_commit,
        "on_failure": "rollback_plan",
        "steps": [
            {
                "id": "apply-change",
                "edits": [graph_edit if graph_addressed else text_edit],
                "checks": [
                    {
                        "kind": "command",
                        "run": f"python3 tools/authoring_task_check.py {case_id}",
                    }
                ],
            }
        ],
    }


def authoring_plan_json_schema(plan: Mapping[str, Any]) -> dict[str, Any]:
    """Build the exact structural grammar for one authoring plan envelope."""

    def schema_for(value: Any) -> dict[str, Any]:
        if isinstance(value, dict):
            return {
                "type": "object",
                "required": list(value),
                "additionalProperties": False,
                "properties": {key: schema_for(item) for key, item in value.items()},
            }
        if isinstance(value, list):
            if len(value) != 1:
                raise RuntimeError("authoring plan grammar requires singleton arrays")
            return {
                "type": "array",
                "minItems": 1,
                "maxItems": 1,
                "items": schema_for(value[0]),
            }
        if type(value) is bool:
            return {"type": "boolean"}
        if type(value) is int:
            return {"type": "integer"}
        if isinstance(value, str):
            return {"type": "string"}
        raise RuntimeError(f"unsupported authoring plan grammar value: {type(value)!r}")

    return schema_for(dict(plan))


def validate_authoring_policy(policy: Mapping[str, Any]) -> Mapping[str, Any]:
    required_fields = {
        "schema_version", "policy_id", "model", "model_manifest_sha256",
        "options", "prompt_protocol", "tasks", "task_intents", "task_sources",
        "semantic_verification", "success",
    }
    if set(policy) != required_fields:
        raise RuntimeError("authoring-cost policy has missing or unknown fields")
    if policy.get("schema_version") != 2 or policy.get("policy_id") != "graph-edit-authoring-cost-v2":
        raise RuntimeError("unsupported authoring-cost policy")
    exact_tasks = [
        f"{language}-{operation}"
        for language in ("rust", "python")
        for operation in ("replace", "rename", "delete", "insert")
    ]
    if policy.get("tasks") != exact_tasks:
        raise RuntimeError("authoring-cost policy must contain the exact eight-task corpus")
    if policy.get("task_intents") != AUTHORING_TASK_INTENTS:
        raise RuntimeError("authoring-cost task intents differ from the precommitment")
    if policy.get("prompt_protocol") != AUTHORING_PROMPT_PROTOCOL:
        raise RuntimeError("authoring-cost prompt protocol differs from the precommitment")
    if policy.get("model") != AUTHORING_MODEL:
        raise RuntimeError("authoring-cost model differs from the precommitment")
    exact_success = {
        "minimum_common_successes": 4,
        "require_graph_solves_every_text_success": True,
        "require_graph_total_input_tokens_lower": True,
    }
    if policy.get("success") != exact_success:
        raise RuntimeError("authoring-cost success thresholds differ from the precommitment")
    exact_options = {
        "temperature": 0,
        "seed": 42,
        "num_predict": 1024,
        "max_attempts": 3,
        "timeout_seconds": 300,
        "max_repair_diagnostic_chars": 2048,
    }
    if policy.get("options") != exact_options:
        raise RuntimeError("authoring-cost sampling and repair options differ from policy")
    manifest = policy.get("model_manifest_sha256")
    if not isinstance(manifest, str) or len(manifest) != 64 or any(
        character not in "0123456789abcdef" for character in manifest
    ):
        raise RuntimeError("authoring-cost model manifest must be an exact SHA-256")
    if policy.get("task_sources") != AUTHORING_TASK_SOURCES:
        raise RuntimeError("authoring-cost task sources differ from the precommitment")
    if policy.get("semantic_verification") != AUTHORING_SEMANTIC_VERIFICATION:
        raise RuntimeError("authoring-cost semantic verification differs from the precommitment")
    return policy


def authoring_prompt_context(
    arm: str,
    language: str,
    allowed_edit: Mapping[str, Any],
    repository: Path,
    *,
    case_id: str | None = None,
    intent: str | None = None,
) -> Mapping[str, Any]:
    if arm == "graph":
        if case_id is None or intent is None:
            raise RuntimeError("graph authoring context requires case id and intent")
        return {
            "node": allowed_edit["node"],
            "language": language,
            "intent": intent,
            "source": authoring_target_node_source(case_id, repository),
        }
    if arm == "text":
        path = allowed_edit["path"]
        return {
            "path": path,
            "projection": (repository / path).read_text(encoding="utf-8"),
        }
    raise RuntimeError(f"unsupported authoring arm: {arm}")


def authoring_target_node_source(case_id: str, repository: Path) -> str:
    """Return bounded current graph-node source without projecting the target file."""
    language, operation = case_id.split("-", 1)
    projection = (
        repository / AUTHORING_TASK_SOURCES[language]["path"]
    ).read_text(encoding="utf-8")
    if language == "rust" and operation == "insert":
        node_source = (
            "pub fn is_empty(&self) -> bool {\n"
            "        self.steps.is_empty()\n"
            "    }"
        )
    elif language == "rust":
        node_source = (
            "pub fn final_env(&self) -> Env {\n"
            "        self.steps.last().map(|s| s.env.clone()).unwrap_or_default()\n"
            "    }"
        )
    elif language == "python" and operation == "insert":
        node_source = (
            "class ScientificCalculator(Calculator):\n"
            "    def square(self, value):\n"
            "        return value * value"
        )
    elif language == "python":
        node_source = "def greet(name):\n    return hello(name)"
    else:
        raise RuntimeError(f"unsupported authoring task: {case_id}")
    if projection.count(node_source) != 1:
        raise RuntimeError(f"authoring target Node.source drifted for {case_id}")
    if node_source == projection:
        raise RuntimeError(f"authoring target Node.source is the full {language} projection")
    return node_source


def authoring_progress_header(
    provenance: Mapping[str, str],
    model_manifest_sha256: str,
    bitcode: Path,
) -> dict[str, str]:
    return {
        "policy_sha256": provenance["policy_sha256"],
        "source_commit": provenance["source_commit"],
        "model_manifest_sha256": model_manifest_sha256,
        "bitcode_sha256": sha256_file(bitcode),
        "harness_sha256": sha256_file(Path(__file__)),
        "harness_support_sha256": sha256_file(
            REPO_ROOT / "tools" / "harness_support.py"
        ),
        "authoring_task_check_sha256": sha256_file(
            REPO_ROOT / "tools" / "authoring_task_check.py"
        ),
    }


def authored_plan_shape_error(
    generated: Any, canonical: Mapping[str, Any], arm: str
) -> str | None:
    if not isinstance(generated, dict):
        return "plan output is not an object"
    if set(generated) != set(canonical):
        return "plan envelope has missing or extra fields"
    if generated.get("plan_version") != canonical["plan_version"]:
        return f"{arm} arm used the wrong plan_version"
    if generated.get("base_commit") != canonical["base_commit"]:
        return "plan used the wrong base_commit"
    if generated.get("on_failure") != "rollback_plan":
        return "plan must use rollback_plan"
    steps = generated.get("steps")
    if not isinstance(steps, list) or len(steps) != 1 or not isinstance(steps[0], dict):
        return "plan must contain exactly one step"
    if set(steps[0]) != {"id", "edits", "checks"}:
        return "authoring measurement requires edits and declared checks only"
    edits = steps[0].get("edits")
    if not isinstance(edits, list) or len(edits) != 1 or not isinstance(edits[0], dict):
        return "plan must contain exactly one edit"
    expected = canonical["steps"][0]["edits"][0]
    if set(edits[0]) != set(expected):
        return f"{arm} arm used an unpermitted edit shape"
    address = "node" if arm == "graph" else "path"
    if edits[0].get(address) != expected[address]:
        return f"{arm} arm addressed the wrong {address}"
    if arm == "graph" and "path" in edits[0]:
        return "graph arm used a text-addressed edit"
    if arm == "text" and "node" in edits[0]:
        return "text arm used a graph-addressed edit"
    if steps[0].get("checks") != canonical["steps"][0]["checks"]:
        return "plan changed the required task-specific semantic check"
    return None


def authored_plan_envelope_error(generated: Any, canonical: Any, path: str = "plan") -> str | None:
    """Defensively verify the shape that Ollama's JSON grammar must enforce."""
    if isinstance(canonical, dict):
        if not isinstance(generated, dict):
            return f"{path} is not an object"
        if set(generated) != set(canonical):
            return f"{path} has missing or extra fields"
        for key in canonical:
            error = authored_plan_envelope_error(
                generated[key], canonical[key], f"{path}.{key}"
            )
            if error is not None:
                return error
        return None
    if isinstance(canonical, list):
        if not isinstance(generated, list) or len(generated) != len(canonical):
            return f"{path} does not have the structurally required length"
        for index, item in enumerate(canonical):
            error = authored_plan_envelope_error(
                generated[index], item, f"{path}[{index}]"
            )
            if error is not None:
                return error
        return None
    if type(generated) is not type(canonical):
        return f"{path} has the wrong JSON type"
    return None


def run_authoring_cost(args: argparse.Namespace) -> int:
    policy_bytes = AUTHORING_POLICY.read_bytes()
    policy = validate_authoring_policy(json.loads(policy_bytes))
    provenance = observation_provenance(AUTHORING_POLICY)
    options = policy["options"]
    prompt_protocol = policy["prompt_protocol"]
    try:
        with urllib.request.urlopen(f"{args.ollama_host.rstrip('/')}/api/tags", timeout=5) as response:
            tags = json.loads(response.read())
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
        raise RuntimeError(f"authoring-cost measurement requires reachable Ollama: {error}") from error
    models = [model.get("name") for model in tags.get("models", []) if isinstance(model, dict)]
    if policy["model"] not in models:
        raise RuntimeError(
            f"authoring-cost measurement requires exact model {policy['model']!r}; available={models!r}"
        )
    manifest_digest = tags["models"][models.index(policy["model"])].get("digest")
    if not isinstance(manifest_digest, str) or not manifest_digest:
        raise RuntimeError("refusing authoring-cost measurement without model manifest digest")
    normalized_manifest = manifest_digest.removeprefix("sha256:")
    if normalized_manifest != policy["model_manifest_sha256"]:
        raise RuntimeError(
            "refusing authoring-cost measurement with a model manifest that differs from policy"
        )

    bitcode = args.bitcode.resolve()
    if not bitcode.is_file():
        raise RuntimeError(f"authoring-cost Bit Code binary does not exist: {bitcode}")
    progress_path = Path(tempfile.gettempdir()) / (
        "bitcode-authoring-cost-"
        f"{provenance['policy_sha256']}-{provenance['source_commit']}.json"
    )
    progress_header = authoring_progress_header(
        provenance, normalized_manifest, bitcode
    )
    completed_arms: dict[str, Any] = {}
    if progress_path.is_file():
        progress = json.loads(progress_path.read_text(encoding="utf-8"))
        if progress.get("header") != progress_header or not isinstance(
            progress.get("completed_arms"), dict
        ):
            raise RuntimeError("authoring-cost progress file belongs to another measurement")
        completed_arms = progress["completed_arms"]

    execution = {"timeout_seconds": 120.0, "max_output_bytes": 1024 * 1024}
    results = []
    status = require_success(
        run_bounded(("git", "status", "--porcelain=v1", "--untracked-files=all"),
                    cwd=REPO_ROOT, timeout_seconds=30, max_output_bytes=1024 * 1024),
        "check authoring-cost source worktree",
    )
    if status.stdout:
        raise RuntimeError(
            "refusing authoring-cost observation because the source tree is not the recorded commit"
        )
    for source in policy["task_sources"].values():
        if sha256_file(REPO_ROOT / source["path"]) != source["sha256"]:
            raise RuntimeError(f"authoring task source drifted: {source['path']}")
    with tempfile.TemporaryDirectory(prefix="bitcode-authoring-cost-") as directory:
        work_root = Path(directory)
        for case_id in policy["tasks"]:
            arms = {}
            for arm in ("text", "graph"):
                progress_key = f"{case_id}:{arm}"
                if progress_key in completed_arms:
                    arms[arm] = completed_arms[progress_key]
                    print(f"authoring {progress_key}: resumed", flush=True)
                    continue
                language, _, _ = authoring_edits(case_id)
                total_tokens = 0
                attempts = []
                success = False
                last_diagnostic = ""
                token_count_complete = True
                for ordinal in range(1, options["max_attempts"] + 1):
                    repository = work_root / f"{case_id}-{arm}-{ordinal}"
                    base_commit = initialize_authoring_repository(repository, execution)
                    canonical = authoring_plan(case_id, base_commit, graph_addressed=arm == "graph")
                    response_schema = authoring_plan_json_schema(canonical)
                    allowed_edit = canonical["steps"][0]["edits"][0]
                    operation = case_id.split("-", 1)[1]
                    context = json.dumps(
                        authoring_prompt_context(
                            arm,
                            language,
                            allowed_edit,
                            repository,
                            case_id=case_id,
                            intent=policy["task_intents"][case_id],
                        )
                    )
                    if arm == "text":
                        edit_schema = {
                            "path": "<project-relative path>",
                            "match": "<exact existing bytes>",
                            "replace": "<replacement bytes>",
                            "occurrences": "<positive integer>",
                        }
                    else:
                        graph_fields = {
                            "replace": {"replace_node": "<complete replacement>"},
                            "rename": {"rename_node": "<new identifier>"},
                            "delete": {"delete_node": True},
                            "insert": {"insert_into_module": "<source to append>"},
                        }
                        edit_schema = {"node": "<exact semantic path>", **graph_fields[operation]}
                    prompt_schema = {
                        "edit": edit_schema,
                        "required_check": canonical["steps"][0]["checks"][0],
                    }
                    prompt = prompt_protocol["initial_template"].format(
                        task=case_id,
                        intent=policy["task_intents"][case_id],
                        version=1 if arm == "text" else 2,
                        context=context,
                        schema=json.dumps(prompt_schema, sort_keys=True),
                        base_commit=base_commit,
                    )
                    if attempts:
                        prompt += prompt_protocol["repair_suffix"].format(
                            diagnostic=last_diagnostic[
                                : options["max_repair_diagnostic_chars"]
                            ]
                        )
                    if arm == "graph":
                        pinned_projection = (
                            repository / policy["task_sources"][language]["path"]
                        ).read_text(encoding="utf-8")
                        if pinned_projection in prompt:
                            raise RuntimeError(
                                f"graph authoring prompt leaked the {language} target projection"
                            )
                    try:
                        response = ollama_chat_json(
                            args.ollama_host,
                            {
                                "model": policy["model"],
                                "messages": [{"role": "user", "content": prompt}],
                                "stream": False,
                                "format": response_schema,
                                "options": {
                                    "temperature": options["temperature"],
                                    "seed": options["seed"],
                                    "num_predict": options["num_predict"],
                                },
                            },
                            options["timeout_seconds"],
                        )
                    except RuntimeError as error:
                        last_diagnostic = str(error)
                        try:
                            tokens = recover_ollama_prompt_tokens(
                                args.ollama_host,
                                policy["model"],
                                prompt,
                                response_schema,
                                options,
                            )
                        except RuntimeError as recovery_error:
                            tokens = None
                            token_count_complete = False
                            last_diagnostic += f"; token recovery failed: {recovery_error}"
                        else:
                            total_tokens += tokens
                        attempts.append({
                            "attempt": ordinal,
                            "tokens": tokens,
                            "success": False,
                            "provider_error": last_diagnostic,
                            "prompt_tokens_recovered": tokens is not None,
                        })
                        break
                    tokens = response.get("prompt_eval_count")
                    if type(tokens) is not int or tokens < 0:
                        raise RuntimeError("Ollama response omitted prompt_eval_count")
                    total_tokens += tokens
                    message = response.get("message")
                    if not isinstance(message, dict):
                        raise RuntimeError("Ollama chat response omitted message object")
                    try:
                        generated = json.loads(message.get("content", ""))
                    except (json.JSONDecodeError, TypeError) as error:
                        raise RuntimeError(
                            "schema-constrained Ollama response was not valid JSON"
                        ) from error
                    envelope_error = authored_plan_envelope_error(generated, canonical)
                    if envelope_error is not None:
                        raise RuntimeError(
                            "schema-constrained Ollama response violated its envelope: "
                            f"{envelope_error}"
                        )
                    shape_error = authored_plan_shape_error(generated, canonical, arm)
                    if shape_error is not None:
                        last_diagnostic = shape_error
                        attempts.append({
                            "attempt": ordinal,
                            "tokens": tokens,
                            "success": False,
                            "shape_error": shape_error,
                        })
                        continue
                    plan_path = write_plan(work_root, f"authored-{case_id}-{arm}-{ordinal}", generated)
                    validated = run((str(bitcode), "plan", "validate", str(plan_path)),
                                    cwd=repository, check=False, **execution)
                    executed = (
                        run((str(bitcode), "plan", "run", str(plan_path)),
                            cwd=repository, check=False, **execution)
                        if validated.returncode == 0
                        else None
                    )
                    declared_checks_passed = (
                        executed is not None and executed.returncode == 0
                    )
                    impacted = (
                        run(
                            (str(bitcode), "test-impact", ".", "--run", "--quiet"),
                            cwd=repository,
                            check=False,
                            timeout_seconds=900,
                            max_output_bytes=1024 * 1024,
                        )
                        if declared_checks_passed
                        else None
                    )
                    impacted_tests_passed = impacted is not None and impacted.returncode == 0
                    success = (
                        validated.returncode == 0
                        and declared_checks_passed
                        and impacted_tests_passed
                    )
                    if validated.returncode != 0:
                        last_diagnostic = f"{validated.stdout}\n{validated.stderr}".strip()
                    elif not declared_checks_passed:
                        last_diagnostic = f"{executed.stdout}\n{executed.stderr}".strip()
                    elif not impacted_tests_passed:
                        last_diagnostic = f"{impacted.stdout}\n{impacted.stderr}".strip()
                    attempts.append({
                        "attempt": ordinal,
                        "tokens": tokens,
                        "success": success,
                        "validated": validated.returncode == 0,
                        "plan_run_passed": declared_checks_passed,
                        "declared_checks_passed": declared_checks_passed,
                        "impacted_tests": (
                            {
                                "command": list(impacted.args),
                                "passed": impacted_tests_passed,
                                "returncode": impacted.returncode,
                                "stdout": impacted.stdout,
                                "stderr": impacted.stderr,
                                "stdout_sha256": impacted.stdout_sha256,
                                "stderr_sha256": impacted.stderr_sha256,
                                "wall_seconds": impacted.wall_seconds,
                            }
                            if impacted is not None
                            else None
                        ),
                    })
                    if success:
                        break
                arms[arm] = {
                    "success": success,
                    "first_attempt_input_tokens": (
                        attempts[0].get("tokens") if attempts else None
                    ),
                    "input_tokens": total_tokens if token_count_complete else None,
                    "completed_input_tokens": total_tokens,
                    "token_count_complete": token_count_complete,
                    "attempts": attempts,
                }
                completed_arms[progress_key] = arms[arm]
                atomic_write_json(
                    progress_path,
                    {"header": progress_header, "completed_arms": completed_arms},
                )
                print(
                    f"authoring {progress_key}: "
                    f"{'passed' if success else 'failed'}; "
                    f"tokens={arms[arm]['input_tokens']}",
                    flush=True,
                )
            results.append({"id": case_id, "arms": arms})

    common = [case for case in results if case["arms"]["text"]["success"] and case["arms"]["graph"]["success"]]
    text_successes = {case["id"] for case in results if case["arms"]["text"]["success"]}
    graph_successes = {case["id"] for case in results if case["arms"]["graph"]["success"]}
    token_counts_complete = all(
        arm["token_count_complete"]
        for case in results
        for arm in case["arms"].values()
    )
    first_attempt_token_counts_complete = all(
        type(arm["first_attempt_input_tokens"]) is int
        for case in results
        for arm in case["arms"].values()
    )
    text_first_attempt_tokens = (
        sum(case["arms"]["text"]["first_attempt_input_tokens"] for case in results)
        if first_attempt_token_counts_complete
        else None
    )
    graph_first_attempt_tokens = (
        sum(case["arms"]["graph"]["first_attempt_input_tokens"] for case in results)
        if first_attempt_token_counts_complete
        else None
    )
    text_completed_tokens = sum(
        case["arms"]["text"]["completed_input_tokens"] for case in results
    )
    graph_completed_tokens = sum(
        case["arms"]["graph"]["completed_input_tokens"] for case in results
    )
    text_tokens = text_completed_tokens if token_counts_complete else None
    graph_tokens = graph_completed_tokens if token_counts_complete else None
    passed = (
        token_counts_complete
        and
        len(common) >= policy["success"]["minimum_common_successes"]
        and text_successes <= graph_successes
        and graph_tokens is not None
        and text_tokens is not None
        and graph_tokens < text_tokens
    )
    observation = {
        "schema_version": 2,
        "policy_id": policy["policy_id"],
        "policy_sha256": provenance["policy_sha256"],
        "model": policy["model"],
        "model_manifest_sha256": normalized_manifest,
        "bitcode_sha256": sha256_file(bitcode),
        "source_commit": provenance["source_commit"],
        "source_worktree_clean": True,
        "source_diff_sha256": hashlib.sha256(
            require_success(
                run_bounded(("git", "diff", "--binary", "HEAD"), cwd=REPO_ROOT,
                            timeout_seconds=30, max_output_bytes=16 * 1024 * 1024),
                "capture authoring-cost source diff",
            ).stdout.encode()
        ).hexdigest(),
        "source_snapshot_sha256": source_snapshot_sha256(REPO_ROOT),
        "host": {
            "hostname": socket.gethostname(),
            "platform": platform.platform(),
            "python": platform.python_version(),
            "cpu_count": os.cpu_count(),
        },
        "run_definition": (
            "fresh-clone-per-attempt-paired-arms-bounded-local-model-"
            "declared-semantic-checks-and-impacted-tests"
        ),
        "tool": {
            "bitcode_sha256": sha256_file(bitcode),
            "harness_sha256": sha256_file(Path(__file__)),
            "harness_support_sha256": sha256_file(
                REPO_ROOT / "tools" / "harness_support.py"
            ),
            "authoring_task_check_sha256": sha256_file(
                REPO_ROOT / "tools" / "authoring_task_check.py"
            ),
            "source_commit": provenance["source_commit"],
        },
        "semantic_verification": policy["semantic_verification"],
        "results": results,
        "summary": {
            "passed": passed,
            "common_successes": len(common),
            "first_attempt_token_counts_complete": first_attempt_token_counts_complete,
            "text_first_attempt_input_tokens": text_first_attempt_tokens,
            "graph_first_attempt_input_tokens": graph_first_attempt_tokens,
            "token_counts_complete": token_counts_complete,
            "text_input_tokens": text_tokens,
            "graph_input_tokens": graph_tokens,
            "text_completed_input_tokens": text_completed_tokens,
            "graph_completed_input_tokens": graph_completed_tokens,
        },
    }
    output = args.output or REPO_ROOT / "docs" / "authoring-cost-observation.json"
    atomic_write_json(output.resolve(), observation)
    progress_path.unlink(missing_ok=True)
    print(f"authoring cost: {'PASS' if passed else 'FAIL'}; observation: {output.resolve()}")
    return 0 if passed else 1


def measure(policy: Mapping[str, Any], policy_bytes: bytes, bitcode: Path) -> dict[str, Any]:
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
        if policy["policy_id"] == "plan-executor-v1":
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
        else:
            results = {
                "P6_lowering_determinism": measure_p6(
                    policy, root / "p6", measured_bitcode, options
                ),
                "P7_text_equivalence": measure_p7(
                    policy, root / "p7", measured_bitcode, options
                ),
                "P8_span_safety": measure_p8(policy, root / "p8", measured_bitcode, options),
                "P9_resolution_fail_closed": measure_p9(
                    policy, root / "p9", measured_bitcode, options
                ),
            }
            mutation_adequacy = measure_graph_mutation_adequacy(
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
    violations = (
        evaluate_policy(policy, results)
        if policy["policy_id"] == "plan-executor-v1"
        else evaluate_graph_policy(policy, results)
    )
    observation = {
        "schema_version": 1,
        "policy_id": policy["policy_id"],
        "bitcode_sha256": original_sha,
        "results": results,
        "mutation_adequacy": mutation_adequacy,
        "policy": {"passed": not violations, "violations": violations},
    }
    observation.update(observation_metadata(policy, policy_bytes, original_sha))
    return observation


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bitcode", type=Path, required=True, help="exact Bit Code binary to measure")
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY, help="precommitted policy JSON")
    parser.add_argument("--output", type=Path, help="observation JSON path")
    parser.add_argument("--json", action="store_true", help="also print the full observation")
    parser.add_argument(
        "--authoring-cost",
        action="store_true",
        help="run the separately precommitted local-model authoring-cost measurement",
    )
    parser.add_argument("--ollama-host", default="http://127.0.0.1:11434")
    args = parser.parse_args()
    if args.authoring_cost:
        return run_authoring_cost(args)
    bitcode = args.bitcode.resolve()
    if not bitcode.is_file():
        parser.error(f"Bit Code binary does not exist: {bitcode}")
    policy_path = args.policy.resolve()
    policy_bytes = policy_path.read_bytes()
    policy = validate_selected_policy(json.loads(policy_bytes))
    provenance = observation_provenance(policy_path)
    output = args.output or (
        GRAPH_OBSERVATION if policy["policy_id"] == "graph-edit-v2" else DEFAULT_OBSERVATION
    )
    observation = measure(policy, policy_bytes, bitcode)
    if (
        observation.get("policy_sha256") != provenance["policy_sha256"]
        or observation.get("tool", {}).get("source_commit") != provenance["source_commit"]
    ):
        raise RuntimeError("refusing to write observation with inconsistent provenance")
    if not observation.get("policy_sha256") or not observation.get("tool", {}).get("source_commit"):
        raise RuntimeError("refusing to write incomplete observation metadata")
    atomic_write_json(output.resolve(), observation)
    if args.json:
        print(json.dumps(observation, indent=2, sort_keys=True))
    print(
        f"plan executor policy: {'PASS' if observation['policy']['passed'] else 'FAIL'}; "
        f"observation: {output.resolve()}"
    )
    for violation in observation["policy"]["violations"]:
        print(f"  - {violation}", file=sys.stderr)
    return 0 if observation["policy"]["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
