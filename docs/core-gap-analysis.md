# Core workflow gap analysis

Last updated: 2026-08-09

This is the prioritized, evidence-based comparison for Bit Code's core
`analyze → navigate/search → edit/refactor → review impact → select tests →
validate → commit/rollback` workflow. It is not a feature checklist or a claim
that graph-native behavior is automatically better. A gap remains open until a
reproducible repository fixture or benchmark proves otherwise.

## Competitive baseline

| Area | Serious-environment baseline | Bit Code evidence | Priority and gap |
|---|---|---|---|
| Indexing and code intelligence | JetBrains project analysis builds an index for navigation, refactoring, inspections, and completion. Cursor uses Merkle-tree change detection and cached semantic chunks for incremental codebase indexing. | Rust/Python tree-sitter projections, cross-file call resolution, and incremental `update_file` reconciliation are implemented. No representative-repository indexing latency or memory benchmark is recorded yet. | **P0 evidence gap:** benchmark cold indexing, one-file updates, peak memory, and stale-edge removal on increasingly large repositories. |
| Navigation and refactoring | VS Code exposes language-service navigation, cross-file rename, and refactor preview; JetBrains provides project-wide dependency analysis and language-aware refactoring. | Stable graph ids, typed callers/callees, impact traversal, and validated rename projection work for the supported Rust/Python subset. | **P0 correctness:** measure resolved/unresolved call edges and false edges. Close common language-semantic gaps before adding refactor kinds. |
| Test discovery and coverage | VS Code's testing API supports framework discovery, execution, debugging, and dynamic coverage when supplied by an extension. | Bit Code selects graph-reachable tests and can run configured Rust/Python commands. Direct Rust test attributes are distinguished from `cfg(test)` wrappers; the checked function-execution oracle measures Rust precision/recall at `0.667/1.000` and Python at `1.000/1.000` on bounded fixtures. | **P0 correctness:** reconcile the remaining framework-inventory mismatch, improve CLI argument-route precision, and expand the oracle to representative repositories and implicit RAII/`Drop`. |
| Diagnostics and validation | JetBrains performs continuous file/project analysis; VS Code language services and tasks surface diagnostics while editing. | Candidate changes are conflict-checked and validated in a disposable project before a journaled commit. | **P1 responsiveness:** validation is strong at commit time, but edit-to-diagnostic latency and cancellation behavior are not benchmarked. |
| Recovery | VS Code provides local file history and refactor preview. Mature IDEs preserve undo/local history across routine editing. | Bit Code uses baseline checks, durable backups, a transaction journal, startup recovery, and graph/source snapshot binding. | **Graph-native opportunity, still P0 to prove:** run a fault-injection matrix at every journal transition and verify all-old/all-new recovery. |
| Agent autonomy | Cursor combines semantic codebase retrieval with agent editing. JetBrains and VS Code expose broad language tooling to AI integrations. | Bit Code agents plan from the graph and generated changes pass the same candidate validator and transaction boundary as manual graph edits. | **P1 evidence gap:** record patch acceptance, validation-failure detection, rollback success, and human rejection rates on real tasks. |

Official baseline references:

