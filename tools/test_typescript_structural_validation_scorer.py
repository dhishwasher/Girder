import unittest

from tools.typescript_structural_validation_scorer import resolve_origin, summarize


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


if __name__ == "__main__":
    unittest.main()
