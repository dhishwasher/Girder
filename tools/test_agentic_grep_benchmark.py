"""Pure/local harness gates; no model or Cargo execution in routine CI."""

import copy
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools import agentic_grep_benchmark as bench


class FileToolsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.base = Path(self.temporary.name)
        self.root = self.base / "agent"
        self.root.mkdir()
        self.backend = bench.GrepBackend(self.root)
        (self.root / "b.py").write_text("first\nneedle two\nneedle three\nlast\n")
        (self.root / "a.py").write_text("needle one\n")

    def tearDown(self):
        self.temporary.cleanup()

    def data(self, tool, args):
        result = self.backend.call(tool, args)
        self.assertFalse(result["isError"], result)
        return json.loads(result["content"][0]["text"])

    def test_listing_pages_are_sorted_and_nonoverlapping(self):
        first = self.data("list_files", {"limit": 1})
        second = self.data("list_files", {"offset": first["next_offset"], "limit": 1})
        self.assertEqual(first, {"items": ["a.py"], "next_offset": 1})
        self.assertEqual(second, {"items": ["b.py"], "next_offset": None})

    def test_listing_filters_are_chosen_by_agent(self):
        self.assertEqual(self.data("list_files", {"glob": "b.*"})["items"], ["b.py"])

    def test_offset_reads_do_not_require_whole_file(self):
        result = self.data("offset_read", {"path": "b.py", "offset": 1, "limit": 2})
        self.assertEqual(result, {"items": [{"line": 2, "text": "needle two"}, {"line": 3, "text": "needle three"}], "next_offset": 3})
        self.assertEqual(self.data("offset_read", {"path": "b.py", "offset": 3})["items"], [{"line": 4, "text": "last"}])

    def test_read_at_and_beyond_eof(self):
        for offset in (4, 100):
            self.assertEqual(self.data("offset_read", {"path": "b.py", "offset": offset}), {"items": [], "next_offset": None})

    def test_unicode_and_missing_final_newline(self):
        (self.root / "unicode.py").write_text("café\nλ")
        self.assertEqual(self.data("offset_read", {"path": "unicode.py", "offset": 1})["items"], [{"line": 2, "text": "λ"}])

    @unittest.skipUnless(shutil.which("rg"), "ripgrep required for local tool integration")
    def test_ripgrep_pagination_preserves_global_matching_line_offsets(self):
        first = self.data("ripgrep", {"query": "needle", "limit": 2})
        second = self.data("ripgrep", {"query": "needle", "offset": first["next_offset"], "limit": 2})
        self.assertEqual([(x["file"], x["line"]) for x in first["items"]], [("a.py", 1), ("b.py", 2)])
        self.assertEqual([(x["file"], x["line"]) for x in second["items"]], [("b.py", 3)])
        self.assertIsNone(second["next_offset"])

    @unittest.skipUnless(shutil.which("rg"), "ripgrep required")
    def test_ripgrep_filters_and_fixed_strings(self):
        (self.root / "literal.py").write_text("literal [abc]\n")
        self.assertEqual(len(self.data("ripgrep", {"query": "[abc]", "fixed_strings": True, "glob": "literal.py"})["items"]), 1)
        self.assertEqual(self.data("ripgrep", {"query": "NEEDLE", "ignore_case": True, "glob": "a.py"})["items"][0]["file"], "a.py")

    @unittest.skipUnless(shutil.which("rg"), "ripgrep required")
    def test_invalid_regex_is_a_countable_error_result(self):
        result = self.backend.call("ripgrep", {"query": "["})
        self.assertTrue(result["isError"])
        self.assertGreater(len(bench.compact(result).encode()), 0)

    def test_bad_pagination_and_unknown_fields_are_rejected(self):
        for args in ({"offset": -1}, {"limit": 0}, {"limit": 201}, {"offset": True}, {"shell": "pwd"}):
            self.assertTrue(self.backend.call("list_files", args)["isError"])

    def test_traversal_absolute_and_symlink_paths_do_not_leak_evaluator_data(self):
        secret = "EVALUATOR_ONLY_EXPECTED_PATH_SENTINEL"
        hidden = self.base / "target-data.json"
        hidden.write_text(secret)
        (self.root / "alias.py").symlink_to(hidden)
        for path in ("../target-data.json", str(hidden), "alias.py", "nested/../../target-data.json", "..\\target-data.json"):
            result = self.backend.call("offset_read", {"path": path})
            self.assertTrue(result["isError"])
            self.assertNotIn(secret, bench.compact(result))
        self.assertNotIn("alias.py", self.data("list_files", {})["items"])

    @unittest.skipUnless(shutil.which("rg"), "ripgrep required")
    def test_evaluator_siblings_are_outside_search_scope(self):
        (self.base / "targets.json").write_text("EVALUATOR_SENTINEL")
        result = self.data("ripgrep", {"query": "EVALUATOR_SENTINEL"})
        self.assertEqual(result["items"], [])

    @unittest.skipUnless(shutil.which("rg"), "ripgrep required")
    def test_ignored_directories_and_graph_artifacts_remain_hidden_under_broad_glob(self):
        for relative in (".git/a", "sub/target/a.py", "node_modules/a.py", "sub/__pycache__/a.py", "project.aether", "sub/project.aether.journal"):
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("HIDDEN_NEEDLE")
        self.assertEqual(self.data("list_files", {"glob": "*"})["items"], ["a.py", "b.py"])
        self.assertEqual(self.data("ripgrep", {"query": "HIDDEN_NEEDLE", "glob": "*"})["items"], [])

    def test_output_limit_is_explicit(self):
        (self.root / "huge.py").write_text("x" * (bench.MAX_BYTES + 1))
        self.assertTrue(self.backend.call("offset_read", {"path": "huge.py"})["isError"])


