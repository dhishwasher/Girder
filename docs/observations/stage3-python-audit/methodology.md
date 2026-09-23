# Stage 3 Python audit methodology, frozen before any site is read

Mirrors Rust's own frozen-before-reading-any-site discipline
(`docs/roadmap.md`'s "Rust audit methodology, frozen before any site is
labeled" section). Every decision below was made and committed before any
of the 105 selected sites in `audit-sites.json` was opened.

## 1. Corpus

The three already-cached, sha256-verified Python packages from
`docs/core-representative-corpus.json`: `click-8.4.1`, `pydantic-2.13.4`,
`requests-2.34.2`, via `.benchmark-cache/core-representative-v1/`. All
three tarballs were present in the local cache and verified against their
pinned sha256 hashes before use -- no network fetch was needed or made.

## 2. Prerequisite check: does Girder even expose per-call-site Python evidence?

Confirmed on a small, throwaway fixture (not click/pydantic/requests)
before any real corpus work: `girder analyze`/`inspect --json` exposes the
same `call_evidence_v1` structure for Python as for Rust -- byte-precise
`site` spans, `class`, `targets`, `reason`, per call. **Correction to this
document's own first draft**: it originally said the existing
`dispatch_audit_scorer.py` "needs no Python-specific rewrite to work."
That was checked afterward and found false: `site_byte_offset` imports
`SHAPE_PATTERNS` from the RUST site selector specifically to relocate a
site's match span, and Python's shape names aren't keys in that dict at
all (`KeyError` on the first Python-shaped site). A parallel
`tools/dispatch_audit_scorer_python.py` was written instead (same
structure, importing Python's own `SHAPE_PATTERNS` and a
`package`-keyed site schema instead of `crate`-keyed), smoke-tested
end-to-end against three real click sites before being trusted, with its
own unit tests including a Python-specific `NEVER_COVERS` entry
(`implicit-runtime-dispatch-not-certified`, Python's whole-module
coverage-gap reason, the analogue of Rust's own
`implicit-drop-or-operator-dispatch-not-certified`).

## 3. Site enumeration is independent of Girder's own parser

Same discipline as Rust: a call site Girder's tree-sitter grammar fails to
see would never enter a Girder-derived sample, so sites are found with a
plain, Girder-independent regex over the packages' `.py` source
(`tools/dispatch_audit_site_selector_python.py`), not `girder query`.

## 4. Masking: string/comment content must be stripped before classification

A whole-pool check (not just the eventual 105-site sample) run BEFORE
freezing the file set found that **33% of the 49,953 initially-matched
"call-shaped" lines fell inside a string or comment token** (verified with
Python's own `tokenize` stdlib module, independent of Girder). Click and
pydantic's docstrings are full of executable-looking code examples,
unlike Rust's corpus, where this wasn't a comparable problem. Unlike
Rust's audit (which only needed to disclose its own false-positive rate,
`53/105 not real call sites`, without an equivalent masking step), this
required an active fix, not just disclosure: the selector now replaces
every STRING/COMMENT token span with spaces (preserving line/column
structure, so line numbers and non-masked content stay correctly
positioned) before running the line-classification regexes.
`tokenize_failure_count: 0` across all three packages (recorded in
`audit-sites.json`'s `masking` field) -- the fallback-to-raw-regex path
for a file `tokenize` can't parse exists in the code but was never
exercised on this corpus. After masking, the discovered pool drops from
49,953 to 45,321 matched lines (roughly 9% of matches were solely inside
masked spans; the reduction is smaller than the 33% figure because many
lines mix real code with a trailing comment, and only the comment portion
is removed).

## 5. File set: no directory exclusions

Checked directly, not assumed: `click-8.4.1/docs/` contains exactly one
`.py` file (`conf.py`, a Sphinx config, not application code);
`pydantic-2.13.4` and `requests-2.34.2` have no `docs/` or `examples/`
`.py` files at all. There is no meaningful "documentation code examples"
directory to decide about, unlike what a cursory guess might expect. All
three packages' `tests/` directories ARE included in the file set --
matching Rust's own precedent exactly, where the frozen audit site itself
(`tests/floyd_warshall.rs:11`) was a test file, and where Stage 2's
dispatch corpus deliberately includes integration-test call sites as real
code, not synthetic fixtures.

## 6. Stratification and its known imbalance, disclosed rather than corrected

Sites are stratified by **shape only** (six Python-appropriate shapes:
`plain_call`, `method_call`, `qualified_attribute_call`, `decorator`,
`dynamic_dispatch`, `operator_dunder`), the same algorithm Rust's own
selector used (deterministic per-shape shuffle under a fixed seed, capped
per shape, topped up round-robin from leftover shapes if any shape is
scarce). This is NOT also stratified by package. Because pydantic's
source is roughly 4.7x click's and 10x requests' by matched-line count,
shape-only stratification lets pydantic dominate the selected 105:
`package_counts_selected` in `audit-sites.json` shows pydantic 81, click
19, requests 5. This is disclosed here rather than corrected by adding a
second stratification dimension, to keep the frozen algorithm identical to
Rust's own precedent rather than introduce new, untested sampling logic
at freeze time. A future audit round could add package-balanced
stratification as its own, separately-justified change if this imbalance
turns out to matter for the measured result.

## 7. Ground-truth labeling rubric

The full case-by-case rubric (self/cls dispatch, rebinding, annotation-only
receivers, direct construction with `__new__`/metaclass interactions,
decorated names, `super()`, targets outside the snapshot, whether decorator
lines count as call sites, `not_a_call_site` criteria) is in
[labeling-rubric.md](labeling-rubric.md), written and committed in the same
piece as this document, before any site was read.

## 8. "Zero classification errors" definition, unchanged from Rust

Per the frozen Stage 3 policy (`docs/roadmap.md`): an error is an
**unsound** audit cell -- `overclaim` (a false Must, or a false May with
no viable candidate set) or `unsafe_exclusion` (a reachable call site
excluded from the graph's reasoning entirely). An honestly-labeled
`unknown` result, even where the true answer is `must`/`may`, is a
conservative miss, not an error. This applies identically to Python; it is
not renegotiated here.

## 9. What is NOT yet done (recorded so a future session doesn't assume otherwise)

- The 105 selected sites have not been read or labeled. `audit-sites.json`
  has no `true_class`/rationale fields yet.
- The Python-specific scorer (`tools/dispatch_audit_scorer_python.py`,
  see the correction in item 2 above) exists and is smoke-tested against
  three real click sites, but has not been run against a full,
  hand-labeled 105-site file yet.
- No before-observation measurement has been run.
- The `#[ignore]`d-equivalent real-package verification step Rust's design
  used (reproducing design-spec facts against the real checkout) has no
  Python analogue yet, since there is no Python resolver design to verify
  against -- Python Stage 3 is still at the "before-observation" phase,
  before any resolver work begins, matching where Rust's Stage 3 started.
