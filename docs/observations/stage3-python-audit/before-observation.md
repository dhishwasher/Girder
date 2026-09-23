# Stage 3 Python before-observation

Binary: `/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder`,
sha256 `dd5ae79a222d2a98b42e6b9221a263ceed5dbd85d4b83f10e2deedf14c9ac17f`
(same binary Stage 3 Rust's `correction-2` measured — no Python-affecting
source has changed since `777c2a7`). No resolver work has been done for
Python; this is the baseline Rust's own before-observation played the same
role for.

## Result up front

**0 overclaim, 2 unsafe_exclusion, 0 May observed, all 13 true-Must sites
score conservative.** Unlike Rust's own before-observation (which happened
to already show 0 unsound cells), Python's shows a real, root-caused gap:
operator/comparison expressions get no per-site call evidence at all in
two cases. Full numbers, root causes, and the next concrete steps are in
[audit-scoring-summary.json](audit-scoring-summary.json); this document is
the narrative companion.

## 1. Real-repository audit: 85 scored, 70 exact, 13 conservative, 2 unsafe_exclusion

[audit-scored-results.json](audit-scored-results.json), scored against
[audit-sites-labeled.json](audit-sites-labeled.json) (105 hand-labeled
sites, verification trail in
[labeling-verification.md](labeling-verification.md)).

- **70 exact**, all `true_class: unknown` correctly observed `unknown`.
  8 of these are `operator_dunder` sites that scored exact only because
  their file also happens to carry an unrelated whole-module
  `duplicate-semantic-path` claim (a real, disclosed ambiguity in that
  specific file) — not because operator dispatch is itself evidenced.
- **13 conservative**, all `true_class: must` observed `unknown`
  (`python-binding-or-dispatch-unproven`). Girder's current Python
  extractor proves Must only for same-file, non-dispatch bindings — every
  one of these 13 real sites needed either cross-file import resolution
  or a class-hierarchy override check, neither of which the extractor
  attempts yet.
- **2 unsafe_exclusion** — the one real gap this audit found, root-caused
  by reading the actual `call_evidence_v1` output, not guessed:
  `tests/test_construction.py:37` (`assert m.a == 3`) and
  `tests/test_arguments.py:284` (`assert value == processed_value`) each
  sit inside a function whose evidence has claims for its real calls but
  NONE at all for the comparison expression between them. Confirmed by
  reading the enclosing function's full `call_evidence_v1`: the claim
  spans for `Model.model_construct(...)` and `m.model_dump()` exist and
  don't cover the `m.a == 3` byte offset in between — no claim does. This
  is a genuine coverage hole, not a scorer artifact (verified: the
  always-present whole-module `implicit-runtime-dispatch-not-certified`
  gap exists in both files too, but is correctly excluded from counting
  as coverage, matching Rust's own treatment of its analogous whole-file
  gap — without that exclusion these would falsely show "exact" while
  disclosing nothing about the specific site).
- **0 overclaim**: no false Must, no false May, anywhere in the 85 scored
  sites.
- **0 May observed**: the sample's 13 Must sites and remaining Unknown
  sites never produced a bounded-but-ambiguous override set under the
  frozen rubric — every self/cls/super dispatch site in this specific
  105-site sample either had zero overrides anywhere in the snapshot
  (Must) or resolved to an external/builtin target (Unknown). Disclosed
  as a property of this sample, not assumed to generalize; the existing
  Python dispatch corpus's own `override-via-subclass`/`super-mro-diamond`
  cases (see below) DO exercise true May-shaped scenarios.

## 2. Existing Python dispatch corpus: consistent finding, corpus predates this session

A 12-case Python dispatch corpus already exists from Stage 2
(`fixtures/dispatch-corpus/python/`, part of `docs/dispatch-corpus.json`,
pooled with Rust/TypeScript/Go). Re-run against the same binary:
[corpus-baseline.json](corpus-baseline.json).

- **Python cells: 6 exact, 7 conservative, 0 unsafe_exclusion, 0
  overclaim.** `must_true_positives: 1` (only `python-direct-same-file`,
  a same-file non-dispatch binding), `must_false_positives: 0`.
- Every override/subclass/MRO/cross-file case scores conservative
  (Unknown), consistent with and independently confirming the real-
  repository audit's own finding: no cross-file or class-hierarchy
  proof exists yet.
- The corpus shows 0 unsafe_exclusion where the real-repository audit
  shows 2 — expected, since the corpus's synthetic fixtures don't happen
  to construct the specific "bare comparison expression with nothing else
  claiming its span" shape the real audit found. This is exactly why the
  real-repository audit exists alongside the corpus: a hand-built fixture
  set can miss a shape real code exercises.

## 3. What Stage 3 Python's criterion legs look like right now

Per the frozen per-language criterion ("measured dispatch-corpus
improvement, nonempty Must precision 1.000, and zero classification
errors on the frozen real-repository audit"):

- `measured_dispatch_corpus_improvement`: not applicable yet — no
  resolver change has been made. This measurement is the "before" both
  future corpus and audit improvements will be compared against.
- `nonempty_must_precision_1000_on_real_repository`: not met — 0/13 Must
  sites proven, so there is no nonempty Must set yet to measure precision
  on.
- `zero_classification_errors_on_audit`: **not met** — 2/85 unsound cells
  (2 unsafe_exclusion, 0 overclaim). Stated plainly, not softened: Rust's
  own before-observation happened to already show 0 unsound cells;
  Python's does not.

None of this is a failure of the audit methodology — it is the accurate,
root-caused starting point for Python resolver work, exactly analogous to
Rust's own Stage 3 before-observation (which found its own dominant cause,
the identifier-only Must-proof filter, before any Rust resolver change was
made).

## 4. Next concrete steps (not started)

1. Emit at least an Unknown `CallClaim` for comparison/binary-expression
   call sites, closing the 2 unsafe_exclusion cells with an honest
   disclosure rather than silence — the direct Python analogue of Rust's
   own `implicit-drop-or-operator-dispatch-not-certified` per-site
   (rather than only whole-module) treatment.
2. Attempt cross-file import resolution and class-hierarchy override
   checking for self/cls/plain-call Must proofs, per
   `labeling-rubric.md`'s own case 1/2/4 rules — the highest-leverage
   next step, since it's what all 13 real conservative sites and 7 of the
   corpus's 7 conservative cells need.
3. Re-verify against both baselines measured here (this audit and the
   existing Python dispatch corpus) after any resolver change, the same
   discipline every Rust resolver round in this program used.
