import json
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from tools.dispatch_audit_scorer_typescript import (
    NEVER_COVERS,
    _decorator_callee_column,
    cell_label,
    extract_call_claims,
    extract_target_locations,
    find_covering_claim,
    line_text_matches,
    resolve_node_id,
    score,
    site_byte_offset,
)


def write_inspect_json(path: Path, nodes: list[dict]) -> None:
    path.write_text(json.dumps({"nodes": nodes}))


def claim_attr(claims_ron: str) -> list:
    return [["call_evidence_v1", claims_ron]]


class DecoratorCalleeColumnUnitTests(unittest.TestCase):
    def test_bare_name(self):
        line = "@IsEmail(undefined, {"
        col = _decorator_callee_column(line, 0, "@IsEmail")
        self.assertEqual(line[col : col + 7], "IsEmail")

    def test_dotted_name_anchors_last_segment(self):
        line = "@foo.bar.Baz()"
        col = _decorator_callee_column(line, 0, "@foo.bar.Baz")
        self.assertEqual(line[col : col + 3], "Baz")

    def test_whitespace_after_at(self):
        line = "@ IsEmail()"
        col = _decorator_callee_column(line, 0, "@ IsEmail")
        self.assertEqual(line[col : col + 7], "IsEmail")


class SiteByteOffsetUnitTests(unittest.TestCase):
    def _write_and_offset(self, tmp: Path, filename: str, content: str, site: dict) -> int | None:
        (tmp / filename).write_text(content, encoding="utf-8")
        return site_byte_offset(tmp, site)

    def test_plain_call_offset(self):
        with TemporaryDirectory() as td:
            tmp = Path(td)
            content = "foo();\n"
            offset = self._write_and_offset(
                tmp, "a.ts", content,
                {"file": "a.ts", "line": 1, "shape": "plain_call", "text": "foo();"},
            )
            self.assertEqual(offset, 0)

    def test_keyword_skip_relocation(self):
        # `if (condition(x)) {` -- classify_line_with_match must skip past
        # the rejected "if" match to find "condition(", the same way
        # classify_line itself does; the scorer must land on the same spot.
        with TemporaryDirectory() as td:
            tmp = Path(td)
            content = "    if (condition(x)) {\n"
            offset = self._write_and_offset(
                tmp, "a.ts", content,
                {"file": "a.ts", "line": 1, "shape": "plain_call", "text": "if (condition(x)) {"},
            )
            self.assertEqual(content.encode("utf-8")[offset : offset + 9].decode(), "condition")

    def test_decorator_offset_anchors_callee_not_at_sign(self):
        with TemporaryDirectory() as td:
            tmp = Path(td)
            content = "  @IsEmail(undefined, {\n"
            offset = self._write_and_offset(
                tmp, "a.ts", content,
                {"file": "a.ts", "line": 1, "shape": "decorator", "text": "@IsEmail(undefined, {"},
            )
            self.assertEqual(content.encode("utf-8")[offset : offset + 7].decode(), "IsEmail")

    def test_crlf_line_endings_do_not_corrupt_the_byte_offset(self):
        # Found live against the real typescript-6.0.3 corpus (CRLF
        # throughout): reconstructing the prefix from `splitlines()`'s own
        # \r-stripped lines undercounts by one byte per preceding line.
        # Three CRLF-terminated lines before the real call is enough to
        # expose a 3-byte error, the same class of bug that produced a
        # 29,876-byte error 29,877 lines into the real file.
        with TemporaryDirectory() as td:
            tmp = Path(td)
            content = "const a = 1;\r\nconst b = 2;\r\nconst c = 3;\r\nreal(1);\r\n"
            (tmp / "a.ts").write_bytes(content.encode("utf-8"))
            offset = site_byte_offset(
                tmp,
                {"file": "a.ts", "line": 4, "shape": "plain_call", "text": "real(1);"},
            )
            raw_bytes = content.encode("utf-8")
            self.assertIsNotNone(offset)
            self.assertEqual(raw_bytes[offset : offset + 4].decode(), "real")
            # Sanity: prove this test would have caught the original bug --
            # the LF-only-assuming computation would have landed 3 bytes
            # short (one missing \r per of the 3 preceding lines).
            self.assertNotEqual(raw_bytes[offset - 3 : offset + 1].decode(), "real")

    def test_utf8_byte_offset_after_non_ascii_line(self):
        # A preceding line with multibyte UTF-8 characters must not throw
        # off the byte-offset computation for a later line -- this is the
        # real shape of site 11 (validation-functions-and-decorators.spec.ts:3373),
        # which sits a few lines below Japanese/Chinese string literals.
        with TemporaryDirectory() as td:
            tmp = Path(td)
            content = (
                "const s = 'ひらがな・カタカナ、．漢字';\n"
                "const t = '中文';\n"
                "real(1);\n"
            )
            offset = self._write_and_offset(
                tmp, "a.ts", content,
                {"file": "a.ts", "line": 3, "shape": "plain_call", "text": "real(1);"},
            )
            raw_bytes = content.encode("utf-8")
            self.assertEqual(raw_bytes[offset : offset + 4].decode(), "real")
            # Sanity check this test actually exercises multibyte content:
            # a naive character-offset (not byte-offset) computation would
            # have produced a different, wrong answer here.
            first_two_lines_chars = len(content.split("\n")[0]) + 1 + len(content.split("\n")[1]) + 1
            first_two_lines_bytes = len(("\n".join(content.split("\n")[:2]) + "\n").encode("utf-8"))
            self.assertNotEqual(first_two_lines_chars, first_two_lines_bytes)

    def test_returns_none_when_shape_no_longer_matches(self):
        with TemporaryDirectory() as td:
            tmp = Path(td)
            content = "not a call at all\n"
            offset = self._write_and_offset(
                tmp, "a.ts", content,
                {"file": "a.ts", "line": 1, "shape": "plain_call", "text": "not a call at all"},
            )
            self.assertIsNone(offset)


