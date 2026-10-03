import json
import stat
import tempfile
import unittest
from pathlib import Path

from tools.typescript_structural_validation_scorer import (
    MANIFEST,
    PINNED_MANIFEST_SHA256,
    main,
    resolve_origin,
    summarize,
)


def case(expected, qualifier="alice::@object", predicted=None, candidates=None):
    return {
        "origin": {"file": "app.test.ts", "symbol": "name", "qualifier": qualifier},
        "expected_origin_resolution": expected,
        "predicted_origin_path": predicted,
        "predicted_candidates": candidates or [],
    }


ALICE = {"path": "crate::app.test::alice::@object::name", "kind": "Function"}
BOB = {"path": "crate::app.test::bob::@object::name", "kind": "Function"}
FIELD = {"path": "crate::app.test::Named::name", "kind": "Field"}


class ResolveOriginTests(unittest.TestCase):
    def test_unique_requires_exact_predicted_identity(self):
        good = resolve_origin(case("unique", predicted=ALICE["path"]), [ALICE, BOB])
        self.assertTrue(good["resolution_passed"])
        wrong = resolve_origin(case("unique", predicted="crate::app.test::alice::name"), [ALICE])
        self.assertFalse(wrong["resolution_passed"])

    def test_field_is_never_an_executable_origin(self):
        result = resolve_origin(case("no-executable-target", qualifier="Named"), [ALICE, BOB, FIELD])
        self.assertTrue(result["resolution_passed"])
        self.assertEqual(result["function_candidates"], [])
        self.assertEqual(result["rejected_non_function_candidates"], ["crate::app.test::Named::name (Field)"])
        as_unique = resolve_origin(case("unique", qualifier="Named", predicted=FIELD["path"]), [FIELD])
        self.assertFalse(as_unique["resolution_passed"])

    def test_ambiguity_is_never_resolved_by_choice(self):
        both = [
            {"path": "crate::app.test::makeA::alice::@object::name", "kind": "Function"},
            {"path": "crate::app.test::makeB::alice::@object::name", "kind": "Function"},
        ]
        result = resolve_origin(case("ambiguous", candidates=[b["path"] for b in both]), both)
        self.assertTrue(result["resolution_passed"])
        self.assertEqual(result["observed_resolution"], "ambiguous")
        self.assertFalse(resolve_origin(case("unique", predicted=both[0]["path"]), both)["resolution_passed"])

    def test_no_identity_requires_zero_functions(self):
        self.assertTrue(resolve_origin(case("no-identity"), [BOB])["resolution_passed"])
        self.assertFalse(resolve_origin(case("no-identity"), [ALICE])["resolution_passed"])


class SummaryTests(unittest.TestCase):
    def test_unsound_or_failed_cells_fail_the_summary(self):
        ok = [{"id": "a", "status": "scored", "resolution_passed": True,
               "tests": [{"status": "scored", "cell": "conservative"}]}]
        self.assertTrue(summarize(ok)["passed"])
        bad = [{"id": "a", "status": "scored", "resolution_passed": True,
                "tests": [{"status": "scored", "cell": "unsafe_exclusion"}]}]
        self.assertFalse(summarize(bad)["passed"])
        failed = [{"id": "b", "status": "failed", "resolution_passed": False}]
        self.assertFalse(summarize(failed)["passed"])


