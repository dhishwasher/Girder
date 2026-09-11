"""codebase-memory-mcp 0.10.8 adapter using its native MCP responses."""

from __future__ import annotations

import json
import os
import re
import tempfile
import time
from pathlib import Path
from typing import Any, Mapping, Sequence

from ..fixtures import apply_mutation as write_mutation
from ..mcp import McpCall, McpSession, tool_text
from ..protocol import Adapter, NativeResult, Status, normalize_paths


SOURCE_SUFFIXES = {".rs", ".py", ".ts", ".tsx", ".go"}
REQUIRED_TOOLS = {
    "check_index_coverage", "delete_project", "detect_changes", "get_architecture",
    "get_code_snippet", "get_graph_schema", "index_repository", "index_status",
    "ingest_traces", "list_projects", "manage_adr", "query_graph", "search_code",
    "search_graph", "trace_path",
}


class CodebaseMemoryAdapter(Adapter):
    name = "codebase-memory-mcp"
    version = "0.10.8"
    commit = "46ae198fc11cda80e817acbc5f5908d7c2de7032"

    def __init__(self, binary: Path, *, limits: Mapping[str, Any], private_home: Path) -> None:
        self.binary = binary.resolve()
        self.limits = limits
        self.requested_private_home = private_home
        self.fixture_root: Path | None = None
        self.artifact_root: Path | None = None
        self.session: McpSession | None = None
        self.project = "benchmark"
        self.private_root: Path | None = None
        self.env: dict[str, str] | None = None

    def prepare(self, fixture_root: Path, artifact_root: Path) -> NativeResult:
        self.fixture_root = fixture_root.resolve()
        self.artifact_root = artifact_root.resolve()
        self.artifact_root.mkdir(parents=True, exist_ok=True)
        self.private_root = Path(tempfile.mkdtemp(prefix="girder-cbm-benchmark-"))
        self.private_root.chmod(0o700)
        home = self.private_root / "home"
        cache = self.private_root / "cache"
        home.mkdir(mode=0o700)
        cache.mkdir(mode=0o700)
        self.env = _isolated_environment(home, cache)
        calls: list[McpCall] = []
        try:
            self.session = McpSession(
                [str(self.binary)], cwd=self.fixture_root, env=self.env,
                artifact_root=self.artifact_root / "mcp",
                minimum_available_bytes=self.limits["minimum_available_bytes"],
                emergency_available_bytes=self.limits["emergency_available_bytes"],
                maximum_tree_rss_bytes=self.limits["maximum_tree_rss_bytes"],
                max_output_bytes=self.limits["max_output_bytes"],
            )
            calls.append(self.session.initialize(self.limits["query_timeout"]))
            calls.append(self.session.request("tools/list", {}, self.limits["query_timeout"]))
            failed = next((call for call in calls if call.status is not Status.PASS), None)
            if failed:
                return _calls_result(calls, query_calls=0)
            actual = {item["name"] for item in calls[-1].result["tools"]}
            if actual != REQUIRED_TOOLS:
                return _calls_result(
                    calls, forced_status=Status.ERROR,
                    detail=f"unexpected MCP tools: {sorted(actual)}", query_calls=0,
                )
            index = self._tool("index_repository", {
                "repo_path": str(self.fixture_root), "name": self.project,
                "mode": "full", "persistence": False,
            }, timeout=self.limits["prepare_timeout"])
            calls.append(index)
            if index.status is not Status.PASS:
                return _calls_result(calls, query_calls=0)
            try:
                indexed = _json_text(index)
                if indexed.get("status") != "indexed":
                    raise ValueError(f"unexpected index status: {indexed.get('status')!r}")
            except (json.JSONDecodeError, TypeError, ValueError) as error:
                return _calls_result(
                    calls, forced_status=_error_status(str(error)),
                    detail=f"index normalization failed: {error}", query_calls=0,
                )
            return _calls_result(calls, query_calls=0)
        except (OSError, RuntimeError, KeyError, TypeError) as error:
            status = Status.RESOURCE_BLOCKED if str(error).startswith("RESOURCE_BLOCKED:") else Status.ERROR
            self.cleanup()
            return NativeResult(status=status, detail=f"MCP startup failed: {error}", tool_calls=0)

    def query_definition(self, target: str) -> NativeResult:
        calls, rows, error = self._search_exact(target)
        if error is not None:
            return error
        answers = [row["identity"] for row in rows]
        if len(rows) != 1:
            return _calls_result(calls, answer=answers)
        snippet = self._tool("get_code_snippet", {
            "project": self.project, "qualified_name": rows[0]["qualified_name"],
            "include_neighbors": False,
        })
        calls.append(snippet)
        if snippet.status is not Status.PASS:
            return _calls_result(calls, answer=answers)
        try:
            data = _json_text(snippet)
            source = data["source"]
            if not isinstance(source, str):
                raise ValueError("snippet source is not text")
            return _calls_result(calls, answer=answers, source_text=source)
        except (json.JSONDecodeError, KeyError, TypeError, ValueError) as error:
            return _calls_result(
                calls, answer=answers, forced_status=_error_status(str(error)),
                detail=f"definition normalization failed: {error}",
            )

    def query_callers(self, target: str) -> NativeResult:
        return self._trace(target, direction="inbound", depth=1)

    def query_callees(self, target: str) -> NativeResult:
        return self._trace(target, direction="outbound", depth=1)

    def query_impact(self, target: str) -> NativeResult:
        return self._trace(target, direction="inbound", depth=32)

    def query_tests(self, target: str) -> NativeResult:
        result = self._trace(target, direction="inbound", depth=32)
        if result.status is not Status.PASS:
            return result
        tests = [identity for identity in result.answer if _is_test_path(identity.split("::", 1)[0])]
        return NativeResult(
            answer=normalize_paths(tests), status=result.status, detail=result.detail,
            tool_calls=result.tool_calls, metadata=result.metadata,
        )

    def apply_mutation(self, mutation: Mapping[str, Any]) -> NativeResult:
        assert self.fixture_root is not None
        started = time.monotonic()
        try:
            write_mutation(self.fixture_root, mutation)
            return NativeResult(
                status=Status.PASS, tool_calls=0,
                metadata={"elapsed_seconds": time.monotonic() - started},
            )
        except (OSError, ValueError) as error:
            return NativeResult(status=Status.ERROR, detail=f"mutation failed: {error}", tool_calls=0)

    def wait_until_ready(self, timeout_seconds: float) -> NativeResult:
        # The pinned server owns its documented watcher. The frozen evaluator's
        # probe loop observes current, stale, rebuilding, or unchanged answers.
        return NativeResult(status=Status.PASS, tool_calls=0, metadata={"timeout_seconds": timeout_seconds})

    def cleanup(self) -> None:
        if self.session is not None:
            self.session.close()
            self.session = None

    def _tool(self, name: str, arguments: Mapping[str, Any], *, timeout: float | None = None) -> McpCall:
        if self.session is None:
            raise RuntimeError("adapter is not prepared")
        return self.session.call_tool(name, arguments, timeout or self.limits["query_timeout"])

    def _search_exact(self, target: str) -> tuple[list[McpCall], list[dict[str, str]], NativeResult | None]:
        calls: list[McpCall] = []
        rows: list[dict[str, str]] = []
        offset = 0
        while True:
            call = self._tool("search_graph", {
                "project": self.project, "name_pattern": f"^{re.escape(target)}$",
                "format": "json", "detail": "default", "limit": 500, "offset": offset,
                "fields": ["signature", "is_test"],
            })
            calls.append(call)
            if call.status is not Status.PASS:
                return calls, rows, _calls_result(calls, answer=[row["identity"] for row in rows])
            try:
                data = _json_text(call)
                page = self._group_rows(data, data.get("cols"))
                rows.extend(page)
                if not data.get("has_more"):
                    return calls, rows, None
                if not page:
                    raise ValueError("search page claimed has_more without rows")
                offset += len(page)
            except (json.JSONDecodeError, KeyError, TypeError, ValueError) as error:
                result = _calls_result(
                    calls, answer=[row["identity"] for row in rows],
                    forced_status=_error_status(str(error)), detail=f"search normalization failed: {error}",
                )
                return calls, rows, result

    def _trace(self, target: str, *, direction: str, depth: int) -> NativeResult:
        calls: list[McpCall] = []
        answers: list[str] = []
        cursor: str | None = None
        seen_cursors: set[str] = set()
        field = "callers" if direction == "inbound" else "callees"
        while True:
            arguments: dict[str, Any] = {
                "project": self.project, "function_name": target, "direction": direction,
                "depth": depth, "mode": "calls", "include_tests": True,
                "format": "json", "limit": 500,
            }
            if cursor is not None:
                arguments["cursor"] = cursor
            call = self._tool("trace_path", arguments)
            calls.append(call)
            if call.status is not Status.PASS:
                return _calls_result(calls, answer=answers)
            try:
                data = _json_text(call)
                container = data.get(field, {})
                if not isinstance(container, dict):
                    raise ValueError(f"{field} result is not an object")
                answers.extend(row["identity"] for row in self._group_rows(container, container.get("cols")))
                next_cursor = data.get("next", container.get("next"))
                if not next_cursor:
                    return _calls_result(calls, answer=answers)
                if not isinstance(next_cursor, str) or next_cursor in seen_cursors:
                    raise ValueError("invalid or repeated trace cursor")
                seen_cursors.add(next_cursor)
                cursor = next_cursor
            except (json.JSONDecodeError, KeyError, TypeError, ValueError) as error:
                return _calls_result(
                    calls, answer=answers, forced_status=_error_status(str(error)),
                    detail=f"trace normalization failed: {error}",
                )

    def _group_rows(self, data: Mapping[str, Any], columns: Any) -> list[dict[str, str]]:
        if not isinstance(columns, list):
            raise ValueError("response columns are missing")
        parsed = []
        for group in data.get("groups", []):
            if not isinstance(group, dict):
                raise ValueError("response group is not an object")
            for raw in group.get("rows", []):
                row = dict(zip(columns, raw, strict=True))
                name = row.get("name")
                if not isinstance(name, str):
                    raise ValueError("response row name is missing")
                identity = self._identity(group.get("qn_prefix", ""), name, group.get("file"))
                qualified = f"{group.get('qn_prefix')}.{name}" if group.get("qn_prefix") else name
                parsed.append({"identity": identity, "qualified_name": qualified})
        return parsed

    def _identity(self, qn_prefix: str, name: str, native_file: Any = None) -> str:
        assert self.fixture_root is not None
        if isinstance(native_file, str) and native_file:
            path = _path(native_file)
            return f"{path}::{name}"
        module = qn_prefix
        if module == self.project:
            module = ""
        elif module.startswith(self.project + "."):
            module = module[len(self.project) + 1:]
        candidates: list[tuple[int, str]] = []
        for path in self.fixture_root.rglob("*"):
            if not path.is_file() or path.suffix not in SOURCE_SUFFIXES:
                continue
            relative = path.relative_to(self.fixture_root).as_posix()
            stem = relative[: -len(path.suffix)].replace("/", ".")
            if module == stem or module.startswith(stem + "."):
                candidates.append((len(stem), relative))
        if candidates:
            longest = max(length for length, _ in candidates)
            paths = {path for length, path in candidates if length == longest}
            if len(paths) == 1:
                return f"{next(iter(paths))}::{name}"
        return f"{{unresolved:{qn_prefix}}}::{name}"


