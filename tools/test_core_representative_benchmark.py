import copy
import io
import json
import os
import stat
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

from tools.core_representative_benchmark import (
    DEFAULT_MANIFEST,
    DEFAULT_POLICY,
    MEASURE_PROCESS,
    atomic_write_json,
    evaluate_semantic_cases,
    evaluate_policy,
    extract_archive,
    safe_archive_name,
    source_inventory,
    summarize_semantics,
    validate_manifest,
    validate_policy,
)
from tools.harness_support import run_bounded


class RepresentativeBenchmarkUnitTests(unittest.TestCase):
    def write_archive(self, path: Path, members):
        with tarfile.open(path, "w:gz") as archive:
            for name, kind, content in members:
                info = tarfile.TarInfo(name)
                if kind == "directory":
                    info.type = tarfile.DIRTYPE
                    info.mode = 0o777
                    archive.addfile(info)
                elif kind == "file":
                    payload = content
                    info.size = len(payload)
                    info.mode = 0o777
                    archive.addfile(info, io.BytesIO(payload))
                elif kind == "symlink":
                    info.type = tarfile.SYMTYPE
                    info.linkname = content.decode()
                    archive.addfile(info)
                else:
                    raise AssertionError(kind)

    def test_checked_manifest_has_exact_language_balance_and_unique_cases(self):
        data = json.loads(DEFAULT_MANIFEST.read_text(encoding="utf-8"))

        repositories = validate_manifest(data)

        self.assertEqual(len(repositories), 6)
        self.assertEqual(
            [repository["language"] for repository in repositories].count("rust"), 3
        )
        self.assertEqual(
            [repository["language"] for repository in repositories].count("python"),
            3,
        )
        policy = json.loads(DEFAULT_POLICY.read_text(encoding="utf-8"))
        self.assertEqual(
            validate_policy(policy, {repository["id"] for repository in repositories})[
                "policy_id"
            ],
            "core-representative-beta-v1",
        )

    def test_manifest_validation_rejects_boolean_counts_and_unsafe_metadata(self):
        data = json.loads(DEFAULT_MANIFEST.read_text(encoding="utf-8"))
        invalid_count = copy.deepcopy(data)
        invalid_count["repositories"][0]["sources"]["files"] = True
        with self.assertRaisesRegex(RuntimeError, "source files"):
            validate_manifest(invalid_count)

        invalid_license = copy.deepcopy(data)
        invalid_license["repositories"][0]["license"]["files"] = ["../LICENSE"]
        with self.assertRaisesRegex(RuntimeError, "license metadata"):
            validate_manifest(invalid_license)

    def test_archive_path_validation_rejects_escape_and_platform_ambiguity(self):
        for unsafe in (
            "",
            "/absolute.py",
            "root/../escape.py",
            "root\\escape.py",
            "C:/drive.py",
            "root//duplicate.py",
        ):
            self.assertFalse(safe_archive_name(unsafe), unsafe)
        self.assertTrue(safe_archive_name("root/pkg/module.py"))

    def test_extraction_and_inventory_are_sanitized_and_deterministic(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "fixture.tar.gz"
            self.write_archive(
                archive,
                [
                    ("fixture", "directory", b""),
                    ("fixture/pkg", "directory", b""),
                    ("fixture/pkg/a.py", "file", b"print('a')\n"),
                    ("fixture/pkg/b.py", "file", b"print('b')"),
                    ("fixture/pkg/ignored.txt", "file", b"ignored\n"),
                    ("fixture/.hidden/ignored.py", "file", b"hidden\n"),
                ],
            )

            project = extract_archive(archive, root / "extracted", "fixture")
            first = source_inventory(project, {".py"})
            second = source_inventory(project, {".py"})

            self.assertEqual(first, second)
            self.assertEqual(first.files, 2)
            self.assertEqual(first.bytes, 21)
            self.assertEqual(first.physical_lines, 2)
            self.assertEqual([record[0] for record in first.records], ["pkg/a.py", "pkg/b.py"])
            self.assertEqual(stat.S_IMODE((project / "pkg/a.py").stat().st_mode), 0o600)
            self.assertEqual(stat.S_IMODE((project / "pkg").stat().st_mode), 0o700)

    def test_extraction_rejects_traversal_links_and_case_collisions(self):
        scenarios = (
            [("fixture/../escape.py", "file", b"bad")],
            [("fixture/link", "symlink", b"/tmp/escape")],
            [
                ("fixture/A.py", "file", b"a"),
                ("fixture/a.py", "file", b"b"),
            ],
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for index, members in enumerate(scenarios):
                archive = root / f"unsafe-{index}.tar.gz"
                self.write_archive(archive, members)
                with self.assertRaises(RuntimeError):
                    extract_archive(archive, root / f"out-{index}", "fixture")
                self.assertFalse((root / f"out-{index}").exists())

    def test_source_inventory_rejects_invalid_utf8_and_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "bad.py").write_bytes(b"\xff")
            with self.assertRaisesRegex(RuntimeError, "not UTF-8"):
                source_inventory(root, {".py"})
            (root / "bad.py").unlink()
            os.symlink("missing.py", root / "linked.py")
            with self.assertRaisesRegex(RuntimeError, "symlink"):
                source_inventory(root, {".py"})

    def test_semantic_cases_require_both_endpoints_and_classify_edges(self):
        repository = {
            "id": "fixture",
            "semantic_cases": [
                {
                    "id": "positive",
                    "kind": "Calls",
                    "source": "crate::caller",
                    "target": "crate::callee",
                    "expected_present": True,
                },
                {
                    "id": "negative",
                    "kind": "Calls",
                    "source": "crate::caller",
                    "target": "crate::decoy",
                    "expected_present": False,
                },
            ],
        }
        nodes = [
            {"path": "crate::caller"},
            {"path": "crate::callee"},
            {"path": "crate::decoy"},
        ]
        edges = [
            {"source": "crate::caller", "target": "crate::callee", "kind": "Calls"}
        ]

        outcomes = evaluate_semantic_cases(repository, nodes, edges)

        self.assertEqual([outcome["classification"] for outcome in outcomes], ["tp", "tn"])
        with self.assertRaisesRegex(RuntimeError, "endpoint missing"):
            evaluate_semantic_cases(repository, nodes[:2], edges)

    def test_semantic_summary_reports_micro_and_macro_rates(self):
        repositories = [
            {
                "language": "rust",
                "summary": {
                    "semantic": {
                        "tp": 2,
                        "fp": 1,
                        "fn": 0,
                        "tn": 1,
                        "precision": 0.666667,
                        "recall": 1.0,
                    }
                },
            },
            {
                "language": "python",
                "summary": {
                    "semantic": {
                        "tp": 1,
                        "fp": 0,
                        "fn": 1,
                        "tn": 0,
                        "precision": 1.0,
                        "recall": 0.5,
                    }
                },
            },
        ]

        summary = summarize_semantics(repositories)

        self.assertEqual(summary["aggregate"]["micro_precision"], 0.75)
        self.assertEqual(summary["aggregate"]["micro_recall"], 0.75)
        self.assertEqual(summary["aggregate"]["macro_precision"], 0.833333)
        self.assertEqual(summary["aggregate"]["macro_recall"], 0.75)

    def test_beta_policy_evaluator_fails_closed_on_eligibility_and_performance(self):
        manifest = json.loads(DEFAULT_MANIFEST.read_text(encoding="utf-8"))
        policy = validate_policy(
            json.loads(DEFAULT_POLICY.read_text(encoding="utf-8")),
            {repository["id"] for repository in manifest["repositories"]},
        )
        repositories = []
        for repository in manifest["repositories"]:
            run = {
                "inspect_wall_ms": 1,
                "graph": {"artifact_bytes": 1024},
            }
            repositories.append(
                {
                    "id": repository["id"],
                    "language": repository["language"],
                    "runs": [copy.deepcopy(run) for _ in range(5)],
                    "summary": {
                        "wall_median_ms": 100,
                        "wall_max_ms": 200,
                        "peak_rss_max_kib": 1024,
                        "artifact_unique_digests": 1,
                        "semantic_unique_digests": 1,
                        "unique_node_edge_count_pairs": 1,
                        "semantic": {
                            "tp": 1,
                            "fp": 0,
                            "fn": 0,
                            "tn": 1,
                            "precision": 1.0,
                            "recall": 1.0,
                        },
                    },
                }
            )
        semantic = summarize_semantics(repositories)

        passing = evaluate_policy(
            repositories,
            semantic,
            policy,
            runs=5,
            rayon_threads=2,
            source_worktree_clean=True,
        )
        self.assertTrue(passing["passed"], passing["failed_check_ids"])

        repositories[0]["summary"]["wall_max_ms"] = 30001
        failing = evaluate_policy(
            repositories,
            semantic,
            policy,
            runs=5,
            rayon_threads=2,
            source_worktree_clean=False,
        )
        self.assertFalse(failing["passed"])
        self.assertIn(
            "eligibility.source_worktree_clean", failing["failed_check_ids"]
        )
        self.assertIn(
            f"performance.{repositories[0]['id']}.analyze_max_ms",
            failing["failed_check_ids"],
        )

    def test_atomic_result_replacement_is_complete_and_private(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "result.json"
            atomic_write_json(output, {"old": True})
            atomic_write_json(output, {"new": True})

            self.assertEqual(json.loads(output.read_text(encoding="utf-8")), {"new": True})
            self.assertTrue(output.read_bytes().endswith(b"\n"))
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)

    @unittest.skipUnless(os.name == "posix" and hasattr(os, "wait4"), "requires wait4")
    def test_measurement_wrapper_records_one_child_peak_rss(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            usage = root / "usage.json"

            result = run_bounded(
                (
                    sys.executable,
                    str(MEASURE_PROCESS),
                    str(usage),
                    "--",
                    sys.executable,
                    "-c",
                    "payload = bytearray(1024 * 1024); print(len(payload))",
                ),
                cwd=root,
                timeout_seconds=10,
                max_output_bytes=64 * 1024,
            )

            measured = json.loads(usage.read_text(encoding="utf-8"))
            self.assertEqual(result.stdout.strip(), "1048576")
            self.assertEqual(measured["schema_version"], 1)
            self.assertGreater(measured["peak_rss_kib"], 0)


if __name__ == "__main__":
    unittest.main()
