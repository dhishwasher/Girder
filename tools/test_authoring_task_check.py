import tempfile
import unittest
from pathlib import Path

from tools.authoring_task_check import VerificationError, verify_authoring_task
from tools.plan_executor_oracle import authoring_edits


class AuthoringTaskCheckTests(unittest.TestCase):
    def python_repository(self, source: str) -> tuple[tempfile.TemporaryDirectory, Path]:
        directory = tempfile.TemporaryDirectory()
        root = Path(directory.name)
        target = root / "sample-project" / "calc.py"
        target.parent.mkdir(parents=True)
        target.write_text(source, encoding="utf-8")
        return directory, root

    def test_semantically_equivalent_differently_spelled_edit_passes(self):
        directory, root = self.python_repository(
            "def greet(name):\n"
            "    return str.upper(hello(name))\n\n"
            "def hello(name):\n"
            "    return 'hello ' + name\n"
        )
        with directory:
            verify_authoring_task("python-replace", root)

    def test_edit_that_changes_behavior_fails(self):
        directory, root = self.python_repository(
            "def greet(name):\n"
            "    return hello(name).lower()\n\n"
            "def hello(name):\n"
            "    return 'hello ' + name\n"
        )
        with directory, self.assertRaisesRegex(VerificationError, "wrong behavior"):
            verify_authoring_task("python-replace", root)

    def test_all_eight_canonical_edits_pass_semantic_verification(self):
        source_root = Path(__file__).parents[1]
        for language in ("rust", "python"):
            source_path = {
                "rust": "crates/aether-debugger/src/trace.rs",
                "python": "sample-project/calc.py",
            }[language]
            original = (source_root / source_path).read_text(encoding="utf-8")
            for operation in ("replace", "rename", "delete", "insert"):
                case_id = f"{language}-{operation}"
                _, _, edit = authoring_edits(case_id)
                changed = original.replace(
                    edit["match"],
                    edit["replace"],
                    edit.get("occurrences", 1),
                )
                with tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    target = root / source_path
                    target.parent.mkdir(parents=True)
                    target.write_text(changed, encoding="utf-8")
                    verify_authoring_task(case_id, root)


if __name__ == "__main__":
    unittest.main()