class FakeBackend:
    docs = bench.GREP_TOOLS

    def __init__(self, result=None):
        self.result = result or bench.envelope("café λ", True)
        self.calls = []

    def call(self, name, arguments, timeout):
        self.calls.append((name, arguments, timeout))
        return self.result


def response(action, tokens=10):
    return {"message": {"content": bench.compact(action)}, "prompt_eval_count": tokens, "eval_count": 7}


TOOL = {"action": "tool", "name": "list_files", "arguments": {}}
FINAL = {"action": "final", "path": "crate::answer"}


class ArmAccountingTests(unittest.TestCase):
    def run_sequence(self, actions, backend=None, counter=lambda messages: 10):
        replies = iter(actions)
        return bench.run_arm("Find a declaration", backend or FakeBackend(), lambda messages, timeout: response(next(replies)), counter)

    def test_utf8_error_envelope_counts_once_and_excludes_model_prose(self):
        backend = FakeBackend()
        result = self.run_sequence([TOOL, FINAL], backend)
        expected = len(bench.compact(backend.result).encode("utf-8"))
        self.assertEqual(result["tool_output_bytes"], expected)
        self.assertEqual(result["produced_tool_output_bytes"], expected)
        self.assertEqual(result["tool_calls"], 1)
        self.assertEqual(result["model_requests"], 2)
        self.assertGreater(expected, len(bench.compact(backend.result)))

    def test_prior_tool_history_is_not_double_counted_as_a_new_response(self):
        backend = FakeBackend()
        result = self.run_sequence([TOOL, TOOL, FINAL], backend)
        self.assertEqual(result["tool_output_bytes"], 2 * len(bench.compact(backend.result).encode()))
        self.assertEqual(result["model_requests"], 3)

    def test_context_exhaustion_has_no_silent_history_truncation_or_delivery_claim(self):
        histories = []
        def count(messages):
            histories.append(copy.deepcopy(messages))
            return 10 if len(messages) == 2 else 8000
        result = self.run_sequence([TOOL], counter=count)
        self.assertEqual(result["reason"], "context_exhausted")
        self.assertEqual(result["model_requests"], 1)
        self.assertEqual(result["tool_output_bytes"], 0)
        self.assertGreater(result["produced_tool_output_bytes"], 0)
        self.assertEqual(histories[0], histories[1][:2])
        self.assertEqual(len(histories[1]), 4)

    def test_initial_context_exhaustion_makes_no_request(self):
        result = self.run_sequence([], counter=lambda messages: 8192)
        self.assertEqual(result["model_requests"], 0)
        self.assertEqual(result["reason"], "context_exhausted")

    def test_30_tool_budget_allows_a_final_request_after_the_last_result(self):
        result = self.run_sequence([TOOL] * 30 + [FINAL])
        self.assertEqual((result["tool_calls"], result["model_requests"]), (30, 31))
        self.assertEqual(result["answer"], "crate::answer")

    def test_31st_tool_action_is_not_executed(self):
        backend = FakeBackend()
        result = self.run_sequence([TOOL] * 31, backend)
        self.assertEqual(len(backend.calls), 30)
        self.assertEqual(result["reason"], "tool_budget_exhausted")

    def test_request_timeout_preserves_attempts_and_uncertain_delivery(self):
        calls = 0
        def model(messages, timeout):
            nonlocal calls
            calls += 1
            if calls == 1:
                return response(TOOL)
            raise TimeoutError("request timed out")
        result = bench.run_arm("task", FakeBackend(), model, lambda messages: 10)
        self.assertEqual(result["reason"], "request_timeout")
        self.assertEqual(result["model_requests"], 2)
        self.assertEqual(result["tool_calls"], 1)
        self.assertEqual(result["tool_output_bytes"], 0)
        self.assertGreater(result["unconfirmed_tool_output_bytes"], 0)

    def test_task_arm_deadline_rejects_a_late_final_answer(self):
        current = [0.0]
        def model(messages, timeout):
            self.assertEqual(timeout, 2)
            current[0] = 3
            return response(FINAL)
        result = bench.run_arm("task", FakeBackend(), model, lambda messages: 10, now=lambda: current[0], arm_timeout=2)
        self.assertEqual(result["reason"], "task_arm_timeout")
        self.assertIsNone(result["answer"])

    def test_tokenizer_disagreement_stops_campaign_instead_of_accepting_truncated_context(self):
        record = bench.new_arm()
        with self.assertRaises(bench.CampaignIntegrityError):
            bench.run_arm("task", FakeBackend(), lambda messages, timeout: response(FINAL, 9), lambda messages: 10, record=record)
        self.assertEqual(record["reason"], "tokenizer_accounting_mismatch")
        self.assertIsNone(record["answer"])

    def test_invalid_envelope_and_abstention_are_no_answer(self):
        for action in ({"action": "final", "path": None}, {"action": "final", "unexpected": "data"}):
            result = self.run_sequence([action])
            self.assertIsNone(result["answer"])

    def test_each_arm_starts_a_fresh_conversation(self):
        starts = []
        def model(messages, timeout):
            starts.append(copy.deepcopy(messages))
            return response(FINAL)
        for _ in range(2):
            bench.run_arm("same task", FakeBackend(), model, lambda messages: 10)
        self.assertEqual(starts[0], starts[1])
        self.assertEqual(len(starts[0]), 2)