class NegativeControlTests(unittest.TestCase):
    """Failures must be persisted in the output with a nonzero status."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def fake(self, body):
        path = self.dir / "girder"
        path.write_text("#!/bin/sh\n" + body + "\n")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
        return path

    def score(self, binary, *extra, **kwargs):
        output = self.dir / "out" / "scoring.json"
        code = main(["--binary", str(binary), "--output", str(output), *extra], **kwargs)
        self.assertTrue(output.is_file(), "output must be written even on failure")
        return code, json.loads(output.read_text())

    def assert_all_failed(self, document, needle):
        results = document["results"]
        self.assertEqual(len(results), 17)
        for result in results:
            self.assertEqual(result["status"], "failed", result)
            self.assertIn(needle, result["reason"])
        self.assertFalse(document["summary"]["passed"])

    def test_command_failure_is_recorded(self):
        code, document = self.score(self.fake("echo boom >&2; exit 3"))
        self.assertEqual(code, 1)
        self.assert_all_failed(document, "exit 3")

    def test_invalid_json_is_recorded(self):
        code, document = self.score(self.fake("echo not-json"))
        self.assertEqual(code, 1)
        self.assert_all_failed(document, "invalid JSON")

    def test_timeout_is_recorded(self):
        code, document = self.score(self.fake("sleep 5"), "--timeout", "0.2")
        self.assertEqual(code, 1)
        self.assert_all_failed(document, "timeout")

    def test_missing_binary_is_recorded(self):
        code, document = self.score(self.dir / "absent")
        self.assertEqual(code, 1)
        self.assert_all_failed(document, "could not execute")
        self.assertTrue(document["summary"]["input_errors"])

    def test_test_impact_failure_after_resolution_is_recorded(self):
        alice = json.dumps([{"path": "crate::app.test::alice::@object::name", "kind": "Function"}])
        binary = self.fake(f"if [ \"$1\" = names ]; then echo '{alice}'; else echo garbage; fi")
        code, document = self.score(binary)
        self.assertEqual(code, 1)
        failed = {r["id"]: r for r in document["results"] if r["status"] == "failed"}
        arrow = failed["ts-structural-v1-arrow-alice"]
        self.assertTrue(arrow["resolution_passed"])
        self.assertIn("invalid JSON", arrow["reason"])

    def test_manifest_hash_mismatch_is_recorded(self):
        copy = self.dir / "manifest.json"
        copy.write_bytes(MANIFEST.read_bytes() + b"\n")
        code, document = self.score(self.fake("exit 0"), manifest_path=copy)
        self.assertEqual(code, 1)
        self.assertEqual(document["results"], [])
        self.assertIn("!= pinned", document["summary"]["input_errors"][0])
        self.assertNotEqual(document["manifest_sha256"], PINNED_MANIFEST_SHA256)

    def test_unpinned_manifest_with_missing_fields_is_recorded(self):
        copy = self.dir / "manifest.json"
        copy.write_text("{}")
        code, document = self.score(self.fake("exit 0"), manifest_path=copy)
        self.assertEqual(code, 1)
        self.assertEqual(document["results"], [])
        self.assertIn("!= pinned", document["summary"]["input_errors"][0])

    def test_pinned_but_malformed_manifest_is_recorded(self):
        copy = self.dir / "manifest.json"
        copy.write_text("{}")
        digest = __import__("hashlib").sha256(b"{}").hexdigest()
        code, document = self.score(self.fake("exit 0"), manifest_path=copy, pinned=digest)
        self.assertEqual(code, 1)
        self.assertEqual(document["results"], [])
        self.assertIn("invalid manifest", document["summary"]["input_errors"][0])

    def malformed_selection(self, selection):
        alice = json.dumps([{"path": "crate::app.test::alice::@object::name", "kind": "Function"}])
        reply = json.dumps(selection)
        binary = self.fake(f"if [ \"$1\" = names ]; then echo '{alice}'; else echo '{reply}'; fi")
        code, document = self.score(binary)
        self.assertEqual(code, 1)
        arrow = next(r for r in document["results"] if r["id"] == "ts-structural-v1-arrow-alice")
        self.assertEqual(arrow["status"], "failed")
        self.assertTrue(arrow["resolution_passed"])
        return arrow["reason"]

    def test_unhashable_selection_paths_are_recorded(self):
        reason = self.malformed_selection({"schema_version": 1, "must": {"paths": [["x"]]},
                                           "may": {"paths": []}, "unknown": {"paths": []},
                                           "boundaries": {"count": 0}})
        self.assertIn("must.paths is not a list of strings", reason)

    def test_string_selection_paths_are_recorded(self):
        reason = self.malformed_selection({"schema_version": 1, "must": {"paths": []},
                                           "may": {"paths": "abc"}, "unknown": {"paths": []},
                                           "boundaries": {"count": 0}})
        self.assertIn("may.paths is not a list of strings", reason)

    def test_non_object_selection_is_recorded(self):
        self.assertIn("unsupported classified output", self.malformed_selection([1, 2]))

    def test_missing_boundary_count_is_recorded(self):
        reason = self.malformed_selection({"schema_version": 1, "must": {"paths": []},
                                           "may": {"paths": []}, "unknown": {"paths": []},
                                           "boundaries": {}})
        self.assertIn("boundaries.count", reason)


if __name__ == "__main__":
    unittest.main()
