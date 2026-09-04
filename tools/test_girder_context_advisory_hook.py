import json
import subprocess
import sys
import unittest
from pathlib import Path

HOOK = Path(__file__).parents[1] / ".claude" / "hooks" / "girder_context_advisory.py"


class GirderContextAdvisoryHookTests(unittest.TestCase):
    def run_hook(self, file_path: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess:
        payload = json.dumps({"tool_input": {"file_path": file_path}})
        return subprocess.run(
            [sys.executable, str(HOOK)],
            input=payload,
            capture_output=True,
            text=True,
            env=env or {},
        )

    def test_default_env_is_advisory_not_deny(self):
        result = self.run_hook("crates/aether-app/src/project.rs")
        self.assertEqual(result.returncode, 0)
        output = json.loads(result.stdout)
        self.assertNotIn("permissionDecision", output["hookSpecificOutput"])
        self.assertIn("additionalContext", output["hookSpecificOutput"])

    def test_enforce_denies_a_non_allowlisted_rust_file(self):
        result = self.run_hook(
            "crates/aether-app/src/project.rs",
            env={"GIRDER_HOOK_ENFORCE": "1"},
        )
        self.assertEqual(result.returncode, 0)
        output = json.loads(result.stdout)
        self.assertEqual(output["hookSpecificOutput"]["permissionDecision"], "deny")
        self.assertIn("girder context", output["hookSpecificOutput"]["permissionDecisionReason"])

    def test_enforce_stays_advisory_for_allowlisted_main_rs(self):
        result = self.run_hook(
            "crates/aether-app/src/main.rs",
            env={"GIRDER_HOOK_ENFORCE": "1"},
        )
        self.assertEqual(result.returncode, 0)
        output = json.loads(result.stdout)
        self.assertNotIn("permissionDecision", output["hookSpecificOutput"])
        self.assertIn("additionalContext", output["hookSpecificOutput"])

    def test_enforce_stays_advisory_for_allowlisted_cargo_toml(self):
        result = self.run_hook("Cargo.toml", env={"GIRDER_HOOK_ENFORCE": "1"})
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "")

    def test_enforce_stays_advisory_for_docs_directory(self):
        result = self.run_hook(
            "docs/some_script.py",
            env={"GIRDER_HOOK_ENFORCE": "1"},
        )
        self.assertEqual(result.returncode, 0)
        output = json.loads(result.stdout)
        self.assertNotIn("permissionDecision", output["hookSpecificOutput"])

    def test_non_source_file_is_silent_regardless_of_enforce(self):
        for env in ({}, {"GIRDER_HOOK_ENFORCE": "1"}):
            with self.subTest(env=env):
                result = self.run_hook("README.md", env=env)
                self.assertEqual(result.returncode, 0)
                self.assertEqual(result.stdout.strip(), "")

    def test_malformed_stdin_is_silent(self):
        result = subprocess.run(
            [sys.executable, str(HOOK)],
            input="not json",
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "")


if __name__ == "__main__":
    unittest.main()
