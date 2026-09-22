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

The signal that found this was independent of Girder: `python3 -m pytest`
run with default discovery against each fixture directory collected zero
tests from any `app.py` file (pytest's own discovery rule, not Girder's).
The bypass — passing the file explicitly, `python3 -m pytest app.py` — is
what let the earlier `python-validation.log` show "1 passed" and mask the
problem: the test ran fine under pytest given an explicit path, but neither
pytest's own default discovery nor Girder's `is_test_fn` would ever find it
in real use. That the fix was a rename (matching the convention) rather
than a change to how tests were invoked is itself worth recording: the
corpus's test-running convention was quietly nonstandard from the start.

## 5. Origin qualifiers added to 14 cases (13 kept; 1 later removed with its case)

**What happened:** the debug run (debug2, timestamped after the resolver
switch in item 6) found that `girder names` returns every same-named
candidate when a bare symbol is ambiguous — e.g. two trait/interface
implementors both defining a method called `greet`. This is visible from
the fixture alone (two classes/impls/structs in one file declaring the same
method name) and needed no reference to what Girder classified any call as;
it is a resolution mechanic, not a dispatch judgment. An `origin.qualifier`
field (a receiver type or enclosing scope) was added to each ambiguous
case's manifest entry so the scorer resolves the one specific implementor
the rationale already named:

| Case | Qualifier |
| --- | --- |
| `rust-dyn-trait-2-impls` | `English` |
| `rust-generic-bound-dispatch` | `Dog` |
| `rust-trait-default-vs-override` | `Describable` (the trait's default body) |
| `python-override-via-subclass` | `Shape` (the base class) |
| `python-super-mro-diamond` | `Base` |
| `python-unconstrained-duck-typing` | `Duck` |
| `typescript-interface-implementors-2` | `English` |
| `typescript-union-type-dispatch` | `Circle` |
| `typescript-class-inheritance-override` | `Shape` |
| `typescript-structural-class-no-implements` (added, item 1) | `Alice` |
| `go-interface-2-impls` | `English` |
| `go-embedding-promotion` | `Base` |
| `go-interface-implicit-satisfaction` | `Meters` |
| `go-generic-function-type-param` | `Dog` |

(`typescript-structural-object-literal` was also qualifier-patched at the
same time, then reverted along with the rest of that case per item 1; it
carries no qualifier in the committed manifest.)

## 6. Scorer resolver switched from `girder search` to `girder names`

**What happened:** the first debug run (`debug1`) used `girder search`
(concept/similarity search) to resolve symbols to graph paths, expecting
substring-style behavior. It returned "no matches" for the large majority
of exact test-function-name lookups — a short, unremarkable identifier like
`test_direct` can score below `search`'s relevance floor in a two-function
project. This is a scorer-tooling defect, not a corpus edit: no case's
fixture or expectation was touched. Switched to `girder names <dir>
<identifier> --json` (documented as "Exact name match (not substring)"),
which resolved correctly. The corpus's expectations were authored, and
never revised, independent of which resolution mechanism the scorer used to
read them back.

## 7. Scorer: a non-matching qualifier now fails instead of silently falling back

**What happened:** found on a later review of the scorer's own code, not
from any scoring result. `resolve_symbol`'s qualifier filter only replaced
the candidate list when the filter matched *something*; a qualifier that
matched nothing left the original, unqualified candidate list in place,
which could silently return the wrong node (never raising an error) if
exactly one unqualified candidate happened to exist. No case in the
committed corpus is affected today — all 14 qualifiers currently in the
manifest do match — confirmed by re-running the scorer after the fix and
diffing its full confusion matrix against the committed
`scoring-results.json`: byte-identical. Fixed as a guard for Stage 3, where
new or edited qualifiers are more likely.
