"""code-review-graph normalization tests; no product process is launched."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from tools.competitor_benchmark.adapters.code_review_graph import CodeReviewGraphAdapter
from tools.competitor_benchmark.mcp import McpCall
from tools.competitor_benchmark.protocol import Status


def call(payload: dict[str, object]) -> McpCall:
    text = json.dumps(payload)
    result = {"content": [{"type": "text", "text": text}], "isError": False}
    return McpCall("tools/call", {}, result, Status.PASS, "", 0.01,
                   len(text.encode()), 0, "out", "err", 100)


class FakeSession:
    def __init__(self, replies: list[McpCall]) -> None:
        self.replies = replies
        self.requests: list[tuple[str, dict[str, object]]] = []

    def call_tool(self, name: str, arguments: dict[str, object], timeout: float) -> McpCall:
        del timeout
        self.requests.append((name, arguments))
        return self.replies.pop(0)


class CodeReviewGraphNormalizationTests(unittest.TestCase):
    def adapter(self, root: Path, replies: list[McpCall]) -> CodeReviewGraphAdapter:
        adapter = CodeReviewGraphAdapter(Path("/bin/true"), limits={
            "query_timeout": 1, "prepare_timeout": 1,
        }, private_home=root / "home")
        adapter.fixture_root = root
        adapter.session = FakeSession(replies)  # type: ignore[assignment]
        return adapter

    def test_definition_uses_exact_search_and_native_source(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = root / "core.py"
            source.write_text("def calculate_total(): return 1\n")
            search = {"status": "ok", "results": [{
                "name": "calculate_total", "qualified_name": f"{source}::calculate_total",
                "file_path": str(source), "kind": "Function",
            }]}
            context = {"status": "ok", "context": {"source_snippets": {
                "core.py": "1: def calculate_total(): return 1",
            }}}
            result = self.adapter(root, [call(search), call(context)]).query_definition("calculate_total")
            self.assertEqual(result.answer, ("core.py::calculate_total",))
            self.assertIn("return 1", result.metadata["source_text"])
            self.assertEqual(result.tool_calls, 2)

    def test_callee_keeps_unresolved_native_builtin_as_false_positive(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            source = root / "core.py"
            source.write_text("def f(): return sum([])\n")
            payload = {"status": "ok", "results": [
                {"name": "sum", "qualified_name": "sum"},
                {"name": "normalize", "qualified_name": f"{source}::normalize",
                 "file_path": str(source)},
            ]}
            result = self.adapter(root, [call(payload)]).query_callees("f")
            self.assertEqual(result.answer, ("core.py::normalize", "sum"))

    def test_impact_searches_target_then_projects_symbol_nodes(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            core = root / "core.py"; service = root / "service.py"
            core.write_text("def f(): pass\n"); service.write_text("def g(): pass\n")
            search = {"status": "ok", "results": [{
                "name": "f", "qualified_name": f"{core}::f", "file_path": str(core),
            }]}
            impact = {"status": "ok", "impacted_nodes": [
                {"kind": "File", "name": str(service), "qualified_name": str(service),
                 "file_path": str(service)},
                {"kind": "Function", "name": "g", "qualified_name": f"{service}::g",
                 "file_path": str(service)},
            ]}
            result = self.adapter(root, [call(search), call(impact)]).query_impact("f")
            self.assertEqual(result.answer, ("service.py::g",))
            self.assertEqual(result.tool_calls, 2)

    def test_mutation_runs_synchronous_incremental_update(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "core.py").write_text("old\n")
            adapter = self.adapter(root, [call({"status": "ok"})])
            result = adapter.apply_mutation({
                "id": "body_edit", "writes": {"core.py": "new\n"}, "deletes": [],
            })
            self.assertEqual(result.status, Status.PASS)
            self.assertEqual(result.tool_calls, 0)
            session = adapter.session
            assert isinstance(session, FakeSession)
            self.assertEqual(session.requests[0][0], "build_or_update_graph_tool")
            self.assertFalse(session.requests[0][1]["full_rebuild"])


if __name__ == "__main__":
    unittest.main()
