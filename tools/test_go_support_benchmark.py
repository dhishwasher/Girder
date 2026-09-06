#!/usr/bin/env python3

import unittest

try:
    from tools.go_support_benchmark import evaluate_cases, summarize_cases
except ModuleNotFoundError:
    from go_support_benchmark import evaluate_cases, summarize_cases


class GoSupportBenchmarkTests(unittest.TestCase):
    def test_is_test_probes_read_the_real_node_attribute(self) -> None:
        repository = {
            "semantic_cases": [
                {
                    "id": "positive",
                    "kind": "IsTest",
                    "source": "crate::TestReal",
                    "target": "crate::TestReal",
                    "expected_present": True,
                },
                {
                    "id": "negative",
                    "kind": "IsTest",
                    "source": "crate::helper",
                    "target": "crate::helper",
                    "expected_present": False,
                },
            ]
        }
        nodes = [
            {"path": "crate::TestReal", "attributes": [["is_test", "true"]]},
            {"path": "crate::helper", "attributes": []},
        ]
        cases = evaluate_cases(repository, nodes, [])
        self.assertEqual([case["classification"] for case in cases], ["tp", "tn"])

    def test_missing_negative_endpoint_is_reported_as_an_invalid_probe(self) -> None:
        repository = {
            "semantic_cases": [
                {
                    "id": "missing",
                    "kind": "Calls",
                    "source": "crate::caller",
                    "target": "crate::missing",
                    "expected_present": False,
                }
            ]
        }
        cases = evaluate_cases(repository, [{"path": "crate::caller"}], [])
        self.assertEqual(cases[0]["classification"], "tn")
        self.assertEqual(summarize_cases(cases)["missing_endpoints"], 1)


if __name__ == "__main__":
    unittest.main()
