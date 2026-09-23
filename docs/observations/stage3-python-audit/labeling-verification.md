# Verification of the 105-site Python ground-truth labels

Before running the scorer or trusting the labels, an `advisor` review of the
initial labeling pass (done by a forked agent applying
`labeling-rubric.md`) found four things to check. All four are resolved
below; none required relabeling any site's `true_class`.

## 1. Scorer bug: `site_byte_offset` searched the raw line, not the masked one

The selector chose each site by matching the MASKED line (see
`methodology.md` item 4), but the scorer's `site_byte_offset` searched the
RAW line. A string or comment earlier on the same line that happened to
also match the shape pattern (e.g. `"call_looking(1, 2)" if x else
real(1)`) could make the scorer find the wrong match and land the byte
offset inside a string.

**Fixed** in `dispatch_audit_scorer_python.py`: `site_byte_offset` now
masks the file the same way the selector did, finds the match's column on
the MASKED line, then reads the byte offset from that same column on the
RAW line (masking preserves column positions). Also switched from
`text.split("\n")` to `splitlines()`, matching the selector exactly
(the two diverge on form feeds and other separators `splitlines()`
recognizes). A new adversarial unit test
(`test_ignores_a_call_shaped_string_earlier_on_the_line`) reproduces the
exact failure mode and confirms the fix.

**Checked whether this bug actually touched any of the 105 real sites**:
computed the raw-line match column and the masked-line match column for
all 85 non-`not_a_call_site` sites and compared them. **Zero diverged.**
The bug was real and general (proven by the adversarial test) but did not
happen to affect this specific sample. Also added `line_text_matches`,
called before scoring every site: if the package's current line text
doesn't match what the selector recorded, the site is marked
`site_relocation_failed` rather than silently scored against a drifted
file.

## 2. Girder-output contamination in one rationale

Before the labeling fork ran, this session smoke-tested the scorer
end-to-end using three real sites from the frozen 105-sample (not
synthetic examples): `click-8.4.1 tests/test_termui.py:1229`,
`src/click/core.py:1764`, and `tests/test_testing.py:408`. The fork,
inheriting that conversation context, wrote one rationale
(`tests/test_testing.py:408`, `@click.command()`) that explicitly cited
"how Girder's own decorator-node claim treats the whole line as one unit"
-- a violation of the audit's own "ground truth is read from the source
before Girder is run on any site" discipline, even though the smoke test
happened before the fork's own reasoning, not the other way around.

**Checked the other two smoke-tested sites' rationales**: both are
override-count arguments with no reference to Girder's output -- clean.
**Fixed** the one contaminated rationale to state the same conclusion
(Unknown) on a rubric-only basis: the decorator-application call's callee
is whatever `click.command()` returns, untraced under rubric case 5. The
**label itself does not change** (it was already `unknown`, independently
verifiable and correct); only the stated reasoning does.
`0d53911`'s commit message and `methodology.md`'s "three real click
sites" description of the smoke test are accurate as written (they do
name it as sites from the real corpus) -- the gap was only in one
rationale text leaking that context into the label's own written
justification.

## 3. Rebinding check on the 13 Must sites

The initial rationales showed override-checking (grepping for a second
definition of the same method name across the whole extracted tree) but
not an explicit rebinding check (`setattr`/`monkeypatch.setattr`/
`patch.object`/`patch(` touching the same name anywhere in the snapshot,
per the rubric's rebinding definition).

**Checked directly**: grepped all three extracted packages for
`monkeypatch`/`.patch(`/`patch.object`/`setattr(` on the same line as each
of the 13 Must sites' relevant name (`fail`, `add_command`,
`model_rebuild`, `DecoratorInfos`/`build`, `deprecated_from_orm`,
`validate_call`, `SafeGetItemProxy`, `model_dump_json`, `is_true`,
`_apply_single_annotation`, `model_validate`, `GenerateSchema`,
`ValueItems`, `ModelMetaclass`, `BaseModel`, `override_environ`,
`__new__`, `__call__`). One hit:
`tests/conftest.py:159`: `monkeypatch.setattr(GenerateSchema,
'generate_schema', generate_schema_call_counter)` -- monkeypatches a
DIFFERENT method (`generate_schema`) on the same class
(`GenerateSchema`) that site #12's Must claim (`_apply_single_annotation`)
is on. Per the rubric's rebinding rule (scoped to the specific name, not
"any monkeypatching anywhere touching this class"), this does not
implicate `_apply_single_annotation`. No other hits. **All 13 Must labels
hold** under the rebinding check.

## 4. Operator-dunder sites: confirmed external, not an in-snapshot override

10 of the 105 sites are `operator_dunder`-shaped and not
`not_a_call_site`; all 10 were labeled `unknown` under rubric case 7
(target outside the snapshot). The risk flagged: if either operand were
an instance of an in-snapshot class with its own `__eq__`/`__add__`
(e.g. pydantic's own `BaseModel.__eq__`), cases 1/4 (Must if no override,
else May) would apply instead of case 7.

**Checked each of the 10 directly** (listed with file:line in this
document's companion `audit-sites-labeled.json`): every one compares a
primitive field VALUE (`m.a == 3`, `m.inner.y == 2`, string/int/bool
comparisons) or a builtin-typed local, never a bare model/class instance
on either side of the operator. None of the 10 actually exercises an
in-snapshot class's own dunder method. Case 7 correctly applies to all
10.

## What this means for scoring

Per the frozen rubric's case 7 and this session's own reasoning about how
Girder's Python extractor works (module-wide
`implicit-runtime-dispatch-not-certified`, no per-site operator claim
exists at all, unlike Rust's per-node evidence): every `unknown`-labeled
`operator_dunder` site is expected to score `unsafe_exclusion`, not
`exact` -- there is no claim covering these byte offsets once the
whole-module gap is correctly excluded from "covering" (it's in
`NEVER_COVERS`, for the same reason Rust's own whole-file gap is). This
is disclosed here in advance of running the scorer, not discovered
afterward and explained away: it is real, useful signal that Python's
resolver has no per-site operator-dispatch evidence yet, the direct
analogue of Rust's own disclosed "operator/drop dispatch not certified"
gap.

## Result

All four checks pass or are resolved without changing any `true_class`.
One rationale text was corrected (not its label). The labeled file is
ready to score.
