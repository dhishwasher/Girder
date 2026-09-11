"""Ripwire adapter normalization tests using frozen native response shapes."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from tools.competitor_benchmark.adapters.ripwire import CliCall, RipwireAdapter
from tools.competitor_benchmark.process import SupervisedResult
from tools.competitor_benchmark.protocol import Status


def call(root: Path, index: int, text: str) -> CliCall:
    stdout = root / f"{index}.out"
    stderr = root / f"{index}.err"
    stdout.write_text(text)
    stderr.write_text("")
    process = SupervisedResult(("ripwire",), 0, Status.PASS, "", len(text.encode()), 0,
                               "0" * 64, "0" * 64, 0.01, 1024)
    return CliCall(("ripwire",), process, stdout, stderr)


class RipwireNormalizationTests(unittest.TestCase):
    def adapter(self, root: Path, replies: list[CliCall]) -> RipwireAdapter:
        adapter = RipwireAdapter(Path("/bin/true"), limits={"query_timeout": 1},
                                 private_home=root / "home", initial_target="calculate_total")
        adapter.fixture_root = root
        adapter.artifact_root = root / "raw"
        adapter._invoke = lambda *args, **kwargs: replies.pop(0)  # type: ignore[method-assign]
        return adapter

    def test_definition_combines_json_identity_and_xml_source(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            first = call(root, 1, json.dumps({"sigs": [{"n": "calculate_total", "p": "core.py", "sig": "def calculate_total():"}]}))
            second = call(root, 2, '<ctx><bodies><b n="calculate_total" p="core.py"><![CDATA[def calculate_total():\n return 1]]></b></bodies></ctx>')
            result = self.adapter(root, [first, second]).query_definition("calculate_total")
            self.assertEqual(result.answer, ("core.py::calculate_total",))
            self.assertIn("return 1", result.metadata["source_text"])
            self.assertEqual(result.tool_calls, 2)

    def test_relations_strip_line_locations(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            native = json.dumps({"count": 1, "callers": [{"n": "render_summary", "p": "service.py:3"}]})
            result = self.adapter(root, [call(root, 1, native)]).query_callers("calculate_total")
            self.assertEqual(result.answer, ("service.py::render_summary",))

    def test_test_file_is_retained_as_whole_file_prediction(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            native = '<affected><test p="test_service.py" hops="2"/></affected>'
            result = self.adapter(root, [call(root, 1, native)]).query_tests("calculate_total")
            self.assertEqual(result.answer, ("test_service.py::*",))

    def test_native_failure_status_remains_distinct(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            failed = call(root, 1, "")
            failed = CliCall(failed.args, SupervisedResult(
                failed.process.command, 1, Status.ERROR, "refused", 0, 0,
                failed.process.stdout_sha256, failed.process.stderr_sha256, 0.01, 1024,
            ), failed.stdout_path, failed.stderr_path)
            result = self.adapter(root, [failed]).query_callers("missing")
            self.assertEqual(result.status, Status.ERROR)


if __name__ == "__main__":
    unittest.main()
