import tempfile
import unittest
from pathlib import Path

from tools.dispatch_audit_gate_profile_python import profile_site


class ProfileSiteUnitTests(unittest.TestCase):
    def _claim(self, file, start, end, reason, caller="crate::a::caller"):
        return {
            "file": file,
            "caller": caller,
            "start_byte": start,
            "end_byte": end,
            "class": "unknown",
            "reason": reason,
            "coverage_gap": True,
            "targets": [],
        }

    def test_identifier_filter_pass_true_for_plain_call(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text("def target():\n    pass\ndef caller():\n    target()\n")
            site = {
                "package": "pkg",
                "file": "a.py",
                "line": 4,
                "shape": "plain_call",
                "true_class": "must",
                "text": "target()",
                "observed_caller": "crate::a::caller",
            }
            row = profile_site(site, root, [])
            self.assertTrue(row["identifier_filter_pass"])
            self.assertTrue(row["same_file_top_level_def_exists"])
            self.assertEqual(row["same_file_top_level_def_count"], 1)
            self.assertFalse(row["would_need_cross_file_resolution"])

    def test_identifier_filter_pass_false_for_method_call(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text("def caller(self):\n    self.fail()\n")
            site = {
                "package": "pkg",
                "file": "a.py",
                "line": 2,
                "shape": "method_call",
                "true_class": "must",
                "text": "self.fail()",
                "observed_caller": "crate::a::caller",
            }
            row = profile_site(site, root, [])
            self.assertFalse(row["identifier_filter_pass"])
            self.assertTrue(row["self_or_cls_method_dispatch"])

    def test_transformed_scope_trips_when_file_has_a_decorator_claim(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text("@click.command()\ndef f():\n    pass\ntarget()\n")
            claims = [self._claim("a.py", 0, 17, "unexpanded-macro-or-decorator")]
            site = {
                "package": "pkg",
                "file": "a.py",
                "line": 4,
                "shape": "plain_call",
                "true_class": "unknown",
                "text": "target()",
                "observed_caller": "crate::a::caller",
            }
            row = profile_site(site, root, claims)
            self.assertTrue(row["transformed_scope_trips"])

    def test_would_need_cross_file_resolution_when_no_same_file_def(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text("from other import target\ndef caller():\n    target()\n")
            site = {
                "package": "pkg",
                "file": "a.py",
                "line": 3,
                "shape": "plain_call",
                "true_class": "must",
                "text": "target()",
                "observed_caller": "crate::a::caller",
            }
            row = profile_site(site, root, [])
            self.assertFalse(row["same_file_top_level_def_exists"])
            self.assertTrue(row["would_need_cross_file_resolution"])

    def test_callee_name_uses_byte_offset_for_a_chained_call(self):
        # `MyModel(x=1234).model_dump_json()` -- the sampled call is
        # `.model_dump_json()`, not the constructor `MyModel(...)` earlier
        # on the same line. A version that took the FIRST identifier-paren
        # match on the line got this wrong.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = "class MyModel:\n    pass\nMyModel(x=1234).model_dump_json()\n"
            (root / "a.py").write_text(source)
            offset = source.index(".model_dump_json(")
            site = {
                "package": "pkg",
                "file": "a.py",
                "line": 3,
                "shape": "method_call",
                "true_class": "must",
                "text": "MyModel(x=1234).model_dump_json()",
                "byte_offset": offset,
                "observed_caller": "crate::a::caller",
            }
            row = profile_site(site, root, [])
            self.assertEqual(row["callee_name"], "model_dump_json")

    def test_callee_name_uses_byte_offset_for_a_nested_call(self):
        # `deprecated_from_orm(State, SimpleNamespace(...))` -- the sampled
        # call is the OUTER one, `deprecated_from_orm`. A version that took
        # the LAST identifier-paren match on the line got this wrong: it
        # picked up the nested `SimpleNamespace(...)` argument instead.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = "deprecated_from_orm(State, SimpleNamespace(x=1))\n"
            (root / "a.py").write_text(source)
            offset = source.index("deprecated_from_orm(")
            site = {
                "package": "pkg",
                "file": "a.py",
                "line": 1,
                "shape": "plain_call",
                "true_class": "must",
                "text": source.strip(),
                "byte_offset": offset,
                "observed_caller": "crate::a::caller",
            }
            row = profile_site(site, root, [])
            self.assertEqual(row["callee_name"], "deprecated_from_orm")

    def test_self_dispatch_detected_when_not_at_line_start(self):
        # `schema = self._apply_single_annotation(...)` -- the receiver
        # isn't at column 0. An earlier version only matched
        # `^\s*(self|cls)\.`, missing this and any other assignment form.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text("def f(self):\n    pass\n")
            site = {
                "package": "pkg",
                "file": "a.py",
                "line": 1,
                "shape": "method_call",
                "true_class": "must",
                "text": "schema = self._apply_single_annotation(schema, field_metadata)",
                "observed_caller": "crate::a::caller",
            }
            row = profile_site(site, root, [])
            self.assertTrue(row["self_or_cls_method_dispatch"])

    def test_cls_dispatch_detected_inside_a_boolean_expression(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text("def f(cls):\n    pass\n")
            site = {
                "package": "pkg",
                "file": "a.py",
                "line": 1,
                "shape": "method_call",
                "true_class": "must",
                "text": "if value is None or cls.is_true(value):",
                "observed_caller": "crate::a::caller",
            }
            row = profile_site(site, root, [])
            self.assertTrue(row["self_or_cls_method_dispatch"])

    def test_same_file_class_definition_is_a_distinct_gate_not_cross_file(self):
        # A call like `MetaclassArgumentsWithDefault(i=None)` constructs a
        # CLASS. claims.rs's `top` collection loop filters on
        # function_item/function_definition/function_declaration ONLY --
        # class_definition is never included, so this can never be proven
        # Must today no matter how "same-file" it is. A version of this
        # tool that folded classes into the same same_file_top_level_def
        # check as functions wrongly implied narrowing transformed_scope
        # alone would unblock this site (it would not) and wrongly implied
        # it needed cross-file resolution (it doesn't -- the class IS in
        # this file; claims.rs just never looks at class_definition nodes).
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text(
                "class MetaclassArgumentsWithDefault:\n    pass\n"
                "MetaclassArgumentsWithDefault(i=None)\n"
            )
            site = {
                "package": "pkg",
                "file": "a.py",
                "line": 3,
                "shape": "plain_call",
                "true_class": "must",
                "text": "MetaclassArgumentsWithDefault(i=None)",
                "observed_caller": "crate::a::caller",
            }
            row = profile_site(site, root, [])
            self.assertFalse(row["same_file_top_level_def_exists"])
            self.assertTrue(row["same_file_top_level_class_not_collected"])
            self.assertFalse(row["would_need_cross_file_resolution"])

    def test_same_file_def_count_flags_a_second_definition(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.py").write_text(
                "def target():\n    pass\ndef target():\n    pass\ndef caller():\n    target()\n"
            )
            site = {
                "package": "pkg",
                "file": "a.py",
                "line": 6,
                "shape": "plain_call",
                "true_class": "must",
                "text": "target()",
                "observed_caller": "crate::a::caller",
            }
            row = profile_site(site, root, [])
            self.assertEqual(row["same_file_top_level_def_count"], 2)


if __name__ == "__main__":
    unittest.main()
