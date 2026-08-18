# Core workflow gap analysis

Last updated: 2026-08-18

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
7. **Closed (scoped) — representative repositories.** See
   `docs/core-representative-benchmark.md`'s "Result" section: the
   precommitted policy's first recorded observation passes —
   `beta_pass: true`, zero failed checks, on six real Rust/Python
   repositories (three each) with zero manual repair, perfect precision/
   recall on all 40 declared call-edge cases, one unique artifact and
   semantic digest per repository across five runs (determinism), and every
   repository within its latency/RSS ceiling (sum of medians 56.8s of a
   180s budget). Genuine cold-cache indexing, one-file incremental latency,
   impact-query latency, and comprehensive dynamic test-selection accuracy
   over the representative corpus remain open — the one declared
   representative-mutation case (gap 11) is a narrow start, not that
   evidence.
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
12. **P0 — corrected capable-model protocol authors working graph plans, but
   the corpus policy still fails.** The precommitted corrected
   `qwen2.5-coder:1.5b` rerun in
   [`authoring-cost.md`](authoring-cost.md) grammar-constrained the plan
   envelope, supplied bounded current node source to graph prompts, and
   replaced reference-tree equality with declared semantic checks plus
   impacted tests. It produced 2/8 semantically verified graph-arm successes
   (Python replace and delete) and 0/8 text-arm successes. None of the seven
   completed responses had a missing or extra envelope field; across 20
   attempts, 13 hit the bounded provider timeout, four used the wrong
   `on_failure` value, one failed plan validation, and two passed after repair.
   Common successes remained 0 against the required 4, and three text-arm
   token recoveries timed out, so the full-corpus token threshold is
   unevaluable and the policy result is **FAIL**. On the five tasks with
   complete paired first-attempt counts, text used 1,765 tokens versus 1,042
   graph tokens, a 41.0% context reduction; the graph arm's exact eight-task
   first-attempt total was 1,775. The preceding 0/8 controls were
   harness-limited by unconstrained envelopes, missing node source, and
   tree-equality scoring. The corrected run establishes some fully local
   graph-addressed authoring, but not the breadth, common success, or provider
   reliability required to close this P0 gap.
13. **P0 — conservative semantic rename can reject a valid exact node on
   repository-wide name collisions.** An independent canonical-plan probe of
   `crate::sample-project::calc::greet` failed closed because identifiers named
   `greet` in `crates/aether-builder/src/lib.rs` were not graph-proven call
   sites. This was not a model-observation result: that arm's generated plans
   failed shape validation before execution. Rejecting is safer than rewriting
   unproven syntax, but `rename_node` is not generally usable for otherwise
   unambiguous node paths until the graph records call-site-level provenance or
   lowering can prove a narrower lexical scope.
14. **Closed — rollback preserves base existence across plan steps.** The v1
   and v2 executors now share a first-touch `BaseExistenceLedger`, so deleting a
   tracked path and recreating it later cannot reclassify that path as base-new.
   The regression
   `rollback_plan_restores_base_file_deleted_then_recreated_across_steps`
   deletes a tracked file in step 1, recreates it in step 3, forces
   `rollback_plan` in step 4, and verifies the original `base_commit` bytes.
   The precommitted P4 corpus now includes
   `delete-recreate-across-steps-then-fail`; the clean-source observation in
   [`plan-executor-observation.json`](plan-executor-observation.json) records
   all four rollback plans with zero dirty worktrees and zero tree mismatches.
   Its deliberately broken P4 binary removes the recreated tracked path and is
   killed by the policy with one dirty worktree, binding the claim to the
   defect rather than only to the three earlier rollback shapes.
