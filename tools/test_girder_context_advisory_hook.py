import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HOOK = Path(__file__).parents[1] / "npm" / "hooks" / "girder_context_advisory.py"


class GirderContextAdvisoryHookTests(unittest.TestCase):
    def run_hook(self, payload: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(HOOK)],
            input=payload,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_advises_only_for_whole_source_reads_with_a_graph(self):
        with tempfile.TemporaryDirectory() as root:
            request = {
                "tool_name": "Read",
                "cwd": root,
                "tool_input": {"file_path": "crates/aether-app/src/project.rs"},
            }
            missing = self.run_hook(json.dumps(request))
            self.assertEqual((missing.returncode, missing.stdout, missing.stderr), (0, "", ""))

            Path(root, "project.aether").touch()
            ready = self.run_hook(json.dumps(request))
            self.assertEqual(ready.returncode, 0)
            self.assertEqual(ready.stdout, "")
            self.assertIn("girder context", ready.stderr)

            for file_path in ("sample.py", "sample.ts", "sample.tsx", "sample.go"):
                with self.subTest(file_path=file_path):
                    request["tool_input"]["file_path"] = file_path
                    self.assertIn("girder context", self.run_hook(json.dumps(request)).stderr)

    def test_ignores_bounded_reads_other_tools_and_non_source_files(self):
        with tempfile.TemporaryDirectory() as root:
            Path(root, "project.aether").touch()
            request = {
                "tool_name": "Read",
                "cwd": root,
                "tool_input": {"file_path": "sample.rs", "offset": 20},
            }
            for change in (
                {},
                {"tool_input": {"file_path": "README.md"}},
                {"tool_name": "Bash", "tool_input": {"file_path": "sample.rs"}},
            ):
                with self.subTest(change=change):
                    candidate = request | change
                    result = self.run_hook(json.dumps(candidate))
                    self.assertEqual((result.returncode, result.stdout, result.stderr), (0, "", ""))

    def test_malformed_input_fails_open(self):
        for payload in ("not json", "[]", "{}"):
            with self.subTest(payload=payload):
                result = self.run_hook(payload)
                self.assertEqual((result.returncode, result.stdout, result.stderr), (0, "", ""))


if __name__ == "__main__":
    unittest.main()
