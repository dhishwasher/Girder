import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HOOK = Path(__file__).parents[1] / "npm" / "hooks" / "girder_context_advisory.py"


class GirderContextAdvisoryHookTests(unittest.TestCase):
    def make_native_stub(self, root: str, source: str) -> Path:
        stub = Path(root, "girder-native-stub")
        stub.write_text(f"#!{sys.executable}\n{source}", encoding="utf-8")
        stub.chmod(0o700)
        return stub

    def run_hook(self, payload: str, binary: Path) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(HOOK), str(binary)],
            input=payload,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_pre_tool_use_forwards_structured_native_stdout(self):
        response = {
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "additionalContext": "Girder: consider `girder context` before this read.",
            }
        }
        native_stdout = json.dumps(response, separators=(",", ":"))
        with tempfile.TemporaryDirectory() as root:
            binary = self.make_native_stub(
                root,
                "import sys\n"
                "sys.stdin.buffer.read()\n"
                f"sys.stdout.write({native_stdout!r})\n",
            )
            request = json.dumps(
                {
                    "hook_event_name": "PreToolUse",
                    "tool_name": "Read",
                    "tool_input": {"file_path": "src/lib.rs"},
                }
            )
            result = self.run_hook(request, binary)

        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, native_stdout)
        self.assertEqual(result.stderr, "")
        self.assertEqual(json.loads(result.stdout), response)

    def test_post_tool_use_forwards_only_native_stderr(self):
        with tempfile.TemporaryDirectory() as root:
            binary = self.make_native_stub(
                root,
                "import sys\n"
                "sys.stdin.buffer.read()\n"
                "sys.stdout.write('internal observation')\n"
                "sys.stderr.write('edit blast-radius advice')\n",
            )
            request = json.dumps(
                {
                    "hook_event_name": "PostToolUse",
                    "tool_name": "Edit",
                    "tool_input": {"file_path": "src/lib.rs"},
                }
            )
            result = self.run_hook(request, binary)

        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr, "edit blast-radius advice")

    def test_malformed_input_and_native_failures_fail_open(self):
        with tempfile.TemporaryDirectory() as root:
            successful = self.make_native_stub(
                root,
                "import sys\n"
                "sys.stdin.buffer.read()\n"
                "sys.stdout.write('native stdout')\n",
            )
            for payload in ("not json", "[]", "{}"):
                with self.subTest(payload=payload):
                    result = self.run_hook(payload, successful)
                    self.assertEqual(
                        (result.returncode, result.stdout, result.stderr),
                        (0, "", ""),
                    )

            failing = self.make_native_stub(
                root,
                "import sys\n"
                "sys.stdin.buffer.read()\n"
                "sys.stdout.write('native stdout')\n"
                "sys.stderr.write('native stderr')\n"
                "raise SystemExit(2)\n",
            )
            request = json.dumps({"hook_event_name": "PreToolUse"})
            failed = self.run_hook(request, failing)
            missing = self.run_hook(request, Path(root, "missing-native"))

        self.assertEqual((failed.returncode, failed.stdout, failed.stderr), (0, "", ""))
        self.assertEqual((missing.returncode, missing.stdout, missing.stderr), (0, "", ""))


if __name__ == "__main__":
    unittest.main()
