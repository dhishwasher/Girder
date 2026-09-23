import unittest

from tools.dispatch_audit_scorer import cell_label, extract_call_claims, find_covering_claim


class CellLabelUnitTests(unittest.TestCase):
    def test_exact_match(self):
        for cls in ("must", "may", "unknown"):
            self.assertEqual(cell_label(cls, cls), "exact")

    def test_no_covering_claim_is_unsafe_exclusion(self):
        # "excluded" is what the scorer passes as observed_class when no
        # CallClaim's byte span covers the site at all.
        for expected in ("must", "may", "unknown"):
            self.assertEqual(cell_label(expected, "excluded"), "unsafe_exclusion")

    def test_false_must_is_overclaim(self):
        for expected in ("may", "unknown"):
            self.assertEqual(cell_label(expected, "must"), "overclaim")

    def test_false_may_against_unknown_is_overclaim(self):
        self.assertEqual(cell_label("unknown", "may"), "overclaim")

    def test_underclaiming_is_conservative(self):
        self.assertEqual(cell_label("must", "unknown"), "conservative")
        self.assertEqual(cell_label("may", "unknown"), "conservative")
        self.assertEqual(cell_label("must", "may"), "conservative")


class FindCoveringClaimUnitTests(unittest.TestCase):
    def _claim(self, file, start, end, cls="unknown"):
        return {"file": file, "start_byte": start, "end_byte": end, "class": cls,
                "reason": "r", "coverage_gap": False, "targets": [], "caller": "c"}

    def test_finds_claim_containing_offset(self):
        claims = [self._claim("a.rs", 10, 20)]
        self.assertIsNotNone(find_covering_claim(claims, "a.rs", 15))

    def test_returns_none_when_nothing_covers_the_offset(self):
        claims = [self._claim("a.rs", 10, 20)]
        self.assertIsNone(find_covering_claim(claims, "a.rs", 25))

    def test_ignores_claims_in_other_files(self):
        claims = [self._claim("other.rs", 0, 100)]
        self.assertIsNone(find_covering_claim(claims, "a.rs", 50))

    def test_offset_at_end_byte_boundary_is_not_covered(self):
        # end_byte is exclusive, matching how call_evidence_v1 spans work.
        claims = [self._claim("a.rs", 10, 20)]
        self.assertIsNone(find_covering_claim(claims, "a.rs", 20))

    def test_prefers_tightest_span_when_multiple_claims_cover_the_offset(self):
        claims = [self._claim("a.rs", 0, 100), self._claim("a.rs", 40, 60)]
        found = find_covering_claim(claims, "a.rs", 50)
        self.assertEqual((found["start_byte"], found["end_byte"]), (40, 60))


class ExtractCallClaimsUnitTests(unittest.TestCase):
    def test_parses_a_single_claim_from_ron(self):
        import json
        import tempfile
        from pathlib import Path

        doc = {
            "nodes": [
                {
                    "file": "src/lib.rs",
                    "path": "crate::lib::caller",
                    "attributes": [
                        [
                            "call_evidence_v1",
                            # targets are NodeId tuple structs (bare numeric hashes), not
                            # string paths -- matches the real RON encoding.
                            '(version:1,source_fingerprint:(1),assumptions:[],calls:'
                            '[(site:(start_byte:5,end_byte:12,start_row:0,start_col:5),'
                            'class:must,targets:[(9876543210)],'
                            'reason:"proven-top-level-lexical-binding",coverage_gap:false)])',
                        ]
                    ],
                }
            ]
        }
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump(doc, f)
            path = Path(f.name)
        try:
            claims = extract_call_claims(path)
            self.assertEqual(len(claims), 1)
            self.assertEqual(claims[0]["start_byte"], 5)
            self.assertEqual(claims[0]["end_byte"], 12)
            self.assertEqual(claims[0]["class"], "must")
            self.assertEqual(claims[0]["targets"], ["(9876543210)"])
        finally:
            path.unlink()


if __name__ == "__main__":
    unittest.main()