15. **P1 — first live-model `bitcode do` run: node over-selection, a truncated
   authoring response, and a repair prompt that didn't attribute failure to
   the node it named.** The first end-to-end run against a local
   `qwen2.5-coder:1.5b` produced 0/3 working plans, but the harness itself
   behaved correctly — provider escalation, the repair loop, and precondition
   checks all ran as designed, and nothing reached the tree. Three defects,
   fixed here in priority order:
   - Node over-selection was the primary cause: concept search returned the
     correct node (`greet`) at 0.17 and four unrelated nodes at
     0.09/0.09/0.05/0.05, and the default top-5 selection put every one of
     them into the schema enum as a legal edit target; all three attempts
     addressed one of the noise nodes instead of `greet`. Fixed three ways in
     `crates/aether-app/src/project/commands/author.rs`: the default
     selection dropped from 5 to 3 (`TOP_K`); a named relative-score floor
     (`NODE_SCORE_FLOOR_RATIO = 0.5`) now discards any hit scoring below half
     the top hit's score; and `bitcode do` gained `--nodes
     <path>[,<path>...]` to bypass search entirely and pin exact node paths,
     which is what makes the command testable in isolation — the question
     "can the model do this job given only the right node?" is the one
     `docs/authoring-cost.md` already answered yes to, and there was
     previously no way to ask it through `bitcode do` itself.
   - Attempt 1 failed with `EOF while parsing a string at line 22 column
     1216` — the response was truncated mid-string, not malformed. The
     `Prompt::new` default of 1024 `max_tokens` looked too low for a
     schema-constrained plan step that carries a full replacement
     `Node.source`, so `build_prompt` initially raised it to `8_192`. **This
     diagnosis was corrected in a follow-up pass — see "Correction" below:**
     1216 characters of JSON is well under 1024 tokens' worth of output, so
     the truncation was never actually a `max_tokens` ceiling; `max_tokens`
     is now set back to `1024`, derived from measured local throughput
     instead, and the real cause (no request timeout) is fixed separately.
   - The repair prompt said an edit failed but never said the node choice
     itself might be wrong: attempt 3 kept the same wrong node across
     repairs and only swapped the edit's `operation`. `build_prompt` now
     scans the prior diagnostic for one of the offered node paths; when a
     diagnostic names a node, the repair prompt restates the intent, pins
     that node's failure to it explicitly ("the failure is attributed to
     node `...`"), and tells the model to reconsider it against the full
     node list rather than only changing the operation.

   These are code fixes verified by unit tests
   (`crates/aether-app/src/project/commands/author.rs::tests`) and the full
   `cargo test --workspace` and Python measurement-harness suites; none of
   the three touch `planfile`/`executor.rs`, so they don't change P1-P5
   oracle behavior. The oracle was rerun after this pass and PASSed (every
   mutation case still killed; see
   [`plan-executor-observation.json`](plan-executor-observation.json)). A
   live local-model rerun of `bitcode do` itself is what surfaced the
   corrections below, so this gap stays open rather than closed.

   **Correction, same run, after measuring local throughput.** Timing the
   local model directly (`/api/generate`, `eval_count`/`eval_duration`)
   measured `qwen2.5-coder:1.5b` on this machine at **1.49 tok/s** (95
   tokens in 63.7s) — slow enough that the `max_tokens = 8_192` set above is
   a roughly 91-minute ceiling per attempt, not a reasonable one. Three
   further fixes:
   - `max_tokens` for `TaskClass::Authoring` is back to `1024`
     (`crates/aether-app/src/project/commands/author.rs`), now derived from
     throughput rather than the truncation guess: at 1.49 tok/s, 1024 tokens
     is roughly an 11-minute ceiling (1024 / 1.49 ≈ 687s), and the original
     truncation (~1216 characters) was well under what 1024 tokens of JSON
     produces, confirming it was never a `max_tokens` problem. The comment
     at the call site notes the bound is throughput-derived and should be
     revisited if the local model changes.
   - The real cause of the unbounded attempt was that `OllamaProvider` set
     no request timeout at all — one attempt ran past an hour uncut.
     `crates/aether-ai/src/ollama.rs` now applies a per-request timeout via
     `OLLAMA_TIMEOUT_SECS` (default 600s, parsed by the pure, unit-tested
     `parse_timeout_secs`, falling back on unset/zero/unparseable values). A
     timeout surfaces through the same `reqwest` error path every other
     transport failure already does — `AiError::Transport`, which
     `bitcode do`'s attempt loop already treats as a decline and escalates
     past — so no new error-handling path was needed, only the missing
     deadline.
   - A further live attempt failed with "command check missing a
     non-empty run": the model emitted a `command` check with an empty
     `run` string despite the existing preference for `graph.*` checks.
     `check_schema` now sets `"run": {"type": "string", "minLength": 1}` so
     this is rejected at the grammar level instead of reaching
     `convert_check`, and the prompt in `build_prompt` was strengthened from
     a soft preference to an explicit "Do NOT use a `command` check" plus
     "the checks array may be left empty — do this unless you have a
     specific `graph.*` check in mind," since `tests.impacted` is added
     automatically either way.

   Also, each attempt now prints how long the model call took
   (`  <provider> responded in <N>s`, or `<N>s elapsed` on a decline/timeout)
   so a slow local model is visibly slow instead of indistinguishable from a
   hang.

   Investigated per this run but not fixed: the node every attempt kept
   targeting was a method inside a Python class in `tools/`, and it had zero
   resolved callers even before any edit. Two separate questions, both
   checked directly against the built graph rather than assumed:
   - **Is it a nested class?** No — there are no nested classes (a `class`
     defined inside another `class` body) anywhere in `tools/`; every one of
     its 12 `Type` nodes is a top-level class. Auditing every class method
     under `tools/` found 62 total, of which 59 have zero static callers, and
     every zero-caller method checked is either a `unittest.TestCase` method
     invoked by test discovery (`setUp`/`test_*`) or
     `RejectRedirects.redirect_request`, an `HTTPRedirectHandler` override
     invoked by `urllib` through polymorphic dispatch — both are a framework
     calling a method by convention/reflection rather than a static call site
     in this codebase, the same category as the already-open
     polymorphic-dispatch gap 11. This alone explains the "resolved 0 times"
     observation: node over-selection above aimed the model at
     framework-dispatched methods because they scored as noise, not because
     class methods are unresolvable.
   - **Do nested-class methods resolve at all, in general?** A same-file
     synthetic fixture (`Outer` containing `class Inner` containing `def
     helper`, called from a sibling method as `Outer.Inner().helper()`)
     resolved correctly, including the arbitrary-depth path
     `crate::main::Outer::Inner::helper` that `collect_defs`'s recursive
     `Scope` walk in `crates/aether-builder/src/mapper.rs` builds generically
     for any nesting depth. But a cross-file variant — two files each
     defining their own `Outer`/`Other` class containing a same-named nested
     `class Inner` with a same-named `def helper` — resolved to zero `Calls`
     edges on *both* sides, because `qualifier_matches_owner` in
     `crates/aether-builder/src/sync.rs` matches a receiver hint against only
     the trailing type-name segment of the candidate's owner path, and
     `select_candidate`'s `only_candidate` fails closed on the resulting
     ambiguity. Repeating the same experiment with two **flat**, non-nested
     classes (`class Widget` with `def helper` in each of two files)
     reproduced the identical zero-edge outcome, which rules out nesting as
     the cause: this is the general same-name-across-files ambiguity
     already covered by the P0 "measure resolved/unresolved call edges" gap
     in the competitive-baseline table and the same fail-closed posture as
     gap 13, not a defect specific to nested classes, and not worth a new
     gap number on its own.
16. **P1 — the plan-executor oracle never checked a passing plan's claims
   against ground truth.** The first non-dry `plan run --authored` through
   the real CLI committed an edit correctly — `git status --porcelain`
   showed the file modified, the bytes were right — but the report claimed
   `"committed": true` / `"description": "committed through the final
   step"` while `git log` showed HEAD unchanged and the edit sitting
   uncommitted (expected: this codebase's "commit" has always meant
   "durably write to the real tree," never a git commit — verified via
   `executor.rs`'s "only commit to the real tree" doc comment, dated to the
   executor's original 2026-08-10 commit, and via `rollback_to_base`'s use
   of `git_checkout_paths(root, &plan.base_commit, ...)`, which targets the
   plan's original base commit directly and never depends on bitcode having
   made one of its own). Not a defect in the report or in `--authored`
   (confirmed identical behavior in an isolated worktree with `--authored`
   absent) — but auditing why the precommitted oracle would have passed a
   binary with a genuinely false "committed" claim found a real one:
   - `measure_p1` (`P1_dry_equals_real`) compares `outcome_projection(dry
     report) == outcome_projection(real report)` — two self-reported
     artifacts compared to each other, never against git or the
     filesystem. Worse, `outcome_projection` didn't carry `committed` at
     all, so even a dry/real disagreement on that one field specifically
     would have passed through unprojected.
   - `measure_p4` (`P4_rollback_fidelity`) does check real ground truth —
     `git status --porcelain` and `git write-tree` against the base tree —
     but only on plans engineered to fail and roll back.
   - No case anywhere in P1-P5 ran a plan to a **passing** conclusion and
     then verified the report's claims against what git and the filesystem
     independently show. A binary where `commit_project_writes` silently
     no-op'd while everything else kept reporting success would have
     sailed through.

   Fixed by adding `P6_commit_ground_truth` to `plan-executor-v1`
   (`docs/plan-executor-policy.json`, `tools/plan_executor_oracle.py`):
   three cases (substitute/create/delete), each a plan expected to
   genuinely pass, run for real. Ground truth is file bytes read directly
   from disk plus `git status --porcelain`; `.bitcode/reports` is read
   exactly once per case, solely to cross-check the report's own claim
   against the ground truth already established above it — that
   cross-check can add a violation but can never suppress one. Mutation-tested
   the same way as P1-P5: a wrapper-script mutant lets the real binary run
   to completion (so the report is genuine, untampered output) and then
   reverts the edited path to `base_commit` content behind its back —
   the literal external observation of "commit silently no-ops while the
   surrounding code still reports success." Confirmed killed
   (`content_mismatches=3` and `report_mismatches=3`, both independently),
   confirmed the real binary passes clean (`0`/`0`), and confirmed the
   full mutation-adequacy sweep across all six properties now completes
   with zero survivors.

   **Worth preserving as a control — a near-miss of the same error class
   this gap is about.** The first version of `measure_p6_ground_truth`
   additionally compared `git write-tree` output against the base tree,
   mirroring `measure_p4`. It failed immediately against the *real,
   correct* binary: `git write-tree` reads the index, and bitcode never
   runs `git add`, so the written tree is identical to `base_commit`'s
   regardless of whether the working tree actually changed — a vacuous
   ground-truth check with exactly the shape of the defect this corpus
   exists to catch, just one level removed, and it was caught only because
   the new case was run against the real binary before being trusted.
   Dropped in favor of `git status --porcelain` alone, which genuinely
   reflects the unstaged real-tree edit.

   **Follow-up: `outcome_projection` drops one other field that carries a
   real guarantee.** Besides `committed` (legitimately excluded from P1's
   comparison — dry mode never commits, so it differs by design, same as
   `final_state.dry_run`), step-level `writes`
   (`before_bytes`/`before_sha256`/`after_bytes`/`after_sha256`) is
   computed by the identical `fingerprint_writes()` call in `executor.rs`
   regardless of dry/real/pass/fail on a passing step — nothing about it
   should legitimately differ between modes, yet it was dropped from the
   comparison with no such justification. Added `"writes":
   step.get("writes")` to `outcome_projection`. Mutation-tested with a
   standalone wrapper mutant that perturbs a dry-run plan's edit content
   (a comment appended to created content, not touching the symbol a later
   check depends on) before delegating to the real binary, so the dry
   pass's fingerprints genuinely reflect different bytes than the real
   pass's — same technique as every other mutant here, built to verify
   this specific case rather than added to the precommitted sweep. Result:
   **P1's own corpus is structurally insensitive to this**, for a reason
   unrelated to the fix. `base_plan()` hardcodes every P1 case to
   `plan_version: 1`, and `executor.rs::run_plan` only computes
   `write_fingerprints` in `run_plan_v2` (`if plan.plan_version == 2`); the
   v1 path sets `write_fingerprints: None` at all four `StepOutcome`
   construction sites, so `"writes"` is absent from every P1 report
   regardless of mode, and the mutated dry run and the correct real run
   both project to `writes: None`. The comparison logic itself is correct
   and covered directly — `test_outcome_projection_compares_write_fingerprints`
   proves it flags a genuine before/after-hash divergence when the reports
   actually carry fingerprint data — but nothing in the current P1 corpus
   exercises it end to end. Reported as found rather than forced: making
   P1 sensitive to this in practice would mean migrating at least one P1
   case to `plan_version: 2`, a corpus change with its own consequences,
   not attempted here.

