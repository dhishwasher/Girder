import unittest

from tools.dispatch_audit_site_selector_python import (
    classify_line,
    mask_strings_and_comments,
    stratified_sample,
)


class ClassifyLineUnitTests(unittest.TestCase):
    def test_decorator(self):
        self.assertEqual(classify_line("@click.command()"), "decorator")

    def test_decorator_bare(self):
        self.assertEqual(classify_line("@property"), "decorator")

    def test_dynamic_dispatch_getattr(self):
        self.assertEqual(classify_line("    value = getattr(obj, name)"), "dynamic_dispatch")

    def test_dynamic_dispatch_callable(self):
        self.assertEqual(classify_line("    if callable(fn):"), "dynamic_dispatch")

    def test_qualified_attribute_call(self):
        self.assertEqual(
            classify_line("    self.ctx.obj.invoke(cmd)"), "qualified_attribute_call"
        )

    def test_method_call(self):
        self.assertEqual(classify_line("    self.callers(id)"), "method_call")

    def test_plain_call(self):
        self.assertEqual(classify_line("    target(x, y)"), "plain_call")

    def test_operator_dunder(self):
        self.assertEqual(classify_line("    total = a + b"), "operator_dunder")

    def test_a_real_call_after_a_keyword_is_still_a_plain_call(self):
        self.assertEqual(classify_line("    if condition(x):"), "plain_call")

    def test_keyword_immediately_followed_by_a_paren_is_not_a_call(self):
        self.assertIsNone(classify_line("    if (True):"))

    def test_comment_line_is_none(self):
        self.assertIsNone(classify_line("    # target(x)"))

    def test_blank_line_is_none(self):
        self.assertIsNone(classify_line("    "))

    def test_decorator_checked_before_qualified_attribute_call(self):
        # A dotted decorator ("@a.b.command()") must classify as decorator,
        # not qualified_attribute_call, since SHAPE_PATTERNS checks
        # decorator first.
        self.assertEqual(classify_line("@app.cli.command()"), "decorator")


class MaskStringsAndCommentsUnitTests(unittest.TestCase):
    def test_line_count_is_preserved(self):
        # The regression this locks in: an earlier version of the
        # multi-line-token branch dropped the newline on the first masked
        # line, shifting every subsequent line number by one -- silently
        # corrupting the site/line correspondence for any file with a
        # multi-line docstring/comment after the first one.
        sample = (
            'def f():\n'
            '    """\n'
            '    Example:\n'
            '        result = target_func(1, 2)\n'
            '    """\n'
            '    return real_call(1)\n'
        )
        masked, ok = mask_strings_and_comments(sample)
        self.assertTrue(ok)
        self.assertEqual(len(masked.splitlines()), len(sample.splitlines()))

    def test_docstring_content_is_blanked(self):
        sample = (
            'def f():\n'
            '    """\n'
            '        obj.method_call()\n'
            '    """\n'
            '    return real_call(1)\n'
        )
        masked, ok = mask_strings_and_comments(sample)
        self.assertTrue(ok)
        lines = masked.splitlines()
        self.assertNotIn("method_call", lines[2])
        self.assertIn("real_call", lines[4])

    def test_trailing_comment_is_blanked_but_call_survives(self):
        sample = "    return real_call(1)  # a trailing comment with fake(x)\n"
        masked, ok = mask_strings_and_comments(sample)
        self.assertTrue(ok)
        self.assertIn("real_call(1)", masked)
        self.assertNotIn("fake(x)", masked)

    def test_single_line_string_literal_is_blanked(self):
        sample = '    x = "call_looking(1, 2)"\n    real_call(x)\n'
        masked, ok = mask_strings_and_comments(sample)
        self.assertTrue(ok)
        lines = masked.splitlines()
        self.assertNotIn("call_looking", lines[0])
        self.assertIn("real_call(x)", lines[1])

    def test_code_outside_strings_is_untouched(self):
        sample = "real_call(1)\n"
        masked, ok = mask_strings_and_comments(sample)
        self.assertTrue(ok)
        self.assertEqual(masked, sample)


class StratifiedSampleUnitTests(unittest.TestCase):
    def _sites(self, shape: str, count: int) -> list[dict[str, object]]:
        return [{"shape": shape, "file": f"f{i}.py", "line": i} for i in range(count)]

    def test_sample_is_deterministic_for_a_fixed_seed(self):
        sites = self._sites("plain_call", 50) + self._sites("method_call", 50)
        a = stratified_sample(sites, total=20, seed=42)
        b = stratified_sample(sites, total=20, seed=42)
        self.assertEqual(a, b)

    def test_sample_tops_up_when_one_shape_is_scarce(self):
        sites = self._sites("plain_call", 50) + self._sites("dynamic_dispatch", 2)
        selected = stratified_sample(sites, total=20, seed=1)
        self.assertEqual(len(selected), 20)

    def test_never_selects_more_sites_than_exist(self):
        sites = self._sites("plain_call", 5)
        selected = stratified_sample(sites, total=20, seed=1)
        self.assertEqual(len(selected), 5)


if __name__ == "__main__":
    unittest.main()
