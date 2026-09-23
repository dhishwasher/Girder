# Addendum to the Stage 3 Python before-observation

Written to cover items an `advisor` review found missing from
`before-observation.md` (already committed, not edited in place — this
follows the same practice `stage3-rust-audit`'s `correction-1`/
`correction-2` used for its own already-committed observations).

## Python version used for masking

`tokenize`-based masking (`methodology.md` item 4,
`mask_strings_and_comments`) was run under **Python 3.11.2**. Python 3.12+
changed how f-strings tokenize (each `{...}` interpolation becomes its own
set of tokens rather than one opaque `STRING` token), which could change
which spans get masked on a file containing f-strings. The frozen sample
(`audit-sites.json`, seed `20260923`) reproduces only under 3.11 or
compatible; re-running the selector under 3.12+ without accounting for
this could silently select a different sample. Recorded here rather than
in `methodology.md` itself, which is already committed.

## The three sites this session's own smoke test touched

Before any labeling happened, this session ran the Python scorer
end-to-end against three real sites from the frozen sample, to confirm
the scorer worked at all: `click-8.4.1 tests/test_termui.py:1229`,
`src/click/core.py:1764`, and `tests/test_testing.py:408`. Stated
plainly here (per an `advisor` review's pushback on
`labeling-verification.md`'s framing, which argued the disclosure alone
was sufficient without also being restated at the observation level):
these three came from the real, frozen 105-site sample, not from a
synthetic example. `labeling-verification.md` already covers what this
means for one rationale that leaked the smoke test's own output; this
section is the fact itself, stated once more for the observation record.

**Sensitivity check**: scored the 85-site set with and without these
three. With all 85: 72 exact / 13 conservative (after the operator-claim
fix; see `after-operator-claim-fix/after-observation.md`). Without the
three: 71 exact / 11 conservative (82 sites) — removing them removes
exactly 1 exact (the decorator site) and 2 conservative (the two Must
sites), consistent with their own labels. No unsafe_exclusion or
overclaim cell involves any of the three, so their presence or absence
does not change the criterion-status conclusions in either
`before-observation.md` or the operator-fix after-observation.

## Per-package cell breakdown

Before the operator-claim fix (`before-observation.md`'s own numbers, 85
scored):

| Package | exact | conservative | unsafe_exclusion |
| --- | --- | --- | --- |
| click-8.4.1 | 12 | 2 | 1 |
| pydantic-2.13.4 | 55 | 10 | 1 |
| requests-2.34.2 | 3 | 1 | 0 |

After the operator-claim fix (`after-operator-claim-fix/audit-scored-results.json`):

| Package | exact | conservative | unsafe_exclusion |
| --- | --- | --- | --- |
| click-8.4.1 | 13 | 2 | 0 |
| pydantic-2.13.4 | 56 | 10 | 0 |
| requests-2.34.2 | 3 | 1 | 0 |

pydantic dominates both tables (81/105 selected sites overall, disclosed
in `methodology.md` item 6) but the fix's effect (both unsafe_exclusion
sites closing) is not pydantic-specific: one site was in pydantic, one in
click.

## `not_a_call_site` cause tally (20 sites)

Computed directly from `audit-sites-labeled.json`'s 20 `not_a_call_site`
rationales, not estimated:

- **7**: PEP 604 union-type annotations (`X | None`, `operator_dunder`
  shape regex-matching the `|`) — e.g. `title: str | None`,
  `def get_help_option(...) -> Option | None:`.
- **6**: class-definition base-class lists (`plain_call` shape
  regex-matching `ClassName(Base1, Base2):`) — e.g.
  `class MessageWrapper(BaseModel, Generic[T]):`.
- **4**: dunder-method *definitions*, not calls (`def __call__(...)`
  matching `dynamic_dispatch`'s `__call__`/`__getattr__` pattern, `def
  __iter__(...)`/`def __str__(...)` matching plain `plain_call`).
- **3**: ordinary function/method `def name():` signatures matching
  `plain_call`'s bare-identifier-paren regex.

All 20 are genuine regex false positives on the underlying shape
patterns, not masking failures (the mask correctly removes string/comment
content; these are all real, unmasked code that simply isn't a call).

## Correction to `before-observation.md`'s own "coincidental coverage" framing

`before-observation.md` said 8 of 10 `operator_dunder` sites scored
`exact` "only because their file also happens to carry an unrelated
whole-module `duplicate-semantic-path` claim... not because operator
dispatch itself is evidenced." **This was true when written, and is no
longer true** after the operator-claim fix
(`after-operator-claim-fix/after-observation.md`): all 10 operator sites
now report `observed_reason: implicit-operator-dispatch-not-certified`,
confirmed by diffing `observed_reason` (not just `cell`) before and after
the fix for all 85 scored sites. The "coincidental coverage" framing
describes the state `before-observation.md` measured, not the current
state — read the operator-fix after-observation for what's true now.
