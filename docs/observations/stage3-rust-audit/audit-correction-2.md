# Second correction to the Rust audit before-observation

Corrects `audit-scoring-summary-v2.json`'s `stage3_criterion_status`. Found
on review before any resolver work began.

## The "nonempty Must precision 1.000: Met" claim was wrong

`audit-scoring-summary-v2.json` said this criterion was "Met, inherited
from the dispatch corpus (3/3)." Those three exact-Must corpus cells are
Python, TypeScript, and Go (`rust-direct-same-file` itself scored
conservative, not exact -- documented in the original `f3e9f56` observation
under `new_finding_not_in_stage1`, and never contradicted; this was simply
never cross-checked against the summary's own criterion-status claim).
**Rust's own Must set is empty on both the corpus and this audit.**
Precision is undefined for Rust, not 1.000. The goal's own wording is
explicit that trustworthiness must be shown "on a real repository, not on
fixtures scoring 1.000" -- borrowing another language's corpus number does
not satisfy a Rust-specific criterion either way.

Corrected `stage3_criterion_status` for Rust, current as of this
correction:
- `nonempty_must_precision_1000`: **Not met.** Zero Must classifications
  exist anywhere in Rust's corpus slice or in this audit.
- `zero_classification_errors_on_audit`: Met (0 unsound / 52 scored),
  reconfirmed after fixing the scorer's gap-matching allowlist (see below).
- `measured_dispatch_corpus_improvement`: Not met -- no resolver change yet.

## Scorer fix: `implicit-drop-or-operator-dispatch-not-certified` must never count as covering

Found before any further analysis: `find_covering_claim` accepted *any*
claim whose byte span contained a site, including
`implicit-drop-or-operator-dispatch-not-certified` -- a gap attached with
`span_of(root)` (the *entire file*) on nearly every real Rust file (any
`let`/binary/unary/try/index/for expression or `impl_item` triggers it).
Left unfixed, every offset in every real Rust file is always "covered" by
this gap alone, which would make `unsafe_exclusion` permanently
unreachable regardless of what Girder actually does -- the scorer's output
would never be able to catch the exact error class it exists to catch.

Fixed with an explicit `NEVER_COVERS` set in
`tools/dispatch_audit_scorer.py` (this one reason, nothing else), with a
regression unit test. Re-ran the full 105-site audit after the fix: the
result is **unchanged** (52 scored, 27 exact, 25 conservative, 0 unsound) --
none of the sites were actually relying on this gap as their only covering
claim; sites 24, 59, and 89 still match legitimate macro/duplicate-path
disclosures, confirmed individually. The corrected `audit-scoring-summary-
v2.json`/`audit-scored-results-v2.json` numbers hold, now verified against
the tightened matcher rather than merely asserted before it existed.

## Multi-gate tally (requested, done)

For the 23 method/path-call conservative sites (excluded by the
identifier-only filter), checked whether `transformed_scope` would
*independently* also block them, by scanning each site's whole file for any
attribute other than `#[test]`/`#[tokio::test]`: **18 of 23 already fail a
second, independent gate** (nearly always `#[cfg]`, which must stay
disqualifying regardless of any future fix, since cfg-gated code changes
what's actually compiled). Only 5 sites' files carry no attribute beyond
ones that could plausibly be treated as inert. This means lifting the
identifier-only filter alone, without also narrowing `transformed_scope`'s
built-in-attribute blast radius, would move at most ~5 of 23 audit sites,
not a majority -- consistent with the corpus finding below.

For the Rust dispatch corpus's `direct-same-file`/`operator-overload-
concrete` cases: every declared test's call to its origin sits inside
`assert_eq!`, which puts the enclosing test function in `macro_owners`,
disqualifying it independently of the identifier filter. **A method/path-
call fix alone moves zero corpus cells** and cannot on its own demonstrate
"measured dispatch-corpus improvement," Stage 3's own criterion.

This changes the plan: the fix that can actually satisfy the corpus
criterion is recognizing direct calls inside standard assertion macro
arguments (`assert!`, `assert_eq!`, `assert_ne!`, `debug_assert*`), not the
method/path-call filter. Implementing that next.
