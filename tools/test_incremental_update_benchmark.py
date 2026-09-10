import copy
import json
import unittest

from tools.incremental_update_benchmark import CORPUS, assess, scaling_fixtures


class IncrementalMeasurementTests(unittest.TestCase):
    def setUp(self):
        self.record = {"id": "fixture/step", "state": "completed", "equal": True,
                       "update": {"report": {"full_rebuild_reasons": [], "parsed_files": ["a"], "reused_files": ["b", "c"]}}}

    def test_mismatch_cannot_be_overridden_by_stored_pass(self):
        self.record.update(equal=False, pass_gate=True)
        self.assertEqual(assess([self.record], 1)["overall"], "FAIL")

    def test_incomplete_campaign_cannot_pass(self):
        self.assertEqual(assess([self.record], 2)["overall"], "FAIL")
        self.record["state"] = "running"
        self.assertEqual(assess([self.record], 1)["completed_mutations"], 0)

    def test_fallback_denominator_includes_normal_updates_and_mismatches(self):
        fallback = copy.deepcopy(self.record)
        fallback["equal"] = False
        fallback["update"]["report"]["full_rebuild_reasons"] = ["configuration_changed", "ownership_changed"]
        result = assess([self.record, fallback], 2)
        self.assertEqual(result["fallback_count"], 1)
        self.assertEqual(result["fallback_denominator"], 2)
        self.assertEqual(result["fallback_percentage"], 50)
        self.assertEqual(result["fallback_reasons"], {"configuration_changed": 1, "ownership_changed": 1})
        self.assertEqual((result["parsed_files"], result["reused_files"]), (2, 4))

    def test_empty_denominator_is_unavailable(self):
        self.assertIsNone(assess([], 1)["fallback_percentage"])
        self.assertEqual(assess([], 0)["overall"], "FAIL")

    def test_duplicate_records_do_not_fill_missing_mutations(self):
        self.assertEqual(assess([self.record, self.record], 2)["overall"], "FAIL")

    def test_scaling_uses_exact_frozen_languages_sizes_and_templates(self):
        corpus = json.loads(CORPUS.read_text())
        fixtures = list(scaling_fixtures(corpus))
        self.assertEqual(len(fixtures), 15)
        self.assertEqual({len(f["files"]) for f in fixtures}, {3, 30, 300})
        self.assertEqual({f["language"] for f in fixtures}, {"rust", "python", "typescript", "tsx", "go"})
        self.assertTrue(all(len(f["mutations"]) == 3 for f in fixtures))
        self.assertEqual(fixtures, list(scaling_fixtures(corpus)))
        self.assertEqual(fixtures[0]["files"]["src/f0001.rs"], "pub fn f0001() -> i32 { crate::f0000::f0000() }\n")
        self.assertIn("renamed", fixtures[0]["mutations"][1]["changes"][0]["source"])
        self.assertEqual(fixtures[0]["mutations"][2]["changes"][0]["source"], fixtures[0]["files"]["src/f0000.rs"] + "\n")


if __name__ == "__main__":
    unittest.main()
