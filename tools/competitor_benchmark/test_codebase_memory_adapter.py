"""codebase-memory-mcp adapter tests using frozen v0.10.8 response shapes."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from tools.competitor_benchmark.adapters.codebase_memory import CodebaseMemoryAdapter
from tools.competitor_benchmark.mcp import McpCall
from tools.competitor_benchmark.protocol import Status


def call(root: Path, index: int, data: dict | None = None, *, error: str | None = None) -> McpCall:
    stdout = root / f"{index}.out"
    stderr = root / f"{index}.err"
    result = {"content": [{"type": "text", "text": error or json.dumps(data)}], "isError": error is not None}
    encoded = (json.dumps({"jsonrpc": "2.0", "id": index, "result": result}) + "\n").encode()
    stdout.write_bytes(encoded)
    stderr.write_bytes(b"")
    return McpCall(
        "tools/call", {}, result, Status.PASS, "", 0.01, len(encoded), 0,
        str(stdout), str(stderr), 2048,
    )


class CodebaseMemoryNormalizationTests(unittest.TestCase):
    def adapter(self, root: Path, replies: list[McpCall]) -> CodebaseMemoryAdapter:
        adapter = CodebaseMemoryAdapter(Path("/bin/true"), limits={"query_timeout": 1}, private_home=root / "home")
        adapter.fixture_root = root
        adapter.artifact_root = root / "raw"
        adapter._tool = lambda *args, **kwargs: replies.pop(0)  # type: ignore[method-assign]
        return adapter

    def test_definition_combines_exact_search_and_source(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "core.py").write_text("def calculate_total():\n    return 1\n")
            search = {
                "total": 1, "count": 1, "cols": ["name", "label", "lines"],
                "groups": [{"qn_prefix": "benchmark.core", "file": "core.py", "rows": [["calculate_total", "Function", "1-2"]]}],
                "has_more": False,
            }
            snippet = {"source": "def calculate_total():\n    return 1\n"}
            result = self.adapter(root, [call(root, 1, search), call(root, 2, snippet)]).query_definition("calculate_total")
            self.assertEqual(result.answer, ("core.py::calculate_total",))
            self.assertIn("return 1", result.metadata["source_text"])
            self.assertEqual(result.tool_calls, 2)

    def test_trace_cursor_paginates_and_maps_qualified_prefixes(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "service.py").write_text("def render_summary(): pass\n")
            (root / "callback.py").write_text("def invoke_callback(): pass\n")
            first = {"callers": {"cols": ["name", "hop"], "groups": [
                {"qn_prefix": "benchmark.service", "rows": [["render_summary", 1]]}
            ]}, "next": "page-2"}
            second = {"callers": {"cols": ["name", "hop"], "groups": [
                {"qn_prefix": "benchmark.callback", "rows": [["invoke_callback", 1]]}
            ]}}
            replies = [call(root, 1, first), call(root, 2, second)]
            adapter = self.adapter(root, replies)
            result = adapter.query_callers("calculate_total")
            self.assertEqual(result.answer, ("callback.py::invoke_callback", "service.py::render_summary"))
            self.assertEqual(result.tool_calls, 2)

    def test_tests_filter_non_test_impact_nodes(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "service.py").write_text("def render_summary(): pass\n")
            (root / "test_service.py").write_text("def test_render_summary(): pass\n")
            native = {"callers": {"cols": ["name", "hop"], "groups": [
                {"qn_prefix": "benchmark.service", "rows": [["render_summary", 1]]},
                {"qn_prefix": "benchmark.test_service", "rows": [["test_render_summary", 2]]},
            ]}}
            result = self.adapter(root, [call(root, 1, native)]).query_tests("calculate_total")
            self.assertEqual(result.answer, ("test_service.py::test_render_summary",))

    def test_explicit_rebuilding_error_is_stale(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            result = self.adapter(root, [call(root, 1, error="index rebuilding; retry")]).query_callers("f")
            self.assertEqual(result.status, Status.STALE)

    def test_rust_module_prefix_maps_to_source_path(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "src").mkdir()
            (root / "src/service.rs").write_text("fn render_summary() {}\n")
            adapter = self.adapter(root, [])
            self.assertEqual(
                adapter._identity("benchmark.src.service", "render_summary"),
                "src/service.rs::render_summary",
            )


if __name__ == "__main__":
    unittest.main()
