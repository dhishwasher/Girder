#!/usr/bin/env python3
"""Task-specific semantic checks for the authoring-cost measurement."""

from __future__ import annotations

import re
import sys
from pathlib import Path


class VerificationError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise VerificationError(message)


def load_python_module(repository: Path) -> dict[str, object]:
    path = repository / "sample-project" / "calc.py"
    source = path.read_text(encoding="utf-8")
    namespace: dict[str, object] = {"__name__": "authoring_cost_target"}
    try:
        exec(compile(source, str(path), "exec"), namespace)
    except Exception as error:
        raise VerificationError(f"Python target does not load: {error}") from error
    return namespace


def call_with_probe(namespace: dict[str, object], function_name: str, *, upper: bool) -> None:
    calls: list[str] = []

    def hello_probe(name: str) -> str:
        calls.append(name)
        return f"MiXeD::{name}"

    namespace["hello"] = hello_probe
    function = namespace.get(function_name)
    require(callable(function), f"{function_name} is not callable")
    for name in ("Ada", "z9"):
        expected = f"MiXeD::{name}"
        if upper:
            expected = expected.upper()
        require(function(name) == expected, f"{function_name} has wrong behavior")
    require(calls == ["Ada", "z9"], f"{function_name} did not delegate to hello")


def verify_python(operation: str, repository: Path) -> None:
    namespace = load_python_module(repository)
    if operation == "replace":
        call_with_probe(namespace, "greet", upper=True)
    elif operation == "rename":
        require("greet" not in namespace, "greet still exists after rename")
        call_with_probe(namespace, "welcome_greeting", upper=False)
    elif operation == "delete":
        require("greet" not in namespace, "greet still exists after deletion")
        hello = namespace.get("hello")
        require(callable(hello), "delete removed the neighboring hello function")
        require(hello("Ada") == "hello Ada", "hello behavior changed during deletion")
    elif operation == "insert":
        marker = namespace.get("sample_fixture_marker")
        require(callable(marker), "sample_fixture_marker was not inserted")
        value = marker()
        require(type(value) is int and value == 1, "sample_fixture_marker must return integer 1")
        call_with_probe(namespace, "greet", upper=False)
    else:
        raise VerificationError(f"unsupported Python operation: {operation}")


def compact_rust(source: str) -> str:
    return re.sub(r"\s+", "", source)


def verify_rust(operation: str, repository: Path) -> None:
    source = (repository / "crates" / "aether-debugger" / "src" / "trace.rs").read_text(
        encoding="utf-8"
    )
    compact = compact_rust(source)
    if operation == "replace":
        match = re.search(
            r"pubfnfinal_env\(&self\)->Env\{([^{}]*)\}pubfnlen",
            compact,
        )
        require(match is not None, "final_env method is missing or malformed")
        body = match.group(1)
        require("unwrap_or_default" not in body, "final_env still uses unwrap_or_default")
        require(
            re.fullmatch(
                r"self\.steps\.last\(\)\.map_or_else\(Env::new,\|([A-Za-z_][A-Za-z0-9_]*)\|\1\.env\.clone\(\)\)",
                body,
            )
            is not None,
            "final_env does not implement the requested map_or_else behavior",
        )
    elif operation == "rename":
        require("pubfnfinal_env(" not in compact, "final_env still exists after rename")
        require("pubfncompleted_env(&self)->Env{" in compact, "completed_env was not created")
    elif operation == "delete":
        require("pubfnfinal_env(" not in compact, "final_env still exists after deletion")
        require("pubfnlen(&self)->usize{" in compact, "deletion damaged the neighboring len method")
    elif operation == "insert":
        require(
            "pubfntrace_fixture_marker()->usize{1}" in compact,
            "trace_fixture_marker was not inserted with the required behavior",
        )
        require("pubfnis_empty(&self)->bool{" in compact, "insertion damaged the target module")
    else:
        raise VerificationError(f"unsupported Rust operation: {operation}")


def verify_authoring_task(case_id: str, repository: Path) -> None:
    try:
        language, operation = case_id.split("-", 1)
    except ValueError as error:
        raise VerificationError(f"invalid authoring task id: {case_id}") from error
    if language == "python":
        verify_python(operation, repository)
    elif language == "rust":
        verify_rust(operation, repository)
    else:
        raise VerificationError(f"unsupported authoring language: {language}")


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: authoring_task_check.py <language-operation>", file=sys.stderr)
        return 2
    try:
        verify_authoring_task(argv[1], Path.cwd())
    except VerificationError as error:
        print(f"authoring semantic check failed: {error}", file=sys.stderr)
        return 1
    print(f"authoring semantic check passed: {argv[1]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
