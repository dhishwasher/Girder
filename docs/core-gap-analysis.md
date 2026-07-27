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

## Prioritized open gaps

1. **P0 — call-edge precision and recall.** Exclude Rust-looking text inside
   macro string literals; model subprocess CLI entry routes and implicit
   RAII/`Drop`; extend scoped type refinement to match arms, `let-else`, and
   let-chains; build equivalent measured Python fixtures.
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
