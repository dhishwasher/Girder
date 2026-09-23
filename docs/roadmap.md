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

**Status: IN PROGRESS** (policy frozen for Rust; audit and resolver work not
yet started)

Order: **Rust → Python → TypeScript → Go**. No fifth language. Each language has
its own frozen baseline, implementation, after-observation, and gate checkpoint:

| Language | Status | Before / after evidence | Dependency |
| --- | --- | --- | --- |
| Rust | IN PROGRESS | none / none | Stage 2 |
| Python | NOT STARTED | none / none | Rust trustworthy on a real repository |
| TypeScript | NOT STARTED | none / none | Python trustworthy on a real repository |
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

**Not yet done:** the resolver change, and the after-observation (re-run
this audit with `tools/dispatch_audit_scorer.py`, the dispatch corpus, and
the Stage 1 trustworthiness oracle, all three must show no regression and
the audit must show fewer conservative / more exact cells) that "measured
dispatch-corpus improvement" requires. Stage 3 (Rust) cannot be marked DONE
on the before-observation alone.

**Gate per language:** before/after corpus and real-repository audit, then all
common gates. **Observation:**
[audit-scoring-summary.json](observations/stage3-rust-audit/audit-scoring-summary.json)
(before-observation only; after-observation pending). **Blockers:** none —
Stage 2 and language order satisfied; next action is the resolver change
named above.

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
- Next: the resolver change. The identifier-only filter is the
  highest-leverage target (92% of the audit's conservative cells), but
  extending it to method/path calls needs care — syntax alone can't give
  receiver types; the provable subset is `self.m()`/`Self::m()` inside an
  inherent `impl T` (or `T::m()`) where `T` has exactly one inherent `m`
  across the whole crate, which needs a crate-wide index of inherent impls
  (project-level work, not something per-file `annotate()` can do alone).
  Guard any change with: adversarial unit tests (a local macro shadowing
  `assert_eq!`, a `#[cfg]` attribute, an attribute macro); the dispatch
  corpus must stay 0 unsound and any false Must fails the gate; the Stage 1
  trustworthiness oracle's union recall must stay 4/4; the audit
  (`tools/dispatch_audit_scorer.py`, re-run) must show 0 unsound. If Rust's
  full criterion (including "measured dispatch-corpus improvement," which
  needs an actual before/after) isn't met, mark FAILED-AND-PUBLISHED and
  stop — do not start Python before Rust is trustworthy on a real repository.
- Completion remains unproven until every criterion above has committed evidence.