class AssessmentTests(unittest.TestCase):
    def setUp(self):
        self.tasks = [{"id": str(i), "prompt_class": "identifier" if i < 12 else "description"} for i in range(15)]
        self.targets = [{"id": str(i), "expected_path": "crate::target" + str(i), "exclusion": None} for i in range(15)]
        self.records = [{"id": target["id"], **{arm: {**bench.new_arm(), "answer": target["expected_path"], "tool_calls": 2, "tool_output_bytes": size} for arm, size in (("grep", 100), ("graph", 80))}} for target in self.targets]

    def assess(self):
        return bench.assess(self.tasks, self.targets, self.records)

    def test_all_15_correct_with_exact_boundary_passes(self):
        self.assertEqual(self.assess()["overall"], "PASS")
        self.assertEqual(self.assess()["groups"]["all"]["comparable_pairs"], 15)

    def test_matching_wrong_answers_fail(self):
        for arm in ("grep", "graph"):
            self.records[0][arm]["answer"] = "crate::same_but_wrong"
        result = self.assess()
        self.assertEqual(result["overall"], "FAIL")
        self.assertEqual(result["groups"]["all"]["comparable_pairs"], 14)
        for arm in ("grep", "graph"):
            self.assertEqual(result["groups"]["identifier"]["outcomes"][arm]["wrong"], 1)

    def test_failed_costs_are_retained_but_excluded_from_comparison(self):
        self.records[12]["grep"].update(answer=None, tool_output_bytes=999999, model_requests=30)
        original = copy.deepcopy(self.records)
        result = self.assess()
        self.assertEqual(result["groups"]["all"]["comparable_costs"]["grep"]["tool_output_bytes"], 1400)
        self.assertEqual(result["groups"]["description"]["outcomes"]["grep"]["no_answer"], 1)
        self.assertEqual(result["groups"]["description"]["comparable_pairs"], 2)
        self.assertEqual(self.records, original)

    def test_one_excess_byte_fails_immutable_cost_gate(self):
        self.records[0]["graph"]["tool_output_bytes"] += 1
        self.assertEqual(self.assess()["overall"], "FAIL")

    def test_one_excess_call_fails_even_when_bytes_win(self):
        self.records[0]["graph"]["tool_calls"] += 1
        self.assertEqual(self.assess()["overall"], "FAIL")

    def test_class_losses_are_reported_even_when_aggregate_passes(self):
        self.records[0]["graph"]["tool_output_bytes"] = 110
        self.records[1]["graph"]["tool_output_bytes"] = 40
        result = self.assess()
        self.assertEqual(result["overall"], "PASS")
        self.assertEqual(result["groups"]["identifier"]["graph_byte_losses"], ["0"])
        self.assertEqual(result["groups"]["description"]["graph_byte_losses"], [])

    def test_exclusion_cannot_claim_original_all_15_pass(self):
        self.targets[14]["exclusion"] = "Explicitly retained ambiguity reason"
        result = self.assess()
        self.assertEqual(result["overall"], "FAIL")
        self.assertFalse(result["all_15_correct"])
        self.assertEqual(result["groups"]["all"]["excluded"][0]["reason"], self.targets[14]["exclusion"])

    def test_incomplete_campaign_and_forged_pass_field_do_not_pass(self):
        self.records[0]["graph"] = {**bench.new_arm(), "pass": True}
        self.assertEqual(self.assess()["overall"], "FAIL")


