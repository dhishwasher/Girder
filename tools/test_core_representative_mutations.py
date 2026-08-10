import unittest

from tools.core_representative_mutations import (
    MUTATIONS,
    PROBE_HELPER_ANCHOR,
    aggregate_metrics,
    apply_mutation,
    parse_bitcode_selected_paths,
    _ratio,
    _selected_for_node,
)


class RepresentativeMutationsUnitTests(unittest.TestCase):
    def test_declared_mutations_are_internally_consistent(self):
        for mutation in MUTATIONS:
            anchor_lines = mutation.anchor.splitlines()
            for line in (anchor_lines[0], anchor_lines[-1]):
                self.assertIn(line, mutation.replacement)
            self.assertIn("_bitcode_mutation_probe", mutation.replacement)
            node_ids = [test.node_id for test in mutation.tests]
            self.assertEqual(len(node_ids), len(set(node_ids)))
            for test in mutation.tests:
                self.assertTrue(test.node_id.startswith("tests/"))
                self.assertIn("::", test.node_id)

    def test_parses_bitcode_impacted_test_lines(self):
        selected = parse_bitcode_selected_paths(
            """
Impacted tests (2):
  ✓ crate::tests::test_commands::test_other_command_forward (python)
  ✓ crate::tests::test_chain::test_basic_chaining (python)

  (1 other test(s) not in impact set — skipped)
"""
        )
        self.assertEqual(
            selected,
            {
                "crate::tests::test_commands::test_other_command_forward",
                "crate::tests::test_chain::test_basic_chaining",
            },
        )

    def test_selected_for_node_maps_pytest_id_to_graph_path(self):
        selected = {"crate::tests::test_commands::test_other_command_forward"}
        self.assertTrue(
            _selected_for_node(
                selected, "tests/test_commands.py::test_other_command_forward"
            )
        )
        self.assertFalse(
            _selected_for_node(
                selected, "tests/test_commands.py::test_other_command_invoke"
            )
        )

    def test_ratio_is_vacuously_perfect_on_an_empty_denominator(self):
        self.assertEqual(_ratio(0, 0), 1.0)
        self.assertEqual(_ratio(1, 2), 0.5)

    def test_aggregate_metrics_sums_across_mutations(self):
        results = [
            {
                "true_positives": ["a"],
                "false_positives": [],
                "false_negatives": ["b"],
                "true_negatives": ["c"],
            },
            {
                "true_positives": ["d", "e"],
                "false_positives": ["f"],
                "false_negatives": [],
                "true_negatives": [],
            },
        ]
        aggregate = aggregate_metrics(results)
        self.assertEqual(aggregate["true_positives"], 3)
        self.assertEqual(aggregate["false_positives"], 1)
        self.assertEqual(aggregate["false_negatives"], 1)
        self.assertEqual(aggregate["true_negatives"], 1)
        self.assertEqual(aggregate["precision"], 0.75)
        self.assertEqual(aggregate["recall"], 0.75)

    def test_apply_mutation_inserts_probe_helper_and_call(self):
        import tempfile
        from pathlib import Path

        mutation = MUTATIONS[0]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / mutation.file
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(
                PROBE_HELPER_ANCHOR + "\n" + mutation.anchor + "\n            pass\n",
                encoding="utf-8",
            )

            apply_mutation(root, mutation)

            mutated = target.read_text(encoding="utf-8")
            self.assertIn("_bitcode_mutation_probe", mutated)
            self.assertIn(mutation.replacement, mutated)
            # The probe helper is only inserted once even though later code
            # calls it.
            self.assertEqual(mutated.count("def _bitcode_mutation_probe"), 1)

    def test_apply_mutation_fails_closed_when_anchors_move(self):
        import tempfile
        from pathlib import Path

        mutation = MUTATIONS[0]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / mutation.file
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("completely different source\n", encoding="utf-8")

            with self.assertRaises(RuntimeError):
                apply_mutation(root, mutation)


if __name__ == "__main__":
    unittest.main()
