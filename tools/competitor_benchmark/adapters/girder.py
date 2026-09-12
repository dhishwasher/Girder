"""Girder 0.2.6 adapter for normal and opt-in watch MCP modes."""

from __future__ import annotations

import json
import os
import re
import shutil
import time
from pathlib import Path
from typing import Any, Mapping, Sequence

from ..fixtures import apply_mutation as write_mutation
from ..mcp import McpCall, McpSession, tool_text
from ..process import run_supervised
from ..protocol import Adapter, NativeResult, Status, normalize_paths


SEMANTIC_PATH = re.compile(r"crate(?:::[A-Za-z0-9_.-]+)+")
SOURCE_SUFFIXES = {".rs", ".py", ".ts", ".tsx", ".go"}
REQUIRED_TOOLS = {
    "get_source", "find_definition", "search_code", "ask_codebase",
    "impacted_tests", "review_changes", "orient",
}


class GirderAdapter(Adapter):
    version = "0.2.6"
    commit = "3f7d5ee645a1b4802cff3c09f1e84494e20e020b"

    def __init__(self, binary: Path, *, watch: bool, limits: Mapping[str, Any], private_home: Path) -> None:
        self.binary = binary.resolve()
        self.watch = watch
        self.name = "girder-watch" if watch else "girder"
        self.limits = limits
        self.private_home = private_home
        self.fixture_root: Path | None = None
        self.artifact_root: Path | None = None
        self.session: McpSession | None = None
        self.env = _isolated_environment(private_home)

    def prepare(self, fixture_root: Path, artifact_root: Path) -> NativeResult:
        self.fixture_root = fixture_root.resolve()
        self.artifact_root = artifact_root.resolve()
        self.artifact_root.mkdir(parents=True, exist_ok=True)
        _stage_existing_license(self.private_home)
        process = run_supervised(
            [str(self.binary), "analyze", str(self.fixture_root), "--json"],
            cwd=self.fixture_root, env=self.env,
            stdout_path=self.artifact_root / "prepare.stdout.json",
            stderr_path=self.artifact_root / "prepare.stderr.txt",
            timeout_seconds=self.limits["prepare_timeout"],
            max_output_bytes=self.limits["max_output_bytes"],
            minimum_available_bytes=self.limits["minimum_available_bytes"],
            emergency_available_bytes=self.limits["emergency_available_bytes"],
            maximum_tree_rss_bytes=self.limits["maximum_tree_rss_bytes"],
        )
        process_metadata = {
            **vars(process),
            "stdout_artifact": str(self.artifact_root / "prepare.stdout.json"),
            "stderr_artifact": str(self.artifact_root / "prepare.stderr.txt"),
        }
        if process.status is not Status.PASS:
            return NativeResult(status=process.status, detail=process.detail, tool_calls=0,
                                metadata={"process": process_metadata})
        command = [str(self.binary), "mcp", str(self.fixture_root)]
        if self.watch:
            command.append("--watch")
        try:
            self.session = McpSession(
                command, cwd=self.fixture_root, env=self.env,
                artifact_root=self.artifact_root / "mcp",
                minimum_available_bytes=self.limits["minimum_available_bytes"],
                emergency_available_bytes=self.limits["emergency_available_bytes"],
                maximum_tree_rss_bytes=self.limits["maximum_tree_rss_bytes"],
                max_output_bytes=self.limits["max_output_bytes"],
            )
            calls = [self.session.initialize(self.limits["query_timeout"])]
            calls.append(self.session.request("tools/list", {}, self.limits["query_timeout"]))
            if any(call.status is not Status.PASS for call in calls):
                return _calls_result(calls)
            actual = {item["name"] for item in calls[-1].result["tools"]}
            if actual != REQUIRED_TOOLS:
                return NativeResult(status=Status.ERROR, detail=f"unexpected MCP tools: {sorted(actual)}",
                                    metadata=_call_metadata(calls))
            call_metadata = _call_metadata(calls)
            return NativeResult(status=Status.PASS, tool_calls=0, metadata={
                "process": process_metadata,
                "elapsed_seconds": process.wall_seconds + call_metadata["elapsed_seconds"],
                "stdout_bytes": process.stdout_bytes + call_metadata["stdout_bytes"],
                "stderr_bytes": process.stderr_bytes + call_metadata["stderr_bytes"],
                "stdout_artifacts": [process_metadata["stdout_artifact"], *call_metadata["stdout_artifacts"]],
                "stderr_artifacts": [process_metadata["stderr_artifact"], *call_metadata["stderr_artifacts"]],
                "peak_rss_bytes": max(process.peak_rss_bytes or 0, call_metadata["peak_rss_bytes"]),
                "methods": call_metadata["methods"],
            })
        except (OSError, RuntimeError, KeyError, TypeError) as error:
            self.cleanup()
            return NativeResult(status=Status.ERROR, detail=f"MCP startup failed: {error}")

    def query_definition(self, target: str) -> NativeResult:
        first = self._tool("find_definition", {"name": target})
        if first.status is not Status.PASS:
            return _calls_result([first])
        try:
            declarations = json.loads(tool_text(first))
            if not isinstance(declarations, list):
                raise ValueError("find_definition result is not a list")
            identities = tuple(self._identity(item["path"]) for item in declarations)
            if len(declarations) != 1:
                return NativeResult(answer=normalize_paths(identities), status=Status.PASS,
                                    detail=f"definition candidates={len(declarations)}",
                                    metadata=_call_metadata([first]))
            second = self._tool("get_source", {"nodes": [declarations[0]["path"]]})
            if second.status is not Status.PASS:
                return _calls_result([first, second], answer=identities)
            source = json.loads(tool_text(second))["nodes"][0]["source"]
            return NativeResult(answer=normalize_paths(identities), status=Status.PASS,
                                tool_calls=2, metadata={"source_text": source, **_call_metadata([first, second])})
        except (KeyError, IndexError, TypeError, ValueError, json.JSONDecodeError) as error:
            return NativeResult(status=Status.ERROR, detail=f"definition normalization failed: {error}",
                                metadata=_call_metadata([first]))

    def query_callers(self, target: str) -> NativeResult:
        return self._relationship("what calls", target)

    def query_callees(self, target: str) -> NativeResult:
        return self._relationship("what does", target, suffix="call")

    def query_impact(self, target: str) -> NativeResult:
        return self._relationship("impact of", target)

    def query_tests(self, target: str) -> NativeResult:
        definition = self._tool("find_definition", {"name": target})
        if definition.status is not Status.PASS:
            return _calls_result([definition])
        try:
            declarations = json.loads(tool_text(definition))
            if len(declarations) != 1:
                raise ValueError(f"definition candidates={len(declarations)}")
            tests = self._tool("impacted_tests", {"nodes": [declarations[0]["path"]]})
            if tests.status is not Status.PASS:
                return _calls_result([definition, tests])
            names = [line.strip() for line in tool_text(tests).splitlines() if line.strip()]
            identities = [self._test_identity(name) for name in names]
            return NativeResult(answer=normalize_paths(identities), status=Status.PASS, tool_calls=2,
                                metadata=_call_metadata([definition, tests]))
        except (ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
            return NativeResult(status=Status.ERROR, detail=f"test normalization failed: {error}",
                                metadata=_call_metadata([definition]))

    def apply_mutation(self, mutation: Mapping[str, Any]) -> NativeResult:
        assert self.fixture_root is not None
        started = time.monotonic()
        try:
            write_mutation(self.fixture_root, mutation)
            return NativeResult(status=Status.PASS, tool_calls=0,
                                metadata={"elapsed_seconds": time.monotonic() - started})
        except (OSError, ValueError) as error:
            return NativeResult(status=Status.ERROR, detail=f"mutation failed: {error}", tool_calls=0)

    def wait_until_ready(self, timeout_seconds: float) -> NativeResult:
        # Girder's watch handler blocks graph-dependent calls while rebuilding;
        # the evaluator's frozen probe loop supplies the readiness deadline.
        return NativeResult(status=Status.PASS, tool_calls=0, metadata={"timeout_seconds": timeout_seconds})

    def cleanup(self) -> None:
        if self.session is not None:
            self.session.close()
            self.session = None

    def _tool(self, name: str, arguments: Mapping[str, Any]) -> McpCall:
        if self.session is None:
            raise RuntimeError("adapter is not prepared")
        return self.session.call_tool(name, arguments, self.limits["query_timeout"])

    def _relationship(self, prefix: str, target: str, *, suffix: str = "") -> NativeResult:
        question = " ".join(part for part in (prefix, target, suffix) if part) + "?"
        call = self._tool("ask_codebase", {"question": question})
        if call.status is not Status.PASS:
            return _calls_result([call])
        try:
            output = tool_text(call)
            native_paths = set(SEMANTIC_PATH.findall(output))
            native_paths = {path for path in native_paths if path.rsplit("::", 1)[-1] != target}
            identities = [self._identity(path) for path in native_paths]
            return NativeResult(answer=normalize_paths(identities), status=Status.PASS,
                                metadata=_call_metadata([call]))
        except ValueError as error:
            return NativeResult(status=Status.ERROR, detail=f"relationship normalization failed: {error}",
                                metadata=_call_metadata([call]))

    def _identity(self, native: str) -> str:
        assert self.fixture_root is not None
        parts = native.split("::")
        if len(parts) < 3 or parts[0] != "crate":
            return f"{{unresolved}}::{native}"
        module = "::".join(parts[1:-1])
        candidates = []
        for path in self.fixture_root.rglob("*"):
            if not path.is_file() or path.suffix not in SOURCE_SUFFIXES:
                continue
            relative = path.relative_to(self.fixture_root).as_posix()
            without_suffix = relative[: -len(path.suffix)]
            native_modules = {without_suffix.replace("/", "::")}
            # Rust crate paths omit the filesystem-only src directory and use
            # foo for either src/foo.rs or src/foo/mod.rs.
            if path.suffix == ".rs":
                conventional = without_suffix.replace("/", "::").removeprefix("src::")
                native_modules.add(conventional.removesuffix("::mod"))
            if module in native_modules:
                candidates.append(relative)
        if len(candidates) == 1:
            return f"{candidates[0]}::{parts[-1]}"
        return f"{{unresolved:{module}}}::{parts[-1]}"

    def _test_identity(self, name: str) -> str:
        assert self.fixture_root is not None
        candidates = []
        for path in self.fixture_root.rglob("*"):
            if not path.is_file():
                continue
            rel = path.relative_to(self.fixture_root).as_posix()
            if (path.name.startswith("test_") and path.suffix == ".py") or \
               ("tests" in path.parts and path.suffix == ".rs") or \
               (".test." in path.name and path.suffix in {".ts", ".tsx"}) or \
               path.name.endswith("_test.go"):
                candidates.append(rel)
        if len(candidates) == 1:
            return f"{candidates[0]}::{name}"
        return f"{{unresolved-test}}::{name}"


def _isolated_environment(private_home: Path) -> dict[str, str]:
    private_home.mkdir(parents=True, exist_ok=True)
    return dict(
        os.environ,
        HOME=str(private_home), XDG_CACHE_HOME=str(private_home / ".cache"),
        CARGO_BUILD_JOBS="1", CMAKE_BUILD_PARALLEL_LEVEL="1", MAKEFLAGS="-j1",
        RAYON_NUM_THREADS="1", npm_config_jobs="1",
    )


def _stage_existing_license(private_home: Path) -> None:
    source = Path.home() / ".config" / "girder" / "license.key"
    if not source.is_file():
        return
    destination = private_home / ".config" / "girder" / "license.key"
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(0o600)


def _call_metadata(calls: Sequence[McpCall]) -> dict[str, Any]:
    return {
        "elapsed_seconds": sum(call.elapsed_seconds for call in calls),
        "stdout_bytes": sum(call.stdout_bytes for call in calls),
        "stderr_bytes": sum(call.stderr_bytes for call in calls),
        "stdout_artifacts": [call.stdout_artifact for call in calls],
        "stderr_artifacts": [call.stderr_artifact for call in calls],
        "peak_rss_bytes": max((call.peak_rss_bytes or 0 for call in calls), default=0),
        "methods": [{"method": call.method, "params": call.params} for call in calls],
    }


def _calls_result(calls: Sequence[McpCall], *, answer: Sequence[str] = ()) -> NativeResult:
    failed = next((call for call in calls if call.status is not Status.PASS), None)
    status = failed.status if failed else Status.ERROR
    detail = failed.detail if failed else "MCP tool returned no usable result"
    return NativeResult(answer=normalize_paths(answer), status=status, detail=detail,
                        tool_calls=len(calls), metadata=_call_metadata(calls))
