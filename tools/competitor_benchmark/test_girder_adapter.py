"""Adapter normalization tests that do not execute Girder itself."""

from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

from tools.competitor_benchmark.adapters.girder import GirderAdapter
from tools.competitor_benchmark.mcp import McpCall, McpSession
from tools.competitor_benchmark.protocol import Status


def response(text: str) -> dict[str, object]:
    return {"content": [{"type": "text", "text": text}], "isError": False}


class FakeSession:
    def __init__(self, replies: list[str]) -> None:
        self.replies = replies

    def call_tool(self, name: str, arguments: dict[str, object], timeout: float) -> McpCall:
        del name, arguments, timeout
        text = self.replies.pop(0)
        return McpCall("tools/call", {}, response(text), Status.PASS, "", 0.01,
                       len(text.encode()), 0, "out", "err", 100)


class GirderNormalizationTests(unittest.TestCase):
    def make_adapter(self, root: Path, replies: list[str]) -> GirderAdapter:
        adapter = GirderAdapter(Path("/bin/true"), watch=False, limits={"query_timeout": 1},
                                private_home=root / "home")
        adapter.fixture_root = root
        adapter.session = FakeSession(replies)  # type: ignore[assignment]
        return adapter

    def test_identity_maps_native_module_to_relative_source(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "src").mkdir()
            (root / "src" / "core.rs").write_text("fn compute_total() {}")
            adapter = self.make_adapter(root, [])
            self.assertEqual(adapter._identity("crate::src::core::compute_total"),
                             "src/core.rs::compute_total")

    def test_relationship_excludes_seed_and_normalizes_result(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "core.py").write_text("def calculate_total(): pass")
            (root / "service.py").write_text("def render_summary(): pass")
            text = "[impact] crate::core::calculate_total\n  · crate::service::render_summary\n"
            result = self.make_adapter(root, [text]).query_impact("calculate_total")
            self.assertEqual(result.answer, ("service.py::render_summary",))

    def test_definition_uses_source_from_second_native_call(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "core.py").write_text("def calculate_total(): pass")
            replies = [
                json.dumps([{"path": "crate::core::calculate_total"}]),
                json.dumps({"nodes": [{"source": "def calculate_total(): pass"}]}),
            ]
            result = self.make_adapter(root, replies).query_definition("calculate_total")
            self.assertEqual(result.answer, ("core.py::calculate_total",))
            self.assertEqual(result.tool_calls, 2)
            self.assertIn("def calculate_total", result.metadata["source_text"])

    def test_test_name_maps_only_when_test_file_is_unambiguous(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            (root / "core.py").write_text("def calculate_total(): pass")
            (root / "test_service.py").write_text("def test_total(): pass")
            replies = [json.dumps([{"path": "crate::core::calculate_total"}]), "test_total\n"]
            result = self.make_adapter(root, replies).query_tests("calculate_total")
            self.assertEqual(result.answer, ("test_service.py::test_total",))


class McpTransportTests(unittest.TestCase):
    def test_initialize_sends_required_client_fields(self) -> None:
        server = (
            "import json,sys\n"
            "q=json.loads(sys.stdin.readline())\n"
            "r={'jsonrpc':'2.0','id':q['id'],'result':{'params':q['params']}}\n"
            "sys.stdout.write(json.dumps(r,separators=(',',':'))+'\\n'); sys.stdout.flush()\n"
        )
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            with McpSession(
                [sys.executable, "-u", "-c", server], cwd=root, env=os.environ,
                artifact_root=root / "raw", minimum_available_bytes=1,
                emergency_available_bytes=1, maximum_tree_rss_bytes=256 * 1024 * 1024,
                max_output_bytes=4096,
            ) as session:
                call = session.initialize(5)
            self.assertEqual(call.status, Status.PASS)
            assert call.result is not None
            self.assertEqual(call.result["params"]["capabilities"], {})
            self.assertEqual(call.result["params"]["clientInfo"]["name"],
                             "girder-competitor-benchmark")

    def test_json_line_transport_preserves_raw_response_bytes(self) -> None:
        server = (
            "import json,sys\n"
            "for line in sys.stdin:\n"
            " q=json.loads(line); r={'jsonrpc':'2.0','id':q['id'],'result':{'echo':q['method']}}\n"
            " sys.stdout.write(json.dumps(r,separators=(',',':'))+'\\n'); sys.stdout.flush()\n"
        )
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            with McpSession(
                [sys.executable, "-u", "-c", server], cwd=root, env=os.environ,
                artifact_root=root / "raw", minimum_available_bytes=1,
                emergency_available_bytes=1, maximum_tree_rss_bytes=256 * 1024 * 1024,
                max_output_bytes=4096,
            ) as session:
                call = session.request("ping", {}, 5)
            self.assertEqual(call.status, Status.PASS)
            self.assertEqual(call.result, {"echo": "ping"})
            self.assertEqual(call.stdout_bytes, Path(call.stdout_artifact).stat().st_size)
            self.assertTrue(Path(call.stdout_artifact).read_bytes().endswith(b"\n"))
            self.assertTrue((root / "raw" / "0002-cleanup.stderr.txt").is_file())


if __name__ == "__main__":
    unittest.main()