- [VS Code refactoring and preview](https://code.visualstudio.com/docs/editing/refactoring)
- [VS Code testing and coverage](https://code.visualstudio.com/docs/debugtest/testing)
- [JetBrains project analysis](https://www.jetbrains.com/help/idea/project-analysis.html)
- [JetBrains dependency analysis](https://www.jetbrains.com/help/idea/dependencies-analysis.html)
- [Cursor codebase-indexing security model](https://www.cursor.com/security)

## Current semantic-correctness phase

Verified defect: a receiver narrowed by
`if let Some(identity) = identity` retained only the outer
`Option<&SessionIdentity>` hint. Consequently,
`apply_session_delta → SessionIdentity::verify_delta_provenance` was absent,
and review/test-impact falsely reported the verifier as uncovered.

Acceptance evidence:

- Before the fix, Bit Code reported zero callers and zero affected tests for
  `SessionIdentity::verify_delta_provenance`.
- The focused Rust fixture now resolves `Some`, `Ok`, and `Err` consequence
  bindings to the correct generic argument even when another type has the same
  method name.
- Incremental source refresh removes the narrowed call edge when the call is
  deleted.
- The focused end-to-end repository selects exactly one true affected test:
  precision `1/1`, recall `1/1`.
- On Bit Code itself, the missing
  `apply_session_delta → verify_delta_provenance` edge is present and the direct
  tamper/unsigned-relay provenance test is selected. The current graph selects
  25 tests for that method, demonstrating that real-repository precision still
  needs work even though this false negative is fixed.

### Macro token-tree precision

Verified defect: the fallback scanner that recovers calls from Rust macro token
trees also scanned string and comment contents. A source fixture embedded in
`format!(r#"..."#)` therefore made its enclosing test appear to call
`GraphBuilder::apply`, even though the function was only text destined for a
temporary repository.

Acceptance evidence:

- Before the fix, Bit Code reported three callers of `GraphBuilder::apply`,
  including the unrelated
  `test_impact_follows_if_let_narrowed_receivers` fixture.
- The scanner now masks normal, byte, raw, raw-byte, C-string, raw-C-string,
  character, byte-character, line-comment, and nested block-comment regions
  while preserving byte offsets used for receiver qualification.
- A focused lexical fixture finds all three real calls and zero calls from the
  literal/comment cases. A graph fixture retains its genuine macro-contained
  call, creates no edge for embedded Rust text, and removes the genuine edge
  after an incremental update.
- On Bit Code itself, `GraphBuilder::apply` now has exactly its two real callers:
  `load_file` and `update_file`. The unrelated fixture caller is absent.

### Match-arm receiver resolution

Verified defect: `Some`/`Ok`/`Err` bindings in Rust `match` arms retained the
outer wrapper's receiver hint. On a fixture with same-named methods on multiple
types, Bit Code reported no callees for the `Option` arm and only one of the two
required `Result` arm callees.

Acceptance evidence:

- Before the fix, the isolated fixture's `apply` function had zero recorded
  callees and `inspect_result` recorded only `Failure::inspect`.
- Arm-local hints now cover both the optional guard and arm value without
  leaking to sibling arms or the enclosing scope.
- The same fixture now resolves exactly
  `SessionIdentity::{inspect, verify_selected_operation}` from `apply`, and
  both `SessionIdentity::inspect` and `Failure::inspect` from `inspect_result`;
  the same-named decoy methods are excluded.
- The graph regression verifies guarded `Some`, `Ok`, and `Err` arms, exact
  owner selection, affected-test propagation, and stale-edge removal after an
  incremental update.
- The end-to-end repository fixture selects exactly its one true provenance
  test after the selected operation changes: precision `1/1`, recall `1/1`.

### Let-else receiver resolution

Verified defect: Rust bindings introduced by `let Some(...) = ... else`,
`let Ok(...) = ... else`, and `let Err(...) = ... else` were not narrowed for
the following statements in their block. On the isolated same-method-name
fixture, the Option and Ok paths had no recorded callees.

Acceptance evidence:

- Before the fix, `apply` and `inspect_ok` each had zero callees;
  `inspect_err` happened to resolve only `Failure::inspect`.
- The block walker now applies the narrowed hint only after the declaration and
  only to following siblings in that block. The diverging alternative and
  enclosing scope retain their prior hints.
- The same fixture now resolves exactly
  `SessionIdentity::{inspect, verify_selected_operation}` from `apply`,
  `SessionIdentity::inspect` from `inspect_ok`, and `Failure::inspect` from
  `inspect_err`.
- The graph regression covers nested block scope, exact owner selection,
  affected-test propagation, and incremental stale-edge removal.
- The end-to-end repository fixture selects exactly its one true provenance
  test after the selected operation changes: precision `1/1`, recall `1/1`.

### Let-chain receiver resolution

Verified defect: bindings introduced by Rust let-chains did not affect either
later condition operands or the `if`/`while` body. The isolated fixture retained
only a free call before the binding and omitted every receiver-qualified call.

Acceptance evidence:

- Before the fix, `apply` recorded only `ready`; `inspect_ok` had zero callees.
- Let-chain operands now update hints left to right after each supported
  `let_condition`. Calls before a binding retain the prior hints, later
  operands see the new binding, and the final hint set is restricted to the
  consequence/body.
- The same fixture now resolves `ready` plus
  `SessionIdentity::{is_valid, inspect, verify_selected_operation}` from
  `apply`, and `SessionIdentity::{is_valid, inspect}` from `inspect_ok`; decoy
  methods are excluded.
- The graph regression covers ordered pre/post-binding operands, both `if` and
  `while`, exact owner selection, affected-test propagation, and incremental
  stale-edge removal.
- The end-to-end repository fixture selects exactly its one true provenance
  test after the selected operation changes: precision `1/1`, recall `1/1`.

### Python annotated receiver resolution

Verified defect: Python parameter annotations were ignored when resolving
receiver-qualified calls. In a two-file fixture with `inspect` on two classes,
`apply(identity: SessionIdentity)` omitted `SessionIdentity::inspect`, while
`inspect_decoy(identity: DecoyIdentity)` had no callee. A uniquely named method
resolved only because there was no competing candidate.

Acceptance evidence:

- Before the fix, `apply` recorded only the unique
  `SessionIdentity::verify_selected_operation` method and `inspect_decoy`
  recorded no callees.
- Direct, dotted, quoted-forward, and typed-default class annotations now
  provide exact receiver owners. Compound unions/generics and import aliases
  deliberately remain unresolved rather than guessing.
- The same fixture now resolves exactly both SessionIdentity methods from
  `apply` and `DecoyIdentity::inspect` from `inspect_decoy`.
- The graph regression verifies cross-file exact-owner selection, decoy
  exclusion, affected-test propagation, and incremental stale-edge removal.
- The end-to-end Python repository selects exactly its one true test and emits
  `pytest -k test_provenance`: precision `1/1`, recall `1/1`.

### Python constructor-assignment resolution

Verified defect: receiver hints did not follow Python assignments such as
`identity = SessionIdentity()`. Same-named methods were therefore unresolved,
and only globally unique methods appeared in the graph.

Acceptance evidence:

- Before the fix, `apply` and an ordered reassignment fixture retained only
  `SessionIdentity::verify_selected_operation`; the dotted Decoy constructor
  had no callee.
- Simple-name assignments from direct/dotted class constructors or direct local
  annotations now update hints in statement order. Unsupported reassignment
  explicitly clears an older hint instead of creating a stale false edge.
- The same fixture now resolves both SessionIdentity methods from `apply`,
  `DecoyIdentity::inspect` from the dotted case, and both true inspect owners
  plus the verifier from ordered reassignment.
- The graph regression also verifies stale-hint invalidation, local variable
  annotations, affected-test propagation, and incremental edge removal.
- The end-to-end Python repository selects exactly its one true test and emits
  `pytest -k test_provenance`: precision `1/1`, recall `1/1`.

### Python import-alias receiver resolution

Verified defect: aliases from
`from models import SessionIdentity as Session` were retained as receiver type
names. Because `Session` does not match the defining `SessionIdentity` owner,
both annotated parameters and constructor assignments became unresolved.

Acceptance evidence:

- Before the fix, the annotated, assigned, and decoy functions in the two-file
  fixture each had zero callees.
- Top-level `import`/`from ... import ...` aliases are now normalized from exact
  tree-sitter `name` and `alias` fields, scoped to their source file.
- The same fixture now resolves both SessionIdentity methods from the annotated
  and assigned functions and exactly `DecoyIdentity::inspect` from the decoy
  function. Dotted module aliases remain exact.
- The graph regression verifies annotation and constructor aliases, module
  aliases, decoy exclusion, affected-test propagation, and incremental stale
  edge removal.
- The end-to-end Python repository selects exactly its one true test and emits
  `pytest -k test_provenance`: precision `1/1`, recall `1/1`.

### Cargo binary subprocess entrypoints

Verified defect: Rust integration tests execute the compiled CLI through
`Command::new(env!("CARGO_BIN_EXE_bitcode"))`, but the process boundary had no
call edge to the binary's `main`. On Bit Code itself, `main` therefore had zero
recorded callers and zero reachable tests despite its subprocess CLI suite.

Acceptance evidence:

- Exact `Command::new(env!("CARGO_BIN_EXE_<target>"))` calls now emit a
  process-entrypoint reference. Arbitrary command strings, unrelated
  environment variables, and non-`env!` macros do not.
- Resolution accepts Cargo's conventional `src/main.rs`, direct
  `src/bin/<target>.rs`, and `src/bin/<target>/main.rs` locations. An exact
  `src/bin` target disambiguates multiple binaries; otherwise multiple
  entrypoints remain unresolved rather than guessed.
- The graph regression covers imported and fully qualified `Command`, false
  positive exclusions, target disambiguation, ambiguous refusal, affected-test
  propagation, and incremental stale-edge removal.
- The end-to-end two-file fixture changes the binary's dispatch function and
  selects exactly its one subprocess test: precision `1/1`, recall `1/1`.
- On Bit Code itself, `main` now has the two real launch callers
  (`run_bitcode_output` and the direct live-collaboration test), and all 27 CLI
  tests are reachable. This closes the entrypoint false negative but remains
  deliberately broad: argument-specific dispatch routes are not yet modeled.
- Custom `[[bin]] path` locations and dynamically constructed executable paths
  remain unresolved because Cargo manifest target metadata is not indexed.

### Python nullable receiver resolution

Verified defect: a Python parameter annotated `SessionIdentity | None` did not
provide a receiver hint, so the checked oracle omitted a test dynamically proven
to execute `SessionIdentity::selected_operation`.

Acceptance evidence:

- PEP 604, parenthesized, and forward-string annotations resolve only when
  exactly one non-null receiver owner remains. `Optional[T]` and
  `Union[T, None]` additionally require import provenance from `typing` or
  `typing_extensions`; module, aliased, `TYPE_CHECKING`, and function-local
  imports are covered.
- Unions containing two different owners remain unresolved, preventing a
  same-method `DecoyIdentity` edge. Custom wrappers named `Optional`/`Union`
  and unrelated dotted receivers remain unresolved rather than borrowing a
  local variable's type hint.
- The focused graph regression covers exact owner selection, affected-test
  propagation, and incremental stale-edge removal.
- The end-to-end CLI regression selects exactly the true nullable test and
  excludes the ambiguous-union decoy test.
- The dynamic oracle improves Python precision/recall from `1.000/0.500` to
  `1.000/1.000`; aggregate precision/recall improves from `0.750/0.750` to
  `0.800/1.000` while Rust results remain unchanged.

## Core Trustworthiness Measurement milestone

The checked oracle in
[`core-trustworthiness-measurement.md`](core-trustworthiness-measurement.md)
materializes one multi-file Rust fixture and one multi-file Python fixture into
disposable Git repositories, applies a mutation, records Bit Code's selected
tests, and runs every test alone. A probe written only by the changed function
provides the dynamic execution set.

After building the exact binary with the required Cargo environment, reproduce
and check the baseline with:

```sh
python3 -m unittest -v tools.test_core_trustworthiness_oracle
python3 tools/core_trustworthiness_oracle.py \
  --bitcode /mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/bitcode
```

Baseline:

| Fixture | TP | FP | FN | TN | Precision | Recall |
|---|---:|---:|---:|---:|---:|---:|
| Rust | 2 | 1 | 0 | 1 | 0.667 | 1.000 |
| Python | 2 | 0 | 0 | 1 | 1.000 | 1.000 |
| Combined | 4 | 1 | 0 | 2 | 0.800 | 1.000 |

The Rust false positive is `rust_cli_unrelated`: it launches the same Cargo
binary as the true CLI test, but its concrete argument cannot reach the changed
branch. The previous Python false negative, `test_python_optional_selected`, is
now selected without selecting the same-method decoy test. The exact sets and
counts are machine-checked against
[`core-trustworthiness-baseline.json`](core-trustworthiness-baseline.json).

### Rust test-attribute discovery precision

Verified defect: Rust test discovery previously marked a function as a test
when any directly preceding attribute's complete text contained `test`.
Consequently, helpers compiled by `#[cfg(test)]`, conditional attributes such
as `#[cfg_attr(...)]`, and unrelated attributes containing that substring
inflated Bit Code's test universe even though Cargo did not list them as tests.

Acceptance evidence:

- Attribute classification now reads the parsed attribute path and recognizes
  only direct `test`, namespaced `::test`, and `rstest` attributes. Attribute
  arguments are not treated as attributes applied to the function.
- The builder regression retains `#[test]`, `#[tokio::test]`, and `#[rstest]`
  while rejecting `#[cfg(test)]`, `#[cfg_attr(...)]`, `#[contest]`, and a doc
  string containing `test`.
- A disposable-Git CLI regression proves a `cfg(test)` helper that calls the
  changed function is absent from both the selected set and skipped count.
- The strict Rust oracle fixture now includes such a helper. Cargo and Bit Code
  still agree on its exact four-test universe, and the checked metrics remain
  `0.667/1.000` for Rust, `1.000/1.000` for Python, and `0.800/1.000` combined.
- On this repository, self-impact now reports 296 tests (`88` impacted plus
  `208` skipped). Cargo lists 277 default-feature tests, including the ignored
  debugpy case, and the oracle adds nine tests, for 286 authoritative cases.
  The overcount fell from 12 to 10; nested/non-collectable Python functions and
  macro-expanded/conditional framework identities remain unproved rather than
  being hidden by the improvement.

### Failure-safe analysis and oracle inputs

Verified defects closed in the failure-safe benchmark foundation:

- `review --since DOES_NOT_EXIST` previously succeeded against an empty
  baseline and reported the whole current graph as added. Baseline refs now
  resolve once to an immutable commit OID; `rev-parse`, NUL-delimited
  `ls-tree`, and every baseline `show` failure propagate. The baseline graph is
  built only from that commit's tracked sources, so a moving symbolic ref
  cannot mix revisions and current-only files are not silently probed at the
  old revision.
- Nested repository roots previously corrupted Git paths: path-valued prefix
  output was whitespace-trimmed, and commands whose output was already relative
  to the project root had that prefix removed a second time. Baseline listing
  now requests full-tree paths before one exact prefix removal, while diff and
  untracked outputs remain project-relative. A leading-space nested-root
  regression prevents silent empty baselines.
- An explicit misspelled `test-impact` node previously printed a warning and
  exited successfully, including when mixed with a valid node. Explicit mode
  now rejects the entire request if any path is unknown.
- Automatic `test-impact` outside Git previously blurred “not a repository”
  with “no changes.” It now exits with an intentional diagnostic requiring
  explicit node paths, which remain supported without Git.
- Source decoding/read errors previously produced a successful partial graph,
  and `analyze` could replace a complete durable graph with it. All graph-build
  callers now receive the read error. The CLI regression proves `analyze`,
  `review`, and `test-impact` fail and preserve the existing graph after a
  source becomes invalid UTF-8.
- The dynamic oracle previously reduced full test paths to leaf names, reused
  one mutated checkout for every test, accepted a Cargo filter that ran zero
  tests, and allowed unbounded child runtime and output. It now independently
  enumerates exact framework ids, explicitly maps full graph ids, checks the
  complete inventory and one-test execution, uses a fresh checkout per dynamic
  test, kills POSIX process groups on timeout/output overflow, and executes a
  private hash-verified Bit Code binary copy.

These changes fail closed but do not yet bound Git subprocess runtime/output or
the product's configured `test-impact --run` child. Those remain required
failure-corpus gates before beta.

### Concurrent analysis workflow correctness

Verified defect: launching `bitcode review .` and `bitcode test-impact .`
concurrently against the same working tree produced incomplete output from
both commands after their graph-building preambles. Two mechanisms were
identified: every graph build ran destructive journal recovery over the
shared `.bitcode/transactions` directory with no locking (one process could
roll back another's in-flight commit or remove `.bitcode` mid-transaction),
and concurrent `git diff` invocations contended on `.git/index.lock` with
fail-closed error handling.

Acceptance evidence:

- Writers hold an exclusive `flock` on the `.bitcode` directory file
  descriptor for the entire commit critical section. Read-only analysis
  recovers non-blocking and never touches a journal whose owning process is
  alive; only dead-owner and abandoned journals are reclaimed. Directory-fd
  locking preserves the invariant that a completed commit removes `.bitcode`
  entirely, and acquisition re-checks inode identity so a lock on an
  unlinked directory is never trusted.
- All Git subprocesses set `GIT_OPTIONAL_LOCKS=0`, eliminating opportunistic
  index-refresh writes from read-only analysis.
- Unit regressions cover live-owner preservation, dead-owner reclamation,
  and lock-held skip (`read_only_recovery_preserves_a_live_writers_journal`,
  `read_only_recovery_reclaims_a_dead_owners_journal`,
  `read_only_recovery_skips_while_the_journal_lock_is_held`). The
  deterministic CLI regression
  `concurrent_review_and_test_impact_produce_complete_output` runs five
  rounds of two simultaneous analysis processes against one working tree and
  requires byte-identical complete output from every run.
- The seven pre-existing recovery regressions pass unmodified.

## Transaction recoverability proof

Verified gap: journal recovery had no fault-injection evidence. Interruption
at each durable transition now has a deterministic regression.

Acceptance evidence:

- Disk-state matrix in `project::source` unit tests: empty journal directory
  removal, staging without a manifest, a torn manifest (fail-closed error
  that modifies nothing and is stable on retry), partial application with
  k-of-n renames, full application without the committed marker (all-old),
  and a committed journal with interrupted cleanup (all-new). Each
  destructive row also asserts a second recovery pass is an idempotent
  no-op.
- Real-crash proof through the actual binary: `BITCODE_FAULT_EXIT` aborts
  `forge` at `after-staging`, `after-manifest`, `mid-apply`, and
  `pre-cleanup`; the four `crash_*` CLI regressions assert the process died
  at the injection point, a later analysis command recovers the journal, and
  the project lands all-old before the COMMITTED marker and all-new after
  it, including a loadable committed graph.

## Bounded product subprocesses

Verified defect: Git subprocesses and configured `test-impact --run`
children ran with no timeout, output cap, or process-group handling.

Acceptance evidence:

- One shared engine (`project::process`) runs every child in its own process
  group with a 25 ms supervision loop and SIGKILLs the whole tree on
  timeout, cancellation, or overflow. The candidate validator's regressions
  (`timed_out_command_is_terminated` and the bounded-diagnostics suite) now
  guard this shared engine.
- Git commands are bounded at 120 s and 16 MiB with
  `GIT_OPTIONAL_LOCKS=0`; captured output is treated as data — overflow
  kills and errors rather than ever returning truncated bytes. Timeouts
  produce an explicit classification
  (`analysis_classifies_a_hung_git_subprocess`).
- `--run` children stream live to the terminal with inherited stdin, under
  configurable `tests.run_timeout_seconds` (default 1800) and
  `tests.run_max_output_bytes` (default 8 MiB). Regressions prove a
  timed-out runner's background grandchild is killed
  (`test_impact_run_kills_a_timed_out_process_tree`) and an overflowing
  runner is killed with a classified error while bitcode's own output stays
  bounded (`test_impact_run_kills_a_child_exceeding_the_output_budget`).
- Remaining unbounded spawns are named residuals: the toy debugger's
  `python_tracer` and the DAP adapter launch (per-request timeout only).

## Test identity, argument routes, and call-semantics closures

Verified defects, closed together because all three change the graph and
the trustworthiness oracle's declared test universe, coordinated into one
baseline re-record (see "Core Trustworthiness Measurement milestone" below
for the refreshed numbers):

- **CLI argument-route precision** (measured `0.667`, one false positive
  `rust_cli_unrelated`): closed by failure-closed route modeling — see
  `crates/aether-builder/src/mapper.rs` (`RouteEvidence`, dispatch-arm
  tagging) and `crates/aether-graph/src/impact.rs` (context-aware BFS
  pruning). Any unprovable evidence anywhere leaves the graph and BFS
  byte-identical to before, so recall is preserved by construction.
- **Framework test identity** (measured overcount of 10: 296 reported vs 286
  authoritative): closed by matching `is_test` to real framework collection —
  Python requires a `test*.py`/`*_test.py` file and module-level-or-method
  position; Rust evaluates `#[cfg(...)]` against the analysis host; nested
  definitions are scoped to their enclosing function, never flattened into
  module scope. `test-impact`'s "skipped" count is a real set difference; test
  selections are path-sorted for deterministic output. On this tree: Python's
  inventory matches exactly (27 = 27). Rust's raw counts still differ (322
  graph `is_test` nodes vs 312 `cargo test --workspace -- --list`), but a
  full by-hand reconciliation (see `core-trustworthiness-measurement.md`)
  matches 312 of 322 graph identities 1:1 to a distinct real Cargo test
  under the already-known `mod`-flattening path-naming residual; the
  remaining 10 are exactly the deliberately fail-open
  `#[cfg(feature = "...")]` tests (`live-providers`, `gui`) this milestone's
  design intentionally keeps marked rather than risk a false negative under
  a differently-configured build. Zero unexplained phantom or missing test
  identities remain.
- **Unmeasured call semantics** (custom Cargo binary paths, RAII/`Drop`):
  closed by reading `[[bin]]` overrides and `[package].name` from the root
  manifest (fixing a latent ambiguity the second declared binary exposed:
  the conventional `src/main.rs` target had no real name to match against),
  and by a narrow, recall-safe RAII model — a resolved `Self`-returning
  constructor call for a `Drop`-implementing type also calls that type's
  `drop`.
- **Oracle breadth**: the trustworthiness fixtures gained two Rust cases
  (`rust_custom_bin_selected`, `rust_raii_drop_selected` — dynamic proof for
  the two models above) and two Python cases
  (`test_python_cross_module_selected`, a three-hop cross-file chain;
  `test_python_third_party_decoy`, a third same-named-method decoy). All
  four are dynamically verified true positives/negatives with `precision
  1.000`/`recall 1.000` in the re-recorded baseline.

## Representative dynamic comparison

Closed: [`docs/core-representative-mutations.md`](core-representative-mutations.md)
extends the trustworthiness oracle's per-test dynamic-proof technique to one
cached real repository (`click-8.4.1`), for a small hand-declared mutation
and its real, unmodified test callers — reproducible via
`tools/core_representative_mutations.py` and checked against
[`core-representative-mutations.json`](core-representative-mutations.json).
This is evidence the extended dynamic comparison exists and runs, not a
claim of general accuracy across Click's behavior; Rust representative
mutations remain out of scope (compiling a mutation-per-checkout across
representative-sized crates is not honest to run repeatedly on this host).

The measurement found a real, previously unmeasured defect, recorded below
as gap 11 rather than silently accepted.

## Prioritized open gaps

1. **Closed — measured CLI precision defect.** See "Test identity, argument
   routes, and call-semantics closures" above.
2. **Closed — framework test identity.** See "Test identity, argument
   routes, and call-semantics closures" above.
3. **Closed — concurrent analysis isolation.** See "Concurrent analysis
   workflow correctness" above: journal locking with dead-owner-only
   read-only recovery, `GIT_OPTIONAL_LOCKS=0`, and a five-round dual-process
   CLI regression requiring complete byte-identical output.
4. **Closed — unmeasured call semantics.** See "Test identity, argument
   routes, and call-semantics closures" above.
5. **Closed — oracle breadth.** See "Test identity, argument routes, and
   call-semantics closures" and "Representative dynamic comparison" above.
6. **Closed — recoverability proof.** See "Transaction recoverability proof"
   above: a deterministic disk-state matrix over every journal transition
   plus real `BITCODE_FAULT_EXIT` crash injection through the binary, with
   all-old/all-new verification and idempotent re-recovery.
7. **P0 — representative repositories.** Record cold/incremental indexing,
   call-edge accuracy, impact latency, test-selection accuracy, and memory on at
   least three real Rust/Python repositories without manual repair.
8. **Closed — subprocess bounds.** See "Bounded product subprocesses" above:
   shared process-group engine with timeout/output classification for Git
   and configured `--run` children; debugger/DAP spawns remain named
   residuals.
9. **P1 — interactive latency.** Measure edit-to-graph, edit-to-diagnostic, and
   navigation latency under sustained edits.
10. **P1 — agent outcome metrics.** Track accepted patches, validation catches,
   rejected patches, rollback success, and time-to-safe-commit.
11. **P1 — fixture-mediated polymorphic dispatch, newly measured.** On real
   Click code, a test reaching a `Group.invoke`-vs-`Command.invoke`
   polymorphic dispatch only through an untyped pytest fixture parameter
   (`runner`) and Click's own internal `Command.main` indirection is not
   selected: two of three declared representative-mutation cases are false
   negatives (recall `0.000` on that one mutation; see
   [`core-representative-mutations.md`](core-representative-mutations.md)).
   No receiver-type inference mechanism currently reaches through an
   untyped fixture parameter and a same-named unqualified `self.invoke`
   dispatch. Lower priority than the original P0 set because it is narrow
   (one dispatch shape) and newly discovered, not a regression.

Bit Code's potential advantage is not generic semantic search. It is one local,
inspectable model connecting code identity, predicted impact, selected tests,
validated projection, and recoverable commit. That advantage is unproven until
the P0 measurements above show better consequence prediction without hiding
false negatives or unacceptable over-selection.
