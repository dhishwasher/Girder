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


def expand_test_file_predictions(actual: Iterable[str], inventory: Iterable[str]) -> tuple[str, ...]:
    """Expand a native whole-file test choice to every test it would run."""

    inventory_norm = normalize_paths(tuple(inventory))
    expanded: list[str] = []
    for identity in normalize_paths(tuple(actual)):
        if not identity.endswith("::*"):
            expanded.append(identity)
            continue
        prefix = identity[:-1]
        matches = [candidate for candidate in inventory_norm if candidate.startswith(prefix)]
        expanded.extend(matches or [identity])
    return normalize_paths(expanded)


def assess_test_predictions(
    actual: Iterable[str],
    expected: Iterable[str],
    inventory: Iterable[str],
    *,
    prior_expected: Iterable[str] | None = None,
    prior_inventory: Iterable[str] | None = None,
    native_status: Status | None = None,
) -> tuple[Status, SetScore, tuple[str, ...]]:
    """Score file- or function-granularity tests against current and prior states."""

    raw = tuple(actual)
    current_actual = expand_test_file_predictions(raw, inventory)
    expected_norm = normalize_paths(tuple(expected))
    score = score_set(current_actual, expected_norm)
    if native_status is not None and native_status is not Status.PASS:
        return native_status, score, current_actual
    if current_actual == expected_norm:
        return Status.PASS, score, current_actual
    if prior_expected is not None and prior_inventory is not None:
        prior_actual = expand_test_file_predictions(raw, prior_inventory)
        prior_norm = normalize_paths(tuple(prior_expected))
        if prior_norm != expected_norm and prior_actual == prior_norm:
            return Status.STALE, score, current_actual
    return Status.WRONG, score, current_actual


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
    if status is Status.WRONG:
        return Status.WRONG, score
    if source_text is None:
        return (Status.STALE if status is Status.STALE else Status.UNSUPPORTED), score
    current = expected_source_marker in source_text
    old = prior_source_marker is not None and prior_source_marker in source_text
    if status is Status.STALE and old:
        return Status.STALE, score
    if status is Status.PASS and current:
        return Status.PASS, score
    if old and not current:
        return Status.STALE, score
    return Status.WRONG, score
