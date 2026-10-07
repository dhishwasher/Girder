# Girder technical roadmap

Status, acceptance criteria, measurement evidence, failures, blockers, and next
actions for call classification, dispatch resolution, client integration, and
verified edits.

## Resume contract

Every session starts by reading this file and ends by updating it. Execute stages
in order, with the language dependency in Stage 3 enforced. Commit and push each
coherent piece separately; do not tag or release (lifted 2026-10-07 by the user's explicit instruction to merge, tag and release 0.4.0). The first program commit is this
roadmap, before implementation. Freeze a stage's policy before measuring it.

Statuses are **NOT STARTED**, **IN PROGRESS**, **DONE**, and
**FAILED-AND-PUBLISHED**. Blockers are recorded separately, never disguised as
passing results. An abandoned stage stays here with its reason and an explicit
abandoned disposition. Never remove a stage to make the program look finished.
DONE requires a previously committed observation demonstrating its criterion.
Failed observations, regressions, corpus cases, and original thresholds remain
in history and in the repository. New attempts get new observation files.

## Common gates and operating constraints

After each stage (and each Stage 3 language checkpoint), run each gate once on
the candidate revision, serially, saving the exit status and a separate log:

```sh
cargo test --workspace -j1 --quiet
cargo clippy --workspace --all-targets -j1 -- -D warnings
cargo fmt --all --check
node --test npm/test/*.test.js
```

Set `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, and
`CARGO_TARGET_DIR=/mnt/chromeos/removable/MOVESPEED/aetherforge-target`.
Each gate is ONE blocking shell command redirecting output to its log and
tailing on completion, preserving its exit status. No polling or mid-build
progress narration. No subagents and nothing in parallel with Cargo: this
approximately 2.7 GB machine previously suffered an OOM during concurrent work.
Use `girder orient`, `girder search`, and bounded source context per CLAUDE.md.
Do not use a stale binary as evidence for a new implementation.

No new runtime dependency, telemetry, runtime network calls, license-text edits,
tags, or releases. The user clarified that documentation verification, pinned
development acquisition, and Git pushes may use the network. Measured workloads
stay offline. A missing dependency, capable agent, or required hardware is a
published blocker, not a fabricated measurement. A gate failure stays published;
a subsequent fix is a new candidate and observation, not an erased run.

Every observation records the candidate commit, policy/corpus hashes, binary
identity, commands, environment, exit statuses, results, and limitations. Never
weaken thresholds or change expectations to improve a reported score.

## Stage 1 — Must / May / Unknown

**Status: FAILED-AND-PUBLISHED** (implementation complete; the precommitted
trustworthy-Must threshold was measured and not met — see below)

The [v1 classification policy](call-classification-policy.md) is specified for
separate preimplementation commitment (`4745a5c`). Shared graph evidence and
classified reachability are implemented (`eb120c1`);
[seven development unit checks passed](observations/stage1/graph-evidence-tests.json).
Extractor call evidence for Rust/Python/TypeScript/Go is attached at every
graph build (`6b11d28`). `impacted_tests`/`test-impact` and `orient` return
labeled Must/May/Unknown answers, including the watched MCP path (`48a9713`,
`d1706d6`); `--quiet` was fixed in the same pass to actually be the
conservative must∪may∪unknown union its description claims, not the
pre-existing unclassified mechanism (`d1706d6` — the mechanism switch is a
real, deliberate behavior change: `--quiet` now frequently returns close to
the full test inventory rather than a scoped-but-sometimes-wrong one, until
Stage 3 closes dispatch holes).

**Measured** (`f53f6dd`, full detail in
[measurement-summary-final.json](observations/stage1/measurement-summary-final.json),
superseding the earlier `c927122` run in
[measurement-summary.json](observations/stage1/measurement-summary.json) —
identical classification numbers, corrected overclaims and completed fields;
see that file's own `supersedes_note`): on both the core-trustworthiness
fixtures (Rust + Python, dynamic-probe-verified) and the one
core-representative-mutations case against Click (a real repository), the
**Must set is empty** (precision undefined/null, not 1.000) and **May is
empty** (0.000 recall wherever a positive existed) — the extractor never
emits `CallClass::May` yet; that enumeration is Stage 3 work. Pooled:
`unknown_count=14` (declared-universe), `unknown_total_count=483` (Click's
whole discovered-Unknown universe alone is 472 — see the observation for
why that gap matters), `boundary_count=7601`. Root causes, corrected on
review (see
[measurement-correction.json](observations/stage1/measurement-correction.json)):
Must proofs are same-file *and* top-level-only — cross-file calls (verified
against the fixture layout: both trustworthiness fixtures' tests live in a
different file from the function they cover, and so does Click's declared
`Group.invoke` caller) can never reach Must, and test *methods* on a class
(not just cross-file) are excluded by the top-level-only rule regardless of
file; `classified_impact`'s flood rule marks every function in the whole
graph Unknown once a single unresolved call exists anywhere, which is
near-universal on real code.

**The trustworthy Must threshold (1.000, nonempty set) is UNMET.** Per the
precommitted criterion this is published as failure, not hidden or softened,
and the program continues to Stage 2 below — closing these holes is
explicitly Stage 3 scope, in language order, not a reason to relax Stage 1's
policy or re-run until the number looks better.

Known, deliberately out-of-scope-for-Stage-1 gaps carried forward: the
advisory hook still consumes the unclassified `tests_for_nodes`/`impact_of`
path, not the classified one; `orient`'s classification section has no
`--unbounded`-equivalent escape hatch (its existing 50-item cap policy is
unchanged, matching its other sections); there is no continuation offset for
a truncated classified list, only a truncated flag and true count; `test-impact`'s
non-quiet human-readable listing (its "skipped" count) and `--run`, plus
`planfile/checks/test_checks.rs::run_tests_impacted`, all still call the
legacy `tests_for_nodes` directly and so can disagree with `--quiet`, which
does not (a real gap — `--run` can still silently skip a test reached only
through the kind of unresolved call `--quiet` would now conservatively
include); `classified_impact(&[])` (a change with no function-node origins,
e.g. a const-only or type-only edit) returns empty with no boundary notice
and no reasoning performed at all, the same as the pre-existing legacy path
for that case, so an agent reading only the "empty means nothing needs
testing" framing has no signal that constants aren't modeled as impact
origins in the first place. CLAUDE.md's guidance was qualified for this in
`cb696db`'s follow-up, but the `impacted_tests` MCP tool description in
`mcp.rs` still says unqualified "An empty result means nothing needs
testing — it does NOT mean run everything"; queued as a small wording fix
(or a stderr notice when a diff touches files but resolves no function-node
origins) for the next commit that touches Rust, not worth a rebuild+gate
cycle on its own.

An error was found and corrected in the first measurement's root-cause
analysis (not its numbers) after review; see
[measurement-correction.json](observations/stage1/measurement-correction.json).

Precommit a separate language-specific classification policy for Rust, Python,
TypeScript/TSX, and Go. Must means a proven resolved call without a viable
alternate target; May means viable alternatives; Unknown means unresolved or
unbounded behavior, including reflection, getattr, FFI, unexpanded macros, and
extractor gaps. Define dispatch assumptions, ambiguous classification, ordering,
caps, continuation, and treatment of empty denominators. Never guess Must from
a unique name match. Preserve Unknown boundaries even without a target node.

Implement separately labeled Must/May/Unknown answers in `impacted_tests` and
`orient`, including watched MCP paths. Keep search confidence separate from
dispatch certainty. Preserve known candidates alongside incomplete candidate
sets, and never present a truncated or uncertain empty result as complete.

**Precommitted criterion:** publish measurements on the existing mutation corpus
with Must precision, May-only recall, combined Must-or-May recall, Unknown counts,
denominators, and every failure. Empty Must precision is undefined, not 1.000.
The trustworthy Must threshold is 1.000 with a nonempty set. If unmet, publish
the failure and continue to Stage 2; do not hide or soften it.

**Gate:** mutation-oracle measurement and classification/API tests, then all
common gates — all run against `f53f6dd` (the final candidate, after the
corrections below), all passing:
`cargo test --workspace -j1 --quiet` (24 suites, 0 failed),
`cargo clippy --workspace --all-targets -j1 -- -D warnings` (clean),
`cargo fmt --all --check` (clean),
`node --test npm/test/*.test.js` (29 passed, 2 pre-existing skips, 0 failed).
**Observation:**
[measurement-summary-final.json](observations/stage1/measurement-summary-final.json)
(current),
[measurement-summary.json](observations/stage1/measurement-summary.json)
(superseded, preserved),
[measurement-correction.json](observations/stage1/measurement-correction.json),
[trustworthiness-measurement.json](observations/stage1/trustworthiness-measurement.json),
[representative-mutations-measurement.json](observations/stage1/representative-mutations-measurement.json).
**Blockers:** none; the criterion was measured and failed honestly.

## Stage 2 — The dispatch corpus

**Status: DONE**

Committed a versioned independent oracle before any *official* scoring run
against a frozen corpus: [docs/dispatch-corpus.json](dispatch-corpus.json),
49 cases (12 each Rust/
Python/Go, 13 TypeScript — see
[docs/dispatch-corpus-changelog.md](dispatch-corpus-changelog.md) item 1 for
why TypeScript has 13), weighted to the hard forms named in the program
goal: Rust trait objects/generics/operator traits/macro-generated calls;
Python inheritance/super-MRO/getattr/unconstrained duck typing/decorators/
`__call__`; TypeScript interface implementors/structural typing/unions/`any`
receivers; Go interfaces/embedding/method values/function variables/
reflect/generics. Each case: an isolated mini-project, a source-level
origin (file + symbol, optional qualifier), declared tests with an expected
class per [docs/call-classification-policy.md](call-classification-policy.md)
or `excluded` (true negative), a policy-table citation, and a rationale
authored from language semantics alone, before Girder was ever run against
any case. Dynamically validated (tests compile and pass) for every case
except one TypeScript case whose decorator syntax this environment's
toolchain cannot execute (recorded blocked, not faked).

While developing the scorer against the still-unfrozen corpus, four debug
scoring passes did run before the corpus was committed, and from the second
pass onward they showed real classified answers, not just resolution
failures — see
[docs/dispatch-corpus-changelog.md](dispatch-corpus-changelog.md), which
records every edit made after that point (including the qualifiers added
to disambiguate same-named methods and the switch from `search` to `names`
for symbol resolution) with a justification checked against "visible from
the fixture/spec alone," and the four raw debug outputs themselves, kept
for the record
([pre-freeze-debug-runs/](observations/stage2-dispatch-corpus/pre-freeze-debug-runs/)).
The most consequential edit: 11 Python fixtures were never marked as tests at all
because the extractor requires the *file* to match pytest/unittest
discovery, not just the function name, found via `girder orient` (not by
adjusting an expectation to match an answer).

**Precommitted criterion:** score Girder against every case and publish all
expected/actual results, including every failure or unavailable execution.
Never tune cases to the product. When the product starts passing a case, retain
it and append a harder case; publish fixed-cohort scores separately.

**Met.** [tools/dispatch_corpus_scorer.py](../tools/dispatch_corpus_scorer.py)
scored all 49 cases (57 test cells) against
[docs/dispatch-corpus.json](dispatch-corpus.json); every result — including
the one unresolvable case — is published in
[scoring-results.json](observations/stage2-dispatch-corpus/scoring-results.json),
summarized in
[scoring-summary.json](observations/stage2-dispatch-corpus/scoring-summary.json).
Headline: **zero unsound cells** (no false Must, no false May, no wrongful
exclusion) across all 57; `must_precision_on_corpus` 1.000 (3/3, Python/
TypeScript/Go's cleanest same-file cases); 36 cells conservative (safe
Unknown flooding, concentrated exactly where Stage 1 predicted);
`must_or_may_recall_on_corpus` 0.086 (3/35) — expected and named as the
number Stage 3 exists to move, not a Stage 2 failure. New finding beyond
Stage 1: Rust's own `assert_eq!`/`assert!` macros hide even a trivial
same-file direct call from Must certification (the call's argument is an
opaque macro token, never its own `call_expression` node) — Python/
TypeScript/Go's non-macro assertion styles don't have this problem, so the
identical direct-call pattern scores exact Must in all three but
conservative in Rust. Full explanation in scoring-summary.json's
`new_finding_not_in_stage1`.

"Zero unsound" is guaranteed, not earned, under the current implementation:
while `classified_impact`'s whole-graph flood is active (any single
Unknown-classified call anywhere makes every function at least Unknown),
`observed=excluded` cannot happen at all, so `unsafe_exclusion` cannot
fire; `overclaim` can only fire through a genuine Must (or, once Stage 3
implements it, May) path actually being found. The corpus has only 4
`excluded`-expected cells (one true negative per language) to even exercise
that path. This number becomes informative, not just structurally
guaranteed, once Stage 3 narrows the flood — recorded here so a future
session doesn't read 0 unsound as evidence the classifier is already
trustworthy on ambiguous dispatch; Stage 1 and this corpus's own recall
numbers say otherwise.

**Gate:** all four common gates, run against `32bd5d2` (the exact committed
candidate — no later commit changed any Rust/JS code, so there is no
candidate-vs-gate-run gap to reconcile this time):
`cargo test --workspace -j1 --quiet` (24 suites, 0 failed),
`cargo clippy --workspace --all-targets -j1 -- -D warnings` (clean),
`cargo fmt --all --check` (clean),
`node --test npm/test/*.test.js` (29 passed, 2 pre-existing skips, 0 failed).
Logs: [gates-32bd5d2/](observations/stage2-dispatch-corpus/gates-32bd5d2/).
**Observation:**
[scoring-summary.json](observations/stage2-dispatch-corpus/scoring-summary.json),
[scoring-results.json](observations/stage2-dispatch-corpus/scoring-results.json).
**Blockers:** none.

## Stage 3 — Close dispatch holes one language at a time

**Status: DONE** (all four languages, each on its frozen criterion; see the 2026-10-06
checkpoints). Rust and Python meet the corrected 100-actual-site
checkpoint, with [committed evidence](observations/stage3-audit-reconciliation/after-observation.md)
at `a97b2ea`. Their original 52/85-site audits did not satisfy the sample-size
requirement; those historical observations and the failed first Rust extension
remain published. Each corrected audit emitted only one Must claim (1/1) and
99 Unknown claims; this is limited evidence, not general dispatch completeness.
**TypeScript is DONE** on its frozen criterion (see the 2026-10-06 checkpoint and
[after-observation](observations/stage3-typescript-audit/esm-import-proof/after-observation.md));
**Go is DONE** on its frozen criterion (see the 2026-10-06 Go checkpoints and the
[Go after-observation](observations/stage3-go-audit/after-observation.md)). Dispatch
completeness is **not** claimed for any language: each result is limited, disclosed
evidence, and every language keeps Unknown holes (methods, interfaces, package-qualified
calls, and more).

Order: **Rust → Python → TypeScript → Go**. No fifth language. Each language has
its own frozen baseline, implementation, after-observation, and gate checkpoint:

| Language | Status | Before / after evidence | Dependency |
| --- | --- | --- | --- |
| Rust | **DONE — corrected 100-site checkpoint** | [before](observations/stage3-rust-audit/audit-scoring-summary-v2.json) / [after](observations/stage3-rust-audit/after-method-call-fix/after-observation.md) + [100-site correction](observations/stage3-audit-reconciliation/after-observation.md) and [retained failed extension](observations/stage3-audit-reconciliation/observation/summary.md) | Stage 2 |
| Python | **DONE — corrected 100-site checkpoint** | [methodology](observations/stage3-python-audit/methodology.md) / [after-transformed-scope-fix](observations/stage3-python-audit/after-transformed-scope-fix/after-observation.md) + [correction-1](observations/stage3-python-audit/after-transformed-scope-fix/correction-1/correction.md) / [correction-2](observations/stage3-python-audit/after-transformed-scope-fix/correction-2/correction.md) / [correction-3](observations/stage3-python-audit/after-transformed-scope-fix/correction-3/correction.md) + [100-site correction](observations/stage3-audit-reconciliation/after-observation.md) | Rust trustworthy on a real repository |
| TypeScript | **DONE — frozen criterion met; CI green at `c78f6c9`** (73/73 ESM contract; corpus 22→23 exact, Must 6/6, 0 unsound; audit 56/44, 0 unsound, Must 3/3; gates on `cb2aa11` and `b0c09b6`) | [100-site baseline](observations/stage3-typescript-audit/extension/before/observation.md) / [structural-identity after-audit](observations/stage3-typescript-audit/lexical-bindings/structural-members-after-local-1/observation.md) + [retained false-Must baseline](observations/stage3-typescript-audit/lexical-bindings/before-observation.md) + [unchanged dispatch results](observations/stage3-typescript-audit/lexical-bindings/structural-members-after-local-1/dispatch-corpus-scoring.json) + [ESM after-observation](observations/stage3-typescript-audit/esm-import-proof/after-observation.md) | Python trustworthy on a real repository |
| Go | **DONE — frozen criterion met; CI green at `4b35c52`** (contract 21/21; corpus 25/31, Go 7/7; audit 27/91/0/0, Must 23/23; 401 Must claims verified, 0 violations; gates on `08fcbdc`) | [baseline](observations/stage3-go-audit/baseline-observation.md) / [after](observations/stage3-go-audit/after-observation.md) + [frozen policy](observations/stage3-go-audit/policy.md) + [methodology](observations/stage3-go-audit/methodology.md) | TypeScript trustworthy on a real repository |

Use existing pinned Rust repositories and Click first; pin TypeScript compiler
and Go standard-library snapshots before their language work. Precommit at
least 100 independently audited real-repository call sites per language before
resolver changes. Cover direct calls, alternate dispatch, and unknown boundaries.

**Precommitted criterion per language:** measured dispatch-corpus improvement,
nonempty Must precision 1.000, and zero classification errors on the frozen
real-repository audit. Publish coverage, sample limits, dispatch assumptions,
and every remaining hole. A hole that cannot be closed is Unknown, never omitted.
Fixture perfection alone is insufficient. Failure blocks the next language.

**"Zero classification errors" is defined, frozen before any Rust audit
site is labeled:** an error is an **unsound** audit cell in the same sense
the dispatch corpus's scorer already uses — `overclaim` (Girder's answer
asserts more certainty than the true class, i.e. a false Must or a false
May with no viable candidate set) or `unsafe_exclusion` (a reachable call
site excluded from the graph's reasoning entirely). Per the frozen
classification policy, "a hole that cannot be closed is Unknown, never
omitted" — so an honestly-labeled `unknown` audit result, even where the
true answer is `must` or `may`, is a **conservative** miss, not an error.
This reading is the only one consistent with the corpus's own scoring rule
committed in `32bd5d2`; the alternative (any mismatch counts as an error)
would make Rust unable to pass while a single unclosable hole exists
anywhere in the audited sample, which contradicts "a hole that cannot be
closed is Unknown, never omitted" being an acceptable outcome.

**Rust audit methodology, frozen before any site is read:**
- **Corpus:** the three already-cached, sha256-verified crates from
  `docs/core-representative-corpus.json` (`petgraph-0.6.5`,
  `serde_json-1.0.150`, `regex-1.12.4`) via
  `.benchmark-cache/core-representative-v1/`. No network fetch.
- **Site enumeration is independent of Girder's own parser** — a call
  site Girder's tree-sitter grammar fails to see would never enter a
  Girder-derived sample. Sites are found with a plain regex over the
  crates' `.rs` source, not `girder query`/`orient`.
- **Stratified, deterministic sample:** a fixed random seed, ≥100 sites
  total across the three crates, stratified across six syntactic shapes —
  plain call (`ident(...)`), method call (`.ident(...)`), path/associated
  call (`Type::ident(...)`), macro invocation (`ident!(...)`), fn-pointer
  or closure call (an opaque callee expression), and operator usage
  (binary/unary on a type implementing the relevant trait).
- **Ground truth is read from the source before Girder is run on any
  site** — same discipline as the dispatch corpus. A site's true class
  (`must`/`may`/`unknown`) is determined by reading the call and its
  candidate resolution, not by observing what Girder answers.
- **Reading Girder's per-site answer:** no public per-call-site command
  exists yet. `girder analyze <dir> --json` followed by
  `girder inspect <graph>.aether --json` exposes each node's raw
  `call_evidence_v1` attribute (RON-encoded: `class`, `reason`,
  `coverage_gap`, `targets` per call site) — confirmed present on this
  binary; no new command needed for the audit itself.

**Rust before-observation complete, corrected.** All 105 audit sites
labeled with ground truth read from source before Girder was run on any of
them, then scored against Girder's actual per-site `call_evidence_v1`
answer. The first scoring pass (`f3e9f56`) had four real errors, found on
review and corrected in `c601a5e` — full explanation in
[audit-correction.md](observations/stage3-rust-audit/audit-correction.md).
`f3e9f56`'s files are preserved unedited per the resume contract; the
corrected, current numbers are in
[audit-scoring-summary-v2.json](observations/stage3-rust-audit/audit-scoring-summary-v2.json)
and
[audit-scored-results-v2.json](observations/stage3-rust-audit/audit-scored-results-v2.json),
using the corrected labels in
[audit-sites-labeled-v2.json](observations/stage3-rust-audit/audit-sites-labeled-v2.json)
and the now-committed, tested
[tools/dispatch_audit_scorer.py](../tools/dispatch_audit_scorer.py)
(byte-precise matching, replacing the original run's uncommitted
row-proximity scratch scripts).

An honest property of the sampling method itself, disclosed rather than
hidden: **53 of 105 (50%) sampled sites were not real call sites at all**
(match-arm patterns, generic bounds, function definitions, doc prose,
`macro_rules!` bodies, one comment block, and raw-string false positives) —
precommitted as a known risk of "crude but independent" regex sampling
before any site was read. Of the 52 sites that could be scored: **zero
unsound cells** (0 overclaim, 0 unsafe_exclusion), now confirmed by
byte-precise matching rather than asserted over an incomplete 49/52 —
converging with the dispatch corpus's own zero-unsound result. Recall is
still 0%: all 25 true must/may sites scored conservative. Root-cause tally,
this time derived from source facts (imports and definitions actually
present per file) rather than inferred from Girder's generic reason string
(the original claim that one reason string "explains 100% of the misses"
was itself wrong — the string is identical for every unproven Rust call
regardless of cause): **23/25 (92%)** trace to
`crates/aether-builder/src/mapper/claims.rs:122`,
`function.filter(|f| f.kind() == "identifier")` — method calls
(`receiver.method()`) and path/associated calls (`Type::method()`)
categorically excluded from Must-proof eligibility regardless of actual
resolvability (e.g. `gr.add_node(...)` on a concrete `Graph<...>`, provably
Must, never even attempted). The remaining **2/25 (8%)** are
identifier-shaped calls whose target is imported rather than defined
top-level in the calling file — the already-documented same-file/
top-level-only limitation, not a new cause.

**First resolver change done, after-observation done, criterion partially
met.** `47c3a06` proves bare-identifier calls found textually inside
trusted, unshadowed `assert!`/`assert_eq!`/`assert_ne!`/`debug_assert*`
macros the same way a same-file top-level direct call is proven (walking the
macro's token-tree leaves directly, since tree-sitter does not parse macro
arguments as expressions). `3f5ed5e` measures it: dispatch corpus improved
(pooled 20/36 → 21/35, Rust 4/10 → 5/9, `rust-direct-same-file` conservative
→ exact, probe-verified against the actual fixture), zero classification
errors held (0 unsound audit cells, oracle 1.0/1.0 precision/recall, both
unchanged) — but **the audit itself is unchanged** (27 exact / 25
conservative, byte-identical). Root-caused, not guessed: the audit sample
has essentially no genuinely eligible same-file/non-doctest sites for this
proof category, and the one real candidate found is blocked by the same
whole-file `transformed_scope` `#[cfg]` gate already shown to independently
block most of the method/path-call misses too. Full details, including the
two disclosed-but-unfixed properties (crate-wide macro shadowing, checked
absent from both corpora; control-flow insensitivity, consistent with the
policy's binding-certainty definition of Must), in
[after-observation.md](observations/stage3-rust-audit/after-assert-macro-fix/after-observation.md).
This first resolver change alone left Stage 3 (Rust) **not DONE** (audit leg
and real-repository nonempty-Must-precision leg unmet) and **not FAILED** (a
specific next resolver step was identified, not a dead end).

**Historical second resolver change and after-observation.** The previous DONE
claim below omitted the audit-size gate and is superseded by the current status;
the measurements and documented failures themselves are unchanged.
`d0ce300`/`c436dbd`/`62141a9`/`2452bc1`/`facc21c`/`777c2a7` prove `x.m()`
Must when the receiver has a single explicit `let x: T<...>` binding and
`m` resolves to a unique, safe, non-cfg-gated public inherent impl with no
earlier- or unsafely-same-step trait competitor (full constraint list in
[gate-profile-correction-2.md](observations/stage3-rust-audit/after-assert-macro-fix/gate-profile-correction-2.md)).
This closed both remaining gaps, but only after **seven correction
rounds**, four of which each found a genuine unsoundness in the
immediately prior round's own fix (three of the four in the same
function, `enclosing_mod_scope`) — not merely a missing test — caught by
an `advisor` review of the "DONE" draft before it was trusted, four times
in a row. The real-repository audit moved from 27/25 to **28/24
exact/conservative, 0 unsound**, exactly the predicted single site
(`tests/floyd_warshall.rs:11`, target verified as `Graph::add_node`,
diffed programmatically against the prior result — not just
re-summarized); dispatch corpus and Stage 1 oracle held unchanged; 49
supplementary Must claims across the real petgraph checkout all resolve to
the three correct targets — all of this reconfirmed byte-identical after
both the sixth and seventh rounds' fixes
([correction-1/correction.md](observations/stage3-rust-audit/after-method-call-fix/correction-1/correction.md),
[correction-2/correction.md](observations/stage3-rust-audit/after-method-call-fix/correction-2/correction.md)).
Full detail, including every mutation-test result proving each guard
actually does something (not vacuous), in
[after-observation.md](observations/stage3-rust-audit/after-method-call-fix/after-observation.md)
and
[supplementary-hand-verification.md](observations/stage3-rust-audit/after-method-call-fix/supplementary-hand-verification.md).
All four Stage 3 Rust criterion legs (`measured_dispatch_corpus_improvement`,
`zero_classification_errors_on_audit`,
`audit_shows_fewer_conservative_more_exact_cells`,
`nonempty_must_precision_1000_on_real_repository`) are **Met**. Disclosed,
carried-forward limitations (glob-import safety is a deviation from the
frozen design spec's constraint 6, not fully traced recursively;
hand-compiled prelude method list; bare-name rather than full
semantic-path type resolution; textual rather than solved trait-bounds
comparison; no macro-token-tree binding rule) are in the after-observation
in full. The original audit-size failure was subsequently repaired by the separately
precommitted 100-site extension; the old sample remains unchanged.

**Gate per language:** before/after corpus and real-repository audit, then all
common gates. **Observation:**
[audit-scoring-summary-v2.json](observations/stage3-rust-audit/audit-scoring-summary-v2.json)
(before) and
[after-observation.md](observations/stage3-rust-audit/after-method-call-fix/after-observation.md)
(historical after-result, not current stage-completion proof).
**Current completion evidence:** [corrected Rust/Python audits](observations/stage3-audit-reconciliation/after-observation.md)
include their checkpoint-candidate corpus results and all four passing gates.
**Blockers for the next stage:** none. Stage 3 is complete for all four languages.
Carried-forward holes, all disclosed in each language's after-observation: TypeScript and Go
Must claims are conditional on stated execution assumptions; TypeScript plan projections and
workspace buffers have no application-level C-3 test; Go package-qualified calls, methods,
interfaces, and generics remain Unknown, and Go safety-net claims anchor on a file's first
function rather than the real caller; the retained `typescript-structural-object-literal`
failure. **Outstanding Stage 2 obligation:** three corpus cases now pass
(`typescript-direct-cross-file`, `go-direct-cross-file`, `go-closure-captures-direct-call`); each
needs a harder successor appended in a new versioned corpus file, never by editing the frozen one.
**Next:** Stage 4 (client-agnostic packaging). Reverify each client's documented formats first.

## Stage 4 — Client-agnostic packaging and orient-first guidance

**Status: DONE** (criterion met on committed evidence and CI green at `2852953`; see the
2026-10-07 Stage 4 checkpoints)

Build **one shared MCP bundle with per-client adapters**, covering **Claude Code,
Cursor, and Codex** at minimum. Start with Claude Code because the advisory hook
already exists; verify its current documented format before packaging it.

Maintain one canonical instruction: in a repository with a graph, call `orient`
before broad reads or grep; disclose low confidence and fall back to source
reading rather than guessing. Express this instruction through each client's
own documented mechanism. Hooks remain advisory and never deny a read.

Verify every adapter's configuration, instruction, and applicable hook formats
against that client's own current published documentation; record source/date.
If a required format cannot be confirmed, record the adapter as blocked and
leave it unbuilt rather than inventing it. Document unsupported hook mechanisms
honestly. Runtime uses installed local Girder; setup acquisition is separate.

**Precommitted criterion:** all three adapters install cleanly, expose Girder
through MCP, and load the shared orient-first instruction through their native
mechanisms, with committed evidence. README covers all three and documents raw
MCP JSON configuration as the universal fallback for any MCP-compatible client,
including client-specific placement. Preserve raw npx setup. A required blocked
adapter keeps this stage incomplete.

**Gate:** isolated per-client install/MCP/instruction smoke checks; advisory-hook
checks for normal, missing-graph, malformed-input, and hook-failure cases where
supported; then all common gates. **Observation:** [stage4-clients/observation.md](observations/stage4-clients/observation.md)
(formats frozen first in [doc-verification.md](observations/stage4-clients/doc-verification.md)).
**Blockers:** none; formats were reverified on 2026-10-06 and one adapter step (the Cursor
hook) is recorded as documented but not built.

## Stage 5 — Verified edits

**Status: DONE** (CI green on `6f7118c`; see the 2026-10-07 Stage 5 DONE checkpoint)

Extend existing graph-addressed edits and journaled projection, not a separate
editor. The agent names an exact node and baseline fingerprint and declares the
promised node/edge delta. Girder projects a minimal patch, reparses/re-resolves,
compares the complete actual delta, and refuses transaction commit on mismatch,
wrong overload, ambiguity, stale input, or insufficient evidence. Preserve the
original source and graph on refusal. Keep old plans compatible without calling
them certified. Do not alter license text.

Print Must/May/Unknown impact and changes in test reachability. Report actual
test execution separately; predicted reachability is not execution evidence.

**Precommitted criterion:** committed wrong-overload fixture is caught/refused
without source or graph changes, while the correct-target counterpart succeeds.

**Gate:** wrong-target/correct-target, stale-input, unexpected-edge, and rollback
checks, then all common gates. **Observation:** [stage5-verified-edits](observations/stage5-verified-edits/observation.md). **Blockers:** none.

## Stage 6 — Recurring comparative measurements

**Status: NOT STARTED**

Extend the existing comparative harness and retain its published baseline.
Schedule at most one campaign every 30 days, starting at this stage; defer
unavailable runs explicitly, without catch-up bursts. Runs take over three hours
and have OOM-killed a session. Freeze all versions, source SHAs, artifact hashes,
adapters, corpus, resource limits, and Girder candidate commit before running.
Reuse existing external-runner pins for the first comparison. Execute serially.
Girder is never the only runner. Keep passing tasks forever and append harder
successors. Retain resource-blocked runners in reporting.

**Precommitted criterion:** at least one fresh rerun with Girder and an external
runner, pinned identities, published deltas including regressions, raw evidence,
and resource failures. Replaying old archives does not count as a rerun.

**Gate:** pin/harness validation, fresh comparative campaign, then all common
gates. **Observation:** none. **Blockers:** preceding stages and hardware capacity.

## Stage 7 — Turns to correct edit

**Status: NOT STARTED**

The previous local 1.5B attempt could not complete the tasks. Its failed evidence
stays in [agentic-grep.md](agentic-grep.md) and linked observations. It is a known
blocker, not a reason to quietly drop this stage or substitute bytes saved.

Attempt only with a capable agent. Before execution freeze tasks, hidden
correctness checks, model/version, equal budgets, stopping rules, and turn
accounting. Run isolated serial Read/Grep-only and Girder-assisted arms with the
same agent. Keep evaluator answers out of both arms.

**Precommitted criterion:** independently verified correct edits from both arms
on precommitted tasks, with turns, failures, and unfinished tasks published.
Compare pairs only where both succeed. No capable agent means a published
blocker and no comparative claim, not substitution with a weak model.

**Gate:** harness/evaluator validation, capable-agent campaign, then all common
gates. **Observation:** none. **Blockers:** capable agent not yet established;
additionally, `docs/agentic-grep-tools.json` (the tool schema
`tools/agentic_grep_benchmark.py` feeds to its Girder arm) is a stale
checked-in snapshot that still contains the pre-`d1706d6` `impacted_tests`
overclaim — regenerate it from the live tool list before any run of this
benchmark, don't run it against the stale snapshot.

## Current checkpoint

### 2026-10-07 (release authorized) — merge to main and release 0.4.0

The user explicitly authorized merging, tagging and releasing ("obviously tag release push merge"), lifting the
no-tag rule above. Plan: bump to 0.4.0 (Cargo.toml, npm/package.json, server.json, READMEs), write RELEASE_NOTES
from `git log v0.3.3..HEAD`, open a PR, merge only after CI is green on it, then tag `v0.4.0` on the merged
main commit and verify the release workflow, npm installation and registry as for 0.3.x. Stage 6 remains
NOT STARTED (pins inspected: the 0.2.6 baseline artifacts exist on the drive; a fresh serial campaign is next).

### 2026-10-07 (Stage 5 DONE) — CI green on the published evidence

CI run 37584566098 passed both jobs on `6f7118c`, the head containing the Stage 5 observation (with binary
identity, commands and disclosures), gate logs for `300fff7`, and the impact-field assertion. The
precommitted criterion is met, so **Stage 5 is DONE**. Carried limits: certified scope is `replace_node` on
Rust/Python; the stale-bytes path relies on an existing non-certified test; impact-list truncation is not
exercised end to end; `~/.cargo/bin/girder` was stale until reinstalled for Stage 6.
**Next: Stage 6**, recurring comparisons: freeze versions, SHAs, hashes, adapters, corpus, limits and the
Girder commit; one serial campaign over three hours; at most one per 30 days.

### 2026-10-07 (Stage 5 measured) — verified edits built; criterion met, DONE pending CI

Certified `replace_node` steps (a `verify` block with path-bound fingerprints and a declared node/edge delta)
are checked on a disposable candidate before commit: refusal categories insufficient_evidence, ambiguity,
wrong_overload, stale_input, delta_mismatch, unexpected_edge; projection exactness and incremental-vs-cold
agreement are enforced. All 19 frozen plans meet their outcome; the wrong-overload plans are refused with the
whole tree hash-identical and the correct-target plan commits. Nine mutation checks each break a guard test;
the three fixture-unreachable guards have unit tests. Gates `300fff7` all pass; first candidate `beae7c2`
failed clippy and is published. Evidence: `docs/observations/stage5-verified-edits/`. Limits: replace_node on
Rust/Python only, structural delta, no live-agent authoring. Stage 5 is marked DONE only after CI is green on
the pushed head (recorded next). **Next: Stage 6**, recurring comparisons (needs a frozen pin set and a serial
campaign over three hours; at most one per 30 days).

### 2026-10-07 (Stage 4 DONE) — CI green on the published evidence

CI run 37570784619 passed both jobs (`fmt · clippy · test`, `windows compiles`) on `2852953`, the head
containing the corrected Stage 4 observation, smoke report, and final-candidate gate logs. The Stage 4
criterion (all three adapters install cleanly, expose Girder through MCP, load the shared orient-first
instruction through their native mechanisms, with committed evidence; README covers all three plus the raw
MCP JSON fallback; raw `npx` setup preserved) is met on that evidence, so **Stage 4 is DONE**. The
disclosed limits carry forward: no live model session loaded the instruction, the published `npx ... setup`
path was not exercised, Cursor's instruction needs `--project` and its hook is documented but not built.
**Next: Stage 5, verified edits.** Extend the existing Plan Format v2 journaled edits (not a separate
editor); read the current edit code with `girder` first and freeze the delta and refusal contract before
implementing.

### 2026-10-07 (Stage 4 corrected) — two review defects fixed; criterion met, DONE pending CI

Candidate `0fc97741` (binary `37739238…`). Detail in the
[Stage 4 observation](observations/stage4-clients/observation.md); formats frozen in
[doc-verification.md](observations/stage4-clients/doc-verification.md). Candidate `aa153de` was
superseded: review found (1) a data-loss bug (a rewritten owned instruction block kept the first install's
restore text, so uninstall could overwrite later edits; fixed, two regression tests, mutation-proven) and
(2) an MCP entry that launched `npx -y girder-mcp .` at every client start, against the stage text "Runtime
uses installed local Girder" (setup now writes `<path-to-girder> mcp .`, falling back to the npx form only
from npx's transient cache).
- **Built:** one canonical orient-first instruction installed through each client's native mechanism
  (Claude Code `~/.claude/CLAUDE.md` block; Codex `$CODEX_HOME/AGENTS.md` block, skipped when a non-empty
  `AGENTS.override.md` exists; Cursor project rule `.cursor/rules/girder-orient.mdc` with `--project`);
  README and `docs/setup.md` cover all three clients, the raw MCP JSON/TOML fallback, and per-client
  placement; raw `npx` setup preserved.
- **Evidence:** 14 new setup tests (guards mutation-checked); isolated per-client smoke: the written
  config parses in each client's documented format, the exact configured command answers `initialize` and
  `tools/list` (7 tools), the real `claude mcp list` (Connected) and `codex mcp list` read the configs;
  hook cases (normal with the exact documented output shape, missing graph, malformed input, missing binary,
  failing binary) pass through the installed commands for Claude Code and Codex; all four gates passed (802
  tests, 0 failed).
- **Disclosed:** no live model session loaded the instruction (vendor docs plus file and CLI-reader
  evidence); the published `npx ... setup` path and the npx-cache fallback entry were not launched; Cursor
  has no CLI here, its instruction needs `--project` (user rules are UI-only) and its hook is documented but
  not built (no pre-read hook can add context without blocking); Linux only.

Stage 4 is marked DONE only after CI is green on the pushed head (recorded next). Stage 5 follows.

### 2026-10-06 (Stage 4 measured) — client adapters built; criterion met, DONE pending CI

Candidate `aa153de6` (binary `625025fa…`). Detail in the
[Stage 4 observation](observations/stage4-clients/observation.md); documentation facts were frozen first
([doc-verification.md](observations/stage4-clients/doc-verification.md), source and date for every format).
- **Built:** one canonical orient-first instruction (`npm/instructions/orient-first.md`) installed by
  `girder setup` through each client's native mechanism: a delimited block in `~/.claude/CLAUDE.md`, a block
  in `$CODEX_HOME/AGENTS.md` (skipped and reported when a non-empty `AGENTS.override.md` exists), and a
  Cursor project rule `.cursor/rules/girder-orient.mdc` with `--project`. Ownership-tracked, idempotent,
  exact uninstall; `--no-instructions` opts out. README and `docs/setup.md` cover all three clients and the
  raw MCP JSON/TOML fallback with per-client placement; raw `npx` setup preserved.
- **Evidence:** 11 new setup tests (22 existing unchanged; three guards mutation-checked); an isolated
  per-client smoke (config parses in each client's documented format, instruction present, MCP server
  answers with all 7 tools, uninstall clean): **Claude Code, Codex, Cursor all PASS**; hook cases (normal,
  missing graph, malformed input, hook failure) pass through the installed commands for Claude Code and
  Codex; all four gates passed (799 tests, 0 failed). The first smoke attempt failed one cold-hook check
  (the hook's 20 ms deadline answers silently) and remains published.
- **Disclosed:** the `npx -y girder-mcp .` launch path and `npx ... setup` (published package) were not
  exercised offline; no live client session was driven, so "loads the instruction" rests on vendor docs plus
  file and protocol checks; Cursor's instruction needs `--project` (user rules are UI-only, no file path is
  documented) and its hook is documented but not built (no pre-read hook can add context without blocking);
  Linux only.

Stage 4 is marked DONE only after CI is green on the pushed head (recorded next). Stage 5 follows.

**Superseded** by the 2026-10-07 entry above after review found two defects (see there); its
artifacts are kept in `observations/stage4-clients/superseded-aa153de/`.

### 2026-10-06 (Go DONE; Stage 3 complete) — CI green on the published evidence

CI run 37563090631 passed both jobs (`fmt · clippy · test`, `windows compiles`) on `4b35c52`, the head
containing the Go after-observation, the final-candidate gate logs, and the whole-tree Must
verification. The frozen Go criterion (corpus improvement, nonempty Must precision 1.000, zero
unsound audit cells) is met on that committed evidence, so **Go is DONE and Stage 3 is complete for
Rust, Python, TypeScript, and Go.** This is not a claim of dispatch completeness: the disclosed holes
carry forward, and three corpus cases still owe harder successors. Stage 4 is next.

### 2026-10-06 (Go measured) — after-observation published; criterion met, DONE pending CI

Final candidate `08fcbdc0` (binary `c8fb49e3…`), attempt 4 of 4; every attempt is published. Full
detail in the [Go after-observation](observations/stage3-go-audit/after-observation.md).
- **Fixture contract (21):** baseline 10 / 7 / 3 / 1 to **21 / 0 / 0 / 0**; Must 7/7 correct.
- **Corpus (unchanged, pins verified):** pooled 23 / 33 / 0 / 1 to **25 / 31 / 0 / 1**; Go 5 / 9 to
  **7 / 7**; Must 6/6 to **8/8**. The pre-written predictions held exactly.
- **Real audit (118 calls):** baseline 15 / 96 / 0 / **7 unsafe exclusion** to **27 / 91 / 0 / 0**;
  Girder Must claims 11 to **23, all correct**. A whole-tree check verifies **401 Must claims across the
  70 files with 0 violations**.
- **Gates:** all four passed on `08fcbdc` (788 tests, 0 failed; clippy, fmt, npm clean).
- **A policy departure was found and fixed:** attempts 1 to 3 certified `go`/`defer` statement calls,
  which frozen G1-a forbids; fixtures and the sampled audit were blind to it. A whole-tree check of the
  attempt-3 graph found 3 violating Must claims of 404; the direct-operand refusal (mutation-checked)
  fixes it. Attempt 3, an earlier failed clippy gate (`type_complexity`), and attempt 1's 3 exclusions
  (generic calls parsed as type conversions) all remain published.

The frozen Go criterion (corpus improvement, nonempty Must precision 1.000, zero unsound audit cells)
is met on committed evidence, so **Go is marked DONE only after CI is green on the pushed head**
(recorded next). **Disclosed holes:** 91 audit cells are conservative Unknown; Must recall on labeled
musts is 23 of 114; Must claims are conditional on compilation; the audit is a non-test stdlib subset
with no `go.mod`; labels were drafted by a second agent and lead-audited; safety-net Unknown claims
anchor on the file's first function and do not record the real caller; a speculative scanner for `ERROR`
regions was removed as untested; the Stage 2 obligation to append harder successors for
`go-direct-cross-file`, `go-closure-captures-direct-call`, and TypeScript's case is **outstanding**.

### 2026-10-06 (Go baseline) — labels frozen; baseline measured: 3 contract overclaims, 7 audit unsafe exclusions

Labels frozen (`bb2c06a`): 140 sites, 118 actual calls, 22 non-calls, drafted by the
second agent in 7 batches (3 needed a second attempt, 0 single-agent fallbacks) and
audited at 100% with 2 disagreements (`unsafe.Sizeof`, a compiler built-in); all 78
in-snapshot `must` targets mechanically verified. Girder was not run on the tree
before this commit. [Baseline observation](observations/stage3-go-audit/baseline-observation.md)
on the unchanged resolver (binary `cbc20b34…`):
- **Fixture contract (21):** 10 exact / 7 conservative / **3 overclaim** / 1 failed.
  The three overclaims are same-file Musts the frozen refusals forbid (an `app` /
  `app_test` identity collision, a build-constrained file, a cgo file); the failure is
  a call in a package-level initializer that leaves no claim at all.
- **Corpus (unchanged):** 23 / 33 / 0 / 1, Go 5 exact / 9 conservative.
- **Audit (118 calls):** 15 exact / 96 conservative / 0 overclaim / **7 unsafe
  exclusion**; existing Must claims 11/11 correct. The 7 exclusions are calls with no
  claim (verified for two as duplicate v1/v2 definitions in `encoding/json` and one
  package-level initializer; four generic-call sites not yet explained).

The Go criterion is **not** met at baseline (7 errors, all lost evidence). Next:
implement the frozen policy (same-package cross-file proof, tightened same-file path,
explicit Unknown claims where evidence is now lost), then the after-observation and
gates. Go remains IN PROGRESS.

### 2026-10-06 (Go started) — snapshot pinned; policy and methodology FROZEN; no labels, no Girder run, no resolver change

Stage 3 Go has begun. Nothing has been measured with Girder and no product code
changed.
- **Snapshot** (`a950897`): Go 1.27.1 source, `go1.27.1.src.tar.gz`, 35,109,201
  bytes, sha256 `4e408abae126d916b6164627193f2c54f0e3ca1312d693b86db45f862ab238b1`
  (verified against go.dev), in the gitignored `.benchmark-cache/stage3-go-v1/`.
  Extraction keeps 15 stdlib packages, 70 non-test files, 26,925 lines
  ([stage3-go-corpus.json](stage3-go-corpus.json), reproduced by
  `tools/go_audit_inventory.py`). The filter drops `_test.go`, `testdata/`,
  `vendor/`, and `//go:build ignore` files so the sampled corpus equals what
  Girder's default walk indexes.
- **Frozen documents** in `observations/stage3-go-audit/`: [policy.md](observations/stage3-go-audit/policy.md)
  (same-package direct-call proof; refusals apply to every Go Must, same-file
  included; predicted corpus result written first: Go 7 exact / 7 conservative,
  pooled 25 / 31, with `go-direct-cross-file` and `go-closure-captures-direct-call`
  expected to flip) and [methodology.md](observations/stage3-go-audit/methodology.md)
  (independent regex enumeration, mechanical selection, call-end-byte matching,
  100-actual-site continuation rule, labeling fallback). Girder must not be run
  on the audit tree before the labels are frozen.
- **Fixtures:** 21 adversarial modules (7 expected Must with `expected_target`,
  14 expected Unknown), all passing runtime validation under go1.19.8
  ([manifest sha256 `fc88e266…`](../fixtures/go-direct-call-proof/v1/manifest.json)).
- **Population and quotas:** 5,589 regex candidates; `bare_cross_file`, the
  stratum the policy targets, has only 107. The initial 140-site list
  (`sites-initial-140.json`) is generated and hash-pinned before any labeling.
- **Worker note:** the second agent (Grok) completes read-only lookup briefs but
  has repeatedly returned only an opening line on drafting briefs, so the Go
  tooling was written by the lead; labeling batches have a precommitted two-try
  fallback to the lead (flagged `single_agent`, audited at 100%).

Not yet done: the context tool and labeling, the baseline observation (fixture
contract, unchanged 49-case corpus, audit) on the current binary, the resolver
change, the after-observation, and the Go gates. Deferred holes: package-qualified
calls (the audit root has no `go.mod`), `may` classification, and everything
beyond bare same-package calls. Go remains IN PROGRESS.

### 2026-10-06 (TypeScript DONE) — CI green on the published evidence

CI run 37538734394 passed both jobs (`fmt · clippy · test` and `windows
compiles`) on `c78f6c9`, the head that contains the committed ESM
after-observation and both gate-log directories. The frozen TypeScript
criterion (measured corpus improvement, nonempty Must precision 1.000, zero
unsound audit cells) is met on that committed evidence, so **TypeScript is
DONE**; the disclosed holes in the entry below and the after-observation carry
forward. Go is NOT STARTED and is the next language. Housekeeping before Go
work: no `girder` binary is installed on PATH (only the debug build under the
MOVESPEED target dir); rerun the release `cargo install` with an explicit
`--target-dir` while nothing else is building.

### 2026-10-06 (TypeScript measured) — ESM after-observation published; criterion met, DONE pending CI

Binary `cbc20b34…` built at `cb2aa11`; final candidate `b0c09b6` (test-only
commit after it). Full detail, hashes, and commands in
[after-observation.md](observations/stage3-typescript-audit/esm-import-proof/after-observation.md).
- 73-case ESM contract: **73/73 exact, 0 errors** (was 66/73).
- 49-case corpus (unchanged, pins verified): **23 exact / 33 conservative /
  0 unsound / 1 failed** (was 22/34/0/1); Must precision **6/6**;
  `typescript-direct-cross-file` conservative to **exact**. The failed
  `typescript-structural-object-literal` is retained.
- 100-call real audit: **56 exact / 44 conservative / 0 unsound, Must 3/3**.
  Checked site by site: identical to the immediate predecessor (0 differences
  over 110 rows); against the frozen baseline `extension/before` it is 53/47 to
  56/44 with three `unknown` to `must` changes (indices 32, 37, 91), all correct.
- All four common gates passed on both `cb2aa11` and `b0c09b6` (783 tests passed,
  0 failed, 2 ignored; clippy and fmt clean; npm 29 passed, 2 skipped); logs in
  `gates-cb2aa11/` and `gates-b0c09b6/`.

The frozen per-language criterion (corpus improvement, nonempty Must precision
1.000, zero unsound audit cells) is met on committed evidence. **TypeScript is
marked DONE only after CI is green on the pushed head** (recorded next).
**Disclosed holes carried forward:** Must claims are conditional on the policy's
ESM execution assumptions; plan projections and workspace buffers have no
application-level C-3 test (no route attests a root and accepts non-disk bytes);
the subdirectory-deletion, symlinked-file, and symlinked-directory tests are not
mutation-checked; the Stage 2 obligation to append a harder successor to the
newly passing corpus case (as a new versioned corpus file, never an edit of the
frozen one) is **outstanding**.

### 2026-10-06 (application tests) — ESM ingestion-route tests added; projection NOT covered

Candidate `ec2b73a` adds four application tests in `watch/tests/esm.rs` (drafted
by the worker agent, reviewed and applied by the lead):
- `esm_watch_subdirectory_target_deletion_and_recreate_match_cold`: baseline
  Must; deleting the whole target directory gives Unknown with watched == cold;
  recreating it restores Must. Not mutation-checked.
- `esm_watch_symlinked_target_file_stays_unknown`: passes because the source
  walk never ingests a symlinked file, so the import has no indexed target. It
  does **not** exercise the resolver's own symlink-component refusal. Not
  mutation-checked.
- `esm_watch_follow_symlinks_keeps_plain_target_unknown`: **mutation-proven**
  (forcing `set_source_root` despite `follow_symlinks = true` flips the claim
  to Must and fails the test: `left: Must, right: Unknown`).
- `esm_stale_environment_candidate_is_not_publishable`: **mutation-proven**
  (disabling the environment comparison in `Snapshot::matches_candidate`
  fails the test at its final assertion).

**Not covered, disclosed gaps.** (1) In-memory projections that differ from
disk: no application route both attests a source root and accepts bytes other
than a disk read, so no application-level test is reachable; the builder
contract is pinned by `source_only_and_modified_projection_loads_have_no_import_certificate`
in `crates/aether-builder/tests/typescript_esm.rs`. (2) A symlinked directory
component in an import path. Ingestion-route validation is therefore **not**
complete. A live watch thread also cannot be driven deterministically onto the
discard path for a stale candidate; the test pins the publication predicate
instead.

Pre-push local gates on the final tree (not Stage 3 gate evidence): fmt clean;
clippy exit 0; workspace tests 27 suites, 782 passed, 0 failed, 2 ignored; npm
29 passed, 0 failed, 2 skipped. Official gate logs must be rerun on the
committed candidate and saved under `docs/observations/` before any DONE claim.

Next: fresh CLI build and identity, then the unchanged 73-case contract, 49-case
corpus (watch `typescript-direct-cross-file`) and 100-call audit under a new
observation name; confirm the pinned TypeScript archives are present first.
TypeScript remains IN PROGRESS; Go remains NOT STARTED.

### 2026-10-06 (resolved) — Watch-test failures root-caused to a harness defect; WIP committed

Candidate `b4a59ea`. CI had been red since the tests-only commit `c06201e`
(3 failing app tests). Root cause for two of them: the shared watch-test
`Fixture::new()` seeds Rust files (`lib.rs`, `math.rs`, `app.rs`), and the ESM
tests then add `app.ts`/`app.test.ts`. Both stems are module path `app`, which
frozen policy rule R-GRAPH-2 refuses (`mixed-language-module-collision`, a case
in the frozen manifest; the 19-step base dir
`fixtures/typescript-esm-import-proof/v1-incremental/base` contains `app.ts`
and `app.test.ts`). The product's `Unknown` was correct; the harness was wrong.
The tests now use an ESM-only fixture (`esm_fixture()`); **every assertion,
the manifest, and the policy are unchanged**. The third failure,
`esm_snapshot_tracks_non_source_proof_inputs`, is fixed by the WIP's product
change (environment capture and snapshot wiring), now committed together with
the `typescript_esm/environment.rs` it depends on. rustfmt was applied to the
WIP files (CI's fmt step would otherwise have failed).

Pre-push local run on the final tree (not Stage 3 gate evidence): `cargo fmt
--all --check` clean; `cargo clippy --workspace --all-targets -j1 -- -D
warnings` exit 0; `cargo test --workspace -j1 --quiet` 27 suites, 778 passed,
0 failed, 2 ignored; `node --test npm/test/*.test.js` 29 passed, 0 failed, 2
skipped. CI's actual result on the pushed head is checked separately.

Still open for Stage 3 TypeScript acceptance: ingestion-route coverage
(directory deletion, projections, symlink following), a fresh CLI build, the
after-contract/corpus/audit measurements, and the four common gates as
committed observations. TypeScript remains IN PROGRESS; Go remains NOT STARTED.

### 2026-10-06 (latest) — Watch tests ran on the WIP: 12 passed, 2 FAILED

With MOVESPEED remounted, `cargo test -p aether-app -j1 --quiet watch` ran on
the uncommitted ESM environment WIP: **12 passed, 2 failed, 1 ignored**. Failing,
both at their first assertion (`watch/tests/esm.rs:84` and `:151`, the baseline
check before any mutation): `esm_watch_non_source_creation_edit_and_removal_match_cold`
and `esm_watch_replays_frozen_19_steps_and_serves_mcp`. Each expected `Must`
and observed `Unknown`; the watched and cold builds agreed, so the resolver
refuses the app-level fixture. This is the conservative direction, not an
unsound claim, but the contracts are unmet. The WIP is therefore **not
committed**. `c06201e` added only these tests (no product code), so whether
HEAD without the WIP fails the same way is not yet known and will be measured
on a separate worktree. No application-level acceptance is claimed.

Next: get the refusal reason for the baseline claim, compare against a clean
`c06201e` worktree, fix product code (never the tests, manifest, or
expectations), and rerun. TypeScript remains IN PROGRESS; Go remains NOT STARTED.

### 2026-10-06 (later) — Uncommitted ESM environment WIP triaged; watch tests BLOCKED

Uncommitted work left after `c06201e` (not committed, not pushed): a new
`sync/typescript_esm/environment.rs` (`TypeScriptEsmEnvironment`),
`GraphBuilder::typescript_environment()` (`resolve_calls` now takes `&mut self`),
and watched-MCP snapshot wiring in `mcp/watch.rs` and `mcp/watch/snapshot.rs`
so environment inputs (configs, mocks, hooks) are never filtered as ignored and
a snapshot publishes only if the builder's environment matches it.

Observed on this working tree: `cargo check -p aether-app --all-targets -j1`
exit 0, no errors or warnings (32m cold build). `cargo test -p aether-builder
-j1 --quiet` exit 0, all suites ok (134+2+2+1+7+1+5+12 = 164 tests, including
the frozen ESM contracts). `cargo test -p aether-app -j1 --quiet watch` did
**not run**: the compile died with SIGBUS writing to the target dir, and
`/mnt/chromeos/removable/MOVESPEED` is no longer mounted in the VM (`dmesg`
also shows `virtio_balloon: Out of puff`). This is an environment blocker, not
a test result. No application-level acceptance is claimed.

Next: re-share the MOVESPEED drive, rerun the watch tests, then commit the WIP
as one piece and continue the Stage 3 TypeScript ESM acceptance list below.
TypeScript remains IN PROGRESS; Go remains NOT STARTED.

### 2026-10-06 — Conditional ESM resolver implemented; application acceptance pending

Candidate `d0cf78b` passes 164 builder tests, including all 73 frozen ESM
contracts and the 19-step cold-versus-incremental sequence. The
[development observation](observations/stage3-typescript-audit/esm-import-proof/development-observation.md)
publishes the command/log, added harder refusal cases, conservative implementation
limits, and the retained first failed run (the test helper wrongly indexed an
excluded mock target; corrected without changing the fixture). These are
conditional Must certificates under the frozen execution assumptions.

Next: validate watched-MCP and application ingestion routes, including non-source
events, directory deletion, projections, and configured symlink following;
build a fresh CLI and measure the after-contract, unchanged 49-case corpus and
100-call audit; then run the common gates. The existing CLI binary predates
this resolver. No application-level acceptance or new common gate is claimed.
TypeScript remains IN PROGRESS; Go remains NOT STARTED.

### 2026-10-05 — ESM cold CLI baseline measured (historical)

The [frozen 73-case contract baseline](observations/stage3-typescript-audit/esm-import-proof/before-observation.md)
has 66 exact refusals and seven conservative misses: every marked call is
explicit Unknown with no target. All expected target declarations were found;
there are no missing marked claims or command errors. The freshly rebuilt
candidate `c209a22` passed its serial offline build; the measurement runner's
nine unit tests passed. All frozen pins were checked, and raw outputs and
the failed contract remain published. No ESM product implementation or new
language gate has run. Next: implement the frozen policy, including snapshot
identity and invalidation, then measure cold/incremental/watched-MCP equality,
ingestion routes, the unchanged dispatch corpus and real audit, and common
gates. TypeScript remains IN PROGRESS; Go remains NOT STARTED.

### 2026-10-03 — ESM named-import proof policy v1 frozen (historical)

The [bounded policy](observations/stage3-typescript-audit/esm-import-proof/policy.md)
is frozen after independent soundness review and correction of the acceptance
count to all 73 manifest cases. The manifest hash is
`daa7310248f8e2adc3bdbbae340a3301dcca6b18983ca29ef3c6738bb97b94cd`.
Local preimplementation checks passed: 67 runnable cases, 6 explicit skips,
and 19 incremental runtime steps. Its Must claims are conditional on the
recorded ESM execution and no-unmodeled-hook assumptions. No product code or
new Girder measurement exists yet. **Next:** implement the exact frozen rule,
validate its full proof contract including cold/incremental equality, then
rerun the unchanged dispatch corpus, 100-call audit, and four common gates.
TypeScript remains IN PROGRESS; Go remains NOT STARTED.

### 2026-10-03 — ESM named-import proof policy correction 2 (historical)

The second high review returned NO-GO on `7fdfdd2`. Correction 2 makes these
changes in the
[policy](observations/stage3-typescript-audit/esm-import-proof/policy.md):
- **Hooks and execution model.** Hook registration (including indexed preloads),
  pinned hook, mock, or runner libraries, and preload flags or rc files now
  refuse the whole snapshot (HOOK-1..3). Two runtime-checked fixtures show
  loader hooks redirecting `./app.ts`. Every other mechanism is excluded by a
  stated assumption, so every Must is explicitly conditional and audits must
  say so.
- **MOCK-2** now refuses the whole snapshot for every failed mock identity
  proof. A symlinked mock path replaced the real-path import at runtime.
- **Incremental sequence.** It grew to 19 steps, adding mock, config,
  `__mocks__`, and hook creation and removal. Each step has an expected answer
  and a cold-versus-incremental equality requirement.
- **Harness.** It now checks the error category of every expected failure,
  backed by negative controls.

The manifest has 73 cases (7 Must and 66 Unknown). No product code ran,
nothing was measured, and the corpus and labels are unchanged. Eight decisions
remain open in the policy. TypeScript remains IN PROGRESS; Go remains NOT
STARTED.

### 2026-10-03 — ESM named-import proof policy correction 1 (historical)

The high review returned NO-GO on `79b7e70`. Correction 1 addresses all four
required points in the
[policy](observations/stage3-typescript-audit/esm-import-proof/policy.md):
1. **Specifier spelling.** The raw text must equal the cooked value and match an
   ASCII allow-list. This refuses percent-encoding, escapes, backslashes,
   controls, whitespace, and non-ASCII characters. It was runtime-checked:
   `%61`, `%2e%2e`, `\`, TAB, LF, and a trailing space each made Node load a
   different file than a lexical path names, or strip characters.
2. **Extensions.** v1 accepts `.ts` and `.mts` only. The policy states its
   ESM-preserving execution assumptions.
3. **Mocking.** A mock refuses by resolved module identity. A setup file's
   `../app.ts` mock replaced the importer's `./app.ts` import at runtime. Any
   uncertain mock, mocking configuration, or `__mocks__` directory refuses the
   whole snapshot.
4. **Acceptance.** It now covers the cycle and mock gates, mixed-language
   collisions, incremental invalidation (a pinned 7-step sequence), canonical
   identity and symlinks, and agreement across ingestion routes.

The manifest now has 64 cases (7 Must and 57 Unknown) plus 7 incremental steps.
The preimplementation checks pass: 58 cases run under Node, 6 are skipped with
reasons, and 7 of 7 steps match.

No product code ran, nothing was measured, and the corpus and labels are
unchanged. Seven decisions remain open in the policy, so this is not a freeze.
TypeScript remains IN PROGRESS; Go remains NOT STARTED.

### 2026-10-03 — Relative ESM named-import proof policy drafted (historical)

The [draft policy](observations/stage3-typescript-audit/esm-import-proof/policy.md)
targets the unchanged `typescript-direct-cross-file` case. It allows a bounded
Must for one form: a static named import from an explicit-extension relative
specifier, bound to a unique, cleanly bound top-level `export function`, and
called directly. It refuses everything else as Unknown with no target. That
includes extensionless or `.js` specifiers, re-exports and stars, type-only
imports, cycles, live-binding writes and `eval`, shadowing, module-path
collisions, parse errors, and module mocks.

The policy has 40 pinned adversarial fixture cases: 5 Must and 35 Unknown. Of
those, 38 pass `node --test` and 2 are skipped with recorded reasons. Two of the
fixtures show at runtime that a Must would be unsound.

No product code ran, nothing was measured, and no improvement is claimed. The
structural object-literal failure stays visible. The corpus, labels, and audit
are unchanged. Five decisions are listed in the policy for review before it is
frozen. TypeScript remains IN PROGRESS; Go remains NOT STARTED.

### 2026-10-03 — Structural member identity audit completed locally (historical)

The MOVESPEED workstation has all four pinned TypeScript archives, unlike the
cloud container. [A fresh offline audit](observations/stage3-typescript-audit/lexical-bindings/structural-members-after-local-1/observation.md)
on candidate `b319556` and locally rebuilt binary `0e6e09c7…` accounted for
100 actual calls and 10 noncalls: **56 exact / 44 conservative / 0 unsound**,
with **3/3 correct Must** and 97 Unknown. Compared with the immediate prior
55/45, 2/2 Must checkpoint, one compiler call moved from Unknown to a correct
Must target; eight other records changed details without changing class.
The separate structural validation scorer passed 17/17 origin contracts with
0 unsound cells. The unchanged 49-case corpus stayed at 22/34/0/0/1; its one
failure is now the predicted alice/bob origin ambiguity, not an improvement.
All four common gates passed locally on `b319556` (769 Rust passed, 2 ignored;
strict Clippy and fmt clean; npm 29 passed, 2 skipped). The earlier cloud audit
blocker and first implementation NO-GO remain published below.

**Next:** freeze a narrow TypeScript dispatch proof policy and adversarial
fixtures before implementation or measurement. A named relative ESM import of a
unique exported function is the bounded existing corpus case to examine first.
Do not infer proof from structural identity alone. TypeScript remains IN PROGRESS;
Go remains NOT STARTED.

### 2026-10-03 — Structural member identity policy v1 implemented (`230418e`); cloud audit re-run blocked (historical)

Implementation `0c9db9b` passed all four gates but got a NO-GO in review. That
result is retained. Correction `230418e` fixes class-field scoping, lexical R3
pruning, the K1 shorthand rule, and scorer failure persistence. Each fix has a
regression test that is proven by mutation.
[Observation](observations/stage3-typescript-audit/structural-members/implementation-observation.md):
- The identity contract passes: 31 files, 44 identities, 62 refusals, 10
  declaration-only spans, and 6 exact-span B1 boundaries.
- The separate validation scorer meets 17/17 resolution contracts with 0 unsound
  cells.
- The 49-case corpus is unchanged at 22/34/0/0/1. The original case now fails as
  **ambiguous** (alice/bob), as predicted. This is not a dispatch improvement.
- All four common gates pass on `230418e`: 769 Cargo tests passed (2 ignored),
  clippy and fmt are clean, and npm has 29 passed / 2 skipped.

The 100-call audit re-run is **BLOCKED** here because the egress proxy returned
HTTP 403 for the pinned archives. It must run on the pinned cache.

Next: user review of `230418e`, then the audit re-run on the MOVESPEED
workstation. TypeScript remains IN PROGRESS and Go remains NOT STARTED.

### 2026-10-03 — Structural member identity policy v1 FROZEN (historical)

The freeze commit is the policy-only commit directly after `2e5c8bc` on
`claude/roadmap-ts-policy`. Its SHA cannot appear in its own content; it is
reported with the push and is the parent of the next commit. That commit marks
[the policy](observations/stage3-typescript-audit/structural-members/policy.md)
as the frozen preimplementation policy. It also closes `const`-only owners as
the chosen v1 default.

The fixtures, both manifests, and every pinned hash are unchanged from
`2e5c8bc`. That commit passed the user's independent Node replay and byte
inspection, and the high review concluded K1, T1, and B1 are sufficient. The
dispatch corpus, the original fixture, and product code are unchanged.

**Next: product implementation of the frozen policy**, after the user reviews
the freeze commit. Its acceptance covers:
- the identity contracts;
- the postimplementation B1 boundaries;
- a separate validation scorer;
- the predicted ambiguity failure of the unchanged original case;
- the re-run 49-case corpus and 100-call audit;
- all four common gates.

TypeScript remains IN PROGRESS; Go remains NOT STARTED.

### 2026-10-03 — Freeze correction 2: escaped-key fixture bytes repaired (historical)

Review found a defect in correction 1 (`d8d1645`). `mixed-key-computed-alias.ts`
lines 15 and 23 contained plain `name`/`list` bytes, not the escaped identifiers
that K1 and the `null` expectations required. Correction 1's Node checks passed
anyway; that record is retained as `preimplementation-checks-correction-1.json`.

Both sites now hold literal backslash-u escapes (`name`, `list`), and
the runtime keys are unchanged. The file and manifest hashes are re-pinned. A
new byte-level check passes on the new bytes and fails on the old bytes (a
committed negative control). All Node checks pass. No rule text, product code,
dispatch corpus, or original fixture changed. Awaiting review. TypeScript is IN
PROGRESS and Go is NOT STARTED.

### 2026-10-03 — Freeze correction 1 to the structural member policy (historical)

This applies the high review of `6ac3124` to the policy and fixtures only. No
product code changed; no Girder or Cargo run.
- **K1:** any string, numeric, computed, or escaped key in a literal refuses every
  callable member of that literal. Other literals keep their identities.
- **T1:** a refused member or branch emits no callable descendants. Same-named
  outer declarations stay single nodes, and sibling branches stay indexed.
- **B1:** every refused subtree containing calls needs an explicit Unknown
  coverage boundary on the nearest existing Function or Module. Its six boundary
  pins are **postimplementation acceptance requirements**. Node cannot observe
  them, and they are not claimed as checked.

The corpus now has 31 identity files with 106 members, plus 17 validation cases
over 11 fixtures. The Node checks pass: 30/31 identity files ran (1 non-erasable
file skipped) and all 11 validation fixtures pass. Prior manifest entries are
byte-unchanged, and the corpus and original-fixture hashes are re-verified.
Awaiting review. TypeScript is IN PROGRESS and Go is NOT STARTED.

### 2026-10-03 — Structural member identity policy drafted for review (historical)

Branch `claude/roadmap-ts-policy` from `e3812c5`. Draft
[structural member identity policy v1](observations/stage3-typescript-audit/structural-members/policy.md)
plus a 22-file identity contract corpus (55 marked members, 10 declaration-only
signatures) and 15 separate qualified validation cases over 10 fixtures. It
assigns identity only (`<scope>::<owner>::@object::<key>`) to const-owned,
identifier-keyed arrow, function-expression, and method-shorthand members. It
declares interface/abstract/overload signatures declaration-only. It refuses
owner-less, `let`/`var`, non-identifier keys, accessors, generators, spread,
`__proto__`, duplicate keys, and colliding paths. It authorizes no Must or May
proof. It predicts that the unchanged original case will move from zero matches
to an **ambiguity failure and must stay failed**. The corpus, the original
fixture, and the classification policy are byte-unchanged (hashes pinned). Only
Node ran ([checks](observations/stage3-typescript-audit/structural-members/preimplementation-checks.json):
all pass, one non-erasable file skipped). No Girder, Cargo, or product code.
Awaiting user corrections before implementation. TypeScript remains IN PROGRESS
and Go remains NOT STARTED. Common gates are pending.

### 2026-10-02 — Pause checkpoint (historical)

2026-10-02 pause checkpoint: no implementation changes or measurements since
`464aa6b`. No Cargo job remains running. Resume with the TypeScript structural
member policy and frozen cases described below; language gates remain pending.

### 2026-10-02 — Real-audit improvement verified; dispatch failure remains

Implementation `5733be6`; [real-audit observation `1c10eff`](observations/stage3-typescript-audit/lexical-bindings/bindings-after-1/observation.md).
Fresh pinned source inventories and every frozen input matched. On 100 actual
calls (plus ten retained noncalls), the audit improved **53 → 55 exact** and
**47 → 45 conservative**, with zero scored unsound answers. The only class
changes were indices 37 and 91: `popNameGenerationScope` and
`verifySolutionScenario` now resolve to their frozen, source-checked targets.
Must precision is **2/2**, with **98 Unknown**. This small denominator is not
broad accuracy evidence; May recall is undefined because there are no expected
May sites in this sample. All raw outputs, changed rows, and timing limits are
published. The separate 22-case proof corpus still has 22/22 exact answers and
19 passing executable probes; all four demonstrated false Must baselines remain.

[Dispatch observation `7f8b18b`](observations/stage3-typescript-audit/lexical-bindings/corpus-observation.md)
attempted all 49 unchanged cases: 22 exact, 34 conservative, zero unsound, and
one unresolved structural-object-literal origin. There is **no dispatch-corpus
improvement yet**. [Reporting correction `55f989a`](observations/stage3-typescript-audit/lexical-bindings/corpus-reporting-correction.md)
fixes missing language metadata/counting for failed cases: the failure is now
explicitly attributed to TypeScript. Fourteen reporting tests passed. No Girder
rerun or observed-class change was used for that correction; old reports remain.

Next: precommit structural-member extraction and further dispatch proofs,
retaining ambiguous-origin refusals and every failed case. Re-measure changed
behavior and run the four common language gates before completion. The existing
146 passing builder tests and two-correct-Must audit do not by themselves satisfy
all language criteria. TypeScript remains IN PROGRESS; Go remains NOT STARTED.
The following checkpoints are historical.


### 2026-10-02 — CLI lexical-proof contract passed; repository audit next

Implementation `5733be6`; [observation `09b5c73`](observations/stage3-typescript-audit/lexical-bindings/after-observation.md).
The rebuilt CLI passes **22/22** frozen contracts (before: 10/22). All six Must
calls name the correct declaration; all sixteen Unknown calls retain explicit
evidence without guessed targets. The four runtime-disproved Must claims are
now Unknown. All 19 executable probes pass; the same three TypeScript-only
cases remain skipped. No case, label, or historical failure was changed.

The 146 builder tests and CLI proof result do not establish language acceptance.
Next: `python3 -m tools.measure_typescript_real_audit --name bindings-after-1`
using the rebuilt binary with SHA-256
`fe6901793bb8055a66359ad235e631219ba87b3f6ab96a396e4741925a7deaef`.
This runs the unchanged 100-call audit on fresh extractions of the four pinned
archives, offline and serially, checking frozen inputs and complete source
inventories. Initial run identity is saved before workload execution so an
interruption remains recoverable. Output is kept under
`observations/stage3-typescript-audit/lexical-bindings/bindings-after-1/`.

After that observation, continue structural extraction and dispatch-corpus work
and the common language gates. TypeScript remains IN PROGRESS; Go NOT STARTED.
The following checkpoints are historical.


### 2026-10-02 — TypeScript false-Must repair implemented; CLI verification next

Policy/corpus `954334d` freezes 22 lexical-binding proof cases before code changes.
[Baseline `1d42ee7`](observations/stage3-typescript-audit/lexical-bindings/before-observation.md)
scored 10/22 exact contracts and demonstrated **four false Must targets** using
executed destructuring, escaped-assignment, direct-eval, and escaped-eval probes.
All 19 executable probes matched their frozen outcomes; three TypeScript-only
forms were skipped. Among seven executed marked Must calls, four contradicted
the certified target. These hostile cases are separate from the unchanged
49-case dispatch corpus and 100-site real-repository audit; they do not change
those denominators or historical observations.

The implementation now indexes identifier uses (including shorthand patterns),
requires module/function-body lexical scope, and refuses eval, escaped
identifiers, and unsupported uses. Existing parse/identity/transformation guards
remain. Nested functions may receive Must only after the scope and use checks;
Rust/Python/Go proof paths are unchanged.

[Builder verification](observations/stage3-typescript-audit/lexical-bindings/builder-tests-1.json):
146 tests passed across seven suites, including all 22 contract cases and the
registration identity regressions. This is not CLI or full language acceptance.
Next: rebuild the CLI and rerun the frozen proof/runtime corpus, then measure the
real-repository audit and remaining structural extraction/dispatch gaps. Common
language gates remain outstanding; TypeScript is IN PROGRESS and Go NOT STARTED.


### 2026-10-02 — Registration bodies recovered on real source

Implementation `67d65f6` and [observation `e4a5316`](observations/stage3-typescript-audit/identity-repair/observation.md)
are committed. The unchanged original collision fixture now retains four tests
instead of three; the precommitted harder fixture retains all five. On the
unchanged real source files, date-fns retains 89 tests instead of 47 and the
TypeScript compiler file retains 30 instead of 26: **46 recovered test bodies**,
with no previously retained registration offset lost. No duplicate-path boundary
remains in these four analyzed files.

Seven focused builder tests and the rebuilt CLI checks passed. Source fingerprints
were checked against the preserved baseline; raw outputs and commands are public.
Audit sites 23 and 91 now have the correct test owner, but remain Unknown. The
real files were analyzed in isolation: this does not establish whole-project
resolution, a new 100-site audit result, or passage of the four common gates.
Registration IDs change and graphs need rebuilding; identical sibling insertion
or reordering can still rekey IDs, as precommitted.

Next: freeze lexical-binding proof rules and adversarial cases, implement the
remaining binding/structural extraction work, and run the complete dispatch
corpus, 100-site audit, and common gates. TypeScript remains IN PROGRESS; Go
remains NOT STARTED. The following checkpoints are historical.


### 2026-10-02 — TypeScript registration identity repair in progress

Preimplementation policy and harder fixture: `69b4c7f`,
[identity-repair/policy.md](observations/stage3-typescript-audit/identity-repair/policy.md).
The label-derived profile records 37 cross-file and ten same-file ground-truth
Must targets; all ten same-file targets fall outside the current top-level
function proof rule. Proof gates and frozen labels remain unchanged.

Recognized literal-title tests now receive distinct paths using suite ancestry,
encoded titles, and occurrence numbers. Nested definitions inherit those paths;
display names remain unchanged. This intentionally changes registration paths;
rebuild existing graphs and refresh stored paths. Reordering or inserting
identically named siblings can still rekey identities.

[Focused observation](observations/stage3-typescript-audit/identity-repair/focused-tests-1.json):
seven TypeScript builder tests passed, including the unchanged collision fixture,
a harder repeated-suite/sibling-title fixture, call ownership, and identity
stability under body edits, byte shifts, and unrelated registrations. This is
an intermediate repair, not language completion; the 100-site audit still has
an empty Must set and the structural-object-literal failure remains published.

Next: rebuild the CLI, measure the original collision fixture and the two real
repository files containing sites 23/91, then implement separately precommitted
binding proofs and structural target extraction. Full language gates and a
fresh 100-site after-observation remain required. Go remains NOT STARTED.


### 2026-10-02 — TypeScript 100-site before-observation published

Policy `cef8681`, reserve `9aa8e3d`, and source-reviewed labels `e1f911b` were
committed before measurement. The original 105 entries/97 actual calls remain
unchanged; a deterministic five-entry reserve prefix adds three calls and two
noncall declarations. All four original repository pins, rubric/addenda, and
scoring rules are retained. Acquisition failures and the verified archive
identities are published alongside the extension.

[Observation `f236e6f`](observations/stage3-typescript-audit/extension/before/observation.md):
100 actual calls, 10 noncalls retained, 53 exact / 47 conservative / 0 unsound,
zero failed relocations. **Every observed answer is Unknown; Must is empty and
precision is undefined. The language criterion is unmet.** All original 97
answers are unchanged. This sample contains no ground-truth May sites and
cannot measure May recall. Go remains NOT STARTED behind the TypeScript gate.

The full compiler analysis took 580.707 seconds; raw outputs, commands, hashes,
and timings are committed. Workloads ran offline and serially. Seventy-nine
selector/scorer/acquisition/acceptance tests passed. No product code changed,
so common Cargo/clippy/fmt/npm gates were not rerun for this before-observation.
They remain mandatory on the next language implementation checkpoint.

`ab44191` additionally fixed the audit precision numerator: a wrong-target Must
is no longer counted as a correct prediction merely because its expected class
is Must. It already failed the unsound-cell gate. Five focused tests passed;
recomputed Rust, Python, and TypeScript acceptance summaries are unchanged,
and no historical observation was edited.

Next: profile the TypeScript proof gates, freeze an implementation policy, and
repair the documented duplicate test-name identities and unresolved structural
object-literal origin without changing the corpus or audit labels. This is the
current resume point; the following checkpoints are historical.

### 2026-10-02 — Interrupted correction completed

The operator correction `c441899` passed all common gates once: 753 Cargo tests
passed (2 ignored), clippy/fmt clean, 29 npm tests passed (2 skipped). Gates and
binary identity were committed at `cfb6d7b`. The requested pause interrupted
Pydantic analysis; the partial run and completed Rust/Click extracts were
preserved at `b6ec8f7`. Resumption verified unchanged inputs and reused completed
extracts. No product code or frozen labels changed across the pause.

[Observation `a97b2ea`](observations/stage3-audit-reconciliation/after-observation.md):
Rust 100 actual sites, 61 exact / 39 conservative / 0 unsound; Python 100 actual
sites, 87 / 13 / 0. Both have Must precision 1/1 and 99 Unknown answers. The two
Rust operator omissions are fixed with local Unknown evidence. The earlier
failed observation remains published. Rust's post-collision re-score is complete.

The current binary's 49-case dispatch re-score has the same matrix as the last
published Python correction: 22 exact, 34 conservative, zero unsound, **one
existing failed TypeScript case** (`typescript-structural-object-literal`,
unresolved origin `name`). Rust/Python meet their current checkpoint; this does
not certify TypeScript or Go, nor retroactively validate the old audit sizes.

Next action: extend TypeScript's independent audit from 97 to at least 100 actual
sites under a frozen policy, then address its documented duplicate-node and
unresolved-origin cases. Stage 4 remains client-agnostic and NOT STARTED. The
following checkpoints are historical; this one is the current resume point.

### 2026-10-01 — Audit-size correction in progress

Rust/Python are IN PROGRESS until at least 100 actual sites per language have
frozen labels and passing current-candidate scores. Their original observations
are retained without modification. A new prospective extension will retain every
original site, add independently selected sites, and report non-call exclusions
separately. It cannot retroactively make the old insufficient audit complete.
Rust's current-binary re-score will also check the node-identity fix against the
unchanged historical cohort. No resolver changes are part of this correction.


### 2026-10-01 — Expanded audit observation

The [prospective extension](observations/stage3-audit-reconciliation/observation/summary.md)
now scores 100 actual sites per language, retaining every original site and label.
Rust failed: 59 exact, 39 conservative, **2 unsafe exclusions**, Must 1/1.
Python passed its current-candidate sample: 87 exact, 13 conservative, zero
unsound cells, Must 1/1. Python remains IN PROGRESS pending Rust's dependency
and the common gates. A single observed Must is a narrow sample, not general
proof of correctness.

The unchanged original cohorts retain their 28/24 and 73/12 exact/conservative
counts. Rust's post-collision re-score is complete: `SerializeSeq` and
`SerializeTuple` have distinct method paths and IDs. One original answer's
caller/reason changed, with its Unknown class unchanged. The old outstanding
correction-3 notes below are historical and superseded by this evidence.

Next: freeze a separate correction for Rust operator-site Unknown boundaries;
keep this failed observation and all labels unchanged. No resolver changed in
the extension campaign itself. Gates will run once on the correction candidate.

### 2026-10-01 — Current-state reconciliation

Read-only code/evidence review against `8a088f7` (114 commits after `af43418`):
working tree initially clean, local binary and package version `0.3.2`.
[CI for that exact HEAD](https://github.com/dhishwasher/Girder/actions/runs/36939256394)
is successful; the [v0.3.2 release](https://github.com/dhishwasher/Girder/releases/tag/v0.3.2)
is published and not a prerelease. No build, gate, or mutation measurement was
rerun during this review.

- Stage 1's published failure and Stage 2's completed 49-case corpus are retained.
- Rust/Python remain recorded DONE; this review does not independently recertify
  them. Their frozen audits contain 52 and 85 scored call sites respectively
  after non-call entries were excluded from the 105-entry samples. Reconcile
  those counts with the original minimum of 100 audited call sites; separately
  recorded supplementary checks must not be silently substituted for that gate.
- Rust's post-collision-fix `correction-3` re-score remains outstanding, as
  recorded in the addendum-12 checkpoint below.
- TypeScript remains IN PROGRESS: its before-observation scored 97 sites with
  zero proven Must claims. Duplicate `it()`/`describe()` node identities and
  corpus re-extraction/gate-profile work remain open. Go has not started.
- Stages 4-7 remain NOT STARTED; existing three-client setup support is not
  evidence that the Stage 4 bundle/adapters/skill criterion has been met.
- Next technical action: reconcile the audit-size criterion and refresh Rust's
  post-fix evidence before relying on prior DONE claims for further dispatch work.


- 2026-09-22: Stage 1 is complete and **FAILED-AND-PUBLISHED**: extractor call
  evidence, classified CLI/MCP answers (including the `--quiet` honesty fix),
  and the frozen mutation measurement are all implemented, run, and committed
  (`6b11d28`, `48a9713`, `d1706d6`, `c927122`, `f53f6dd`). A review pass
  after `c927122` found and fixed a second live overclaim (the MCP server's
  own `INSTRUCTIONS` string, plus CLAUDE.md/npm docs) and completed missing
  observation fields; `f53f6dd` is the corrected final candidate, with
  identical classification numbers to `c927122` (reproducibility confirmed).
  The trustworthy-Must gate (precision 1.000, nonempty set) was not met —
  Must is empty on every corpus measured; see
  [measurement-summary-final.json](observations/stage1/measurement-summary-final.json)
  for root causes. All four common gates pass on `f53f6dd`.
- 2026-09-22: Stage 2 is **DONE**: the 49-case dispatch corpus
  (`af82959`), its scorer (`e35c298`), and every scored result
  (`32bd5d2`) are committed. Zero unsound cells across all 57 test cells —
  Girder never overclaimed on this corpus. `must_precision_on_corpus` 1.000
  (3/3); `must_or_may_recall_on_corpus` 0.086 (3/35), the number Stage 3
  exists to move. One case unresolvable (TypeScript object-literal methods
  aren't indexed at all) and published as a failure, not tuned away. New
  finding beyond Stage 1: Rust's `assert_eq!`/`assert!` hide even a trivial
  same-file direct call from Must (the call is an opaque macro token, never
  its own node) — see
  [scoring-summary.json](observations/stage2-dispatch-corpus/scoring-summary.json).
  All four common gates pass on `32bd5d2`
  ([gates-32bd5d2/](observations/stage2-dispatch-corpus/gates-32bd5d2/)).
  Stages 3–7 remain NOT STARTED.
- 2026-09-22: Stage 3 (Rust) is **IN PROGRESS**, before-observation done
  (see below), resolver work not started. `25f7572` froze,
  before any site was read: "zero classification errors" = zero unsound
  audit cells (overclaim/unsafe_exclusion, matching the corpus's own scoring
  rule — an honest Unknown on an unclosable hole is conservative, not an
  error), and the audit methodology (regex-based site enumeration
  independent of Girder's own parser, fixed-seed stratified sample,
  ground-truth-before-Girder discipline). `858d549` committed
  `tools/dispatch_audit_site_selector.py` and its output,
  [audit-sites.json](observations/stage3-rust-audit/audit-sites.json): 105
  sites (≥100 minimum) across petgraph/serde_json/regex, stratified over 6
  syntactic shapes (~17-18 each). Confirmed `girder analyze --json` +
  `girder inspect --json` already exposes per-site `call_evidence_v1`
  (class/reason/coverage_gap/targets) — no new read command needed.
- 2026-09-22: Rust audit labeling + scoring done (`f3e9f56`), then
  corrected (`361dd08` scorer tool committed properly with tests,
  `c601a5e` fixes four real errors: 8 mislabeled external-target sites, a
  row-proximity matching bug, 3 sites wrongly filed as a neutral status
  instead of being properly investigated, and a wrong "one reason string
  explains everything" root-cause claim). Corrected, verified result: 52
  scored, 0 unsound (0 overclaim, 0 unsafe_exclusion), 25 conservative —
  23/25 (92%) trace to the identifier-only Must-proof filter
  (`mapper/claims.rs:122`), 2/25 (8%) to the already-documented same-file/
  top-level-only limitation via imports. See
  [audit-correction.md](observations/stage3-rust-audit/audit-correction.md).
- MOVESPEED is available: approximately 291 GB free; system reports approximately
  2 GB available RAM and no swap. Cargo and rustc are available from its cache.
- Stage 4 requires Claude Code, Cursor, and Codex adapters plus raw MCP JSON fallback.
- 2026-09-22: First Stage 3 Rust resolver change committed and measured.
  `47c3a06` proves bare-identifier calls inside trusted, unshadowed
  `assert!`/`assert_eq!`/`assert_ne!`/`debug_assert*` macros (walking the
  macro's token-tree leaves directly; fails closed on shadowing, blocks,
  closures, nested macros, and any other untrusted macro in the same
  function — 13 unit tests, including an earlier, over-broad version of the
  fix caught and reverted in review before commit). `3f5ed5e` measured it:
  dispatch corpus improved by exactly the predicted single cell
  (`rust-direct-same-file` conservative → exact, probe-verified against the
  real fixture; pooled 20/36 → 21/35), zero classification errors held (0
  unsound, oracle 1.0/1.0 unchanged) — but the 105-site real-repository
  audit is byte-identical before and after (27 exact / 25 conservative).
  Root-caused: the audit sample has almost no genuinely eligible sites for
  this proof category (most textual matches are doctest comments or
  cross-file calls), and the one real candidate is blocked by the same
  whole-file `transformed_scope` `#[cfg]` gate that independently blocks
  most of the method/path-call misses too. Full detail, including two
  disclosed-but-unfixed properties (crate-wide macro shadowing — checked
  absent from both corpora; control-flow insensitivity — consistent with
  the policy's binding-certainty definition of Must, not a new gap), in
  [after-observation.md](observations/stage3-rust-audit/after-assert-macro-fix/after-observation.md).
  Two of three criterion legs met; audit-improvement and real-repository
  nonempty-Must-precision legs not met. Stage 3 (Rust) stays **IN
  PROGRESS** — not DONE (criterion unmet), not FAILED (next step
  identified, not exhausted).
- Corrected (see
  [correction.md](observations/stage3-rust-audit/after-assert-macro-fix/correction.md)):
  the previous version of this entry proposed narrowing `transformed_scope`
  as the next step, reasoning from a single supplementary candidate
  (`serde_json`'s `math.rs:632`) that turned out to be double-blocked —
  it also sits inside `mod large { }`, so the pre-existing `top_level`
  check would reject it independently, narrowed `transformed_scope` or not.
  Checked directly against the frozen 25 conservative audit sites instead:
  23 are method/path-shaped (blocked by the identifier-only filter,
  regardless of `transformed_scope`) and 2 are bare-identifier calls to an
  imported (cross-file) target (blocked by the same-file-only restriction
  on `proven`, regardless of `transformed_scope`). No path through the 25
  frozen sites is solely blocked by `transformed_scope`, so narrowing it
  first cannot move the frozen audit either.
- 2026-09-22: Per-site gate profile done (`7dde679`), covering all 25
  frozen conservative sites against every extractor gate, read from
  Girder's own evidence (no build needed). Finding: `transformed_scope`
  moves zero of the 25 regardless of how it's narrowed — it was never
  the primary blocker for any of them (23 blocked by the identifier-only
  filter, which fires first; 2 blocked by the same-file-only restriction
  and independently also carry real `#[cfg(feature = ...)]`). Checked
  the policy's Rust row first ("exact concrete dispatch" is explicitly
  in scope for Must), then hand-verified 5 method/path candidates that
  clear every other gate: exactly one, `tests/floyd_warshall.rs:11`
  (`graph.add_node(())`, `Graph<(), (), Directed>` explicitly annotated,
  `add_node` has exactly one inherent impl, and `petgraph::data::Build`'s
  same-named trait method is confirmed moot since Rust always prefers an
  inherent method on the exact type), survives a no-inference design
  (explicit receiver-type annotation, crate-wide single inherent impl,
  inherent-beats-trait). Guard-checked against all 7 method/path-shaped
  sites among the 27 currently-exact cells — none would become a false
  Must. Full design spec, with an explicit before-implementing
  prediction (moves exactly one of the 52 scored sites), in
  [gate-profile.md](observations/stage3-rust-audit/after-assert-macro-fix/gate-profile.md).
- 2026-09-23: **Correction to constraint 4** (`gate-profile.md`'s
  "inherent-over-trait resolution encoded directly (a fixed Rust rule)"
  was wrong as a general rule — Rust matches candidate receiver types in
  order (`T`, `&T`, `&mut T`, then the deref chain), and inherent only
  beats trait *within the same step*; a trait method matching an
  *earlier* step wins outright). `tests/floyd_warshall.rs:11` still
  survives — verified, not assumed: every `add_node` in petgraph
  (eleven definitions, grepped) is `&mut self`, including
  `Build::add_node`, so both the inherent method and the only in-crate
  trait competitor sit at the identical step. Full correction, and the
  additional guards a sound implementation needs (named-import-only type
  resolution via the actual `use`-tree AST, exact `T<...>` receiver
  syntax with no `&`/`Box`/`dyn` wrapping, single-binding-with-shadow
  counting, generic-bounds coherence, replacing rather than appending
  the existing Unknown claim, and a call-expression-start span matching
  where the scorer's byte offset actually lands), in
  [gate-profile-correction.md](observations/stage3-rust-audit/after-assert-macro-fix/gate-profile-correction.md).
  The prediction is unchanged (moves exactly `tests/floyd_warshall.rs:11`).
- 2026-09-23: **Second correction**, before any implementation code
  (`gate-profile-correction-2.md`): three gaps the first correction left
  unaddressed (impl-applicability/bounds checking for the same-step rule;
  target-side `cfg`-gating on the struct/impl/module, entirely missing
  from the original spec; external glob imports, not just "non-glob,
  non-prelude"), the type-resolution choice pinned in writing (bare-name
  uniqueness across the indexed crate, not full semantic-path/re-export
  resolution — a deliberate simplification, not an oversight), and two
  factual slips (12 `fn add_node(` definitions in petgraph, not 11;
  `adj.rs:205` is a different method, `add_node_from_edges`, not a
  `&self`-shaped `add_node`). All checked directly against the whole
  checkout (not just `src/`): `tests/floyd_warshall.rs:11` still
  survives every gap — the inherent and `Build for Graph` impls have
  identical bounds, no blanket trait impl or `Deref` impl for `Graph`
  exists anywhere, nothing on the path from the struct/impl to the crate
  root carries `cfg`, and `prelude::*`'s own `pub use` lines are all
  in-crate. Also notes a scorer blind spot for measurement time:
  `dispatch_audit_scorer.py` compares claim class only, never `targets`,
  so the after-observation and the implementation's own tests must
  explicitly assert the produced claim's target is
  `graph_impl/mod.rs:525`'s inherent `add_node`, not `Build::add_node` or
  another type's `add_node`.
- Next: implement the corrected design — method-call Must proof
  restricted to (1) an explicit, unshadowed `let x: T<...>` receiver
  binding in the same function (no type inference, exact `T<...>` syntax
  only), (2) `T` resolved through a **named, non-glob** import (or
  in-file definition) whose first path segment is `crate`/`self`/`super`
  or the crate's own package/`[lib]` name, walked via the real `use`-tree
  AST, to the *bare name* being the only type-namespace item
  (struct/enum/union/trait/type-alias) of that name anywhere in the
  indexed crate, with nothing in the crate `use`ing or `pub use`ing an
  external item of the same bare name (bare-name uniqueness, a deliberate
  simplification of full re-export resolution — see the second
  correction), (3) exactly one inherent method of that name across all
  `impl T` blocks crate-wide, generic over all of `T`'s own parameters
  (Unknown if a `duplicate-semantic-path` gap touches either), (4) for
  any in-crate trait declaring that method name: its bounds on any impl
  covering `T` must be a superset of the inherent impl's bounds (so
  inherent-over-trait only applies when both actually apply), no blanket
  impl of that trait over a bare type parameter may exist, no in-crate
  `Deref`/`DerefMut` impl may exist for `T`, and the method name must not
  be in the std prelude's fixed method-name set; (5) neither the struct,
  the inherent impl, any competing trait impl, nor any enclosing module
  declaration from the crate root down may carry `cfg`/`cfg_attr`; (6)
  any glob import in scope (including the crate's own "prelude", no
  exception by name) must resolve entirely within the indexed crate,
  checked recursively through any module it points at. Lives in or after
  `resolve_calls` (project-wide), since it needs a crate-wide inherent-
  impl index — `annotate()`'s per-file pass can't build this alone;
  persist the per-file gates (`transformed_scope`, `duplicate_paths`,
  parse-error, `macro_owners`) on `BuildOutput` rather than re-deriving
  them, and recompute on every resolve (including the incremental path)
  so a change elsewhere in the crate can flip an untouched file's claim.
  Stage as two pieces landed together once all four gates pass: (A) the
  crate-wide index itself (type-namespace items, inherent impls with
  normalized bounds, trait method declarations with receiver kind, trait
  impls with bounds, `Deref` impls, `cfg` marks), with its own unit tests
  verified to reproduce every fact established in both correction
  documents against the real petgraph checkout before moving on; (B) the
  post-`resolve_calls` pass that replaces (never appends alongside) the
  existing Unknown claim with the Must claim, span starting at the call
  expression (not the method name, to match where the scorer's byte
  offset lands). Guard with: adversarial unit tests across a multi-file
  `GraphBuilder` (the positive cross-file case; an external crate's
  same-named type; an in-crate `&self` trait method beside a `&mut self`
  inherent one at an earlier step; a same-step trait method whose impl
  bounds are narrower than the inherent impl's; a blanket trait impl; a
  `cfg`-gated inherent impl; an external glob import; two non-generic
  inherent methods of the same name; a `&mut T`/`Box<T>` receiver; `x`
  shadowed in a nested block; an unannotated `let`; the incremental
  staleness flip; and asserting the produced claim's `targets` NodeId is
  the correct inherent method, not a same-named trait or sibling-type
  method), plus the 7 guard-checked std/external/non-unique cases from
  `gate-profile.md`; the dispatch corpus must stay 0 unsound; the Stage 1
  oracle's union recall must stay 4/4; the audit must show 0 unsound and,
  this time, actually fewer conservative / more exact cells (the
  prediction is exactly one: `tests/floyd_warshall.rs:11`, 28 exact / 24
  conservative and a nonempty real-repository Must set, with `targets`
  pointing at `graph_impl/mod.rs:525`) — if the implementation produces a
  different count than predicted, or the wrong target, that is itself a
  signal to stop and re-examine before trusting the result.
  After implementing, hand-verify a random sample of every new Must
  claim across all three crates (it will fire beyond the 52 audited
  sites) and publish that as a supplementary check, kept out of the
  frozen audit. If constraint 4 turns out unsound in practice even after
  this correction, or the predicted site doesn't survive implementation
  and no other frozen site does either, that is the point to apply
  FAILED-AND-PUBLISHED once and stop — do not start Python before Rust
  is trustworthy on a real repository.
- 2026-09-23: **Implementation-plan addendum**, found before writing code
  (banked here first so it survives a session boundary mid-implementation):
  - **Architecture: extract facts in `claims.rs`, don't re-parse in
    `resolve_calls`.** `annotate()` already has the tree and already
    computes `transformed_scope`/`duplicate_paths`/parse-error/
    `macro_owners`; a second, `resolve_calls`-side re-parse would be a
    second implementation of those same gates that could drift from the
    first, which is itself a false-Must path. Record per-file facts on a
    new `BuildOutput` field instead: type-namespace item names; impl
    blocks (inherent vs. trait via the `trait` field, type name/params,
    normalized bounds, cfg marks, and per method: name, receiver kind,
    visibility, span-matched NodeId); trait declarations (every method's
    receiver kind, including bodyless `function_signature_item`
    declarations, not just `function_item` — a scan over `function_item`
    alone would miss `Build::add_node` itself); `Deref`/`DerefMut` impls;
    `mod` declarations with their cfg and `#[path]` attributes; use-tree
    imports (named vs. glob, first segment, introduced name, `as`
    aliases); candidate method calls (call span, receiver identifier,
    method name, the qualifying same-function `let` if exactly one, and
    the owner's gates). In `resolve_calls`: build the crate index from
    every file's stored facts, then restore each Rust node's
    `call_evidence_v1` from its file's cached extraction before writing
    upgrades — mirror the existing route-evidence reset-then-rebuild
    pattern already in `resolve_calls`. This gives incremental staleness
    handling for free. Read `aether-graph/src/claims.rs` first (decode/
    attach/fingerprinting) and check whether `apply()`/`UpdateReport`
    diffs node attributes, since a re-encoded record rejected as stale
    would silently surface as Unknown and kill the prediction.
  - **Narrow only what's proven, never the competitor scan.** Restricting
    *provable receivers* to structs is fine; the uniqueness count, trait
    scan, and `Deref` scan must still cover every struct/enum/union/
    trait/type-alias, or a narrowed competitor set creates a false-Must
    path. Receiver steps order as `self`/`mut self`, then `&self`, then
    `&mut self`; a competitor with a typed self (`self: Box<Self>`,
    `Pin<...>`) gives Unknown, and the inherent method must not be proven
    if it itself has a typed self. Add a visibility guard: the inherent
    method must be plain `pub` (rustc's method probe skips an
    inaccessible inherent candidate and keeps probing, which could let a
    private inherent method lose to an accessible trait method at the
    same step).
  - **cfg chain: fail closed unless every link is positively confirmed** —
    the item's own attributes, enclosing inline `mod` items in the same
    file, `#![cfg]` at the top of that file, and every ancestor `mod X;`
    declaration (at least one must be found for each ancestor segment,
    and every declaration found with that name must itself be cfg-free).
    Any `#[path]` on a `mod` declaration, or a target unreachable from
    `src/lib.rs` through ordinary `mod` declarations, gives Unknown.
  - **Four facts to confirm before coding, all required for the
    prediction to hold, none yet confirmed:** (a) duplicate-path checking
    must be node-level (the inherent method's own path not duplicated
    within its file's extraction, matched by span like `annotate()`
    already does), not file-level like the existing `duplicate_paths`
    flag — `gate-profile.json` already shows `data.rs` (a different file)
    as `dup=Y`, which must not disqualify `graph_impl/mod.rs`; (b) the
    crate's package name (from `Cargo.toml`, `-` mapped to `_`, `[lib]
    name` override respected) must count as in-crate, the way
    `go_module_path` already does for Go — check whether `girder analyze`
    already reads `Cargo.toml` for anything; (c) external named imports in
    the caller (`floyd_warshall.rs` imports `std::collections::HashMap`)
    must be treated as known non-trait items via an explicit allowlist, or
    the site fails closed on the caller's own imports; (d) glob checking
    must confirm, flatly: the target module has no glob `pub use` of its
    own, no external-rooted `use` anywhere in the index introduces a name
    it re-exports (as either the last segment or an `as` alias), and no
    external-rooted `pub use ext::*` exists anywhere in the index — apply
    this to the caller's named in-crate imports too
    (`Directed`/`Graph`/`Undirected`/`floyd_warshall`), not just its glob.
  - **Grammar fields to verify in `grammar.js` before relying on them**
    (the way `impl_item`'s `trait` field was verified before use):
    `let_declaration`, `mutable_specifier`, `generic_type`,
    `field_expression`, `self_parameter` (and how a typed self parses),
    `where_clause`, `type_parameters`, `mod_item`, `scoped_use_list`,
    `use_as_clause`, and the wildcard/glob node kind.
  - **Binding rule:** the receiver must be a bare `identifier`; its `let`
    must be in the same owning function, in a block containing the call,
    and textually before the call; `x` may have exactly one binding
    occurrence in the function, counting `let` patterns, fn/closure
    params, and `for`/`match`/`if let`/`while let` patterns — any
    occurrence this can't classify makes the result Unknown.
  - **If a soundness guard above blocks `floyd_warshall.rs:11` once
    implemented, that is the FAILED-AND-PUBLISHED trigger — don't loosen
    the guard to pass it.** If only plumbing (e.g. package-name reading)
    is missing, build the plumbing; a missing-plumbing gap is not itself
    a failure.
  - **Sequencing:** write the positive test and the full adversarial list
    (both the `crate::`-from-`src/` and package-name-from-`tests/` import
    forms) before implementing, since each compile on this machine takes
    minutes. Run the real-petgraph verification (reproducing every fact
    established in the two correction documents) as an `#[ignore]`d test
    reading the checkout path from an env var, kept out of the four
    common gates.
- 2026-09-23: **Stage 3 (Rust): DONE.** Implemented in `d0ce300`, then
  corrected four more times before trusting a "DONE" measurement:
  `c436dbd` wired in guards the first commit collected but never used;
  `62141a9` closed seven more soundness gaps (that commit's own message
  miscounts them as six) found by a review specifically hunting for
  realistic false-Must paths, then found and fixed two implementation
  bugs its own fixes introduced (an over-broad "externally aliased"
  check, and `in_crate` not recognizing a bare sibling-`mod` re-export);
  `2452bc1` found and fixed two further genuine unsoundnesses via two
  separate `advisor` reviews of the "DONE" draft, each catching a real
  false-Must path the immediately prior round's own fix had introduced or
  missed (widening `in_crate` to crate-wide mod recognition, then to
  file-scoped, both unsound — the correct rule is module-scoped, via a
  new `scope` field on `ModDeclFact`/`ImportFact`), plus a corrected claim
  about the evidence-reset loop's necessity (load-bearing on
  `load_file`/`load_files`, not on `update_files`, as an earlier draft of
  this checkpoint would have said). The module-scope rule, the
  destructured-pattern check, and three previously-vacuous tests were
  each verified by mutation (revert the fix, confirm the relevant test
  fails; restore, confirm it passes) — not blanket "every fix", corrected
  below.
  Measured: real-repository audit 27/25 → **28/24 exact/conservative, 0
  unsound**, the single predicted site
  (`tests/floyd_warshall.rs:11`, target verified as `Graph::add_node`,
  diffed programmatically against the prior result); dispatch corpus and
  Stage 1 oracle unchanged; 49 supplementary Must claims across real
  petgraph, all resolving to the three correct targets, 0 in
  serde_json/regex. All four common gates pass. All four Stage 3 Rust
  criterion legs met. Full history and every mutation-test result in
  [after-observation.md](observations/stage3-rust-audit/after-method-call-fix/after-observation.md)
  and
  [supplementary-hand-verification.md](observations/stage3-rust-audit/after-method-call-fix/supplementary-hand-verification.md).
- 2026-09-23: **Correction to the above** (`facc21c`,
  [correction-1/correction.md](observations/stage3-rust-audit/after-method-call-fix/correction-1/correction.md)):
  a THIRD `advisor` review, of the just-committed "DONE" state itself,
  found `enclosing_mod_scope` stopped only at `mod_item`, not at `block` —
  a `mod` declared inside a function body computed the same scope (0) as
  the file's own top-level module, reproducing a false Must live
  (`use quickcheck::Gen;` at module level, `mod quickcheck {}` inside a
  same-file function body). Fixed by also stopping at `block`; verified by
  mutation with a new permanent regression test. Also corrected: the prior
  entry's own test-count claim ("12 new tests (49 total, up from 37)" —
  the 49 was a copy-confusion with the unrelated supplementary-claims
  figure; the real, directly-counted numbers are 28 → 40, 12 new, which
  matches `62141a9`'s own "11 new tests" being a miscount of 12 as well);
  the "every fix verified by mutation" overclaim (the glob-scope narrowing
  and prelude-list additions have no dedicated mutation test); and a
  silently-dropped plan item (the implementation-plan addendum's
  `#[ignore]`d real-petgraph verification test was never written — the
  supplementary hand-verification documents serve the same purpose by a
  different mechanism, but that specific artifact doesn't exist).
  **Re-measured: all numbers held identical** (28/24 audit, 0 unsound;
  corpus/oracle unchanged; 49 supplementary claims, same split). All four
  common gates and the build pass, this time chained sequentially in one
  background job rather than run as two concurrent `cargo` invocations (an
  earlier round in this same session violated this repository's "never a
  second cargo job running concurrently" rule; cargo's own build-lock
  serialized it safely, but the discipline itself was violated and is
  called out so it isn't repeated). **Stage 3 (Rust) remains DONE** — this
  is the third round (of six total across this resolver's whole lifetime)
  where a review found a genuine unsoundness in the immediately prior
  round's own fix, not just a test gap; none of the three changed any
  measured number on the audited sample, which is a property of this
  specific sample, not a guarantee the rule is now exhaustively correct.
  Per the language-order rule ("do not start Python before Rust is
  trustworthy on a real repository"), **Stage 3 Python may now begin** —
  next session should start there: read the Python row in
  `call-classification-policy.md` first, find or pin a Python
  real-repository corpus (roadmap line 275 says "Click", so check
  `docs/core-representative-corpus.json`/`.benchmark-cache/` for an
  existing pin before fetching anything new), freeze its own audit
  methodology (fixed seed, ≥100 sites, Python-appropriate shapes: plain
  call, method call, qualified-attribute call, decorator, dynamic dispatch
  via `getattr`/callable, operator/dunder) in its own roadmap commit
  before reading any site, and expect the same discipline this Rust round
  needed (measure honestly, publish FAILED-AND-PUBLISHED if a criterion
  leg isn't met, never weaken the frozen policy to pass, and don't assume
  a design is sound just because it compiles and its own tests pass).
- 2026-09-23: **Second correction** (`777c2a7`,
  [correction-2/correction.md](observations/stage3-rust-audit/after-method-call-fix/correction-2/correction.md)):
  a FOURTH `advisor` review, re-checking `correction-1`'s own
  "under-recognize only" claim specifically, found a THIRD distinct
  collision source in `enclosing_mod_scope`: it returned `0` both for "no
  enclosing container found" (true file-root scope) and for a real
  container whose own `start_byte()` happens to be `0` (a `mod`/`block`
  that is the very first thing in a file) — reproduced live with `mod m {
  use quickcheck::Gen; ... }` as a file's first bytes, colliding with a
  sibling top-level `mod quickcheck {}`. Fixed by using `u64::MAX` as the
  "no container" sentinel instead of `0`; verified by mutation. Also
  started this same session: Stage 3 Python's site selector
  (`tools/dispatch_audit_site_selector_python.py`), NOT yet committed —
  an `advisor` review of the staged files, before commit, flagged that the
  Python labeling rubric must be frozen (tied to the Python policy row's
  assumptions: self/cls dispatch under the source-snapshot assumption,
  rebinding definition, annotation-only receivers, decorator effects,
  `super()`, targets outside the indexed package) and a docstring/example
  masking check run (via `tokenize`, on the whole discovered pool, not the
  sample) BEFORE any site is read — none of that is done yet, so no
  Python site has been read and no Python commit exists. **Re-measured:
  all numbers held identical** to `correction-1` (28/24 audit, 0 unsound;
  corpus/oracle unchanged; 49 supplementary claims, same split). All four
  common gates and the build pass. **Stage 3 (Rust) remains DONE** — the
  fourth round (of seven total) where a review found a genuine
  unsoundness in an immediately prior round's own fix or safety-direction
  claim, all four in the same function (`enclosing_mod_scope`/`in_crate`).
  That function should be treated with particular suspicion in any future
  change, not assumed sound because it compiles.
- 2026-09-23: **Stage 3 Python audit methodology frozen, before any site
  read.** All five items the prior checkpoint entry required are done:
  (1) [labeling-rubric.md](observations/stage3-python-audit/labeling-rubric.md)
  -- self/cls dispatch (Must only with no in-snapshot override anywhere in
  the hierarchy, else May with a bounded candidate set, else Unknown),
  rebinding (defined once, generously toward disqualifying Must),
  annotation-only receivers (never Must, per the policy's own text), direct
  construction (`__new__`/metaclass interactions checked, not assumed
  absent), decorated names (Unknown unless the decorator's effect is
  traced), `super()` (always May, per the policy's explicit "super/MRO
  targets"), targets outside the snapshot (labeled `unknown` with an
  external-target note, the exact mislabeling Rust's own
  `audit-correction.md` had to fix after the fact -- fixed here in
  advance), decorator lines as call sites (yes, `@property` is a real
  call), and `not_a_call_site` criteria. (2) Whole-pool (not sample)
  `tokenize`-based check found **33% of the 49,953 initially-matched
  lines fell inside a string/comment token** (Click/pydantic docstrings
  are full of code examples) -- fixed by masking STRING/COMMENT spans
  before classification (`mask_strings_and_comments`, 5 new unit tests
  including a regression test for a real bug the masking logic itself had
  — an early version dropped the newline on a multi-line token's first
  line, shifting every subsequent line number), not just disclosed;
  `tokenize_failure_count: 0` across all three packages. (3) File set:
  `docs/`/`examples/` are negligible (checked directly: one `conf.py`
  across all three packages, not application code) so no exclusion rule
  was needed; `tests/` IS included, matching Rust's own precedent where
  the frozen audit site itself was a test file. (4) Confirmed on a small
  throwaway fixture (never on click/pydantic/requests before labeling):
  Python nodes expose the identical `call_evidence_v1` structure Rust's
  scorer already reads byte-precisely, so no scorer rewrite is needed, only
  a Python-specific selector and labeled-sites file. (5) This commit
  (rubric + methodology +roadmap) lands first; the selector tool, its
  tests, and the regenerated 105-site sample land as a separate, second
  commit — per the precommitment order. Full detail, including the
  disclosed (not corrected) package-imbalance in stratify-by-shape-only
  sampling (pydantic 81/105 selected, click 19, requests 5 — proportional
  to each package's own size, not rebalanced, to keep the algorithm
  identical to Rust's own precedent), in
  [methodology.md](observations/stage3-python-audit/methodology.md).
- 2026-09-23: **Stage 3 Python: all 105 sites hand-labeled
  ([audit-sites-labeled.json](observations/stage3-python-audit/audit-sites-labeled.json)),
  verified before scoring
  ([labeling-verification.md](observations/stage3-python-audit/labeling-verification.md)),
  and the first before-observation measured
  ([before-observation.md](observations/stage3-python-audit/before-observation.md)).**
  Labeling done by a forked agent applying `labeling-rubric.md`; an
  `advisor` review before trusting it found and this session fixed: a
  scorer bug (`site_byte_offset` searched the raw line, not the masked
  one the selector used to choose sites — fixed, verified to have
  affected 0 of the 105 real sites though the bug was real and general);
  one rationale that leaked this session's own earlier Girder smoke-test
  output into its stated reasoning (rewritten on a rubric-only basis, the
  label itself was already correct and unchanged); confirmed via direct
  grep that none of the 13 Must sites' names are touched by
  `setattr`/`monkeypatch`/`patch` anywhere in the snapshot; confirmed all
  10 `operator_dunder` sites compare primitive values, never an
  in-snapshot class's own dunder. **Result: 85 scored, 70 exact, 13
  conservative, 2 unsafe_exclusion, 0 overclaim.** The 2 unsafe_exclusion
  sites are root-caused, not guessed: two bare comparison expressions
  inside `assert` statements get NO `CallClaim` at all from Girder's
  Python extractor — confirmed by reading the enclosing function's full
  `call_evidence_v1` directly. All 13 true-Must sites score conservative
  (Girder's Python extractor proves Must only for same-file, non-dispatch
  bindings; every real Must site needed cross-file import resolution or a
  class-hierarchy override check, neither attempted yet) — independently
  confirmed against the existing 12-case Python dispatch corpus from
  Stage 2 (`fixtures/dispatch-corpus/python/`, re-run here for the first
  time as a Stage 3 baseline:
  [corpus-baseline.json](observations/stage3-python-audit/corpus-baseline.json),
  6 exact / 7 conservative / 0 unsound, `must_true_positives: 1`, same
  finding). 0 May observed in this specific sample, disclosed as a sample
  property, not assumed to generalize. **Stage 3 Python criterion status,
  stated plainly, not softened**:
  `measured_dispatch_corpus_improvement` not applicable yet (no resolver
  change), `nonempty_must_precision_1000_on_real_repository` not met
  (empty Must set), `zero_classification_errors_on_audit` **NOT met**
  (2/85 unsound) — unlike Rust's own before-observation, which happened
  to already show 0 unsound cells, Python's does not, and that is real
  information, not smoothed over. Next step (not started): implement a
  Python resolver change closing the 2 unsafe_exclusion sites (emit at
  least an Unknown `CallClaim` for comparison/binary-expression call
  sites) and/or attempting cross-file import resolution and
  class-hierarchy override checking for Must proofs — the two concrete,
  root-caused gaps this before-observation found. Re-verify any change
  against both baselines measured here (this audit and the existing
  Python dispatch corpus), the same discipline every Rust resolver round
  in this program used, including the mutation-verification and
  advisor-review-before-trusting-DONE discipline the Rust rounds needed
  repeatedly.
- 2026-09-23: **Stage 3 Python first resolver change: per-site
  operator-dispatch claim (`d4d8d8f`), measured
  (`b7ad01a`).** Closed both unsafe_exclusion cells the before-observation
  found (`tests/test_construction.py:37`, `tests/test_arguments.py:284`)
  by emitting a per-node Unknown claim for
  `binary_operator`/`comparison_operator`/`unary_operator`/`not_operator`
  nodes — the same pattern already used for macros/decorators, gated to
  `lang == Lang::Python`. An `advisor` review before trusting this found
  the before-observation's own "8 of 10 operator sites covered only
  coincidentally" claim would very likely flip once this landed (spans
  overlap: the new per-site claim is narrower than the
  `duplicate-semantic-path` claim it competes with) — checked, confirmed
  true: all 10 operator sites now carry real per-site evidence, not just
  the 2 predicted. **Result: 85 scored, 72 exact, 13 conservative, 0
  unsound** (was 70/13/2/0). Corpus and Stage 1 oracle precision/recall
  unchanged; oracle `boundary_count` impact (+2, one real `is None` in the
  oracle's own fixture) traced and disclosed, not assumed away. Rust's own
  audit re-checked empirically (shared `claims.rs` code) — unchanged,
  28/24/0-unsound. `zero_classification_errors_on_audit` is now **Met for
  this 105-site sample** — explicitly not a claim that every
  implicit-dispatch Python construct is covered:
  `augmented_assignment`/subscripts/`for`/`with`/attribute access still
  get no per-site claim, disclosed in
  [after-operator-claim-fix/after-observation.md](observations/stage3-python-audit/after-operator-claim-fix/after-observation.md).
  `measured_dispatch_corpus_improvement` is **not met by this change**
  (stated plainly, not softened to "not applicable" — a resolver change
  was made and the existing corpus's cells didn't move, because its
  fixtures don't construct this fix's target shape).
  `nonempty_must_precision_1000_on_real_repository` still not met (0/13
  Must proven, unaffected by design). Missing items an `advisor` review
  found in the prior `before-observation.md` entry (per-package
  breakdown, the smoke-test-site sensitivity check, the `not_a_call_site`
  cause tally, the Python 3.11.2 masking dependency) are now in
  [before-observation-addendum.md](observations/stage3-python-audit/before-observation-addendum.md),
  added rather than editing the already-committed original. **Stage 3
  Python remains IN PROGRESS, not DONE.** Next step, per the same
  `advisor` review: **gate-profile the 13 conservative audit sites and 7
  conservative corpus cells against every existing extractor gate**
  (mirroring `dispatch_audit_gate_profile.py`'s role for Rust) *before*
  designing a Must-proof rule for cross-file/class-hierarchy dispatch —
  `transformed_scope` (set by any decorator anywhere in a Python file)
  may already block most of the 13 regardless of what a new rule proves,
  and finding that out before writing code is exactly what the Rust
  gate-profile step (`7dde679`) did before its own method-call resolver
  design. Plan mutation tests for any new scope/rebinding logic from the
  start — Rust's own module-scope logic needed four separate corrections
  because this wasn't done early enough there.
- 2026-09-24: **Stage 3 Python: DONE.** Gate-profiled the 13 conservative
  audit sites first (`c68587d`): `transformed_scope` (any decorator
  anywhere in a Python file blocking the whole file's Must proofs) was
  the sole blocker for 1 of 13, a co-blocker for most others. Designed a
  narrowed rule (`38e3d1d`, written after the measurement it "predicted"
  — file mtimes on the committed measurement artifacts predate this
  commit; see correction-3), implemented it
  (`8acde01`): removed `transformed_scope` from gating Python's proven-map
  computation (the pre-existing `top_level`/`clean` checks already
  excluded decorated targets and shadowing), added a same-file and
  project-wide string-literal rebinding guard after finding a real
  cross-file case (`mocker.patch(...)` naming a same-file-called
  function) — not just a theoretical one. Result: **85 scored, 73 exact,
  12 conservative, 0 unsound** (`9510863`), all three criterion legs
  (`measured_dispatch_corpus_improvement`, `nonempty_must_precision_
  1000_on_real_repository`, `zero_classification_errors_on_audit`) Met
  for the first time.
  A three-round `advisor`-driven correction chain followed before trusting
  that milestone, each round committed separately rather than editing a
  prior one:
  [correction-1](observations/stage3-python-audit/after-transformed-scope-fix/correction-1/correction.md)
  found and fixed a real cross-language soundness bug (`cccbef3`): the
  project-wide rebinding-revert pass had no language gate, so an
  unrelated Python string anywhere in a mixed-language project could
  wrongly revert a Rust or TypeScript same-file Must claim — quantified
  directly against this repository itself (6 wrong reverts before, 0
  after). Also closed a real attribute-assignment/`del`/`for`/`with`
  rebinding gap the string-literal-only guards had missed (`9160402`,
  widened in `86fd130` after the first implementation only matched the
  simplest shape).
  [correction-2](observations/stage3-python-audit/after-transformed-scope-fix/correction-2/correction.md)
  found and fixed a hex-padding bug in correction-1's own verification
  scripts (silently dropping ~1/16 of resolved Must targets from every
  downstream check) and re-ran both soundness checks clean on the
  corrected, complete population; re-ran the Rust audit against the
  final binary instead of only asserting it unaffected (28/24/0,
  matching precommitment, since none of the three audited Rust crates
  contain any `.py` file); named the corpus's one pre-existing
  non-Python failed cell.
  [correction-3](observations/stage3-python-audit/after-transformed-scope-fix/correction-3/correction.md)
  found that `design-and-prediction.md`'s prediction, while genuinely
  committed before the code commit, was written with the actual
  measurement numbers already on disk (file mtimes predate the
  prediction's own commit) — not a blind prediction; recorded that the
  stage's first genuinely blind, in-session-verified prediction is
  `correction-1/prediction.md` (`f811daa`). Completed a hand-read
  call-site sample that two prior rounds had each claimed was complete
  while actually being partial (regenerated programmatically: 20/20
  original-sample sites plus 8/8 previously-unresolved-target sites).
  Ran the two checks required before declaring DONE: ground-truth labels
  committed ~12 minutes before the first audit scoring (frozen before
  any comparison — confirmed, not assumed), and no code has changed
  since the last gated commit (`86fd130`, `correction-1/gates.log`,
  `ALL_GATES_PASSED`) — `git diff --stat 86fd130 HEAD -- .
  ':(exclude)docs'` is empty. Both pass.
  Final measured state, binary `f87d1d058bb91418e817af35efb4956096a34e486ea39d7346a17477bb50d96f`
  (commit `86fd130`): real-repository audit 73 exact / 12 conservative /
  0 unsound; dispatch corpus 56 of 57 cells scored (pooled 22 exact / 34
  conservative / 0 unsound) plus 1 pre-existing failed cell
  (`typescript-structural-object-literal`, origin symbol unresolved —
  confirmed unchanged since `after-operator-claim-fix`, not Python, the
  first item to resolve when TypeScript's own Stage 3 work starts), Rust/
  TypeScript/Go per-language cells unchanged from their pre-round
  baseline (5/9, 5/10, 5/9), Python 6/7 → 7/6; Stage 1 oracle
  precision/recall 1.0/1.0 for both Rust and Python. Disclosed,
  not-yet-closed limitations carried forward:
  class construction never proven Must; method-call/qualified-attribute-
  call dispatch and cross-file import resolution unimplemented; May
  never emitted for Python at all; `patch.multiple`/`__dict__.update`
  keyword-argument rebinding and `setattr`/`globals()[...]` with a
  non-literal name are unguarded (checked empty against the current
  three-package snapshot, not closed); the same-file
  `python_string_literals` guard treats any `__all__ = (...)` export
  tuple as a rebinding risk for every name it lists, a likely-systematic
  source of over-conservatism across any Python file using that common
  idiom, newly disclosed this round and not fixed (would need its own
  gate-profile-first design round).
- **Stage 3 next language: TypeScript** (per the frozen `Rust → Python →
  TypeScript → Go` order — Go is NOT next). Dependency
  ("Python trustworthy on a real repository") is now satisfied. No
  TypeScript-specific baseline, corpus extension beyond the existing
  pooled dispatch-corpus TypeScript cells, or audit sites are frozen yet.
  **First steps, from this stage's own text above:** pin a TypeScript
  compiler snapshot and real TS repositories with sha256 in
  `core-representative-corpus.json`; freeze a TypeScript methodology and
  labeling rubric (mirroring `stage3-python-audit/methodology.md` and
  `labeling-rubric.md`); select and commit at least 100 independently
  audited real-repository call sites; only then run the before-observation
  — labels committed before any scoring, this round's own late-checked
  lesson (see below).
  **Lessons from this correction chain, carried forward as one-liners:**
  commit every prediction before any measurement file exists on disk (a
  file's mtime, not just commit order, is the check — this round's own
  `design-and-prediction.md` failed this and was only caught in
  correction-3); check the label-commit-vs-first-scoring order while
  labeling, not after the fact; the dispatch-corpus harness runs every
  case in an isolated single-language directory, so it can validate a
  same-file/per-file gate but can NEVER exercise a project-wide pass's
  language handling on its own — any new project-wide resolver pass needs
  its own mixed-language-root check, the same way this round's Bit-code
  self-analysis was the only thing that caught `python_rebinding.rs`'s
  bug; commit the known-good state before any mutation test, never rely
  on `git checkout`/`cp`-from-backup to undo a mutation against
  uncommitted work (this round lost and had to reconstruct real work
  this way); reuse `correction-2/common.py`'s zero-padded hex `NodeId`
  resolution for any new verification script reading `girder inspect`
  output, rather than re-deriving it and re-hitting the same silent-drop
  bug.
  **Known leads for TypeScript's own gate-profile step:**
  `transformed_scope` (any decorator anywhere in a file) still gates
  TypeScript's whole-file Must computation exactly as it did for Python
  before this round — deliberately left untouched this round to respect
  language order, and the obvious first gate-profile suspect for
  TypeScript's own resolver design. `typescript-structural-object-literal`
  is the corpus's one pre-existing failed cell (origin symbol
  unresolved) and the first concrete item to fix.
- 2026-09-24: **Stage 3 TypeScript: repositories pinned, methodology
  frozen, 105 sites selected (`0b2ba8c`, `f0cf901`).** First attempt
  (`fbd39f8`) pinned the three repos directly into the shared
  `docs/core-representative-corpus.json` and broke that file's own
  already-gated test suite (`validate_manifest()` hard-codes "exactly six
  repositories", a rust/python-only language allowlist, and requires
  nonempty `semantic_cases`) — reverted (`00ba4b1`), re-pinned in a
  dedicated `docs/stage3-typescript-corpus.json` instead (`e7270bd`).
  Four repositories: `typescript-6.0.3` (the compiler's own `src/`, the
  "TypeScript compiler snapshot" this stage's order names explicitly —
  memory-feasibility checked directly, 473MB peak RSS on this 2.7GB VM,
  and the full upstream tarball's 84,334 members exceeded the shared
  tooling's archive-member limit, needing a path-filtered subset
  extraction rather than the whole repo), `zod-3.23.8`, `date-fns-4.1.0`,
  and `class-validator-0.15.1` (added after a review found the original
  three had ZERO genuinely-executing decorator usage — every apparent
  hit was inside a template-literal test fixture — which would have made
  the `decorator` shape empty in the sample entirely; corrected by adding
  a decorator-saturated fourth repo rather than lowering the shape count.
  **Not production coverage**: all 15 sampled `decorator` sites are
  factory calls in `class-validator`'s own `sample`/`test` files, not
  `src/` — `transformed_scope` will trip for those 26 files, correctly
  gating other same-file sites in them, but no `src/`-internal same-file
  Must proof depends on it, so this doesn't make the gate-profile step
  exercise it against production dispatch code, only against test/sample
  code. See
  [methodology-addendum-2.md](observations/stage3-typescript-audit/methodology-addendum-2.md)).
  `tools/dispatch_audit_site_selector_typescript.py`: a hand-written
  character-level masker (`mask_ts_source`) handles comments,
  strings, template literals with nested `${...}` interpolation tracking,
  and JS's regex-vs-division ambiguity — there is no Python `tokenize`
  equivalent for TypeScript and this repo's tooling has no third-party
  dependency infrastructure to add tree-sitter-typescript bindings
  instead. 46 unit tests; a masker over-masking spot-check against the
  real corpus (20 lines read directly, fixed seed) found zero bugs. Seven
  shapes (no `operator_dunder` analogue — TypeScript has no operator
  overloading; `optional_chaining_call` is a TypeScript/JavaScript-
  specific dispatch uncertainty neither Rust nor Python has), evenly
  represented in the frozen 105-site sample (15 each).
  All three criterion legs remain unmeasured — no before-observation, no
  scorer, no gate-profile, no labeling rubric yet. **Next step, per the
  frozen sequence**: the labeling rubric (`labeling-rubric.md`, quoting
  `docs/call-classification-policy.md`'s TypeScript row directly, per the
  Python precedent — not derived from Python's own rubric by analogy),
  written before any site's `true_class` is decided, covering
  TypeScript-specific cases the Python rubric has no analogue for:
  interface/structural receivers (no nominal guarantee, so May at best),
  `obj?.m()` with an otherwise-provable target (the call may simply not
  execute), function/method overloads (several signatures, one
  implementation), rebinding via `Foo.prototype.m = ...`/
  `Object.defineProperty`/module augmentation/namespace merging,
  `declare`/ambient targets (outside the snapshot), and `.d.ts`
  declaration-only lines (`not_a_call_site`, `typescript-6.0.3` alone has
  108 `.d.ts` files among its 709 matched files).
  Before writing the scorer: the TypeScript compiler source contains
  non-ASCII text (locale/contributor-credit strings), so byte-offset
  matching must be tested against a non-ASCII file, not assumed to work
  from the Python scorer's own byte-offset logic unchanged; `inspect
  --json` output size on `typescript-6.0.3` should be measured before any
  script loads it wholesale (pydantic's own output was already 22MB at a
  much smaller source size).
- 2026-09-24: **Stage 3 TypeScript: labeling rubric frozen, all 105 sites
  hand-labeled (`a20afd8`, `c5f4c45`, `675528d`).** Rubric quotes
  `docs/call-classification-policy.md`'s TypeScript row directly (not
  derived from Python's by analogy) and resolves TypeScript-specific
  cases neither Rust nor Python needed: structural/interface receivers
  are never Must from the interface type alone; `obj?.m()` is scored by
  the same target-binding rule as `obj.m()` (the policy describes which
  target is called if the call happens, not whether it happens);
  overloaded functions resolve Must if the single shared implementation
  is otherwise unique; legacy-decorator lines are the factory call itself
  (scored ordinarily), not the implicit application; `.d.ts` files can
  never contain a real call under any circumstance. A further review
  found two internal contradictions before any site was labeled (case 5
  didn't say explicitly that the selected site IS the factory call; case
  10 wrongly called a bare `@name` `not_a_call_site`, contradicting
  Python's own rubric case 8) — fixed in `c5f4c45`, checked that neither
  contradiction actually mislabels any of the 105 sites in this specific
  sample (zero bare-`@name` sites exist in it) before moving on.
  Labeling itself: every Must candidate's target definition was opened
  and checked for overriding subclasses (grep, not assumed) before
  labeling Must; every interface-typed receiver was checked for how many
  concrete implementations actually exist in the snapshot before deciding
  Unknown; built-in targets (`Date`, `Map`/`Set` methods,
  `Function.prototype.bind`/`.call`, Jest matchers) labeled Unknown as
  external to the four-repository snapshot. Distribution: **47 must, 50
  unknown, 8 not_a_call_site, 0 may**. **Correction to this entry's own
  first draft**: it claimed May "has never occurred naturally in a random
  sample for any language measured so far" — checked directly against the
  committed labeled-sites files rather than assumed, and found false:
  Rust's `audit-sites-labeled-v2.json` has 3 May sites (Python's has 0).
  TypeScript's 0-May result matches Python's, not the whole program.
  Confirmed directly before committing: no `dispatch_audit_scorer_typescript.py` and no
  scoring output exist anywhere in the repo, so these labels are
  genuinely frozen before any comparison against Girder's own answer.
  **Next step**: write `tools/dispatch_audit_scorer_typescript.py`,
  reusing the Python scorer's corrected byte-offset logic (mask the file,
  find the match column on the masked line, read the offset from the raw
  line, fail closed on drift) but computing UTF-8 byte offsets rather
  than character offsets and testing against a non-ASCII file before
  trusting it (the compiler source has locale/contributor-credit strings
  with non-ASCII text) — then run the before-observation. No resolver
  design work has started; this stage is still entirely at the
  measurement-before-any-code-change phase, matching where Rust's and
  Python's Stage 3 rounds each began.
- 2026-09-24: **Stage 3 TypeScript: before-observation measured
  (`48b6c11`...`2c7f6e8`).** Wrote `dispatch_audit_scorer_typescript.py`
  (innermost-covering-claim selection and `NEVER_COVERS` reused from the
  Rust/Python scorers' own pattern, both confirmed to already pick the
  tightest span among nested claims before trusting this — no re-audit of
  either DONE stage needed; `NEVER_COVERS` built from a real
  `girder inspect` run on `class-validator-0.15.1`, not copied from
  Python's; a `true_target` field added to every Must label so a claim
  resolving to a same-named-but-wrong definition scores `overclaim`, not
  `exact` — a real risk in this corpus, given zod's parallel `src`/
  `deno/lib` trees and `typescript-6.0.3`'s four separate `TestSession`
  declarations). Precommitted a blind prediction
  (`before-observation-prediction.md`, `a1428b4`) before running the
  scorer: derived from `claims.rs`'s actual current TypeScript behavior
  (the unmodified `transformed_scope` gate; no cross-file import
  resolution for any language yet), checked per-candidate which of the 46
  Must sites are same-file and in a decorator-free file (8, by direct
  grep) — predicted those 8 provable, 0 unsound anywhere.
  First real run: **24 `unsafe_exclusion` cells**, a severe deviation —
  treated as the stop signal the prediction document itself committed to,
  investigated before trusting it rather than reported as-is. Root cause:
  `typescript-6.0.3` uses CRLF line endings throughout (54,434 in
  `checker.ts` alone); `Path.read_text()`'s default universal-newline
  translation silently converts them to LF while Girder's own byte
  offsets are against the raw file, undercounting the scorer's
  byte-offset reconstruction by one byte per preceding line — confirmed
  exactly (a real site 29,877 lines in was 29,876 bytes short). Fixed by
  reading bytes directly and reconstructing without translation, verified
  against the raw-file ground truth exactly, a regression test added,
  then re-run.
  **Final result: 97 scored, 0 unsound cells** (0 `overclaim`, 0
  `unsafe_exclusion`) — all 51 Unknown-labeled sites score exact, all 46
  Must-labeled sites score conservative. The prediction's own 8-exact
  count was still wrong (0 observed) — investigated rather than silently
  adjusted: every one of the 8 predicted-provable sites' target functions
  turned out to be nested inside another function's own closure (e.g.
  `narrowTypeByTypeFacts` sits deep inside `createTypeChecker`'s own body,
  confirmed by direct character-offset comparison), not at true module
  scope — `claims.rs`'s shared `top_level` check (`parent.id() ==
  root.id()`) was never eligible for them regardless of decorators. This
  is a genuine, useful finding: TypeScript's "one giant factory function
  with everything nested inside" idiom (pervasive in this compiler's own
  architecture, and common more broadly) defeats the same-file proof
  mechanism in a way Rust/Python/Go's typically module-level-function
  code rarely hits — recorded as the next gate-profile step's own leading
  hypothesis, to be checked against the full 46-site conservative sample,
  not assumed from these 8 alone (a further review round found only 6 of
  these 8 are genuinely same-file — sites 29/34 are cross-file to
  `tracing.ts` — and isolated `emitter.ts` specifically as a clean,
  `duplicate_paths`-free test of the nesting hypothesis alone; see
  `docs/observations/stage3-typescript-audit/before-observation-addendum.md`).
  **Criterion status**: `zero_classification_errors_on_audit` **Met**
  (0/97 unsound); `nonempty_must_precision_1000_on_real_repository`
  **Not Met** (zero Must currently proven, precision undefined on an
  empty set); `measured_dispatch_corpus_improvement` **Not Met** (no
  resolver change made yet). **Stage 3 TypeScript remains IN PROGRESS** —
  this is the honest baseline before any resolver design begins, exactly
  where Rust's and Python's own Stage 3 rounds each started. All four
  common gates pass (no Rust code changed this round).
  **Next step**: gate-profile the 46 real-audit conservative sites and
  the pooled dispatch corpus's TypeScript-shaped conservative cells
  against every existing extractor gate (mirroring
  `dispatch_audit_gate_profile_python.py`'s role), checking specifically
  whether nested-closure scope (this round's own finding) or the
  still-unmodified `transformed_scope` decorator gate is the dominant
  blocker, **before** designing any TypeScript-specific Must-proof rule —
  the same discipline every prior language's Stage 3 resolver round in
  this program used.
- 2026-09-30: **PRIORITY FINDING, ahead of the gate-profile above: a
  node-id-collision bug silently discards a losing node's entire call
  evidence, AND produces a false-empty `test-impact` result for an
  ordinary edit inside that node's body -- confirmed via direct
  reproduction in BOTH TypeScript and Rust** (the already-DONE
  language), not yet fixed. When two functions/closures in the same
  file compute to an identical semantic path (TypeScript: two
  `it("same description", ...)` blocks under different `describe`
  scopes -- confirmed at scale in real code, e.g. 19 occurrences of
  `it("works with future", ...)` in one date-fns test file; Rust: two
  `impl Trait for Type` blocks providing the same method name, e.g.
  `serde_json/src/ser.rs`'s two `fn serialize_element` -- confirmed
  this is the true mechanism behind Rust's own DONE audit's one
  `duplicate-semantic-path` hit, index 89), only the later-built
  occurrence survives as a graph node; the earlier occurrence's node,
  and everything attached to it, is silently dropped -- not disclosed
  as a coverage gap, just gone. **Confirmed this is NOT merely an
  annotation/scoring-methodology question**: a real source edit made
  ONLY inside the lost occurrence's body (repro'd in both languages,
  `docs/observations/stage3-typescript-audit/collision-repro/`) makes
  `girder review --quiet` report only the module as changed (no
  function-level node) and `girder test-impact --quiet` return **empty
  -- zero tests selected**, not even the conservative must∪may∪unknown
  union. A matched control edit inside the surviving occurrence's body
  correctly flags the right test in both cases. This is the
  `classified_impact(&[])` empty-selection gap already documented in
  `CLAUDE.md`, but triggered here by an ordinary function-body edit,
  not only by a const/type-only change -- a **false-empty test-impact
  result on real, reachable code**, the central failure mode this
  entire program exists to rule out. Checked and confirmed NOT to
  affect the separate, project-wide resolved `Calls` graph
  (`sync::resolve_calls`) that `orient`'s `callers`/`callees` actually
  read: a function called only from inside the lost occurrence's body
  still gets a correctly-resolved (path-collapsed) caller edge in both
  languages -- the loss is specific to `call_evidence_v1` and to
  `review`/`test-impact`'s function-level change detection, not to
  basic reachability. Swept all 97 of this round's TypeScript scored
  sites and all of Rust's/Python's own committed DONE audits for
  further instances, using a validated criterion (covering claim
  starts on an earlier source line than the site, excluding disclosed
  whole-file gap claims): exactly 2 in TypeScript (sites 23, 91,
  already known), 1 already-known instance in Rust (index 89; two
  other row-mismatched Rust sites checked directly against source and
  confirmed to be ordinary multi-line method chains, not collisions),
  **zero in Python** (checked, not merely unconfirmed). Checked for the
  most dangerous variant -- a lost-node site whose borrowed claim
  happens to be `must`, scoring `exact` unnoticed: none exists in this
  round for any language (TypeScript observed zero `must` sites at all
  this round; Rust's and Python's borrowed sites were individually
  checked and none is a masked false-must). Full details, including
  the exact repro steps and commands to reproduce:
  `docs/observations/stage3-typescript-audit/before-observation-addendum.md`,
  `-addendum-2.md`, `-addendum-3.md`, `-addendum-4.md`. **Root cause
  located in source**: `crates/aether-graph/src/lib.rs`'s
  `SemanticGraph::upsert_node`/`upsert_projection_node` treat a
  repeated `NodeId` as "update this node in place" (by design, for the
  legitimate case of re-parsing the SAME logical entity after an edit)
  -- `if let Some(&idx) = self.index.get(&id) { self.graph[idx] = node; }`
  wholesale-overwrites whatever was at that index. `claims::annotate()`
  itself sees both colliding occurrences correctly (confirmed: both get
  their own, independently-correct claims at extraction time) --
  the loss happens one layer up, in `crates/aether-builder/src/sync.rs`'s
  `apply()` (`for node in &out.nodes { graph.upsert_projection_node(node.clone()); }`,
  called from `load_file_unresolved`/`update_file`), which upserts every
  extracted node in sequence with no collision check: the second
  colliding node's upsert destroys the first's entry, attributes and
  all. This is a correctness gap in an already-DONE language (Rust),
  found by a still-IN-PROGRESS language's (TypeScript's) own review
  process -- recorded here rather than only in the TypeScript stage's
  own documents, since it is not specific to TypeScript.
  **The false-empty `test-impact` SYMPTOM is FIXED for every invocation
  form** (two commits: the first fixed only the bare `--quiet` path and
  was briefly, incorrectly, described as closing it entirely -- see
  `before-observation-addendum-5.md` and its own correction in
  `-addendum-6.md`). `semantic_changed_impact_with_config`
  (`crates/aether-app/src/project/git.rs`) no longer filters a changed
  Module node out of `origin_ids`, so a change whose only detectable
  effect is on a Module (whether from this collision, a const/type-only
  edit, or a `describe`/`beforeEach`-level statement) now flows through.
  Two different consumers of `origin_ids` both needed a fix:
  `quiet_from_graph` (bare `--quiet`) already used `classified_impact`'s
  own conservative must∪may∪unknown escalation once the origin stopped
  being dropped -- fixed by the first commit alone. `test_impact.rs`'s
  FULL path (no flags, `--out`, `--run` -- confirmed separately broken:
  `--run` would execute **zero tests** on a real change) uses the
  older, narrower `tests_for_nodes` (resolved-Calls-reachability only,
  no incoming edge into a Module, no escalation) -- fixed by falling
  back to the SAME classified union only when `tests_for_nodes` is
  empty and an origin exists, preserving the existing, already-tested
  narrower behavior for the common (non-empty) case (a
  broader/unconditional swap was tried first and reverted after it
  broke 3 pre-existing tests whose exact impacted-test sets are
  asserted). Verified properly: both the bare-`--quiet` and the
  `--run`/full-path regression tests
  (`test_impact_quiet_is_not_empty_for_a_module_level_only_change`,
  `crates/aether-app/tests/cli.rs`) were confirmed to fail on the
  respective pre-fix code before being confirmed to pass on the fix;
  all four gates pass twice (once per commit)
  (`cargo test --workspace`, `clippy -D warnings`, `fmt --check`,
  `node --test`). `CLAUDE.md` and the `impacted_tests` MCP tool
  description updated to match (the documented const/type-only-edit gap
  is closed; the tool description's "reach the functions that changed"
  corrected to "reach what changed" since a Module can now be an
  origin). A pre-existing test
  (`test_impact_uses_baseline_tests_for_removed_functions`) needed its
  own fix: the "Removed functions were detected" message's trigger
  condition was `origin_ids.is_empty()`, which is no longer exclusive
  with a real removal once that removal's Module-level side effect is
  itself an origin -- changed to trigger off `baseline_test_paths`
  directly instead.
  **A third, independent gap in the same symptom was found and fixed**
  (`docs/observations/stage3-typescript-audit/before-observation-
  addendum-8.md`): a precommitted prediction, then confirmed exactly,
  showed Go -- unlike Python/TypeScript (an unconditional whole-file
  gap claim) and Rust (the `#[test]` attribute's own claim) -- has NO
  language construct that unconditionally produces an Unknown claim,
  so a same-file Go call can be Must-proven with ZERO boundaries
  anywhere in the file (confirmed against the existing cross-language
  unit test `direct_local_bindings_have_evidence_in_all_four_languages`,
  `crates/aether-builder/src/mapper/claims.rs`, which already proves
  this). A package-level Go var change with no function body touched
  produced a FULLY silent empty `test-impact` result -- no stdout, no
  stderr boundary notice, in every invocation form -- worse than the
  Rust/TypeScript cases, which always at least printed a boundary
  notice even when empty. Fixed in `classified_impact`
  (`crates/aether-graph/src/claims.rs`): a Module-kind origin with no
  Function-kind origin from the same file now gets its own
  `coverage_gap` boundary, triggering the existing "any boundary
  escalates every function" rule -- deliberately scoped to that
  specific condition (not every Module origin) to avoid disturbing the
  bounded trustworthiness fixtures the same way the first, reverted
  `test_impact.rs` attempt did. Verified end-to-end against the real
  rebuilt binary (sha256
  `48553d3a52ce674283fa2c7c11fddaab21bc51fab4523679807e86ce3208ab14`):
  the fallback now correctly selects and (via `--run`) actually
  executes the real test, which genuinely fails
  (`panic: runtime error: index out of range`) -- a real regression
  this exact mechanism would have silently missed before the fix.
  **The underlying node-id-collision data loss itself is STILL NOT
  fixed** -- `call_evidence_v1`/Must-Unknown classification fidelity for
  the LOST occurrence specifically is still silently wrong (its own
  per-call evidence is still destroyed, not merged or preserved); the
  three fixes above (addenda 5, 6, 8) only ensure `test-impact`/`review`
  no longer stay silent about the resulting Module-level change, for
  every cause of it (the collision bug, const/type-only edits,
  `describe`/`beforeEach`-level statements, and now Go package-level
  var changes). Recommended next step for the deeper fix, before the
  TypeScript gate-profile above: locate and fix the node-insertion
  collision handling itself (either disambiguate colliding paths, e.g.
  by including an impl-target/enclosing-scope discriminator in the
  semantic path, or detect the collision and merge/preserve both
  nodes' evidence instead of silently dropping one). Also still open,
  not blocking: `--quiet --out`/`--quiet --run` reach a correct answer
  via a different mechanism (the `tests_for_nodes`-then-fallback path)
  than bare `--quiet` (the unconditional classified union) with a
  different boundary message; a single commit combining a Module-only-
  origin file with an unrelated, reachable function-body change
  elsewhere would make `tests_for_nodes` non-empty overall and so never
  trigger the fallback, potentially still missing the Module-only
  file's own tests from that combined selection, in BOTH
  `test_impact.rs` and (see immediately below) `test_checks.rs`.
  **This residual is now confirmed on a real extracted Go graph, not
  just reasoned about** (`before-observation-addendum-10.md`): editing
  a module-level `var` together with an unrelated, ordinary, fully-
  resolved same-file function in one commit makes `test-impact --quiet
  --classified` select only the unrelated function's own test, with
  the module-level-affected test completely missing and zero
  boundaries reported. The current "no same-file Function origin"
  heuristic in `claims.rs::classified_impact` is too coarse for this
  case; the correct longer-term fix is "Module diff not explained by
  child-function diffs" (check whether the Module's OWN source change
  is accounted for by the specific origin Function nodes present, not
  merely whether any Function origin from the same file exists at
  all) -- not yet implemented.
  **A second mandatory gate had the identical bug, now also fixed**
  (`docs/observations/stage3-typescript-audit/before-observation-
  addendum-9.md`): `run_tests_impacted`
  (`crates/aether-app/src/project/planfile/checks/test_checks.rs`),
  which Plan Format v2's own **mandatory** `tests.impacted` check
  gates on, called `tests_for_nodes` directly with no fallback at
  all -- a plan step editing only a module-level const could have
  passed its mandatory test gate while a real, reachable test
  silently never ran. Fixed with the identical scoped fallback,
  verified not to regress the existing, intentionally-pinned "Gap 24"
  vacuous-create-only-pass test, plus a new regression test
  (`tests_impacted_check_is_not_vacuous_for_a_module_level_only_edit`,
  `executor.rs`). All four gates pass.
  **Oracle and representative-mutation harness re-run against the
  final binary** (sha256
  `beadb6c91907bf0e20830319e4cbbeff19eab4172cacc17caea6980cb336113f`),
  both required by the prior review, now done:
  `core_trustworthiness_oracle.py` shows precision/recall UNCHANGED
  (1.000/1.000 throughout) with only a benign `+2 coverage_gap`
  boundary-count side effect, confirmed field-by-field against the
  prior baseline and the baseline updated to match.
  `core_representative_mutations.py` shows a genuinely substantive,
  understood change: the long-documented `group-invoke` dynamic-
  dispatch defect CLAUDE.md itself cites by name (recall `0.000`) now
  measures recall `1.000`/precision `0.667` -- NOT because the
  underlying dispatch-resolution gap closed (it didn't; the dispatch
  still classifies `unknown`, confirmed directly), but because the
  bare `test-impact` form this harness uses is exactly the path
  addendum-6 fixed: the previously-silent empty result on an
  unresolved-dispatch origin now falls back to the conservative union.
  `docs/core-representative-mutations.md`, `docs/core-gap-analysis.md`
  (item 11), and `CLAUDE.md` all updated to state plainly that the
  resolution gap is unchanged and still open; only the empty-selection
  symptom is fixed. The pinned TypeScript corpus checkouts
  used for this round's measurement lived in a session-scratchpad
  directory that does not persist across sessions and is now gone;
  before any TypeScript gate-profile or resolver work resumes, the
  corpus must be re-extracted from `docs/stage3-typescript-corpus.json`
  to a path under the repo's own working tree (or another persistent
  location), not the scratchpad.
- 2026-10-01: **Design decision for the actual node-collision fix,
  recorded before writing code.** Two options were weighed: (a)
  qualify every trait-impl method's path with its trait name
  (semantically cleanest, stable, one-time id churn for every
  trait-impl method in every language's corpus), or (b) qualify only
  methods that actually collide with a same-named sibling in the same
  file (minimal id churn, but adding a second colliding impl later
  renames the first). **Chose (b)**, based on a concrete blast-radius
  check, not a guess: `grep -rn 'format!("{}::'` across
  `aether-builder`/`aether-app` found exactly one place that hardcodes
  an unqualified trait-impl method path --
  `crates/aether-builder/src/sync.rs:967`'s RAII-drop heuristic,
  `NodeId::from_path(&format!("{}::drop", candidate.owner))`. `Drop` is
  always a trait impl (there is no inherent `drop`), so option (a)
  would have broken this specific, already-working, cross-cutting
  heuristic for every `Drop` impl in every corpus -- confirmed by
  reading the code, not assumed. Option (b) leaves this lookup (and
  every other non-colliding trait method -- `fmt`, `next`, `clone`,
  etc.) completely untouched, since those never collide with a
  same-named sibling in the same file in ordinary code.
  Also ran the cheap, no-build check on Rust's DONE Must set this
  review round required: across `correction-2/audit-after.json`,
  exactly **one** scored site has `observed_class: must` at all (index
  53, `petgraph` `floyd_warshall.rs:11`, target `Graph::add_node`) --
  confirmed via direct source grep that `add_node` has exactly one
  definition (a plain inherent `impl<N,E,Ty,Ix> Graph<...> { ... }`
  block, no competing trait impl), so this one Must claim is not
  collision-affected. More broadly: `crates/aether-builder/src/sync/
  rust_methods.rs`'s same-file method Must-proof mechanism (the one
  that produced this claim, reason `proven-inherent-method-on-
  annotated-receiver`) is already scoped to INHERENT methods
  specifically (it explicitly checks for and refuses competing trait
  declarations, `inherent_wins`) -- by construction, it never targets a
  trait-impl method at all, so trait-impl-method collisions cannot
  silently corrupt any currently-published Rust Must claim. The
  node-collision bug's confirmed impact remains scoped to
  `call_evidence_v1` fidelity and `review`/`test-impact`'s attribution
  for the LOST occurrence specifically (per addenda 4/5/6/8/9), not to
  any Must-proof correctness.
- 2026-10-01: **The actual node-collision fix is implemented and
  verified** (`docs/observations/stage3-typescript-audit/
  before-observation-addendum-12.md`) -- the root cause every prior fix
  in this thread (addenda 5, 6, 8) worked around without closing.
  `crates/aether-builder/src/mapper.rs` now qualifies a trait-impl
  method's path with its trait name (`{name}@{trait}`), but ONLY when
  it actually collides with a same-named sibling in the same file
  (the design decision recorded above) -- both the Node-creation pass
  (`collect_defs`) and the separate caller-id-computing pass inside
  `collect_calls`'s `walk` use one shared helper
  (`qualified_method_name`), with a new regression test confirming
  their ids stay consistent for a call made inside a qualified
  method's body. All three predictions precommitted in addendum-11
  were checked against the real binary (sha256
  `1a502ab70162bf5897f810515384dec86c09b5058f9574b53ac0cb44cea7d8d4`)
  and confirmed exactly, including the one honestly flagged as
  uncertain (explicit trait-qualified call syntax, `A::go(&S)`,
  remains unresolved to a specific candidate -- `sync.rs`'s
  `select_candidate` was deliberately left unchanged -- but
  `test-impact` still conservatively selects the right test regardless,
  via the escalation already in place). `review . --quiet` now shows
  the actual previously-lost node (`crate::lib::S::go@A`) when only
  its body changes -- the first point in this entire thread where the
  root cause itself, not just a downstream symptom, became visible and
  fixed. All four gates pass; the trustworthiness oracle's baseline
  still matches exactly (specifically confirmed `rust_raii_drop_
  selected` -- the fixture that would have broken had the design
  decision gone the other way -- is unaffected); the representative-
  mutation result is byte-identical (unrelated, Python-only mutation).
  **Still open**: Rust's DONE audit needs a `correction-3` re-score
  against a fresh `inspect` of the pinned `petgraph`/`regex`/
  `serde_json` corpus -- not done in this round (the pinned checkouts
  are not present in this session's scratchpad); `ser.rs`'s two
  `serialize_element` methods (the confirmed real-world collision from
  addendum-4) should move from the whole-file `duplicate-semantic-
  path` gap claim to individually-attributed evidence once re-scored.
  TypeScript's own collision shape (duplicate `it()`/`describe()`
  description strings, sites 23/91) is a different mechanism, entirely
  unaffected by this Rust-only fix, and remains unaddressed.
- 2026-10-01: **The combined-origin residual (addendum-10) is also now
  fixed and verified** (`docs/observations/stage3-typescript-audit/
  before-observation-addendum-13.md`) -- the second, final item from
  the standing directive's "first priority." A new shared function,
  `aether_graph::origins_excluding_explained_modules`
  (`crates/aether-graph/src/diff.rs`), excludes a Module origin from
  `origin_ids` only when its own source, with every sibling Function
  origin's span masked out, is byte-identical between the git baseline
  and current graph -- applied identically in both callers that build
  origins from a before/after graph pair (`git.rs`'s `test-impact` CLI
  path and `test_checks.rs`'s `tests.impacted` plan check). The first
  implementation attempt failed its own new regression test; traced
  (not assumed) to `classified_impact`'s OWN separate "same-file
  Function origin exists" guard independently undoing the more precise
  upstream decision -- fixed by simplifying that guard to unconditional
  now that both callers pre-filter correctly before it ever runs. All
  four gates pass; end-to-end confirmed on the rebuilt binary (sha256
  `2861fe7cf70d016a025a4bc1e385447a62fdb9fdc1dd8b7edbfbb9bb6f327092`)
  that the combined-edit case now selects both the module-affected and
  the ordinarily-edited function's tests, while the common
  single-function-edit case stays exactly as narrow as before. Oracle
  baseline matches exactly; representative-mutation result shows one
  benign, disclosed `+1` boundary count with precision/recall/TP/FP
  sets all unchanged.
  **Both fixes are implemented**: the node-identity/collision problem
  (addendum-12) and the combined-origin false-empty case (this entry).
  Still open, unaffected by either fix: TypeScript's own
  collision mechanism (sites 23/91), Rust's DONE audit's
  `correction-3` re-score, and the TypeScript corpus re-extraction /
  gate-profile step.

## Release and CI verification checkpoints

- The local Rust 1.97.1 Clippy pass did not establish CI compatibility. A newer
  stable CI toolchain reported `clippy::double_must_use` on two async-trait
  declarations. `716531f` fixed those sites; live CI run `36895144641` passed.
- The v0.3.0-rc.1 pipeline built all four platforms and published a prerelease;
  npm and registry publication correctly skipped. The downloaded Linux binary's
  checksum, version, and MCP initialization were independently verified.
- v0.3.0 was published and tested through the real npm installation path. Initial
  npm dist-tag and tarball propagation delays were observed before successful
  installation; metadata alone was not treated as install evidence.
- v0.3.1's MCP registry job failed because its description exceeded 100 characters.
  v0.3.2 shortened it to 94 characters. GitHub release, npm installation/MCP
  initialization, and the registry's authoritative latest-version record were
  verified. Registry search temporarily lagged the versions endpoint.
- The current command-dispatch function returns no license requirement for
  `test-impact` or `orient`. No new CLI enforcement command is implemented.
- A clean working tree's default `test-impact` compares against HEAD, not a PR's
  base branch. CI base-reference comparison remains an unimplemented interface.
- No build, release, or gate was performed by the 2026-10-01 reconciliation pass;
  earlier execution records remain historical evidence for their own candidates.
