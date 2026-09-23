# Stage 3 Python after-observation: per-site operator-dispatch claim

Source commit: `d4d8d8f`. Binary measured:
`/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder`,
sha256 `9549a1bbdbbec9057b3c0c61ec55b2d7bd6ad0833a6867b4ec03b2adb1894bf3`.

## Result up front: closes both unsafe_exclusion cells, and gives all 10 operator_dunder sites real per-site evidence -- not just the 2 predicted

The before-observation
([../before-observation.md](../before-observation.md), corrected by
[../before-observation-addendum.md](../before-observation-addendum.md))
found 2/85 unsound cells (both `unsafe_exclusion`):
`tests/test_construction.py:37` and `tests/test_arguments.py:284`, both
bare comparison expressions that got no `CallClaim` at all. It also noted
that 8 of the 10 `operator_dunder`-shaped sites in the sample scored
`exact` only "coincidentally," via an unrelated whole-module
`duplicate-semantic-path` claim that happened to also be present in those
files -- not because operator dispatch was itself evidenced.

**This fix closes more than the 2 predicted sites.** Diffing
`observed_reason` (not just `cell`) for all 85 scored sites before and
after: **all 10** `operator_dunder` sites now report `observed_reason:
implicit-operator-dispatch-not-certified`, including the 8 that were
previously covered only by coincidence. The "coincidental coverage"
framing in the before-observation no longer describes the current state.

## 1. Real-repository audit: 85 scored, 72 exact, 13 conservative, 0 unsound

[audit-scored-results.json](audit-scored-results.json) (was 70/13/2/0).

Diffed programmatically against the before-observation's results by
index, comparing both `cell` and `observed_reason`:
- **2 sites' `cell` changed**: both `unsafe_exclusion -> exact`
  (`tests/test_construction.py:37`, `tests/test_arguments.py:284`) --
  exactly the two predicted.
- **10 sites' `observed_reason` changed** (all `operator_dunder`): the 2
  above (`None -> implicit-operator-dispatch-not-certified`) plus 8 more
  whose `cell` was already `exact` but whose reason changed from
  `duplicate-semantic-path` to `implicit-operator-dispatch-not-certified`
  -- confirmed by direct diff, not assumed. No other site's cell or
  reason changed.
- **Per-package breakdown** (full before/after table in
  [../before-observation-addendum.md](../before-observation-addendum.md)):
  click-8.4.1 12->13 exact (1 unsafe_exclusion closed), pydantic-2.13.4
  55->56 exact (1 unsafe_exclusion closed), requests-2.34.2 unaffected
  (its own unsafe_exclusion-free before this fix).
- **9 additional PEP 604 union-annotation sites** (`X | None`,
  `not_a_call_site`-labeled, regex false positives on `|`) are
  unaffected: they were never scored, and this fix doesn't change that --
  noted since the new operator claim's node-kind match
  (`comparison_operator`/`binary_operator`) is a different AST shape than
  a type-annotation subscript, confirmed by the cell counts holding
  (0 `not_a_call_site` count change, still 20).

## 2. Existing Python dispatch corpus and Stage 1 oracle

[corpus-after.json](corpus-after.json): pooled 21 exact / 35 conservative,
`must_true_positives: 4`, `must_false_positives: 0` -- identical to Stage
3 Rust's already-committed `correction-2` pooled numbers (confirms no
cross-language regression; this change touches shared `claims.rs` code
but is gated `lang == Lang::Python`). Python-specific cells unchanged: 6
exact / 7 conservative, same per-case breakdown as
`../corpus-baseline.json`. The existing corpus's own fixtures don't
happen to construct the "bare operator expression with nothing else
covering it" shape this fix targets, so its cells don't move -- expected,
not a concern (Rust's own first resolver change, `47c3a06`, moved its
corpus's `rust-direct-same-file` cell specifically and left every other
corpus cell untouched too; a resolver change moving exactly the cells it
targets and no others is the normal, correct outcome, not a red flag).

