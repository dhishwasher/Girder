"""code-review-graph 2.3.8 adapter using its native MCP responses."""

from __future__ import annotations

import json
import os
import re
import shutil
import tempfile
from pathlib import Path
from typing import Any, Mapping, Sequence

from ..fixtures import apply_mutation as write_mutation
from ..mcp import McpCall, McpSession, tool_text
from ..protocol import Adapter, NativeResult, Status, normalize_paths


REQUIRED_TOOLS = {
    "build_or_update_graph_tool",
    "get_impact_radius_tool",
    "get_review_context_tool",
    "query_graph_tool",
    "semantic_search_nodes_tool",
}


class CodeReviewGraphAdapter(Adapter):
    name = "code-review-graph"
    version = "2.3.8"
    commit = "2c6dae32643572ee528eb9b77dbcc17f58f3a8c9"

    def __init__(self, binary: Path, *, limits: Mapping[str, Any], private_home: Path) -> None:
        self.binary = binary.resolve()
        self.limits = limits
        self.requested_private_home = private_home
        self.fixture_root: Path | None = None
        self.artifact_root: Path | None = None
        self.session: McpSession | None = None
        self.private_root: Path | None = None
        self.env: dict[str, str] | None = None

    def prepare(self, fixture_root: Path, artifact_root: Path) -> NativeResult:
        self.fixture_root = fixture_root.resolve()
        self.artifact_root = artifact_root.resolve()
        self.artifact_root.mkdir(parents=True, exist_ok=True)
        # SQLite writes on the ChromeOS removable 9p mount fail with EIO. Keep
        # product-owned database state in a fresh private local directory.
        self.private_root = Path(tempfile.mkdtemp(prefix="girder-crg-benchmark-"))
        self.private_root.chmod(0o700)
        home = self.private_root / "home"
        data = self.private_root / "data"
        home.mkdir(mode=0o700)
        data.mkdir(mode=0o700)
        self.env = _isolated_environment(home, data)
        calls: list[McpCall] = []
        try:
            self.session = McpSession(
                [str(self.binary), "serve", "--repo", str(self.fixture_root)],
                cwd=self.fixture_root, env=self.env,
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
            missing = REQUIRED_TOOLS - actual
            if missing:
                return _calls_result(
                    calls, forced_status=Status.ERROR,
                    detail=f"missing required MCP tools: {sorted(missing)}", query_calls=0,
                )
            build = self._tool("build_or_update_graph_tool", {
                "full_rebuild": True,
                "repo_root": str(self.fixture_root),
                "postprocess": "minimal",
            }, timeout=self.limits["prepare_timeout"])
            calls.append(build)
            if build.status is not Status.PASS:
                return _calls_result(calls, query_calls=0)
            try:
                data_out = _json_text(build)
                if data_out.get("status") != "ok":
                    raise ValueError(f"unexpected build status: {data_out.get('status')!r}")
            except (json.JSONDecodeError, TypeError, ValueError) as error:
                return _calls_result(
                    calls, forced_status=_error_status(str(error)),
                    detail=f"build normalization failed: {error}", query_calls=0,
                )
            result = _calls_result(calls, query_calls=0)
            return NativeResult(
                answer=result.answer, status=result.status, detail=result.detail,
                tool_calls=result.tool_calls,
                metadata={**result.metadata, "private_data_dir": str(data)},
            )
        except (OSError, RuntimeError, KeyError, TypeError) as error:
            status = Status.RESOURCE_BLOCKED if str(error).startswith("RESOURCE_BLOCKED:") else Status.ERROR
            self.cleanup()
            return NativeResult(status=status, detail=f"MCP startup failed: {error}", tool_calls=0)

    def query_definition(self, target: str) -> NativeResult:
        calls, rows, error = self._search_exact(target)
        if error is not None:
            return error
        answers = [_identity(row, self.fixture_root) for row in rows]
        if len(rows) != 1:
            return _calls_result(calls, answer=answers)
        relative = _relative_file(rows[0], self.fixture_root)
        if relative is None:
            return _calls_result(
                calls, answer=answers, forced_status=Status.UNSUPPORTED,
                detail="native definition result has no fixture-relative source path",
            )
        source_call = self._tool("get_review_context_tool", {
            "changed_files": [relative],
            "max_depth": 0,
            "include_source": True,
            "max_lines_per_file": 500,
            "repo_root": str(self.fixture_root),
            "detail_level": "standard",
            "max_results": 200,
            "max_files": 1,
        })
        calls.append(source_call)
        if source_call.status is not Status.PASS:
            return _calls_result(calls, answer=answers)
        try:
            data = _json_text(source_call)
            source = data["context"]["source_snippets"][relative]
            if not isinstance(source, str):
                raise ValueError("source snippet is not text")
            return _calls_result(calls, answer=answers, source_text=source)
        except (json.JSONDecodeError, KeyError, TypeError, ValueError) as error:
            return _calls_result(
                calls, answer=answers, forced_status=_error_status(str(error)),
                detail=f"definition source normalization failed: {error}",
            )

    def query_callers(self, target: str) -> NativeResult:
        return self._relationship("callers_of", target)

    def query_callees(self, target: str) -> NativeResult:
        return self._relationship("callees_of", target)

    def query_impact(self, target: str) -> NativeResult:
        calls, rows, error = self._search_exact(target)
        if error is not None:
            return error
        if len(rows) != 1:
            return _calls_result(calls, answer=[_identity(row, self.fixture_root) for row in rows])
        relative = _relative_file(rows[0], self.fixture_root)
        if relative is None:
            return _calls_result(calls, forced_status=Status.ERROR,
                                 detail="impact target has no fixture-relative source path")
        impact = self._tool("get_impact_radius_tool", {
            "changed_files": [relative],
            "max_depth": 64,
            "repo_root": str(self.fixture_root),
            "detail_level": "standard",
        })
        calls.append(impact)
        if impact.status is not Status.PASS:
            return _calls_result(calls)
        try:
            data = _json_text(impact)
            _require_ok(data)
            answers = [
                _identity(row, self.fixture_root)
                for row in data.get("impacted_nodes", [])
                if row.get("kind") != "File"
            ]
            return _calls_result(calls, answer=answers)
        except (json.JSONDecodeError, KeyError, TypeError, ValueError) as error:
            return _calls_result(
                calls, forced_status=_error_status(str(error)),
                detail=f"impact normalization failed: {error}",
            )

    def query_tests(self, target: str) -> NativeResult:
        return self._relationship("tests_for", target)

    def apply_mutation(self, mutation: Mapping[str, Any]) -> NativeResult:
        assert self.fixture_root is not None
        try:
            write_mutation(self.fixture_root, mutation)
        except (OSError, ValueError) as error:
            return NativeResult(status=Status.ERROR, detail=f"mutation failed: {error}", tool_calls=0)
        update = self._tool("build_or_update_graph_tool", {
            "full_rebuild": False,
            "repo_root": str(self.fixture_root),
            "postprocess": "minimal",
        }, timeout=self.limits["prepare_timeout"])
        if update.status is not Status.PASS:
            return _calls_result([update], query_calls=0)
        try:
            data = _json_text(update)
            if data.get("status") != "ok":
                raise ValueError(f"unexpected update status: {data.get('status')!r}")
            return _calls_result([update], query_calls=0)
        except (json.JSONDecodeError, TypeError, ValueError) as error:
            return _calls_result(
                [update], forced_status=_error_status(str(error)),
                detail=f"update normalization failed: {error}", query_calls=0,
            )

    def wait_until_ready(self, timeout_seconds: float) -> NativeResult:
        # apply_mutation synchronously awaits the product's native incremental
        # update. The common probe loop then measures the resulting generation.
        return NativeResult(status=Status.PASS, tool_calls=0,
                            metadata={"timeout_seconds": timeout_seconds})

    def cleanup(self) -> None:
        if self.session is not None:
            self.session.close()
            self.session = None
        if self.private_root is not None:
            shutil.rmtree(self.private_root, ignore_errors=True)
            self.private_root = None

    def _tool(self, name: str, arguments: Mapping[str, Any], *, timeout: float | None = None) -> McpCall:
        if self.session is None:
            raise RuntimeError("adapter is not prepared")
        return self.session.call_tool(name, arguments, timeout or self.limits["query_timeout"])

    def _search_exact(self, target: str) -> tuple[list[McpCall], list[Mapping[str, Any]], NativeResult | None]:
        call = self._tool("semantic_search_nodes_tool", {
            "query": target,
            "kind": "Function",
            "limit": 500,
            "repo_root": str(self.fixture_root),
            "detail_level": "standard",
        })
        calls = [call]
        if call.status is not Status.PASS:
            return calls, [], _calls_result(calls)
        try:
            data = _json_text(call)
            _require_ok(data)
            results = data.get("results", [])
            if not isinstance(results, list):
                raise ValueError("search results are not a list")
            rows = [row for row in results if isinstance(row, dict) and row.get("name") == target]
            return calls, rows, None
        except (json.JSONDecodeError, TypeError, ValueError) as error:
            result = _calls_result(
                calls, forced_status=_error_status(str(error)),
                detail=f"search normalization failed: {error}",
            )
            return calls, [], result

    def _relationship(self, pattern: str, target: str) -> NativeResult:
        call = self._tool("query_graph_tool", {
            "pattern": pattern,
            "target": target,
            "repo_root": str(self.fixture_root),
            "detail_level": "standard",
            "max_results": 500,
        })
        if call.status is not Status.PASS:
            return _calls_result([call])
        try:
            data = _json_text(call)
            _require_ok(data)
            rows = data.get("results", [])
            if not isinstance(rows, list):
                raise ValueError("relationship results are not a list")
            answers = [_identity(row, self.fixture_root) for row in rows]
            return _calls_result([call], answer=answers)
        except (json.JSONDecodeError, KeyError, TypeError, ValueError) as error:
            return _calls_result(
                [call], forced_status=_error_status(str(error)),
                detail=f"relationship normalization failed: {error}",
            )


def _relative_file(row: Mapping[str, Any], root: Path | None) -> str | None:
    if root is None or not isinstance(row.get("file_path"), str):
        return None
    try:
        return Path(row["file_path"]).resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return None


def _identity(row: Mapping[str, Any], root: Path | None) -> str:
    relative = _relative_file(row, root)
    name = row.get("name")
    if relative is not None and isinstance(name, str) and name:
        return f"{relative}::{name}"
    qualified = row.get("qualified_name")
    if isinstance(qualified, str) and qualified:
        return qualified.replace("\\", "/")
    raise ValueError("native node has no usable identity")


def _isolated_environment(home: Path, data: Path) -> dict[str, str]:
    return dict(
        os.environ, HOME=str(home), XDG_CACHE_HOME=str(home / ".cache"),
        CRG_DATA_DIR=str(data), CARGO_BUILD_JOBS="1", CMAKE_BUILD_PARALLEL_LEVEL="1",
        MAKEFLAGS="-j1", RAYON_NUM_THREADS="1", npm_config_jobs="1",
    )


def _json_text(call: McpCall) -> dict[str, Any]:
    data = json.loads(tool_text(call))
    if not isinstance(data, dict):
        raise ValueError("tool result is not a JSON object")
    return data


def _require_ok(data: Mapping[str, Any]) -> None:
    if data.get("status") != "ok":
        detail = str(data.get("error") or data.get("summary") or data.get("status"))
        raise ValueError(detail)


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
