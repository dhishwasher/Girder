# Stage 3 Python gate-profile: why each conservative cell is conservative

Per the roadmap's own next-step instruction: profile every conservative
cell against the Python extractor's existing gates *before* designing any
Must/May-proof rule, mirroring `dispatch_audit_gate_profile.py`'s role for
Rust. No build was needed — everything here is read from already-generated
`girder inspect --json` output and the packages' own source text.

**This document was corrected once, by an `advisor` review, before being
trusted.** The first draft's tool had four bugs (callee-name extraction
took the wrong call on lines with nested/chained calls; self/cls detection
only matched column 0; class-construction sites were folded into the same
check as same-file functions, hiding a real, distinct gate; the corpus
count was off by one) and its own design proposal turned out to be
redundant with existing code. All four are fixed below, verified by new
unit tests, and the design section is rewritten around what the
corrected data actually shows.

## Method

[tools/dispatch_audit_gate_profile_python.py](../../../tools/dispatch_audit_gate_profile_python.py),
[audit-gate-profile.json](audit-gate-profile.json) (13 rows). For each
conservative site: shape (only `plain_call` can ever pass claims.rs's
identifier-only filter), `transformed_scope_trips` (any decorator/
decorated-definition anywhere in the file), `duplicate_paths`,
`parse_error`, whether a same-file top-level `def` of the callee exists
(functions only — see Finding 3), whether a same-file top-level `class`
of the callee exists but isn't currently collected, whether the call is
`self.`/`cls.` method dispatch, and the callee name itself (read starting
at the site's own byte offset, not guessed from the whole line — see the
tool's own comments for why two earlier approaches, first-match and
last-match, were each wrong for different real sites in this sample).

## Finding 1: exactly one real site is blocked *solely* by `transformed_scope`

Sole-blocker analysis (not just "does this gate trip", which 12/13 sites
show — most sites have several gates tripping at once, and the identifier
filter fires first for 9 of them regardless of anything else):

- **`pydantic-2.13.4 tests/test_deprecated.py:271`** (`deprecated_from_orm(...)`):
  `shape: plain_call`, `same_file_top_level_def_exists: true`
  (unambiguous, count 1), `duplicate_paths: false`, `parse_error: false`.
  `transformed_scope` is the ONLY gate blocking it — five decorators
  elsewhere in the same file (`@pytest.mark.parametrize`,
  `@pytest.mark.filterwarnings`, `@pytest.mark.skipif`,
  `@computed_field`), none on `deprecated_from_orm` itself or its caller.
- **9 sites** are `method_call`/`qualified_attribute_call` shaped — the
  identifier-only filter blocks these regardless of `transformed_scope`,
  `duplicate_paths`, or anything else. 4 of these are also `self.`/`cls.`
  dispatch, needing class-hierarchy override checking; the rest need
  qualified/attribute import resolution, not attempted for Python at all.
- **2 sites** (`validate_call`, `override_environ`) need BOTH a
  `transformed_scope` fix AND cross-file import resolution — narrowing
  `transformed_scope` alone doesn't move them.
- **1 site** (`MetaclassArgumentsWithDefault`) is blocked by neither
  `transformed_scope` nor cross-file resolution — see Finding 3.

## Finding 2: the existing dispatch corpus already has a fixture for exactly the sole-blocker shape

`fixtures/dispatch-corpus/python/decorator-elsewhere-in-file/`:
`target()` called from `test_direct`, with an unrelated
`@functools.lru_cache` decorator on a different function
(`unrelated_cached`) in the same file — the same shape as Finding 1,
already `conservative` in the corpus's own scoring
(`corpus-baseline.json`). Very likely built in Stage 2 to probe exactly
this gap before Stage 3 Python resolver work existed to close it.

**Corrected corpus count: 7 conservative cells, not 6** (an earlier draft
of this document missed `python-negative-unreachable`'s `test_unrelated`
case: `expected: excluded, observed: unknown` also scores `conservative`
under `cell_label`'s rules, since "excluded" only matches "unsafe_exclusion"
when Girder's answer is the OPPOSITE direction). This 7th case is a
different kind of conservative (over-inclusion of an unreachable test, not
under-proving a real Must site) and isn't affected by anything proposed
below — read directly:
`fixtures/dispatch-corpus/python/negative-unreachable/test_app.py` has no
decorator and no shadowing; `unrelated()` is simply never reached from
`test_unrelated`'s own call graph in a way this design attempts to prove
exclusion for. Out of scope for this round.

## Finding 3: class construction is never proven Must, a distinct gate from cross-file resolution

`claims.rs`'s `top` collection loop filters strictly on
`"function_item" | "function_definition" | "function_declaration"` —
**`class_definition` is never included.** A same-file class-construction
call (`MetaclassArgumentsWithDefault(i=None)`, confirmed same-file: the
class is defined at `tests/mypy/outputs/mypy-plugin-very-strict_ini/metaclass_args.py:23`)
can never be proven Must today, independent of `transformed_scope`,
`duplicate_paths`, or anything else — not because it needs cross-file
resolution (it doesn't; the class IS in the same file), but because
`claims.rs` never looks at `class_definition` nodes in the first place.
An earlier draft of this document's tool folded this into the same
same-file check as functions, which wrongly implied narrowing
`transformed_scope` would unblock it. It would not. Recorded as a
separate, disclosed gap — out of scope for the change proposed here.

## Design direction: narrow `transformed_scope`'s effect on Python's proven-map gating

See [design-and-prediction.md](design-and-prediction.md) for the full
design, the redundancy this profile's own first draft proposed (checked
against `claims.rs` directly and found already handled by an existing
check), the string-rebinding soundness question this raised and how it
was investigated, the reverse-direction guard-check (confirming no new
false Must across all 85 scored audit sites and all 12 corpus cases), and
the exact precommitted prediction: **one real audit site
(`deprecated_from_orm`, conservative → exact) and one corpus cell
(`decorator-elsewhere-in-file/test_direct`, conservative → exact); nothing
else moves.**
