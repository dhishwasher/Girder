import unittest

from tools.dispatch_audit_site_selector_typescript import (
    classify_line,
    mask_ts_source,
    stratified_sample,
)


class ClassifyLineUnitTests(unittest.TestCase):
    def test_decorator(self):
        self.assertEqual(classify_line("@Injectable()"), "decorator")

    def test_decorator_bare(self):
        self.assertEqual(classify_line("@readonly"), "decorator")

    def test_dynamic_dispatch_bracket_call(self):
        self.assertEqual(classify_line("    obj[methodName]()"), "dynamic_dispatch")

    def test_dynamic_dispatch_call_method(self):
        self.assertEqual(classify_line("    fn.call(thisArg, x)"), "dynamic_dispatch")

    def test_dynamic_dispatch_apply_method(self):
        self.assertEqual(classify_line("    fn.apply(thisArg, args)"), "dynamic_dispatch")

    def test_dynamic_dispatch_reflect_apply(self):
        self.assertEqual(classify_line("    Reflect.apply(fn, obj, args)"), "dynamic_dispatch")

    def test_dynamic_dispatch_new_function(self):
        self.assertEqual(classify_line("    const f = new Function('return 1')"), "dynamic_dispatch")

    def test_optional_chaining_call(self):
        self.assertEqual(classify_line("    obj?.method()"), "optional_chaining_call")

    def test_optional_chaining_bare_call(self):
        self.assertEqual(classify_line("    fn?.()"), "optional_chaining_call")

    def test_new_expression(self):
        self.assertEqual(classify_line("    const x = new Foo(1, 2)"), "new_expression")

    def test_new_expression_with_generic(self):
        self.assertEqual(classify_line("    const m = new Map<string, number>()"), "new_expression")

    def test_qualified_attribute_call(self):
        self.assertEqual(classify_line("    this.ctx.obj.invoke(cmd)"), "qualified_attribute_call")

    def test_method_call(self):
        self.assertEqual(classify_line("    this.callers(id)"), "method_call")

    def test_method_call_with_generic(self):
        self.assertEqual(classify_line("    list.map<string>(fn)"), "method_call")

    def test_plain_call(self):
        self.assertEqual(classify_line("    target(x, y)"), "plain_call")

    def test_plain_call_with_generic(self):
        self.assertEqual(classify_line("    identity<number>(1)"), "plain_call")

    def test_a_real_call_after_a_keyword_is_still_a_plain_call(self):
        self.assertEqual(classify_line("    if (condition(x)) {"), "plain_call")

    def test_keyword_immediately_followed_by_a_paren_is_not_a_call(self):
        self.assertIsNone(classify_line("    if (x) {"))

    def test_super_call_is_a_real_plain_call(self):
        # super(args) is a real parent-constructor call, unlike `if (x)`.
        self.assertEqual(classify_line("    super(a, b);"), "plain_call")

    def test_dynamic_import_is_a_real_plain_call(self):
        self.assertEqual(classify_line("    const m = import('./mod');"), "plain_call")

    def test_function_declaration_is_still_plain_call_shaped_at_the_line_level(self):
        # Matches Python's own established precedent (labeling-rubric.md
        # case 9): a declaration's own `name(...)` is picked up as a
        # call-shaped candidate by this line-level selector; excluding it
        # is a ground-truth LABELING decision (`not_a_call_site`), not a
        # selector-level exclusion -- the selector is deliberately crude.
        self.assertEqual(classify_line("function foo(x) {"), "plain_call")

    def test_class_extends_clause_has_no_parens_so_is_not_call_shaped(self):
        # Unlike Python's `class Foo(Bar):`, TypeScript's extends clause
        # has no parens (`class Foo extends Bar {`), so this specific
        # collision Python's selector has to handle does not arise here.
        self.assertIsNone(classify_line("class Foo extends Bar {"))

    def test_for_loop_is_not_a_call(self):
        self.assertIsNone(classify_line("    for (let i = 0; i < n; i++) {"))

    def test_empty_line_is_none(self):
        self.assertIsNone(classify_line("    "))

    def test_more_specific_shape_wins_over_plain_call(self):
        # this.foo() must be method_call, not plain_call.
        self.assertEqual(classify_line("    this.foo()"), "method_call")

    def test_dynamic_dispatch_wins_over_method_call_for_bracket_notation(self):
        self.assertEqual(classify_line("    obj[key]()"), "dynamic_dispatch")