def _path(value: str) -> str:
    return value.replace("\\", "/").removeprefix("./")


def _is_test_path(path: str) -> bool:
    name = Path(path).name
    return (
        (name.startswith("test_") and name.endswith(".py"))
        or (path.startswith("tests/") and name.endswith(".rs"))
        or ".test." in name
        or name.endswith("_test.go")
    )


def _isolated_environment(home: Path, cache: Path) -> dict[str, str]:
    return dict(
        os.environ, HOME=str(home), XDG_CACHE_HOME=str(home / ".cache"), CBM_CACHE_DIR=str(cache),
        CARGO_BUILD_JOBS="1", CMAKE_BUILD_PARALLEL_LEVEL="1", MAKEFLAGS="-j1",
        RAYON_NUM_THREADS="1", npm_config_jobs="1",
    )


def _json_text(call: McpCall) -> dict[str, Any]:
    data = json.loads(tool_text(call))
    if not isinstance(data, dict):
        raise ValueError("tool result is not a JSON object")
    return data


def _error_status(detail: str) -> Status:
    return Status.STALE if re.search(r"\b(stale|rebuild(?:ing)?|index(?:ing)? in progress)\b", detail, re.I) else Status.ERROR


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


def _calls_result(
    calls: Sequence[McpCall], *, answer: Sequence[str] = (), source_text: str | None = None,
    forced_status: Status | None = None, detail: str = "", query_calls: int | None = None,
) -> NativeResult:
    failed = next((call for call in calls if call.status is not Status.PASS), None)
    status = forced_status or (failed.status if failed else Status.PASS)
    if not detail and failed:
        detail = failed.detail
    metadata = _call_metadata(calls)
    if source_text is not None:
        metadata["source_text"] = source_text
    return NativeResult(
        answer=normalize_paths(answer), status=status, detail=detail,
        tool_calls=len(calls) if query_calls is None else query_calls, metadata=metadata,
    )