17. **Closed — `bitcode context` emitted a schema `plan run` would reject.**
    `authoring_context::step_schema` is the flat, all-operation-fields-required
    shape `bitcode do`'s local model uses, which `author::convert_edit`/
    `convert_check` translate into real Plan Format v2 before it ever reaches a
    plan file. `bitcode context` handed that same schema, unfiltered, to an
    external model that writes a plan file directly with no translation layer.
    Confirmed before any fix, with a test: a step built to satisfy the emitted
    schema exactly (`operation` plus all four operation fields; `run`/
    `expect_exit` on every check) is rejected by the real loader —
    `operation` isn't even in `planfile::schema::Edit`'s known-field list, so
    it fails before the discriminator check ever runs, and a `run`/
    `expect_exit` field on `graph.node_exists` hits that variant's
    `deny_unknown_fields`
    (`crates/aether-app/src/project/commands/context_cmd.rs`,
    `a_step_satisfying_the_flat_local_authoring_schema_is_rejected_by_load_plan`).
    So the entire external-authoring loop the command exists to support was
    unusable end to end: paste `context`'s schema to any chat model, follow it
    exactly, and `plan run --authored` rejects the result.

    Fixed by adding `authoring_context::plan_schema()` — a discriminated-union
    (`oneOf`) shape matching `planfile::schema`'s actual `Edit`/`Check`
    deserializers exactly — and emitting that from `context_cmd::build_output`
    instead of the flat local-authoring schema. `bitcode do`'s local path
    keeps `step_schema` unchanged; `author.rs` was not touched. A second test
    (`a_step_satisfying_plan_schema_round_trips_through_load_plan`) proves a
    step satisfying `plan_schema` loads successfully, closing the loop the
    first test opened. `load_plan` is now `pub(crate)` so both tests drive the
    real loader directly instead of trusting a description of its behavior.
    `context_cmd.rs` also gained coverage for the `--json` requirement, both
    `SelectionError` paths, and the full top-level output shape (previously
    one test asserting almost nothing). The P1-P6 oracle was rerun against a
    release build of this fix and passed clean.

18. **Closed — the Python measurement-harness suite's canonical-edit checks
    failed against a live-mutated `sample-project/calc.py`; the fixture was
    wrong, not the harness. Root cause was a matched two-commit pair, and an
    incomplete first fix reported success by verifying only half of it.**
    `tools/test_authoring_task_check.py::test_all_eight_canonical_edits_pass_semantic_verification`
    reads whatever is currently on disk at `sample-project/calc.py` as its
    `original` baseline (not a pinned snapshot) and mechanically applies each
    of the eight canonical tasks' `match`/`replace` text edits
    (`tools/plan_executor_oracle.py::authoring_edits`) to it before running
    `tools/authoring_task_check.py`'s semantic checks. Two commits from the
    same live run drifted that fixture, three days apart from every other
    commit in this pass: `fe0f476` ("Uppercase greet's return value",
    2026-08-15, authored externally via `bitcode context`/`plan run
    --authored` — the loop gap 17 above concerns) permanently changed
    `greet`'s body from `return hello(name)` to `return
    hello(name).upper()`, and `44e7fae` ("Add test for greet uppercase",
    minutes later, same run) added `sample-project/test_calc.py` — a file
    that did not exist before that commit — asserting the new uppercase
    behavior. The `python-rename` task's check still assumed the original
    body — rename `greet` to `welcome_greeting`, then expect un-uppercased
    behavior (`upper=False`) — so it failed with "welcome_greeting has wrong
    behavior" against the drifted fixture.

    **Root cause, not just the trigger:** the eight canonical tasks
    (`docs/authoring-cost-policy.json`'s corpus) are each defined as an
    independent one-step transformation of the *same pristine* starting
    file — `python-replace` is literally "change `greet` so it returns
    `hello(name).upper()`". A live, permanent uppercase of `greet` outside
    the harness silently satisfies that task's precondition for free while
    breaking every other task that assumes the original lowercase body, and
    a live, permanent test asserting the new behavior turns that drift into
    a second, independent source of failure the moment the fixture is
    corrected back. Confirmed against
    `git show fd74d3468b65fddce6e853103aa9368767a0c90d:sample-project/calc.py`
    (the exact `clean source commit` `docs/authoring-cost.md`'s precommitted
    corrected method cites) that `greet` was lowercase at measurement time
    and that `sample-project/test_calc.py` did not exist at all at that
    commit (`git show <commit>:sample-project/test_calc.py` — "exists on
    disk, but not in" that commit), and every `docs/authoring-cost*` commit
    predates `fe0f476`/`44e7fae` by a day — the corpus really was measured
    against the pristine fixture both commits later mutated out from under
    it. `docs/authoring-cost.md`'s own Observation table records the graph
    arm's `python-replace` task as **passed** — direct evidence a real
    uppercase transformation was authored and verified starting from
    lowercase `greet`, which is only possible if the fixture was still
    pristine at that time.

    **Decision:** restore both files to their state at
    `fd74d3468b65fddce6e853103aa9368767a0c90d` — `sample-project/calc.py`'s
    `greet` back to `return hello(name)`, and `sample-project/test_calc.py`
    deleted outright, since it never existed at that commit — reverting both
    halves of the live run's drift rather than changing
    `tools/authoring_task_check.py`'s expectations. The corpus and its
    already-measured, precommitted results depend on the pristine fixture;
    changing the harness to match the drifted files would have silently
    invalidated `docs/authoring-cost.md`'s Observation table (in particular
    the one recorded `python-replace` pass) without changing the doc.

    **The first attempt at this fix (same pass, hours earlier) reverted only
    `calc.py` and reported the gate as closed. It was incomplete, and the way
    it was verified is why that went unnoticed:** the fix was checked by
    re-running the one previously-failing test
    (`tools/test_authoring_task_check.py`, plus `python3 -m pytest -q
    tools`), which only exercises files under `tools/` — never
    `sample-project/test_calc.py`, which pytest's default discovery from the
    repository root does collect but a `tools`-scoped invocation does not.
    `sample-project/test_calc.py::GreetTests::test_returns_uppercase` — the
    other half of the same drift, added by `44e7fae` — kept asserting
    uppercase against the now-reverted lowercase `greet` and failed on a
    tree the gate had just reported clean. Confirmed reproducible with the
    exact command a real authored run's declared checks would use together —
    `python3 -m pytest -q sample-project tools/test_authoring_task_check.py`
    — which shows `1 failed, 3 passed` against the fixture in that
    intermediate state. The lesson generalizes past this one gap: verifying
    a fixture-drift fix by re-running the test that happened to be reported
    failing is not the same as verifying the fixture, and the correct check
    is always the full suite from the repository root
    (`python3 -m pytest -q`, no path restriction), not a scoped rerun of
    whatever was already known to be broken.

    **This also explains a masked third symptom, fixed by the same
    `calc.py` change:** `tools/test_authoring_task_check.py`'s canonical-edit
    test is a single unparameterized loop over all eight cases with no
    `subTest`, so it stops at the *first* failure (`python-rename`, second
    in iteration order) — `python-delete` and `python-insert` were never
    reached in the originally reported failure. `python-insert`'s check also
    asserts `greet` returns un-uppercased
    (`call_with_probe(namespace, "greet", upper=False)` in
    `authoring_task_check.py`), and `python-delete`'s mechanical `match`
    text is a *prefix* of the drifted body (`"def greet(name):\n    return
    hello(name)"` matches inside `"...hello(name).upper()"`), so deleting it
    would have left a syntactically invalid stray `.upper()` behind — both
    would have failed too, once reached.

    **Verified both ways this time:** the full Python suite from the
    repository root (`python3 -m pytest -q`, no path restriction) passes 63
    of 63 with both files restored, and the reproduction command above
    (`python3 -m pytest -q sample-project tools/test_authoring_task_check.py`)
    now collects 3 items (only `tools/test_authoring_task_check.py`'s —
    `sample-project` has no test file left to collect) with 0 failures.

