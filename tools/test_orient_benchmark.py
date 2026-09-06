#!/usr/bin/env python3
"""Unit tests for the orient-tool harness's pure logic.

The measurement itself needs a built `girder` binary and real extracted
repositories; these cover the parts that decide correctness and the
gating outcome, including the truncation-tolerance and identity-mismatch
cases the real run actually hit (see docs/orient-tool.md's Honest limits).
"""

from __future__ import annotations

import unittest
from pathlib import Path

try:
    from tools.orient_benchmark import (
        build_baseline_argv,
        build_checks,
        build_orient_argv,
        node_last_segment,
        parse_bullet_paths,
        parse_test_names,
        section_matches,
    )
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from orient_benchmark import (
        build_baseline_argv,
        build_checks,
        build_orient_argv,
        node_last_segment,
        parse_bullet_paths,
        parse_test_names,
        section_matches,
    )


ROOT = Path("/tmp/orient-test-root")


class ArgvBuildersMirrorTheMcpToolsTests(unittest.TestCase):
    """These must stay byte-for-byte in step with each Tool's `argv` closure
    in crates/aether-app/src/project/commands/mcp.rs, or the baseline this
    harness measures stops being the real MCP cost."""

    def test_get_source_pins_nodes_with_source_only(self) -> None:
        argv = build_baseline_argv("get_source", {"nodes": ["crate::a::b"]}, ROOT)
        self.assertEqual(
            argv, ["context", str(ROOT), "--json", "--source-only", "--nodes", "crate::a::b"]
        )

    def test_get_source_falls_back_to_intent(self) -> None:
        argv = build_baseline_argv("get_source", {"intent": "parse a file"}, ROOT)
        self.assertEqual(
            argv, ["context", str(ROOT), "--json", "--source-only", "parse a file"]
        )

    def test_ask_codebase_passes_the_question_as_one_argument(self) -> None:
        argv = build_baseline_argv("ask_codebase", {"question": "what calls x?"}, ROOT)
        self.assertEqual(argv, ["query", str(ROOT), "what calls x?"])

    def test_impacted_tests_appends_explicit_nodes(self) -> None:
        argv = build_baseline_argv("impacted_tests", {"nodes": ["crate::a::b"]}, ROOT)
        self.assertEqual(argv, ["test-impact", str(ROOT), "--quiet", "crate::a::b"])

    def test_search_code_passes_the_query_as_one_argument(self) -> None:
        argv = build_baseline_argv("search_code", {"query": "parse config"}, ROOT)
        self.assertEqual(argv, ["search", str(ROOT), "parse config"])

    def test_orient_argv_pins_a_symbol(self) -> None:
        argv = build_orient_argv({"kind": "symbol", "value": "crate::a::b"}, ROOT)
        self.assertEqual(argv, ["orient", str(ROOT), "--json", "--nodes", "crate::a::b"])

    def test_orient_argv_falls_back_to_intent(self) -> None:
        argv = build_orient_argv({"kind": "intent", "value": "parse a file"}, ROOT)
        self.assertEqual(argv, ["orient", str(ROOT), "--json", "parse a file"])


class ParseHelpersTests(unittest.TestCase):
    def test_parse_bullet_paths_strips_the_distance_suffix(self) -> None:
        stdout = "Q: impact of x\n[impact] 2 node(s)\n  · crate::a::b (distance 1)\n  · crate::c::d (distance 2)\n"
        self.assertEqual(parse_bullet_paths(stdout), {"crate::a::b", "crate::c::d"})

    def test_parse_bullet_paths_leaves_a_bare_caller_path_untouched(self) -> None:
        stdout = "Q: callers of x\n[callers] 1 function(s) call 'x'\n  · crate::a::caller\n"
        self.assertEqual(parse_bullet_paths(stdout), {"crate::a::caller"})

    def test_parse_test_names_ignores_blank_lines_only(self) -> None:
        self.assertEqual(parse_test_names("test_a\ntest_b\n\n"), {"test_a", "test_b"})

    def test_parse_test_names_of_empty_output_is_empty(self) -> None:
        self.assertEqual(parse_test_names(""), set())

    def test_node_last_segment_takes_the_final_path_component(self) -> None:
        self.assertEqual(node_last_segment("crate::a::b::test_add"), "test_add")

    def test_node_last_segment_of_a_bare_name_is_itself(self) -> None:
        self.assertEqual(node_last_segment("test_add"), "test_add")


