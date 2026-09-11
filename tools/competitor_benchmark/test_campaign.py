"""Pure evaluator tests; no product process is launched."""

from __future__ import annotations

import unittest

from tools.competitor_benchmark.campaign import assess_native
from tools.competitor_benchmark.protocol import NativeResult, Status


class CampaignScoringTests(unittest.TestCase):
    def setUp(self) -> None:
        self.current = {
            "expected": {"definition": ["new.py::f"], "callers": ["caller.py::g"]},
            "definition_source_marker": "new body",
        }
        self.prior = {
            "expected": {"definition": ["old.py::f"], "callers": []},
            "definition_source_marker": "old body",
        }

    def test_evaluator_keeps_adapter_and_oracle_separate(self) -> None:
        native = NativeResult(answer=("new.py::f",), status=Status.PASS,
                              metadata={"source_text": "new body"})
        status, score, scored = assess_native(native, "definition", self.current, self.prior)
        self.assertEqual(status, Status.PASS)
        self.assertEqual(score["recall"], 1.0)
        self.assertEqual(scored, ("new.py::f",))

    def test_native_resource_failure_cannot_become_wrong_or_pass(self) -> None:
        native = NativeResult(answer=(), status=Status.RESOURCE_BLOCKED)
        status, _, _ = assess_native(native, "callers", self.current, self.prior)
        self.assertEqual(status, Status.RESOURCE_BLOCKED)


if __name__ == "__main__":
    unittest.main()