class MaskTsSourceUnitTests(unittest.TestCase):
    def test_line_comment_is_masked(self):
        src = "foo(); // bar(baz)\n"
        masked = mask_ts_source(src)
        self.assertNotIn("bar(baz)", masked)
        self.assertIn("foo();", masked)
        self.assertEqual(len(masked), len(src))

    def test_block_comment_is_masked_single_line(self):
        src = "foo(/* bar(baz) */);\n"
        masked = mask_ts_source(src)
        self.assertNotIn("bar(baz)", masked)
        self.assertEqual(len(masked), len(src))

    def test_block_comment_is_masked_multi_line_preserving_newlines(self):
        src = "foo(/* bar(\nbaz)\n*/);\n"
        masked = mask_ts_source(src)
        self.assertNotIn("bar(", masked)
        self.assertNotIn("baz)", masked)
        # line count must be preserved
        self.assertEqual(src.count("\n"), masked.count("\n"))

    def test_single_quoted_string_is_masked(self):
        src = "x = 'call(me)';\n"
        masked = mask_ts_source(src)
        self.assertNotIn("call(me)", masked)
        self.assertEqual(len(masked), len(src))

    def test_double_quoted_string_is_masked(self):
        src = 'x = "call(me)";\n'
        masked = mask_ts_source(src)
        self.assertNotIn("call(me)", masked)

    def test_escaped_quote_inside_string_does_not_end_it_early(self):
        src = "x = 'it\\'s call(me)';\nreal(1);\n"
        masked = mask_ts_source(src)
        self.assertNotIn("call(me)", masked)
        self.assertIn("real(1);", masked)

    def test_template_literal_text_is_masked(self):
        src = "x = `plain text call(me)`;\n"
        masked = mask_ts_source(src)
        self.assertNotIn("call(me)", masked)

    def test_template_literal_interpolation_is_not_masked(self):
        src = "x = `hello ${getName()}`;\n"
        masked = mask_ts_source(src)
        self.assertIn("getName()", masked)

    def test_nested_template_interpolation_is_not_masked(self):
        src = "x = `outer ${`inner ${real(1)}`}`;\n"
        masked = mask_ts_source(src)
        self.assertIn("real(1)", masked)

    def test_object_literal_inside_interpolation_does_not_confuse_brace_tracking(self):
        src = "x = `val ${JSON.stringify({a: 1, b: 2})} end`;\nafter(1);\n"
        masked = mask_ts_source(src)
        self.assertIn("JSON.stringify({a: 1, b: 2})", masked)
        self.assertIn("after(1);", masked)

    def test_text_after_interpolation_inside_template_is_still_masked(self):
        src = "x = `${a} literal call(me) text`;\n"
        masked = mask_ts_source(src)
        self.assertIn("a", masked)
        self.assertNotIn("call(me)", masked)

    def test_regex_literal_is_masked(self):
        src = "const re = /foo(bar)/;\nreal(1);\n"
        masked = mask_ts_source(src)
        self.assertNotIn("foo(bar)", masked)
        self.assertIn("real(1);", masked)

    def test_division_after_identifier_is_not_treated_as_regex(self):
        src = "const x = a / b(1);\n"
        masked = mask_ts_source(src)
        self.assertIn("b(1)", masked)

    def test_division_after_closing_paren_is_not_treated_as_regex(self):
        src = "const x = foo() / bar(1);\n"
        masked = mask_ts_source(src)
        self.assertIn("bar(1)", masked)

    def test_regex_after_return_keyword(self):
        src = "return /foo(bar)/.test(s);\nreal(1);\n"
        masked = mask_ts_source(src)
        self.assertNotIn("foo(bar)", masked)
        self.assertIn(".test(s)", masked)
        self.assertIn("real(1);", masked)

    def test_regex_with_character_class_containing_slash(self):
        src = "const re = /[a/b]call(x)/;\nreal(1);\n"
        masked = mask_ts_source(src)
        # The whole regex, including the char class, must be masked --
        # a bare `/` inside `[...]` must not end the regex early.
        self.assertNotIn("call(x)", masked)
        self.assertIn("real(1);", masked)

    def test_escaped_backslash_before_quote_does_not_disable_the_escape(self):
        # 'a\\' + call(x) -- the string is 'a\\', properly terminated;
        # call(x) is real code outside the string.
        src = "x = 'a\\\\' + call(x);\n"
        masked = mask_ts_source(src)
        self.assertIn("call(x)", masked)

    def test_preserves_total_length_and_line_count_on_realistic_snippet(self):
        src = (
            "class Foo {\n"
            "  @decorator()\n"
            "  bar(x: string): void {\n"
            "    // a comment with fake(call)\n"
            "    const re = /x(y)/;\n"
            "    const s = `hi ${this.baz(x)}`;\n"
            "    this.qux(s);\n"
            "  }\n"
            "}\n"
        )
        masked = mask_ts_source(src)
        self.assertEqual(len(masked), len(src))
        self.assertEqual(src.count("\n"), masked.count("\n"))
        self.assertIn("this.baz(x)", masked)
        self.assertIn("this.qux(s);", masked)
        self.assertNotIn("fake(call)", masked)
        self.assertNotIn("x(y)", masked)


class StratifiedSampleUnitTests(unittest.TestCase):
    def test_deterministic_for_a_fixed_seed(self):
        sites = [
            {"package": "p", "file": "f.ts", "line": i, "shape": s, "text": "x"}
            for i, s in enumerate(["plain_call", "method_call", "decorator"] * 5)
        ]
        first = stratified_sample(sites, 6, seed=1)
        second = stratified_sample(sites, 6, seed=1)
        self.assertEqual(first, second)

    def test_respects_total_count(self):
        sites = [
            {"package": "p", "file": "f.ts", "line": i, "shape": "plain_call", "text": "x"}
            for i in range(50)
        ]
        selected = stratified_sample(sites, 10, seed=1)
        self.assertEqual(len(selected), 10)


if __name__ == "__main__":
    unittest.main()
