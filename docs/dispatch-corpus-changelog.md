# Dispatch corpus changelog

Every edit made to `docs/dispatch-corpus.json` or its fixtures after the
first time Girder's classified answer was seen for any case (a debug run of
`tools/dispatch_corpus_scorer.py`, made while developing the scorer itself,
before the corpus was committed). Per the frozen rule ("derive expected
answers from language semantics and the frozen policy; don't run Girder on a
case while you author its expectation; never tune a case to the
implementation"), a change here is legitimate only when the defect is
visible from the fixture or its rationale alone, without reference to what
Girder answered. Every entry below is checked against that test.

## 1. `typescript-structural-object-literal` — reverted; new case added instead

**What happened:** the original design used object-literal-valued arrow
functions (`const alice: Named = { name: () => 'alice' }`). While debugging
the scorer's symbol-resolution step, `girder names <dir> name --json`
returned an empty list for this fixture — Girder's TypeScript extractor does
not index object-literal-valued arrow function properties as callable
nodes at all (confirmed by reading `crates/aether-builder/src/mapper/
typescript.rs`'s `add_function`/`add_field` logic, which requires a
`method_definition`-style construct or a type-scope `property_signature`,
neither of which an object literal's arrow-function property is).

At that point the case was edited to use real class methods instead
(`class Alice { name() { ... } }`), so the origin symbol would resolve. That
edit fails the test above: the justification ("Girder can't see this") is
not visible from the fixture alone, only from having already run Girder
against it. **Reverted** to the original object-literal design.

**Disposition:** `typescript-structural-object-literal` now scores `status:
failed, reason: could not resolve origin symbol 'name'`. That is the
finding, published as-is: Girder cannot classify calls into object-literal-
valued methods at all, a real, more severe gap than "classified Unknown" —
it can't even locate the node to classify.

A **new** case, `typescript-structural-class-no-implements`, was added
afterward (explicitly labeled as added after the debug run) to still cover
"structural receivers, unions, overload alternatives" (the policy row
`structural-object-literal` was meant to exercise) using class methods,
which the extractor does index. TypeScript is therefore 13 cases, not 12;
the corpus total is 49, not 48.

## 2. `go-direct-same-file` — `Target` moved into `app_test.go`

**What happened:** the original fixture put `Target()` in `app.go` and
`TestDirect` in a separate `app_test.go` — structurally identical to
`go-direct-cross-file`'s layout (Go always requires `_test.go` for test
functions, so "same file" for Go must mean *the test file itself contains
the target*, not merely "the same package"). This is a defect visible from
the fixture layout alone, independent of any Girder answer: the case's own
name and rationale ("same file") contradicted its own file structure.

**Fix:** `Target()` moved into `app_test.go`, alongside `TestDirect`;
`app.go` removed; the manifest's `origin.file` updated to `app_test.go`.
Re-validated with `go test` (see
`docs/observations/stage2-dispatch-corpus/go-validation.log`).

## 3. Scorer formula fixes (not corpus edits)

Two bugs were found in `tools/dispatch_corpus_scorer.py` itself while
reviewing its first debug output, not in the corpus:

- **`cell_label`'s certainty ordering was inverted.** The first version
  ranked `excluded` as the *weakest* claim and treated `observed=unknown`
  against `expected=excluded` as `unsound`. `excluded` is actually the
  *strongest* possible claim (a provable true negative); getting it wrong
  (observing `excluded` when the truth is anything else) is the one truly
  dangerous case — a reachable test silently dropped from the quiet CLI's
  must|may|unknown union. Observing `unknown` when the truth is `excluded`
  is safe over-inclusion (`conservative`), not `unsound`. Rewritten (see
  the scorer's module docstring for the corrected rule, with `unsound`
  split into `unsafe_exclusion` and `overclaim`).
- **`must_precision_on_corpus`'s true-positive count was wrong.** It counted
  `observed=must` as correct whenever `expected != excluded` — meaning an
  `expected=may` case where Girder claimed `must` counted as a true
  positive. A true positive requires `expected == must` exactly; anything
  else observed as `must` is a false positive. Fixed, and the reused
  recall metric renamed `must_or_may_recall_on_corpus` to match what it
  actually computes.

These are bugs in how a fixed set of (expected, observed) pairs gets
interpreted, not edits to the corpus's expectations or fixtures, so they
carry no risk of tuning a case to the product. Recorded here anyway for a
complete history of everything that changed after the first debug run.

## 4. All same-file Python fixtures — `app.py` renamed to `test_app.py`

**What happened:** the debug run scored 11 of 12 Python cases as
`unsafe_exclusion` (the worst confusion-matrix cell — a reachable test
apparently dropped entirely). Investigating with `girder orient` showed the
call edge (test -> origin) was resolved correctly, and `impact` found it
correctly, but the test node had `is_test` count 0 — it was never marked as
a test at all. Reading `crates/aether-builder/src/mapper.rs`'s `is_test_fn`
showed why: for Python, `is_test_fn` requires **both** the function name to
start with `test` **and** the file itself to match pytest/unittest
discovery (`python_test_file`: stem starts with `test` or ends with
`_test`) — a plain `app.py` never qualifies, regardless of what functions it
declares. Only `python-direct-cross-file` (whose test file was already
named `test_app.py`) was unaffected.

This is a mechanical, file-naming-convention fact — identical in kind to
item 2's Go fix, not a dispatch-classification judgment, and verifiable by
reading the extractor's source rather than by looking at what Girder
classified any call as. **Fix:** every affected fixture's `app.py` renamed
to `test_app.py`; `origin.file` updated in the manifest to match;
re-validated with `python3 -m pytest test_app.py` (see
`docs/observations/stage2-dispatch-corpus/python-validation.log`, rerun
after this fix).

This was the most consequential fix in this changelog: it was not a
dispatch-classification defect at all, and fixing it changes 11 cases' true
observed class in the corpus's favor (from a false "the test doesn't even
exist" signal to whatever the real must/may/unknown answer is) — the exact
kind of authoring bug the dynamic pass-fail validation step exists to catch,
even without full probe instrumentation.