[oracle-after.json](oracle-after.json): Rust and Python both `precision:
1.0`, `recall: 1.0` -- unchanged, stderr empty. **Boundary-count impact
checked directly** (this change adds boundary records to every Python
function containing an operator, a user-visible `orient`/
`test-impact --classified` change): Python's `classified.boundary_count`
went from 39 to 41 (+2, both in the `coverage_gap` category: 6->8;
`missing_or_invalid_evidence` and `unresolved_call_site` unchanged at 10
and 23). Traced to source: the oracle's own fixture
(`fixtures/core-trustworthiness/python/baseline/service.py.txt`)
contains one `is None` comparison (`optional()`'s `if identity is
None:`), counted once in each of the oracle's two measurement passes
(baseline and mutation) since that file isn't touched by the mutation.
`classified.must` stayed `[]` (empty) before and after -- the new
boundary records didn't change which tests get selected, since Python's
classification already floods broadly on the existing Unknown sources in
this fixture.

**New claims across the three real packages**, counted directly from
regenerated inspect JSON (also committed alongside this document):
click-8.4.1 2,667, pydantic-2.13.4 9,945, requests-2.34.2 1,163 -- 13,775
total. This is a real, user-visible increase in disclosed coverage gaps
across real Python codebases, consistent with operator expressions being
common; it does not change any test-selection *correctness* property
measured here (oracle precision/recall unchanged), but it is a genuine
volume change to what `orient`/`test-impact --classified` will report as
boundaries going forward.

## 3. Common gates

All four, run once each chained in a single background job, plus the
final build: [gates.log](gates.log) — `cargo test --workspace -j1
--quiet`, `cargo clippy --workspace --all-targets -j1 -- -D warnings`,
`cargo fmt --all --check`, `node --test npm/test/*.test.js`, all exit 0.

**Rust real-repository audit re-checked** (shared-code risk, checked
empirically rather than trusted from the language gate alone): identical
28 exact / 24 conservative, 0 unsound -- matches Stage 3 Rust's
`correction-2` result exactly.

## 4. Unit tests, verified non-vacuous by mutation

5 new tests in `crates/aether-builder/src/mapper/claims.rs`:
- `a_bare_comparison_expression_gets_an_unknown_claim_not_no_claim_at_all`
  -- reproduces the exact real-world shape; mutated (disabled the new
  branch), confirmed it fails with the exact "gap between two Must
  claims" evidence shape the real audit found, before restoring.
- `unary_and_not_operators_are_flagged_as_operator_dispatch` -- also
  mutation-verified.
- `boolean_and_or_are_not_flagged_as_operator_dispatch` -- asserts
  absence (passes under both the fix and the mutation, as expected for a
  negative test); documents the deliberate `boolean_operator` exclusion.

## 5. Disclosed, not fixed: operator coverage remains partial

This fix covers exactly four node kinds
(`binary_operator`/`comparison_operator`/`unary_operator`/
`not_operator`). Several other Python constructs invoke a dunder method
implicitly and still get no per-site claim -- only the whole-module,
excluded-from-coverage disclosure:
- `augmented_assignment` (`x += y`, invokes `__iadd__`/`__add__`).
- Subscript expressions (`x[i]`, invokes `__getitem__`/`__setitem__`).
- `for`/`with` statements (invoke `__iter__`/`__enter__`/`__exit__`).
- Attribute access itself (`x.attr`, can invoke `__getattr__`/
  `__getattribute__`/a descriptor's `__get__`).

"Zero classification errors" (Section 6 below) is Met **for this
105-site sample**, not as a claim that every implicit-dispatch construct
in Python now has per-site evidence -- a larger or differently-sampled
audit could still find an `unsafe_exclusion` through one of these
uncovered kinds. Recorded here so a future session doesn't assume this
gap is fully closed.

## Updated Stage 3 Python criterion status

- `measured_dispatch_corpus_improvement`: **not met by this change** --
  the existing Python dispatch corpus's cells are unchanged (its fixtures
  don't construct the shape this fix targets). Not softened to "not
  applicable": a resolver change was made, and it measurably did not move
  the corpus.
- `nonempty_must_precision_1000_on_real_repository`: still not met -- this
  fix closes an Unknown-coverage gap, not a Must-proof gap; 0/13 Must
  sites are still proven. Unaffected by design.
- `zero_classification_errors_on_audit`: **Met, for this 105-site
  sample** -- 0/85 unsound cells (0 overclaim, 0 unsafe_exclusion). See
  Section 5 for what "for this sample" means and does not mean.

Two of three criterion legs remain unmet. Stage 3 Python is **not DONE**.
The next step (cross-file import resolution and class-hierarchy override
checking for Must proofs, per `labeling-rubric.md`'s cases 1/2/4) remains
the highest-leverage move toward the two remaining legs -- and, per the
same `advisor` review this document responds to, should start with a
gate-profile pass (mirroring `dispatch_audit_gate_profile.py`'s role for
Rust) against the 13 conservative audit sites and 7 conservative corpus
cells before designing any Must-proof rule, since `transformed_scope`
(set by any decorator anywhere in a Python file) may already block most
of them regardless of what a new rule proves.