19. **P1 — open measurement gap, made unmissable rather than fixed:
    `docs/authoring-cost-policy.json` is `graph-edit-authoring-cost-v3`
    (`schema_version` 3) but `docs/authoring-cost-observation.json` is still
    `graph-edit-authoring-cost-v2` (`schema_version` 2).** The v3 policy has
    never actually been measured against a live model — the checked-in
    observation predates it. Nothing previously compared the two files, so
    the mismatch was only visible to a human who happened to diff their
    `policy_id` fields by hand; `docs/authoring-cost.md`'s narrative
    describing the v2 result could otherwise be read as describing v3.
    Deliberately not re-run here: the measurement is a live-model corpus that
    takes hours at this machine's measured 1.49 tok/s throughput (see gap 15),
    and re-running it was explicitly out of scope for this pass.

    Instead, added `validate_authoring_observation_matches_policy(policy,
    observation)` to `tools/plan_executor_oracle.py`, wired into
    `run_authoring_cost` as the first check — before the Ollama reachability
    check, before touching git status, before any of the expensive work —
    so a future `--authoring-cost` invocation with a stale checked-in
    observation fails immediately with a message naming both
    `policy.policy_id` and `observation.policy_id`, instead of silently
    overwriting (or coexisting beside) data measured against a different
    policy version. Two tests in `tools/test_plan_executor_oracle.py`:
    `test_authoring_observation_validation_refuses_a_policy_id_mismatch`
    covers the function in isolation (matching pair passes, deliberately
    stale pair raises with both ids in the message), and
    `test_checked_in_authoring_observation_is_stale_against_the_current_policy`
    asserts the *actual* checked-in v2 observation is currently rejected
    against the *actual* checked-in v3 policy — a test written to fail on
    purpose today and start passing again only once someone re-runs the v3
    measurement for real, so the repository's own test suite states the gap
    rather than quietly tolerating it.

20. **Closed — the GUI Author tab's read-only clipboard action could serve a
    stale build, and its default node selection worked against gap 15's
    score floor.** Found by manual click-through of the new Author tab
    after `cargo test --workspace`, clippy `-D warnings` on default/
    `live-providers`/`gui`, `cargo fmt`, and the Python suite had all passed
    clean — none of those gates exercise clicking the actual buttons.
    Two defects:
    - **Copy Context JSON could serve a previous click's result.**
      `copy_author_context_json` (`crates/aether-app/src/app.rs`) already
      recomputed `checked`/`pinned` from live checkbox state on every call —
      that was never wrong. The defect was the gate around it:
      `pub(crate) fn author_busy()` OR'd together all four Author-tab
      background operations (search, local-model run, run-authored, *and*
      copy-context-json), so a click on "Copy context JSON" while any of
      the other three was still in flight — most plausibly a Search or a
      full-repository build taking longer than expected — was silently
      dropped: the button stayed disabled, `copy_author_context_json` never
      ran a second time, and the clipboard kept whatever an earlier click
      had put there. Two unchecking/rechecking passes that both landed in
      that window looked exactly like "the button is serving a cached
      result rather than rebuilding" — including an identical
      `plan_skeleton.plan_id`, which is only possible if the second click's
      `build_context_json` call, and therefore its `generate_plan_id()`,
      never actually ran. Fixed by no longer gating Search or Copy Context
      JSON on `author_busy()` at all: both are read-only, so a repeated
      click now always supersedes whatever is still in flight instead of
      being swallowed — the previous `oneshot::Receiver` is simply dropped,
      which makes the corresponding orphaned background thread's
      `tx.send(...)` a no-op, so only the latest click's result is ever
      applied. `author_busy()` itself is kept, but only for gating
      `open_project`/`reload_project` (swapping the workspace root out from
      under an in-flight background thread that captured the old root by
      value). A new `author_write_busy()` — `author_run_rx.is_some() ||
      author_run_authored_rx.is_some()` — gates Run and Run-authored
      specifically, since those two, unlike Search/Copy, write to the real
      tree and must never run concurrently against the same project.
      Regression test:
      `build_context_json_with_different_pinned_nodes_differs_in_nodes_and_plan_id`
      in `crates/aether-app/src/project/commands/context_cmd.rs` calls
      `build_context_json` twice with different pinned node sets and
      asserts both `nodes` and `plan_skeleton.plan_id` differ between the
      two calls — pinning the exact invariant the GUI's button depends on.
    - **Search defaulted every hit to checked.** `run_author_search`
      (`crates/aether-app/src/app.rs`) set `selected: true` on every result
      from `search_nodes_for_authoring`. Gap 15 introduced `TOP_K = 3` and
      `NODE_SCORE_FLOOR_RATIO = 0.5` specifically to shrink what a model can
      touch after over-selection put unrelated nodes into the schema enum;
      defaulting every search hit to checked reopened exactly that hole one
      layer up; on the intent "make hello end with an exclamation mark" the
      default selection included
      `crate::crates::aether-debugger::src::interp::Interpreter<'a>::run`, an
      unrelated Rust function in the debugger crate, as a legal edit target.
      Fixed by defaulting only the top-scored hit (index 0 of
      `search_nodes_for_authoring`'s best-first-sorted results, per
      `select_nodes`'s existing doc comment and the `apply_score_floor`
      tests that already assume that order) to checked; the user opts
      additional nodes in deliberately instead of opting stray ones out.
    Audited every other place in `author_panel` that reads `AetherApp`
    state for the same class of defect (state captured earlier instead of
    read live at click/build time): `run_author` (Mode 1 "Run") and
    `run_author_authored` (Mode 2 "Run authored") both already recompute
    `checked`/`intent`/`dry`/`max_repairs`/`pasted_plan`/`authored_by` fresh
    at the top of the method on every call, and `author_run_button` (the
    shared confirm/cancel button helper in `crates/aether-app/src/panels.rs`)
    calls straight into whichever method was passed at click time with no
    earlier capture — neither has the caching defect. Their `author_busy()`
    (now `author_write_busy()`) gate is not the same class of bug: it is a
    deliberate, necessary serialization against two authoring runs
    concurrently writing to the same real tree, which Search and Copy
    Context JSON have no equivalent of, since they only read.

