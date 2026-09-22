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

**Status: IN PROGRESS**

The [v1 classification policy](call-classification-policy.md) is specified for
separate preimplementation commitment. No classified API or measurement exists yet.

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
common gates. **Observation:** none. **Blockers:** none established for policy.

## Stage 2 — The dispatch corpus

**Status: NOT STARTED**

Commit a versioned independent oracle before scoring: at least 40 cases,
initially 48 (12 per language), weighted to Rust trait objects/generics, Python
inheritance/super/getattr, TypeScript structural typing/unions, and Go interfaces.
Record sources, queries, expected sets, assumptions, rationale, controls, and
negative targets.

**Precommitted criterion:** score Girder against every case and publish all
expected/actual results, including every failure or unavailable execution.
Never tune cases to the product. When the product starts passing a case, retain
it and append a harder case; publish fixed-cohort scores separately.

**Gate:** oracle/harness validation, complete corpus run, then all common gates.
**Observation:** none. **Blockers:** Stage 1 checkpoint.

## Stage 3 — Close dispatch holes one language at a time

**Status: NOT STARTED**

Order: **Rust → Python → TypeScript → Go**. No fifth language. Each language has
its own frozen baseline, implementation, after-observation, and gate checkpoint:

| Language | Status | Before / after evidence | Dependency |
| --- | --- | --- | --- |
| Rust | NOT STARTED | none / none | Stage 2 |
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

**Gate per language:** before/after corpus and real-repository audit, then all
common gates. **Observation:** none. **Blockers:** Stage 2 and language order.

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
gates. **Observation:** none. **Blockers:** capable agent not yet established.

## Current checkpoint

- 2026-09-22: roadmap and project resume instructions committed. Stage 1 policy
  specified; remaining stages NOT STARTED. No stage measurements or gates run.
- MOVESPEED is available: approximately 291 GB free; system reports approximately
  2 GB available RAM and no swap. Cargo and rustc are available from its cache.
- Stage 4 requires Claude Code, Cursor, and Codex adapters plus raw MCP JSON fallback.
- Next: commit the Stage 1 policy separately, then implement shared call evidence
  and classified reachability before changing CLI/MCP presentation.
- Completion remains unproven until every criterion above has committed evidence.
