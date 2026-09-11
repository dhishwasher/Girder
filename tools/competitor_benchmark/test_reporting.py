"""Tests for generated benchmark summaries."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from tools.competitor_benchmark.reporting import render_markdown, summarize_result, write_outputs


def record(root: Path, task: str, kind: str, status: str, *, tp: int = 1, fp: int = 0,
           fn: int = 0, calls: int = 1, stdout: bytes = b"x") -> dict:
    artifact = root / "raw" / (task.replace(":", "-") + ".stdout")
    artifact.parent.mkdir(exist_ok=True)
    artifact.write_bytes(stdout)
    return {
        "task_id": task, "query_kind": kind, "status": status,
        "elapsed_seconds": 0.1, "stdout_artifacts": [str(artifact)], "stderr_artifacts": [],
        "stdout_bytes": len(stdout), "stderr_bytes": 0, "tool_calls": calls,
        "peak_rss_bytes": 1024, "version": "1.0", "commit": "abc",
        "score": {"true_positive": tp, "false_positive": fp, "false_negative": fn},
    }


class ReportingTests(unittest.TestCase):
    def make_result(self, root: Path) -> Path:
        records = [record(root, "prepare", "prepare", "PASS")]
        for phase in ("warmup", "warm_query"):
            for kind in ("definition", "callers", "callees", "impact", "tests"):
                records.append(record(root, f"state0:{phase}:{kind}", kind, "PASS"))
        records.extend([
            record(root, "state1:probe1:callers", "callers", "STALE", fn=1),
            record(root, "state1:probe2:callers", "callers", "PASS"),
        ])
        result = {
            "product": "girder", "fixture": "tiny-python", "campaign_state": "COMPLETE",
            "elapsed_seconds": 2.0, "records": records,
            "mutation_summaries": [{"terminal_status": "PASS", "update_to_correct_seconds": 0.4}],
            "policy_id": "policy", "policy_sha256": "p", "freeze_manifest_sha256": "f",
            "harness_commit": "h",
        }
        path = root / "result.json"
        path.write_text(json.dumps(result))
        return path

    def test_summary_uses_one_warm_query_and_last_probe(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            row = summarize_result(self.make_result(Path(directory)), "raw.tar.gz")
        self.assertEqual(row["base_status_by_kind"]["callers"], "PASS")
        self.assertEqual(row["freshness_status_by_kind"]["callers"], {"PASS": 1})
        self.assertEqual(row["query_record_count"], 12)
        self.assertEqual(row["update_to_all_correct_seconds_mean"], 0.4)

    def test_raw_byte_mismatch_fails_generation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = self.make_result(root)
            data = json.loads(path.read_text())
            data["records"][0]["stdout_bytes"] = 99
            path.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, "raw byte mismatch"):
                summarize_result(path, "raw.tar.gz")

    def test_explicit_artifact_root_replays_moved_archive(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            original = base / "original"
            original.mkdir()
            path = self.make_result(original)
            moved = base / "extracted-campaign"
            original.rename(moved)
            row = summarize_result(moved / path.name, "raw.tar.gz", moved)
        self.assertEqual(row["query_record_count"], 12)

    def test_outputs_include_required_sections_and_csv(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            row = summarize_result(self.make_result(root), "raw.tar.gz")
            output = root / "out"
            write_outputs([row], output, "Measured result", "Scope note.")
            report = (output / "report.md").read_text()
            self.assertIn("## Where Girder Lost", report)
            self.assertIn("## Where Girder Won", report)
            self.assertIn("## What This Benchmark Does NOT Establish", report)
            self.assertIn("source_result_sha256", (output / "summary.csv").read_text())


if __name__ == "__main__":
    unittest.main()
