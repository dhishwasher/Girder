# Core workflow gap analysis

Last updated: 2026-07-27

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
| Test discovery and coverage | VS Code's testing API supports framework discovery, execution, debugging, and dynamic coverage when supplied by an extension. | Bit Code selects graph-reachable tests and can run configured Rust/Python commands. Focused fixtures cover direct, macro-contained, untracked, removed-function, and narrowed-receiver cases. | **P0 correctness:** subprocess CLI routes and implicit RAII/`Drop` execution remain false negatives; shared infrastructure can over-select tests. Measure precision and recall against dynamic coverage. |
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

## Prioritized open gaps

1. **P0 — call-edge precision and recall.** Model subprocess CLI entry routes
   and implicit RAII/`Drop`; extend Python receiver inference across
   control-flow joins and compound annotations without guessing.
2. **P0 — affected-test oracle.** Add dynamic-coverage comparison fixtures so
   precision/recall claims are reproducible rather than inferred from static
   tests alone.
3. **P0 — recoverability proof.** Inject interruption at every durable
   transaction transition and verify both source and graph state after restart.
4. **P0 — representative repositories.** Record cold/incremental indexing,
   call-edge accuracy, impact latency, test-selection accuracy, and memory on at
   least three real Rust/Python repositories without manual repair.
5. **P1 — interactive latency.** Measure edit-to-graph, edit-to-diagnostic, and
   navigation latency under sustained edits.
6. **P1 — agent outcome metrics.** Track accepted patches, validation catches,
   rejected patches, rollback success, and time-to-safe-commit.

Bit Code's potential advantage is not generic semantic search. It is one local,
inspectable model connecting code identity, predicted impact, selected tests,
validated projection, and recoverable commit. That advantage is unproven until
the P0 measurements above show better consequence prediction without hiding
false negatives or unacceptable over-selection.