21. **Closed — `sample-project/` was both a live authoring demo target and a
    pinned measurement fixture, sharing one tree with no guard that reliably
    detects drift between the two roles. All three candidate mitigations
    are now implemented.** Gap 18 above is the concrete incident, and it
    happened twice: `fe0f476`/`44e7fae`, a genuine (non-dry) `plan run
    --authored` execution demonstrating external authoring against
    `sample-project/calc.py` and its paired test, went unnoticed by every
    gate for three days (2026-08-15 to 2026-08-18) because nothing in the
    test suite or CI compared the live files against the baseline the
    measurement corpus assumes — and the first attempt at fixing it, hours
    into this same pass, repeated the shape of the mistake by verifying
    incompletely. Every future `bitcode do` or `plan run --authored` run
    against `sample-project` — including from the Author tab GUI this pass
    added, which makes triggering one easier than a terminal invocation
    did — had the identical exposure: a real (non-dry) run permanently
    mutates the same files `tools/authoring_task_check.py` and
    `tools/plan_executor_oracle.py::authoring_target_node_source` treat as
    pristine.

    **1. Drift guard hardened (defense in depth).**
    `authoring_target_node_source` asserted `projection.count(node_source)
    != 1` before trusting a hardcoded expected node source against the live
    file — a guard that looked purpose-built for exactly this, but is a
    bare substring *count*, not a boundary check. `fe0f476`'s edit only
    *appended* `.upper()` after the checked text; `"def greet(name):\n
    return hello(name)"` stayed a literal prefix of `"def greet(name):\n
    return hello(name).upper()"`, so the count stayed exactly 1 and the
    guard stayed silent — it failed open on an append. Replaced with
    `_require_isolated_occurrence` in `tools/plan_executor_oracle.py`: the
    single occurrence must now also start at a line boundary (only
    indentation may precede it on its line — needed so an indented `impl`
    block method, like the Rust cases, still passes) and end at one
    (immediately followed by a newline or end of file, not more code on the
    same line). `test_authoring_target_node_source_rejects_an_appended_drifted_body`
    in `tools/test_plan_executor_oracle.py` uses the exact `fe0f476` append
    as the case it must now catch;
    `test_authoring_target_node_source_accepts_an_indented_impl_method`
    pins the case it must not reject.

    **2. The fixture refuses to be a target (the mitigation that actually
    holds).** `reject_measurement_fixture_root` in
    `crates/aether-app/src/project/planfile/mod.rs` rejects any root whose
    canonical final path component is `sample-project`, naming
    `demo-project/` in the error. It runs first inside
    `apply_authored_guarantees` (so `plan run --authored`, and the GUI's
    "Run authored", refuse it — one call site, shared by both since gap 20)
    and first inside `author::author` (so `bitcode do`, and the GUI's
    local-model Run, refuse it — the single function both already share).
    Read-only commands (`context`, `search`, `analyze`, `test-impact`, plain
    `plan run` without `--authored`) never call either function and stay
    unaffected — confirmed the measurement harness itself never invokes
    `--authored` or `bitcode do` at all (`grep` over `tools/*.py` for
    `--authored`/`"do"` found nothing), so this closes the hole without
    touching the harness's own real `plan run`/`plan validate` invocations.
    Unlike the other two mitigations, this one does not depend on a person
    remembering to point somewhere else — it fails closed by construction,
    regardless of which entry point (CLI or GUI) is used.

    **3. `demo-project/` exists as the disposable target.** A small Python
    project (`demo-project/greeter.py` + `demo-project/test_greeter.py`,
    real call edges so test-impact has something to traverse, tests that
    pass) that nothing under `tools/` or `docs/` reads — confirmed with
    `grep -rln "demo-project" tools/ docs/` before this closed, which found
    nothing. `README.md`'s Quickstart, "External authoring", and a new "The
    GUI" Author-tab walkthrough all point at it instead of
    `sample-project/`; the two `sample-project`-authored examples that
    mitigation 2 would otherwise have made literally broken to follow
    (`bitcode do sample-project ...` and the `plan run --authored` loop)
    were rewritten against `demo-project/`, and a new "Demo target" section
    documents the refusal and why.

    **Deliberately not implemented:** mitigation 2 is app-level, inside
    `bitcode` itself — it stops `bitcode do`/`plan run --authored` from
    writing to `sample-project/`, not a hand edit and `git commit` made
    outside `bitcode` entirely, which remains as possible as it always was.
    Mitigation 1 (the hardened drift guard) is the reason that residual
    path is still covered: it protects the measurement harness even against
    a drift that never went through `bitcode` at all, which is exactly why
    it's real defense in depth and not redundant with mitigation 2. The
    root check is also a plain final-path-component name match, not
    hardened against deliberate circumvention (a symlink or a differently
    named copy would bypass it) — sufficient for the ordinary mistake this
    gap records twice, not designed as a security boundary against someone
    trying to get around it on purpose.

    **Verified the way the previous two fixes failed to:** the full Python
    suite from the repository root with no path restriction
    (`python3 -m pytest -q`) passes 68 of 68 (63 from before this pass, plus
    3 real `demo-project` tests and 2 new drift-guard tests), and a real
    authored dry run against `demo-project/` — `bitcode plan run
    plan.json --authored --authored-by claude-sonnet-5 --dry` run from
    inside `demo-project/`, targeting `crate::greeter::farewell` — passes
    precondition checks, executes, and reports passed with no writes to the
    real tree. `bitcode do sample-project ...` and `plan run --authored`
    against `sample-project/` both confirmed rejected with the documented
    error before this was called done.