class FrozenCorpusTests(unittest.TestCase):
    def setUp(self):
        self.tasks = json.loads((bench.ROOT / "docs/agentic-grep-prompts.json").read_text())["tasks"]
        self.targets = json.loads((bench.ROOT / "docs/evaluator/agentic-grep-targets.json").read_text())["targets"]
        self.originals = json.loads((bench.ROOT / "docs/orient-tool-policy.json").read_text())["corpus"]

    def test_all_original_targets_are_pinned_without_prompt_leaks(self):
        bench.validate_corpus(self.tasks, self.targets, self.originals)

    def test_repinning_an_answer_is_rejected(self):
        self.targets[0]["expected_path"] = "crate::observed_answer"
        with self.assertRaises(bench.CampaignIntegrityError):
            bench.validate_corpus(self.tasks, self.targets, self.originals)

    def test_duplicate_or_removed_tasks_are_rejected(self):
        self.tasks[0] = self.tasks[1]
        with self.assertRaises(bench.CampaignIntegrityError):
            bench.validate_corpus(self.tasks, self.targets, self.originals)

    def test_description_cannot_reveal_identifier(self):
        self.tasks[12]["prompt"] += " dijkstra"
        with self.assertRaises(bench.CampaignIntegrityError):
            bench.validate_corpus(self.tasks, self.targets, self.originals)

    def test_wrong_or_absent_local_model_stops_without_substitution(self):
        policy = json.loads(bench.POLICY.read_text())
        for models in ([], [{"name": bench.MODEL, "digest": "wrong"}]):
            with patch.object(bench, "http_json", return_value={"models": models}):
                with self.assertRaises(bench.CampaignIntegrityError):
                    bench.verify_model("http://localhost", policy)

    def test_changed_threshold_is_rejected_before_git_or_model_execution(self):
        policy = json.loads(bench.POLICY.read_text())
        policy["gates"]["max_graph_to_grep_output_bytes"] = 1.2
        with self.assertRaises(bench.CampaignIntegrityError):
            bench.verify_frozen(policy)


class ProcessTimeoutTests(unittest.TestCase):
    def test_timeout_terminates_and_reaps_the_tool(self):
        process = bench.ProcessLines([sys.executable, "-c", "import time; time.sleep(30)"], bench.ROOT)
        try:
            with self.assertRaises(TimeoutError):
                process.line(0.03)
        finally:
            process.close()
        self.assertIsNotNone(process.process.poll())


if __name__ == "__main__":
    unittest.main()