class LineTextMatchesUnitTests(unittest.TestCase):
    def test_matches(self):
        with TemporaryDirectory() as td:
            tmp = Path(td)
            (tmp / "a.ts").write_text("foo();\n", encoding="utf-8")
            self.assertTrue(
                line_text_matches(tmp, {"file": "a.ts", "line": 1, "text": "foo();"})
            )

    def test_drift_detected(self):
        with TemporaryDirectory() as td:
            tmp = Path(td)
            (tmp / "a.ts").write_text("bar();\n", encoding="utf-8")
            self.assertFalse(
                line_text_matches(tmp, {"file": "a.ts", "line": 1, "text": "foo();"})
            )


class FindCoveringClaimUnitTests(unittest.TestCase):
    def test_innermost_claim_wins_among_nested_claims(self):
        # expect(() => schema.parse([...])).toThrow() -- site 96's real
        # shape: multiple claims can contain the same byte offset (the
        # outer expect(...) call, the arrow function body, and .parse(...)
        # itself). The tightest (smallest span) claim must win.
        claims = [
            {"file": "a.ts", "start_byte": 0, "end_byte": 100, "class": "unknown", "reason": "r1", "targets": [], "caller": "outer"},
            {"file": "a.ts", "start_byte": 10, "end_byte": 50, "class": "unknown", "reason": "r2", "targets": [], "caller": "middle"},
            {"file": "a.ts", "start_byte": 20, "end_byte": 30, "class": "must", "reason": "proven-top-level-lexical-binding", "targets": ["abc"], "caller": "innermost"},
        ]
        claim = find_covering_claim(claims, "a.ts", 25)
        self.assertEqual(claim["caller"], "innermost")

    def test_never_covers_excludes_whole_module_gap(self):
        claims = [
            {"file": "a.ts", "start_byte": 0, "end_byte": 1000, "class": "unknown", "reason": "implicit-runtime-dispatch-not-certified", "targets": [], "caller": "module"},
        ]
        claim = find_covering_claim(claims, "a.ts", 25)
        self.assertIsNone(claim)

    def test_duplicate_semantic_path_also_never_covers(self):
        claims = [
            {"file": "a.ts", "start_byte": 0, "end_byte": 1000, "class": "unknown", "reason": "duplicate-semantic-path", "targets": [], "caller": "module"},
        ]
        claim = find_covering_claim(claims, "a.ts", 25)
        self.assertIsNone(claim)

    def test_parse_error_also_never_covers(self):
        # Confirmed whole-module in typescript-6.0.3 (4 instances, each
        # start_byte == 0 spanning the entire file) -- see
        # before-observation-addendum.md.
        claims = [
            {"file": "a.ts", "start_byte": 0, "end_byte": 1000, "class": "unknown", "reason": "parse-error", "targets": [], "caller": "module"},
        ]
        claim = find_covering_claim(claims, "a.ts", 25)
        self.assertIsNone(claim)

    def test_narrow_decorator_gap_claim_is_not_excluded(self):
        # unexpanded-macro-or-decorator is narrow (checked directly against
        # real inspect output, see the module docstring) and must still be
        # scoreable, unlike the two whole-module reasons above.
        claims = [
            {"file": "a.ts", "start_byte": 10, "end_byte": 24, "class": "unknown", "reason": "unexpanded-macro-or-decorator", "targets": [], "caller": "x"},
        ]
        claim = find_covering_claim(claims, "a.ts", 15)
        self.assertIsNotNone(claim)