22. **Closed — `bitcode test-impact` returned zero tests for changed
    functions that real, passing tests do reach, whenever the reaching call
    was chained onto another call's result, and `--quiet` could not
    distinguish that false negative from "nothing changed."**
    Reproduced on a clean tree (`git status --short` showed only the
    pending AGENTS.md edit before this began):
    ```
    sed -i 's/pub fn run(&self) -> Trace {/pub fn run(\&self) -> Trace { \/\/ probe/' crates/aether-debugger/src/interp.rs
    bitcode review . --quiet
      crate::crates::aether-debugger::src::interp
      crate::crates::aether-debugger::src::interp::Interpreter<'a>::run
    bitcode test-impact . --quiet
      (0 bytes, exit 0)
    ```
    `review` resolves the changed node; `test-impact --quiet` prints
    nothing. The non-`--quiet` form is more informative but still
    conclusory: `Changed functions (1): ...Interpreter<'a>::run` /
    `No tests found in the impact set. The changed functions have no test
    coverage reachable via the call graph.`

    **First check: is that specific claim true for `run()`?** Yes.
    `bitcode query . "who calls ...Interpreter<'a>::run"` returns "has no
    recorded callers," and a repo-wide `grep -rn "\.run()"` confirms no
    call site anywhere in the workspace invokes it — `Timeline::record`
    calls `Interpreter::new(&program).run_with_counts(None)`, a sibling
    method, not `run`. So for `run()` alone, zero is the technically
    correct selection and `--quiet`'s failure is only presentational.

    **But probing an adjacent, definitely-covered method in the same impl
    block exposes a real false negative, not a presentational one.**
    `run_with_counts` is called by `run_with` (same file) and, separately,
    by `Timeline::record` at `crates/aether-debugger/src/timeline.rs:34`
    and `:72` — and `Timeline::record` is called directly, by name, from
    all four `#[test]` functions in `crates/aether-debugger/src/lib.rs`
    (`records_a_full_trace`, `what_if_branch_propagates_the_fix_forward`,
    `divergence_points_at_the_intervened_step`,
    `hot_functions_rank_by_execution_count`), which pass under
    `cargo test -p aether-debugger`. Changing `run_with_counts` the same
    way still selects zero tests, and both the quiet and non-quiet output
    are byte-identical to the `run()` case — including the same "no test
    coverage reachable via the call graph" claim, which is now false.
    `bitcode query` confirms the mechanism: `Interpreter::new` and
    `Timeline::record` themselves both report "has no recorded callers,"
    even though both are called from real, non-test-fixture source.

    **Not specific to a generic impl, not specific to aether-debugger.**
    `Timeline` is not generic (`impl Timeline`, no `<'a>`), so the generic-
    impl hypothesis is ruled out directly. Checking a third, unrelated
    crate: `Node::with_language` (`crates/aether-graph/src/node.rs:107`,
    non-generic) is chained onto `Node::new(...)` in production code at
    `crates/aether-builder/src/mapper.rs:845` and is exercised by nearly
    every builder test in the workspace — `bitcode query` still reports
    "has no recorded callers." Same defect, third crate, no generics
    involved, confirming `aether-builder::sync::resolve_calls` (project-
    wide, one implementation per CLAUDE.md) is affected in general, not
    only for aether-debugger.

    **The common shape across every failing case is a call chained onto
    another call's result, or a fully-qualified call whose argument is
    itself a call.** `Interpreter::new(&program).run_with_counts(None)` and
    `Node::new(...).with_language(...)` are both `Type::assoc_fn(args)
    .method(args)` fluent chains. `Timeline::record(buggy_demo_program())`
    is unchained but its sole argument is a call expression. By contrast,
    `Program::new()` — leftmost, unchained, argument-free, in the same
    `buggy_demo_program` function — resolves correctly to its real callers
    via `bitcode query`. The calls chained after it in the same builder
    expression fare worse than a false negative: `Program::stmt` (called
    four times in `buggy_demo_program`) shows zero callers, and
    `Program::function` (called twice in the same function) shows exactly
    one recorded caller — `crate::tools::authoring_task_check::
    call_with_probe`, an unrelated function, not `buggy_demo_program` at
    all. So the defect family ranges from missing edges to misattributed
    edges, not only "conservatively silent" ones.

    **Root cause, precisely, and the fix.** Every failing case above
    bottoms out in one function, `qualifier_matches_owner`
    (`crates/aether-builder/src/sync.rs`), reached through
    `select_candidate`, via three separable mechanisms:
    - *Generic-parameter corruption.* `qualifier_matches_owner` normalized
      an owner's bare name via `owner.rsplit("::").next()` without first
      stripping a generic parameter list, so `Interpreter<'a>` kept the
      lifetime letter and normalized to `"interpretera"` — matching neither
      `"interpreter"` nor its suffix. This broke *any* plain, unchained call
      to a method on a generic type, independent of chaining; found while
      diagnosing `Interpreter::new(&program)` itself, which has no chain at
      all. Fixed with a new `owner_tail()` that strips a trailing `<...>`
      before normalizing, applied to both `qualifier_matches_owner` and the
      analogous `return_type_matches_owner`.
    - *Chained-call qualifier corruption.* `callee_target`
      (`crates/aether-builder/src/mapper.rs`) derived a call's qualifier by
      finding the last `.`/`:` in the whole "function" field's raw source
      text. For `Interpreter::new(&program).run_with_counts(None)`, chained
      directly onto a constructor call with no `let` binding, that captured
      the *entire* preceding call — including nested calls, struct
      literals, and comments for deeper builder chains like `Program::new()
      .function(Function {..}).stmt(..)` — as the "qualifier," which then
      matched no real owner. Fixed with `rust_call_target_ref`/
      `rust_chained_receiver_factory` (`mapper.rs`), which recover the
      receiver from the actual `field_expression`/`call_expression` AST
      nesting instead of text, recursing to arbitrary chain depth and
      reusing the pre-existing `receiver_factory`/`resolve_factory_receiver`
      machinery (built for `let x = Type::new(); x.method()`) unchanged.
      This fix needed no `sync.rs` change at all.
    - *Ambiguous suffix matching.* `qualifier_matches_owner`'s
      `owner.ends_with(hint)` fallback let `PyTimeline` satisfy qualifier
      `"Timeline"` by bare substring, so `only_candidate` saw two matches
      for `Timeline::record` and returned `None` — a real, unambiguous call
      read as ambiguous. Fixed by trying an exact match
      (`qualifier_matches_owner_exactly`) first in `select_candidate`, only
      widening to the suffix-inclusive check when nothing matches exactly.
    - *Local-shadow misattribution, Rust only.* `call_with_probe`'s
      `commit: impl Fn() -> i64` parameter collided by bare name with the
      globally-unique `Widget::commit`, and `select_candidate`'s unqualified
      "if this name is globally unique, assume it" fallback wrongly linked
      them — turning a missing edge into a *wrong* one, gap 22's worst case.
      Fixed by tracking each Rust function's own parameter names
      (`rust_parameter_names`, independent of whether their type is
      hintable, so it does not touch existing `ReceiverHint` behavior) and
      failing closed — `None`, not a guess — whenever a bare callee name is
      locally shadowed. **Deliberately not fixed: the equivalent Python
      collision.** Investigating the real repo's own instance of this
      exact mechanism —
      `tools::authoring_task_check::call_with_probe`'s local `function`
      variable wrongly linking to the unrelated Rust `Program::function` —
      found it is genuinely Python-side (`tools/authoring_task_check.py:39`,
      `function = namespace.get(function_name)`), and closing it generally
      requires Python local-scope tracking this pass did not build. See
      gap 23.

    **Before/after, the real CLI, on the real repo.** The literal repro
    command is a no-op — `\&` in a sed replacement is just an escaped
    literal `&`, identical to what the file already had:
    ```
    sed -i 's/pub fn run_with_counts(&self/pub fn run_with_counts(\&self/' crates/aether-debugger/src/interp.rs
    git diff --stat                   ->  (nothing)
    bitcode review . --quiet          ->  (empty, exit 0)
    bitcode test-impact . --quiet     ->  (empty, exit 0)
    ```
    With an actual edit (mirroring this gap's own original `// probe`
    pattern), against the fixed binary on the fixed source:
    ```
    sed -i 's/pub fn run_with_counts(&self, intervention: Option<&Intervention>) -> (Trace, CallCounts) {/pub fn run_with_counts(\&self, intervention: Option<\&Intervention>) -> (Trace, CallCounts) { \/\/ probe/' crates/aether-debugger/src/interp.rs

    bitcode review . --quiet
      crate::crates::aether-debugger::src::interp
      crate::crates::aether-debugger::src::interp::Interpreter<'a>::run_with_counts

    bitcode test-impact . --quiet
      ai_root_cause_returns_an_explanation
      divergence_points_at_the_intervened_step
      hot_functions_rank_by_execution_count
      records_a_full_trace
      what_if_branch_propagates_the_fix_forward
      ... [67 more aether-app tests] ...
    ```
    Before the fix, this selected nothing (the byte-for-byte-empty output
    documented above). After, it selects five `aether-debugger` tests — not
    the four originally guessed; `ai_root_cause_returns_an_explanation` also
    reaches `Timeline::record` and had been missed in the original
    write-up — plus roughly 67 `aether-app` tests. That larger set is
    correct, not over-selection: `bitcode query` on `run_with_counts` now
    returns exactly `{run_with, Timeline::fork_what_if, Timeline::record}`,
    no spurious extras, and `AetherApp::new` (`crates/aether-app/src/app.rs:204`)
    and `smoke::run` (`crates/aether-app/src/smoke.rs:157`) both really call
    `Timeline::record(buggy_demo_program())` in production code. Every test
    that constructs an `AetherApp` genuinely had `run_with_counts` in its
    blast radius the whole time; the fix simply stopped hiding it.

    Four tests in `crates/aether-builder/src/lib.rs` pin these shapes:
    `gap22_chained_call_resolves_to_its_real_caller`,
    `gap22_chained_call_is_not_misattributed_to_an_unrelated_caller`,
    `gap22_unchained_call_still_resolves_regression_guard`, and
    `gap22_unchained_call_to_generic_type_method_resolves` for the
    standalone generic-parameter case found along the way — plus a fifth,
    `gap22_nested_argument_call_resolves_to_its_real_caller`, added
    initially expecting a defect and left in, not ignored, once
    investigation showed the nested-argument shape alone was never actually
    broken: `Timeline::record(buggy_demo_program())`'s failure was the
    ambiguous-suffix mechanism above, not argument nesting.

    **The `--quiet` ambiguity is real and measured, not inferred:** a
    clean tree with zero source changes and a tree with the
    `run_with_counts` probe applied produce identical `bitcode test-impact
    . --quiet` output — 0 bytes on stdout, exit code 0, in both cases.
    Nothing in the guarded line AGENTS.md now specifies
    (`T=$(bitcode test-impact . --quiet); if [ -n "$T" ]; then cargo test
    $T; else echo "no impacted tests"; fi`) can tell "nothing changed" apart
    from "something changed but the tool lost the covering test." Two
    consequences follow directly, both already true today: an agent
    following AGENTS.md's now-guarded instruction skips real verification
    for a change like the `run_with_counts` probe above, believing nothing
    needs testing; and before that guard existed, the same empty selection
    fed into a bare `cargo test $(bitcode test-impact . --quiet)` ran the
    entire suite instead of the intended subset (reported by the user
    triggering this investigation as 48,791 bytes of output — the opposite
    of the intended saving).

    **The representative benchmark's blind spot, closed the same way it was
    found.** `docs/core-representative-benchmark.md` reported perfect
    precision/recall (1.000/1.000) on 40 declared call-edge cases and did
    not catch any of the above — gap 11 was the identical failure mode (a
    declared-case corpus that never contains a shape cannot fail on it).
    Rather than add synthetic fixtures, three declared cases were added
    from real code already inside the pinned archives:
    `regexset-new-chained-builder-call`
    (`RegexSet::new` -> `RegexSetBuilder::build`, regex's own
    `RegexSetBuilder::new(exprs).build()` with no `let` binding — this
    gap's case 1, verbatim, in a third-party crate) and
    `from-slice-nested-argument-call` (`from_slice` ->
    `SliceRead<'a>::new`, serde_json's own `from_trait(read::SliceRead
    ::new(v))` — case 2's shape, and it exercises the generic-owner fix
    too, since `SliceRead` is generic) both now measure true positives:
    Rust's slice of the corpus is 13 TP, 4 TN, 0 FP, 0 FN, **1.000/1.000**
    — the fix holds on real, previously-uncurated code, not only the unit
    tests written against it. The third case,
    `getchar-not-testing-isolation-mock`, is a declared *negative*:
    Click's own `click.termui.getchar()` reassigns the module global
    `_getchar` to `_termui_impl.getchar` under `global _getchar` and calls
    that — it must never resolve to `click.testing.CliRunner.isolation`'s
    unrelated, same-named nested mock function. It does anyway: this is a
    real, reproducible instance of gap 22's misattribution mechanism on the
    Python side, confirmed via `bitcode query` returning
    `crate::click::termui::getchar` as `CliRunner::isolation::_getchar`'s
    sole recorded caller, and it is deliberately not fixed (see gap 23).
    Declaring it as a case rather than leaving it undeclared means the
    benchmark now says so instead of staying silent: Python's slice is 20
    TP, 5 TN, **1 FP**, 0 FN — micro precision 0.952381, macro precision
    0.933333, recall still 1.000/1.000 (nothing became a false negative;
    one specific, real edge is a false positive). Aggregate: 33 TP, 9 TN,
    1 FP, 0 FN, micro precision 0.970588, macro precision 0.966667, recall
    1.000/1.000, `beta_pass: false` — the precommitted policy requires zero
    false positives and 1.0 precision, and this correctly fails it. The
    numbers were not tuned to keep the old 1.0; a corpus that only ever
    reads 1.0 was the actual lesson of gap 11 and, now, of this gap too.

23. **P1 — open, deliberately not fixed: a Python local variable that
    coincides by bare name with an unrelated, globally-unique function
    elsewhere in the graph gets wrongly linked as that function's caller.**
    Split out of gap 22, which fixed the identical mechanism for Rust
    (`shadowed_by_local` in `crates/aether-builder/src/mapper.rs` and
    `sync.rs`) but left the Python side open. Confirmed still present on
    Bit Code's own repo today, after gap 22's fix: asking who calls
    `crate::crates::aether-debugger::src::lang::Program::function` returns
    three callers — `buggy_demo_program` and
    `hot_functions_rank_by_execution_count`, both real (correctly restored
    by gap 22's chain fix), and `crate::tools::authoring_task_check::call_with_probe`,
    still wrong. Its source (`tools/authoring_task_check.py:31-40`):
    ```python
    def call_with_probe(namespace: dict[str, object], function_name: str, *, upper: bool) -> None:
        ...
        namespace["hello"] = hello_probe
        function = namespace.get(function_name)
        require(callable(function), f"{function_name} is not callable")
        ...
        require(function(name) == expected, f"{function_name} has wrong behavior")
    ```
    `function` is a local variable holding whatever callable
    `namespace.get(function_name)` returned — nothing to do with Rust's
    `Program::function` at all. The bare call `function(name)` is emitted
    as an unqualified `CallRef`, and because `Program::function` is the
    only graph node anywhere named `function`, `select_candidate`'s
    unqualified-branch "globally unique -> assume it" fallback links them.
    This is confirmed live in the representative benchmark too: the
    `getchar-not-testing-isolation-mock` declared negative case added to
    gap 22's closing entry above is the same mechanism on
    `click.termui.getchar()`'s reassigned `_getchar` global, and it
    measures as a real false positive, not a hypothetical one.

    **The real repo's edge is very likely still wrong** — nothing in this
    pass touched it — and the same risk applies anywhere a Python function
    binds a local variable or parameter whose bare name happens to match
    some unrelated, uniquely-named function or method elsewhere in a
    project's graph.

    **What closing it would require.** The Rust fix worked by collecting
    each function's own parameter names (`rust_parameter_names`) — cheap,
    because Rust requires every parameter to carry an explicit, locally-
    complete type annotation, so the enclosing function's own AST node is
    enough. Python has no such requirement: a name can be bound by a
    parameter, a plain assignment (`function = namespace.get(...)`), a
    `for` target, a `with`/`except` target, a comprehension variable, a
    nested `def`/`class`, or an import, and any later reassignment or
    nested scope can shadow or unshadow it mid-function. Suppressing the
    "globally unique" fallback correctly needs real local-scope tracking —
    walking a function body (and, transitively, whatever else it defines)
    for every name it binds by any of those forms before deciding whether a
    bare call's name is a local or a genuine reference to something else —
    not a single-pass parameter list.

    **Why this was scoped out of gap 22 rather than attempted.** Two
    reasons, both from this pass's own findings, not caution in the
    abstract. First, mapper.rs's Python extraction is explicitly a
    "pragmatic extractor, not a full type checker" (its own doc comment);
    building real scope tracking is a materially larger, riskier change
    than gap 22's other three fixes, each of which stayed inside one
    function or added one small, structurally-scoped helper. Second, gap 22
    constraint 2 was explicit that guessing when ambiguous is worse than
    staying silent — a wrong caller is a false positive that propagates
    into rename, while a missing edge is a false negative that stays
    contained. Attempting a fast, partial Python heuristic (e.g., only
    tracking assignment targets, as this pass's own exploratory search
    script did to find `getchar`) risks exactly that: correctly catching
    some shadowing shapes while creating false confidence about the ones it
    doesn't, which is worse than leaving the known gap declared and open.

24. **Closed — `tests.impacted` returns `passed: true` on an empty impact
    set, indistinguishable from real verification, and two mandatory-check
    injection points relied on it.** `run_tests_impacted`
    (`crates/aether-app/src/project/planfile/checks/test_checks.rs`) reports
    success with the detail `"no impacted tests for this step's changed
    nodes"` whenever a step's changed nodes have zero impacted tests — a
    property that is not a bug in itself (a step that genuinely touches
    nothing test-relevant should pass), but is *always* true for a node a
    `create` edit just introduced: it has no prior callers and no prior
    tests by construction, so `graph.tests_for_nodes` on it is always empty.
    A plan whose only edit is `{"path": ..., "create": ...}` and whose only
    check is `tests.impacted` therefore already passed — verifying
    nothing — **before this pass, with no new code required to trigger
    it**: `Edit::Create` has been fully wired through `apply_edit`,
    `apply_step_edits_v2`, `check_preconditions`, and rollback since before
    this gap was found. This was shipped and exploitable, not introduced by
    the work that closes it. Pinned as a fact about the raw executor in
    `executor.rs::create_only_step_verified_only_by_tests_impacted_passes_while_verifying_nothing`,
    which runs such a plan directly through `run_plan` (bypassing
    `Plan::validate()`, exactly as every other test in that module already
    does) and asserts both the overall `Passed` outcome and the check
    detail proving zero tests ran — the same "pin the defect as a test
    first" order gap 22 phase 1 used.

    This is the third instance of a check that reads as verification and
    isn't, not the first. Gap 16's near-miss (`measure_p6_ground_truth`'s
    original `git write-tree` comparison) failed the same way one level
    removed: bitcode never runs `git add`, so the tree hash is identical to
    `base_commit` regardless of whether the working tree actually changed —
    a vacuous ground-truth check caught only because the new case was run
    against the real binary before being trusted. Gap 22 found the general
    form of this instance directly: an empty `bitcode test-impact` selection
    is indistinguishable from "nothing changed," and a bare `cargo test
    $(bitcode test-impact . --quiet)` ran the entire suite instead of the
    intended subset as a result. This gap is gap 22's exact phrase
    ("indistinguishable from nothing changed") recurring in the mandatory-
    check machinery itself: two different call sites both leaned on
    `tests.impacted` as the property a model-authored or externally-authored
    plan cannot skip, and neither could tell "verified" apart from "nothing
    to verify" for a freshly created node.

    Closed by extending `Plan::validate()`
    (`crates/aether-app/src/project/planfile/schema.rs`) with the same
    "this check would verify nothing" precedent already used for the
    empty-expect-set superset/absent rule: any step containing an
    `Edit::Create` must now contain at least one `Check::Command` in that
    same step, or the plan is rejected at parse time — before any edit
    runs, with no graph build required. Since `load_plan`/`parse_plan`
    (`planfile/mod.rs`) already call `Plan::validate()` unconditionally,
    this closes the hole for every caller (`plan run`, `plan run
    --authored`, `plan validate`, and the GUI's paste-a-plan-back-in flow)
    with one change, not three.

    Two call sites push a `tests.impacted` check onto a plan the way
    `Plan::validate()` can't see coming (a check added *after* validation
    already ran): `apply_authored_guarantees` (`planfile/mod.rs`), which
    injects onto the last step of any `--authored` plan lacking one, and
    `author::wrap_step_into_plan`, which does the same unconditionally for
    every `bitcode do` plan. Only the first is a live injection point for a
    `create` edit. `wrap_step_into_plan` builds its edit from
    `author::convert_edit`, which requires an `edit.node` field validated
    against the offered `node_paths` (pre-existing graph nodes) before it
    will produce anything — a `create` edit has no `node` field at all, so
    the local `do` schema cannot structurally produce one; conditioning the
    injection there would be dead code guarding a shape that can never
    reach it, so `wrap_step_into_plan` is unchanged. `apply_authored_guarantees`
    now skips the injection when the last step contains a `create` edit —
    `Plan::validate()` already guarantees that step carries a real `command`
    check by the time this function runs, so skipping the addition doesn't
    leave anything unverified; it just stops adding a second check that
    would read as a safety net it isn't. `bitcode context`'s `plan_skeleton`
    was suspected as a third injection point during design but is not one:
    it builds its envelope with `edits: []`, before the model has written
    anything, so there is nothing yet to condition the injection on — the
    real enforcement for that path is `apply_authored_guarantees`, exercised
    at `plan run --authored` time once the model's edits actually exist.

    Also pinned, ahead of anything relying on it: `EditState`'s span-safety
    re-resolution already handles a node a `create` edit introduces earlier
    in the same plan, in both the same step
    (`edit::tests::a_node_created_earlier_in_the_same_step_can_be_graph_edited_in_that_step`)
    and a later one
    (`edit::tests::a_node_created_in_an_earlier_step_can_be_graph_edited_in_a_later_step`),
    plus the real gate `plan run`/`plan validate` invoke end to end
    (`precondition::tests::create_then_graph_edit_in_a_later_step_passes_preconditions`).
    This was true by inspection before this pass (creating a file is just
    another file touch to the same incremental-refresh machinery graph
    edits already depend on) — these tests supply the proof that was
    missing, not a mechanism change.

    **The precommitted P1 corpus itself carried the vacuous pattern —
    more evidence for this gap, not an inconvenience to fixing it.**
    `tools/plan_executor_oracle.py::p1_plan`'s `created-node` case (step
    `create-source`: `{"path": "src/new_module.rs", "create": "pub fn
    fresh() {}\n"}`) and `cross-file-call` case (step `create-dependency`:
    `{"path": "src/dep.rs", "create": "pub fn added_target() -> i64 { 3
    }\n"}`) both had a `create` edit with zero checks on that step before
    this pass — the exact shape gap 24 closes, sitting undetected in the
    P1-P6 policy's own measurement fixtures the whole time `P1_dry_equals_real`
    has been precommitted. `Plan::validate()`'s new rule rejects both as
    written, so each step gained one `command` check
    (`grep -q 'fn fresh' src/new_module.rs` and `grep -q 'added_target'
    src/dep.rs` respectively) that asserts the created content landed.
    `P1_dry_equals_real` measures whether a dry run's report matches a real
    run's report; both added checks run against the disposable candidate
    workspace identically in dry and real mode, before the dry/real branch
    point in `executor.rs` diverges, so they cannot introduce a dry/real
    asymmetry the property would need to tolerate — they add one more
    check outcome, deterministic in both modes, not a source of drift. The
    P1-P6 rerun after this change (`source_commit: bc3fdfd...`) confirms
    the property is unweakened: `P1_dry_equals_real`'s precommitted mutant
    is still killed (`outcome_mismatches=4` against
    `max_outcome_mismatches=0`), and the full policy still reports `PASS`.
    The finding underneath the fixture edit is the real one: a corpus
    built to measure plan-executor correctness had, itself, been carrying
    an unverified create-only step since it was written — the same
    "a declared-case corpus that never contains a shape cannot fail on it"
    lesson gap 22's benchmark-blind-spot closing entry already drew, now
    recurring in the measurement harness rather than the benchmark.

    **Deliberately not fixed: the general case.** A step that edits an
    already-existing node with genuinely zero callers and zero tests is
    just as vacuous under `tests.impacted` as a freshly created one, and
    `Plan::validate()` cannot detect it — telling "no callers" apart from
    "some callers" requires the graph, and the design constraint for this
    fix was rejecting at parse time with no graph build. Only the
    guaranteed-always-vacuous case (a node that provably has no history
    because it did not exist before this plan) is closed here.

Bit Code's potential advantage is not generic semantic search. It is one local,
inspectable model connecting code identity, predicted impact, selected tests,
validated projection, and recoverable commit. That advantage is unproven until
the P0 measurements above show better consequence prediction without hiding
false negatives or unacceptable over-selection.