class SectionMatchesTests(unittest.TestCase):
    def test_untruncated_section_requires_exact_equality(self) -> None:
        section = {"paths": ["crate::a", "crate::b"], "count": 2, "truncated": False}
        ok, _ = section_matches(section, {"crate::a", "crate::b"})
        self.assertTrue(ok)

    def test_untruncated_section_with_a_missing_path_fails(self) -> None:
        section = {"paths": ["crate::a"], "count": 1, "truncated": False}
        ok, _ = section_matches(section, {"crate::a", "crate::b"})
        self.assertFalse(ok)

    def test_truncated_path_section_passes_when_count_matches_and_listed_is_a_subset(self) -> None:
        # This is the click-command-invoke case: orient lists only the first
        # MAX_LISTED_PER_SECTION of a 303-node impact set, but reports the
        # true count.
        section = {"paths": ["crate::a"], "count": 3, "truncated": True}
        ok, _ = section_matches(section, {"crate::a", "crate::b", "crate::c"})
        self.assertTrue(ok)

    def test_truncated_path_section_fails_when_the_count_disagrees(self) -> None:
        section = {"paths": ["crate::a"], "count": 5, "truncated": True}
        ok, _ = section_matches(section, {"crate::a", "crate::b", "crate::c"})
        self.assertFalse(ok)

    def test_truncated_path_section_fails_when_a_listed_path_is_not_in_the_baseline(self) -> None:
        section = {"paths": ["crate::wrong"], "count": 1, "truncated": True}
        ok, _ = section_matches(section, {"crate::a"})
        self.assertFalse(ok)

    def test_truncated_name_only_section_ignores_a_count_disagreement(self) -> None:
        # This is the real click discrepancy: impacted_tests --quiet
        # deduplicates by bare name, so its distinct-name count can be
        # legitimately smaller than orient's true node count when two
        # different test files share a function name. Only subset is
        # checked for name_only sections.
        section = {
            "paths": ["crate::tests::a::test_x", "crate::tests::b::test_x"],
            "count": 2,
            "truncated": True,
        }
        ok, _ = section_matches(section, {"test_x"}, name_only=True)
        self.assertTrue(ok)

    def test_untruncated_name_only_section_still_requires_exact_name_equality(self) -> None:
        # This is the real websocket-writejson finding: impacted_tests
        # --quiet silently drops non-Rust/Python test names, so a genuine
        # extra name orient reports is a real correctness failure, not
        # tolerated just because the section happens to be name_only.
        section = {"paths": ["crate::TestDeprecatedJSON"], "count": 1, "truncated": False}
        ok, _ = section_matches(section, set(), name_only=True)
        self.assertFalse(ok)

    def test_missing_baseline_set_is_treated_as_empty(self) -> None:
        section = {"paths": [], "count": 0, "truncated": False}
        ok, _ = section_matches(section, None)
        self.assertTrue(ok)


class BuildChecksTests(unittest.TestCase):
    POLICY = {"threshold": {"max_round_trips_for_composite": 1, "max_byte_ratio_vs_baseline": 1.2}}

    def test_a_symbol_task_gates_all_three_checks(self) -> None:
        record = {
            "id": "some-task",
            "round_trips_composite": 1,
            "byte_ratio": 0.5,
            "correctness": True,
            "comparable": True,
        }
        checks = build_checks(self.POLICY, [record])
        self.assertTrue(all(c["gated"] for c in checks))
        self.assertTrue(all(c["passed"] for c in checks))

    def test_a_named_go_task_is_not_gated_on_byte_ratio_even_when_it_fails(self) -> None:
        record = {
            "id": "websocket-writejson",
            "round_trips_composite": 1,
            "byte_ratio": 5.0,
            "correctness": True,
            "comparable": True,
        }
        checks = build_checks(self.POLICY, [record])
        byte_ratio_check = next(c for c in checks if c["id"].endswith(".byte_ratio"))
        self.assertFalse(byte_ratio_check["gated"])
        self.assertFalse(byte_ratio_check["passed"])

    def test_a_non_comparable_intent_miss_gates_neither_byte_ratio_nor_correctness(self) -> None:
        record = {
            "id": "intent-task",
            "round_trips_composite": 1,
            "byte_ratio": 3.0,
            "correctness": None,
            "comparable": False,
        }
        checks = build_checks(self.POLICY, [record])
        gated_ids = {c["id"] for c in checks if c["gated"]}
        self.assertEqual(gated_ids, {"intent-task.round_trips"})

    def test_round_trips_is_always_gated(self) -> None:
        record = {
            "id": "any-task",
            "round_trips_composite": 2,
            "byte_ratio": 0.1,
            "correctness": True,
            "comparable": True,
        }
        checks = build_checks(self.POLICY, [record])
        round_trips_check = next(c for c in checks if c["id"].endswith(".round_trips"))
        self.assertTrue(round_trips_check["gated"])
        self.assertFalse(round_trips_check["passed"])


if __name__ == "__main__":
    unittest.main()
