"""Frozen correctness, freshness, and precision/recall scoring."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable

from .protocol import Status, normalize_paths


@dataclass(frozen=True)
class SetScore:
    true_positive: int
    false_positive: int
    false_negative: int
    precision: float
    recall: float
    f1: float

    def to_dict(self) -> dict[str, float | int]:
        return vars(self)


def score_set(actual: Iterable[str], expected: Iterable[str]) -> SetScore:
    actual_set = set(normalize_paths(tuple(actual)))
    expected_set = set(normalize_paths(tuple(expected)))
    true_positive = len(actual_set & expected_set)
    false_positive = len(actual_set - expected_set)
    false_negative = len(expected_set - actual_set)
    precision = true_positive / len(actual_set) if actual_set else (1.0 if not expected_set else 0.0)
    recall = true_positive / len(expected_set) if expected_set else (1.0 if not actual_set else 0.0)
    f1 = 0.0 if precision + recall == 0 else 2 * precision * recall / (precision + recall)
    return SetScore(true_positive, false_positive, false_negative, precision, recall, f1)


def assess(
    actual: Iterable[str],
    expected: Iterable[str],
    *,
    prior_expected: Iterable[str] | None = None,
    native_status: Status | None = None,
) -> tuple[Status, SetScore]:
    """Classify exact correctness; matching an old oracle is explicitly stale."""

    actual_norm = normalize_paths(tuple(actual))
    expected_norm = normalize_paths(tuple(expected))
    score = score_set(actual_norm, expected_norm)
    if native_status is not None and native_status is not Status.PASS:
        return native_status, score
    if actual_norm == expected_norm:
        return Status.PASS, score
    if prior_expected is not None:
        prior_norm = normalize_paths(tuple(prior_expected))
        if prior_norm != expected_norm and actual_norm == prior_norm:
            return Status.STALE, score
    return Status.WRONG, score


def assess_definition(
    actual: Iterable[str],
    expected: Iterable[str],
    *,
    source_text: str | None,
    expected_source_marker: str,
    prior_expected: Iterable[str] | None = None,
    prior_source_marker: str | None = None,
    native_status: Status | None = None,
) -> tuple[Status, SetScore]:
    """Require both declaration identity and frozen source evidence."""

    status, score = assess(
        actual,
        expected,
        prior_expected=prior_expected,
        native_status=native_status,
    )
    if native_status is not None and native_status is not Status.PASS:
        return status, score
    if source_text is None:
        return Status.UNSUPPORTED, score
    current = expected_source_marker in source_text
    old = prior_source_marker is not None and prior_source_marker in source_text
    if status is Status.STALE and old:
        return Status.STALE, score
    if status is Status.PASS and current:
        return Status.PASS, score
    if old and not current:
        return Status.STALE, score
    return Status.WRONG, score
