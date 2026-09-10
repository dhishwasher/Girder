import copy
import tempfile
import sys
import unittest
from unittest import mock
from pathlib import Path
from tools.mcp_watch_benchmark import assess, claim_output, equal, read_records, write_artifact
from tools import harness_support

CORPUS = {"cases": [{"id": "case", "mutations": [{"id": "body"}]}]}
ORACLE = {"equal_source": True, "equal_persisted": True, "source_sha256": "a" * 64,
          "cold_source_sha256": "a" * 64, "persisted_sha256": "b" * 64, "cold_reconciled_sha256": "b" * 64}
INITIAL = {"kind": "initial", "case": "case", "seconds": 1, "oracle": ORACLE}
RECORD = {"kind": "completed", "id": "case/body", "oracle": ORACLE, "mutation_to_publication_seconds": 0.8,
          "metrics": {"attempts": 3, "builds": 2, "published": 1, "discarded_candidates": 1, "failed_attempts": 1,
                      "full_parsing": 1, "parsed_files": 4, "reused_files": 2, "fallback_reasons": {"uncertain_event_mapping": 1}}}
TRANSCRIPT = [{"kind": "initial_started", "case": "case"}, INITIAL,
              {"kind": "started", "id": "case/body"}, RECORD]


class WatchAssessmentTests(unittest.TestCase):
    def test_retry_costs_and_denominator_remain_visible(self):
        result = assess(TRANSCRIPT, CORPUS)
        self.assertEqual(result["overall"], "PASS")
        self.assertEqual(result["transcript_errors"], [])
        self.assertEqual(result["fallback_denominator"], 2)
        self.assertEqual(result["fallback_percentage"], 50)
        self.assertEqual(result["metrics_for_completed_mutations"]["discarded_candidates"], 1)
        self.assertEqual(result["metrics_for_completed_mutations"]["failed_attempts"], 1)

    def test_source_and_persistence_must_both_equal(self):
        for key in ("cold_source_sha256", "cold_reconciled_sha256"):
            record = copy.deepcopy(RECORD)
            record["oracle"][key] = "c" * 64
            record["overall"] = "PASS"
            self.assertFalse(equal(record))
            result = assess([*TRANSCRIPT[:-1], record], CORPUS)
            self.assertEqual(result["overall"], "FAIL")
            self.assertEqual(result["metrics_for_completed_mutations"]["parsed_files"], 4)

    def test_missing_duplicate_and_unexpected_records_cannot_pass(self):
        self.assertEqual(assess([RECORD], CORPUS)["overall"], "FAIL")
        self.assertEqual(assess([*TRANSCRIPT, RECORD], CORPUS)["overall"], "FAIL")
        wrong = dict(RECORD, id="case/replacement")
        self.assertEqual(assess([*TRANSCRIPT[:-1], wrong], CORPUS)["overall"], "FAIL")

    def test_every_phase_must_appear_once_in_frozen_order(self):
        for index in range(len(TRANSCRIPT)):
            with self.subTest(missing=index):
                result = assess(TRANSCRIPT[:index] + TRANSCRIPT[index + 1:], CORPUS)
                self.assertEqual(result["overall"], "FAIL")
                self.assertTrue(result["transcript_errors"])
            with self.subTest(duplicate=index):
                result = assess(TRANSCRIPT[:index] + [TRANSCRIPT[index]] + TRANSCRIPT[index:], CORPUS)
                self.assertEqual(result["overall"], "FAIL")
                self.assertTrue(result["transcript_errors"])
        reversed_mutation = [*TRANSCRIPT[:2], RECORD, TRANSCRIPT[2]]
        self.assertEqual(assess(reversed_mutation, CORPUS)["overall"], "FAIL")
        for index, identity in ((0, "case"), (1, "case"), (2, "id"), (3, "id")):
            with self.subTest(missing_identity=index):
                incomplete = copy.deepcopy(TRANSCRIPT)
                del incomplete[index][identity]
                result = assess(incomplete, CORPUS)
                self.assertEqual(result["overall"], "FAIL")
                self.assertTrue(result["transcript_errors"])

    def test_invalid_extra_rows_fail_without_losing_completed_retry_costs(self):
        for extra in ({"kind": "incomplete_record", "raw": '{"kind":'},
                      {"kind": "started", "id": "case/unexpected"},
                      {"kind": "initial_started", "case": "unexpected"},
                      {"kind": "unknown"}, None):
            with self.subTest(extra=extra):
                result = assess([*TRANSCRIPT, extra], CORPUS)
                self.assertEqual(result["overall"], "FAIL")
                self.assertTrue(result["transcript_errors"])
                self.assertEqual(result["equal_mutations"], 1)
                for key in ("attempts", "builds", "discarded_candidates", "failed_attempts", "parsed_files", "reused_files"):
                    self.assertEqual(result["metrics_for_completed_mutations"][key], RECORD["metrics"][key])
                self.assertEqual(result["fallback_denominator"], 2)
                self.assertEqual(result["fallback_percentage"], 50)

    def test_interrupted_mutation_preserves_previous_completed_counters(self):
        corpus = copy.deepcopy(CORPUS)
        corpus["cases"][0]["mutations"].append({"id": "second"})
        result = assess([*TRANSCRIPT, {"kind": "started", "id": "case/second"}], corpus)
        self.assertEqual(result["overall"], "FAIL")
        self.assertEqual(result["completed_mutations"], 1)
        self.assertEqual(result["metrics_for_completed_mutations"]["failed_attempts"], 1)
        self.assertEqual(result["fallback_denominator"], 2)

    def test_no_completed_builds_is_unavailable_not_zero(self):
        result = assess([], CORPUS)
        self.assertIsNone(result["fallback_percentage"])
        self.assertIsNone(result["median_mutation_to_publication_seconds"])
        self.assertEqual(result["overall"], "FAIL")

    def test_partial_last_line_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "probe.jsonl"
            path.write_text('{"kind":"started","id":"case/body"}\n{"kind":')
            records = read_records(path)
            self.assertEqual(records[0]["id"], "case/body")
            self.assertEqual(records[1], {"kind": "incomplete_record", "raw": '{"kind":'})