class CellLabelUnitTests(unittest.TestCase):
    def test_exact(self):
        self.assertEqual(cell_label("must", "must"), "exact")

    def test_unsafe_exclusion(self):
        self.assertEqual(cell_label("must", "excluded"), "unsafe_exclusion")

    def test_overclaim_from_must(self):
        self.assertEqual(cell_label("unknown", "must"), "overclaim")

    def test_conservative(self):
        self.assertEqual(cell_label("must", "unknown"), "conservative")


class ResolveNodeIdUnitTests(unittest.TestCase):
    def test_converts_decimal_with_parens_to_padded_hex(self):
        # The same conversion, and the same bug class, as
        # correction-2/common.py's resolve_target: format(x, "x") strips
        # leading zeros and silently fails to match ids that start with 0.
        self.assertEqual(resolve_node_id("(999)"), format(999, "016x"))

    def test_matches_an_id_starting_with_a_leading_zero_hex_digit(self):
        # 4 maps to hex "4", padded "0000000000000004" -- exactly the
        # class of id an unpadded conversion would fail to match.
        self.assertEqual(resolve_node_id("(4)"), "0000000000000004")


class ExtractHelpersUnitTests(unittest.TestCase):
    def test_extract_call_claims(self):
        with TemporaryDirectory() as td:
            tmp = Path(td) / "inspect.json"
            ron = (
                '(version:1,source_fingerprint:(1),assumptions:[],calls:['
                '(site:(start_byte:0,end_byte:5,start_row:0,start_col:0),'
                'class:must,targets:[(123)],reason:"proven-top-level-lexical-binding",coverage_gap:false)])'
            )
            write_inspect_json(
                tmp,
                [{"id": "abc", "file": "a.ts", "path": "crate::a::foo", "attributes": claim_attr(ron)}],
            )
            claims = extract_call_claims(tmp)
            self.assertEqual(len(claims), 1)
            self.assertEqual(claims[0]["class"], "must")
            # Raw RON target text, parenthesized decimal -- NOT yet
            # converted to the zero-padded hex node id format; see
            # resolve_node_id and its own tests below.
            self.assertEqual(claims[0]["targets"], ["(123)"])

    def test_extract_target_locations(self):
        with TemporaryDirectory() as td:
            tmp = Path(td) / "inspect.json"
            write_inspect_json(
                tmp,
                [
                    {
                        "id": "abc123",
                        "file": "a.ts",
                        "span": {"start_row": 19, "end_row": 21},
                    }
                ],
            )
            locations = extract_target_locations(tmp)
            self.assertEqual(locations["abc123"], ("a.ts", 20))


