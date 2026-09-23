import unittest

from tools.dispatch_audit_gate_profile import attribute_or_macro_text


class AttributeOrMacroTextUnitTests(unittest.TestCase):
    def _claim(self, start, end):
        return {"start_byte": start, "end_byte": end}

    def test_inner_attribute_is_attr_not_macro(self):
        # A prior ad hoc version of this check matched only `#[`, which
        # misses inner attributes (`#![...]`) entirely -- they would be
        # silently classified as a macro invocation and dropped from the
        # transformed_scope attribute set.
        source = b"#![feature(test)]\nfn f() {}\n"
        kind, text = attribute_or_macro_text(source, self._claim(0, 18))
        self.assertEqual(kind, "attr")
        self.assertEqual(text, "#![feature(test)]")

    def test_outer_attribute_is_attr(self):
        source = b"#[cfg(test)]\nfn f() {}\n"
        kind, text = attribute_or_macro_text(source, self._claim(0, 12))
        self.assertEqual(kind, "attr")
        self.assertEqual(text, "#[cfg(test)]")

    def test_macro_invocation_is_macro(self):
        source = b"fn f() { println!(\"x\"); }\n"
        kind, text = attribute_or_macro_text(source, self._claim(9, 24))
        self.assertEqual(kind, "macro")
        self.assertEqual(text, "println!(\"x\");")


if __name__ == "__main__":
    unittest.main()
