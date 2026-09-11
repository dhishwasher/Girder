"""Pure evaluator tests; no product process is launched."""

from __future__ import annotations

import unittest

from tools.competitor_benchmark.campaign import (
    _deadline_terminal,
    _terminal_stability_count,
    assess_native,
)
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

    def test_stale_answer_resets_wrong_stability_window(self) -> None:
        signature = (("definition", "STALE", ("old.py::f",)), ("callers", "WRONG", ()))
        self.assertEqual(
            _terminal_stability_count([Status.STALE, Status.WRONG], signature, signature, 2),
            0,
        )

    def test_current_wrong_answer_advances_stability_window(self) -> None:
        signature = (("definition", "PASS", ("new.py::f",)), ("callers", "WRONG", ()))
        self.assertEqual(
            _terminal_stability_count([Status.PASS, Status.WRONG], signature, signature, 2),
            3,
        )

    def test_transient_error_advances_bounded_terminal_window(self) -> None:
        signature = (("definition", "WRONG", ()), ("callers", "ERROR", ()))
        self.assertEqual(
            _terminal_stability_count([Status.WRONG, Status.ERROR], signature, signature, 2),
            3,
        )

    def test_probe_deadline_preserves_stale_classification(self) -> None:
        self.assertEqual(_deadline_terminal([Status.PASS, Status.STALE]), "STALE")
        self.assertEqual(_deadline_terminal([]), "UNSUPPORTED")


if __name__ == "__main__":
    unittest.main()
