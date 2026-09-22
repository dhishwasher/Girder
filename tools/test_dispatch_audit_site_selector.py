import unittest

from tools.dispatch_audit_site_selector import classify_line, stratified_sample


class ClassifyLineUnitTests(unittest.TestCase):
    def test_macro_invocation(self):
        self.assertEqual(classify_line("    assert_eq!(a, b);"), "macro_invocation")

    def test_path_or_associated_call(self):
        self.assertEqual(classify_line("    Node::new(kind, name)"), "path_or_associated_call")

    def test_method_call(self):
        self.assertEqual(classify_line("    self.callers(id).map(|n| n.id)"), "method_call")

    def test_plain_call(self):
        self.assertEqual(classify_line("    target(x, y)"), "plain_call")

    def test_a_real_call_after_a_keyword_is_still_a_plain_call(self):
        # "condition(x)" is a genuine call; only the keyword ITSELF must
        # never be mistaken for the callee.
        self.assertEqual(classify_line("    if condition(x) {"), "plain_call")

    def test_keyword_immediately_followed_by_a_paren_is_not_a_call(self):
        # Unidiomatic but legal Rust: parens directly around a condition.
        # The regex alone would match "if (" as ident="if"; the keyword
        # guard must reject it, and nothing else on this line matches.
        self.assertIsNone(classify_line("    if (true) {"))

    def test_comment_line_is_none(self):
        self.assertIsNone(classify_line("    // target(x)"))

    def test_blank_line_is_none(self):
        self.assertIsNone(classify_line("    "))


class StratifiedSampleUnitTests(unittest.TestCase):
    def _sites(self, shape: str, count: int) -> list[dict[str, object]]:
        return [{"shape": shape, "file": f"f{i}.rs", "line": i} for i in range(count)]

    def test_sample_is_deterministic_for_a_fixed_seed(self):
        sites = self._sites("plain_call", 50) + self._sites("method_call", 50)
        a = stratified_sample(sites, total=20, seed=42)
        b = stratified_sample(sites, total=20, seed=42)
        self.assertEqual(a, b)

    def test_sample_tops_up_when_one_shape_is_scarce(self):
        # Only 2 fn_pointer sites exist; the shortfall must be made up from
        # other shapes so the precommitted total is still met.
        sites = self._sites("plain_call", 50) + self._sites("fn_pointer_or_closure", 2)
        selected = stratified_sample(sites, total=20, seed=1)
        self.assertEqual(len(selected), 20)

    def test_never_selects_more_sites_than_exist(self):
        sites = self._sites("plain_call", 5)
        selected = stratified_sample(sites, total=20, seed=1)
        self.assertEqual(len(selected), 5)


if __name__ == "__main__":
    unittest.main()