class WatchArtifactTests(unittest.TestCase):
    def test_timeout_preserves_already_captured_output(self):
        captured = []
        with self.assertRaisesRegex(RuntimeError, "timed out"):
            harness_support.run_bounded(
                [sys.executable, "-S", "-c", "import time; print('checkpoint', flush=True); time.sleep(10)"],
                cwd=Path.cwd(), timeout_seconds=1, max_output_bytes=1024,
                on_output=lambda name, chunk: captured.append((name, chunk)),
            )
        self.assertEqual(b"".join(chunk for name, chunk in captured if name == "stdout"), b"checkpoint\n")

    def test_interruption_preserves_already_captured_output(self):
        captured = []
        original = harness_support._read_available

        def interrupt_after_checkpoint(descriptor, limit):
            if captured:
                raise KeyboardInterrupt
            return original(descriptor, limit)

        with mock.patch.object(harness_support, "_read_available", side_effect=interrupt_after_checkpoint):
            with self.assertRaises(KeyboardInterrupt):
                harness_support.run_bounded(
                    [sys.executable, "-S", "-c", "import sys,time; print('checkpoint',flush=True); time.sleep(.05); print('next',file=sys.stderr,flush=True); time.sleep(10)"],
                    cwd=Path.cwd(), timeout_seconds=2, max_output_bytes=1024,
                    on_output=lambda name, chunk: captured.append((name, chunk)),
                )
        self.assertEqual(b"".join(chunk for name, chunk in captured if name == "stdout"), b"checkpoint\n")

    def test_any_existing_artifact_refuses_claim_without_changing_history(self):
        for suffix in (".json", "-probe.jsonl", "-stdout.log", "-stderr.log", "-failure.log"):
            with self.subTest(suffix=suffix), tempfile.TemporaryDirectory() as directory:
                output = Path(directory) / "observation.json"
                previous = Path(directory) / ("observation" + suffix)
                previous.write_bytes(b"original incomplete evidence\n")
                with self.assertRaises(FileExistsError):
                    claim_output(output)
                self.assertEqual(previous.read_bytes(), b"original incomplete evidence\n")
                self.assertEqual(set(Path(directory).iterdir()), {previous})

    def test_different_output_extension_cannot_reuse_existing_sidecars(self):
        with tempfile.TemporaryDirectory() as directory:
            previous = Path(directory) / "observation-stderr.log"
            previous.write_text("original counters\n")
            with self.assertRaises(FileExistsError):
                claim_output(Path(directory) / "observation.corrected")
            self.assertEqual(previous.read_text(), "original counters\n")

    def test_late_log_collision_cannot_overwrite_existing_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "observation.json"
            raw, stdout, stderr, failure = claim_output(output)
            self.assertEqual(output.read_text(), "{}\n")
            self.assertFalse(raw.exists())  # The Rust probe claims its JSONL itself.
            for path in (stdout, stderr, failure):
                write_artifact(path, "original\n")
                with self.assertRaises(FileExistsError):
                    write_artifact(path, "replacement\n")
                self.assertEqual(path.read_text(), "original\n")


if __name__ == "__main__":
    unittest.main()