class ScoreIntegrationUnitTests(unittest.TestCase):
    def test_must_pointing_at_wrong_definition_is_overclaim_not_exact(self):
        # Synthetic reproduction of the zod src/ vs deno/lib/ risk: Girder
        # claims "must" and resolves to a node that IS named the same, but
        # lives at the WRONG file:line relative to true_target. The node
        # "id" must be a real zero-padded hex string (format(999, "016x")),
        # not the bare "999" an earlier draft of this test used -- that
        # version passed too, but for the wrong reason (resolve_node_id
        # couldn't find "999" in `locations` at all, an unresolved-id
        # failure, not an actual file/line mismatch); this repeats the
        # exact hex-padding bug class correction-2 found, inside this
        # test's own fixture rather than the code under test.
        with TemporaryDirectory() as td:
            tmp = Path(td)
            pkg_root = tmp / "pkg"
            pkg_root.mkdir()
            (pkg_root / "a.ts").write_text("foo();\n", encoding="utf-8")

            sites_path = tmp / "sites.json"
            sites_path.write_text(
                json.dumps(
                    {
                        "sites": [
                            {
                                "package": "pkg",
                                "file": "a.ts",
                                "line": 1,
                                "shape": "plain_call",
                                "text": "foo();",
                                "true_class": "must",
                                "confidence": "high",
                                "true_target": "a.ts:99",
                            }
                        ]
                    }
                )
            )

            inspect_path = tmp / "pkg-inspect.json"
            ron = (
                '(version:1,source_fingerprint:(1),assumptions:[],calls:['
                '(site:(start_byte:0,end_byte:5,start_row:0,start_col:0),'
                'class:must,targets:[(999)],reason:"proven-top-level-lexical-binding",coverage_gap:false)])'
            )
            write_inspect_json(
                inspect_path,
                [
                    {"id": "caller1", "file": "a.ts", "path": "crate::a::caller", "attributes": claim_attr(ron)},
                    {"id": "00000000000003e7", "file": "a.ts", "path": "crate::a::wrong_foo", "span": {"start_row": 49, "end_row": 51}},
                ],
            )

            result = score(sites_path, {"pkg": pkg_root}, {"pkg": inspect_path})
            self.assertEqual(result["results"][0]["cell"], "overclaim")

    def test_must_pointing_at_correct_definition_is_exact(self):
        with TemporaryDirectory() as td:
            tmp = Path(td)
            pkg_root = tmp / "pkg"
            pkg_root.mkdir()
            (pkg_root / "a.ts").write_text("foo();\n", encoding="utf-8")

            sites_path = tmp / "sites.json"
            sites_path.write_text(
                json.dumps(
                    {
                        "sites": [
                            {
                                "package": "pkg",
                                "file": "a.ts",
                                "line": 1,
                                "shape": "plain_call",
                                "text": "foo();",
                                "true_class": "must",
                                "confidence": "high",
                                "true_target": "a.ts:20",
                            }
                        ]
                    }
                )
            )

            inspect_path = tmp / "pkg-inspect.json"
            ron = (
                '(version:1,source_fingerprint:(1),assumptions:[],calls:['
                '(site:(start_byte:0,end_byte:5,start_row:0,start_col:0),'
                'class:must,targets:[(999)],reason:"proven-top-level-lexical-binding",coverage_gap:false)])'
            )
            write_inspect_json(
                inspect_path,
                [
                    {"id": "caller1", "file": "a.ts", "path": "crate::a::caller", "attributes": claim_attr(ron)},
                    {"id": "00000000000003e7", "file": "a.ts", "path": "crate::a::foo", "span": {"start_row": 19, "end_row": 21}},
                ],
            )

            result = score(sites_path, {"pkg": pkg_root}, {"pkg": inspect_path})
            self.assertEqual(result["results"][0]["cell"], "exact")

    def test_must_with_no_true_target_is_failed_not_exact(self):
        with TemporaryDirectory() as td:
            tmp = Path(td)
            pkg_root = tmp / "pkg"
            pkg_root.mkdir()
            (pkg_root / "a.ts").write_text("foo();\n", encoding="utf-8")

            sites_path = tmp / "sites.json"
            sites_path.write_text(
                json.dumps(
                    {
                        "sites": [
                            {
                                "package": "pkg",
                                "file": "a.ts",
                                "line": 1,
                                "shape": "plain_call",
                                "text": "foo();",
                                "true_class": "must",
                                "confidence": "high",
                            }
                        ]
                    }
                )
            )
            inspect_path = tmp / "pkg-inspect.json"
            ron = (
                '(version:1,source_fingerprint:(1),assumptions:[],calls:['
                '(site:(start_byte:0,end_byte:5,start_row:0,start_col:0),'
                'class:must,targets:[(999)],reason:"proven-top-level-lexical-binding",coverage_gap:false)])'
            )
            write_inspect_json(
                inspect_path,
                [{"id": "caller1", "file": "a.ts", "path": "crate::a::caller", "attributes": claim_attr(ron)}],
            )
            result = score(sites_path, {"pkg": pkg_root}, {"pkg": inspect_path})
            self.assertEqual(result["results"][0]["status"], "failed")


if __name__ == "__main__":
    unittest.main()
