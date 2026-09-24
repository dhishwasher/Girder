# Girder technical roadmap

Status, acceptance criteria, measurement evidence, failures, blockers, and next
actions for call classification, dispatch resolution, client integration, and
verified edits.

## Resume contract

Every session starts by reading this file and ends by updating it. Execute stages
in order, with the language dependency in Stage 3 enforced. Commit and push each
coherent piece separately; do not tag or release. The first program commit is this
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

**Status: IN PROGRESS** (Rust: DONE, all four criterion legs met with
committed evidence, after seven correction rounds; Python: DONE, all three
criterion legs met with committed evidence, after a three-round correction
chain; TypeScript: repositories pinned, methodology frozen, 105 sites
selected, not yet labeled; see "Current checkpoint")

Order: **Rust → Python → TypeScript → Go**. No fifth language. Each language has
its own frozen baseline, implementation, after-observation, and gate checkpoint:

| Language | Status | Before / after evidence | Dependency |
| --- | --- | --- | --- |
| Rust | **DONE** | [before](observations/stage3-rust-audit/audit-scoring-summary-v2.json) / [after](observations/stage3-rust-audit/after-method-call-fix/after-observation.md) | Stage 2 |
| Python | **DONE** | [methodology](observations/stage3-python-audit/methodology.md) / [after-transformed-scope-fix](observations/stage3-python-audit/after-transformed-scope-fix/after-observation.md) + [correction-1](observations/stage3-python-audit/after-transformed-scope-fix/correction-1/correction.md) / [correction-2](observations/stage3-python-audit/after-transformed-scope-fix/correction-2/correction.md) / [correction-3](observations/stage3-python-audit/after-transformed-scope-fix/correction-3/correction.md) | Rust trustworthy on a real repository |
| TypeScript | IN PROGRESS (repos pinned, methodology frozen, 105 sites selected AND labeled; scorer not yet written, no measurement run) | [methodology](observations/stage3-typescript-audit/methodology.md) + 2 addenda + [rubric](observations/stage3-typescript-audit/labeling-rubric.md) / none | Python trustworthy on a real repository |
| Go | NOT STARTED | none / none | TypeScript trustworthy on a real repository |

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

**Second resolver change done, after-observation done, criterion fully
met — Stage 3 (Rust): DONE.**
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
in full, none of them blocking DONE.

**Gate per language:** before/after corpus and real-repository audit, then all
common gates. **Observation:**
[audit-scoring-summary-v2.json](observations/stage3-rust-audit/audit-scoring-summary-v2.json)
(before) and
[after-observation.md](observations/stage3-rust-audit/after-method-call-fix/after-observation.md)
(after — Rust DONE, all four criterion legs met).
**Blockers:** none — next action is narrowing `transformed_scope`'s
whole-file scope (see "Next" below), not a missing dependency.

## Stage 4 — Client-agnostic packaging and orient-first guidance

**Status: NOT STARTED**

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
supported; then all common gates. **Observation:** none.
**Blockers:** preceding stages; formats must be reverified at implementation.

## Stage 5 — Verified edits

**Status: NOT STARTED**

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
checks, then all common gates. **Observation:** none. **Blockers:** preceding stages.

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
  unknown, 8 not_a_call_site, 0 may** — matching every other language
  audited in this whole program (May has never occurred naturally in a
  random sample for any language measured so far). Confirmed directly
  before committing: no `dispatch_audit_scorer_typescript.py` and no
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
- Completion remains unproven until every criterion above has committed evidence.
