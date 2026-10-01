# Stage 3 TypeScript before-observation: fifth addendum -- the false-empty test-impact symptom is fixed

Follow-up to `before-observation-addendum-4.md` (`eef00d9`), which
found that editing inside a node lost to the semantic-path-collision
bug (or, more broadly, any change whose only effect is on a Module
node rather than any Function node) makes `girder test-impact --quiet`
return a completely empty selection -- not even the conservative
must∪may∪unknown union, and with no boundary notice.

## The fix

Confirmed by a quick, cheap check (not assumed): the false-empty
behavior is NOT specific to the collision mechanism. Editing an
ordinary `describe`-level statement or a `beforeEach` body (no
collision involved at all) reproduces the identical empty
`review`/`test-impact` result. The collision bug is one way to reach
this state; ordinary module-level/setup code is a more common one.

Root cause: `crates/aether-app/src/project/git.rs`'s
`semantic_changed_impact_with_config` filtered `origin_ids` to
`NodeKind::Function` only, dropping any changed `Module` node entirely.
When the ONLY thing that changed was a Module (because no Function
node's own `source` changed), `origin_ids` ended up empty, and
`aether_graph::SemanticGraph::classified_impact` short-circuits
immediately on an empty origin slice (`if origins.is_empty() { return
ClassifiedImpact::default(); }`) -- before ever computing the boundary
list that would otherwise trigger its own, already-existing
conservative escalation (any Unknown call-evidence boundary anywhere in
the graph marks every Function node Unknown).

**Fix**: include `NodeKind::Module` alongside `NodeKind::Function` in
that filter. This is a minimal change that reuses existing,
already-tested logic rather than inventing a new escalation path:
making `origin_ids` non-empty lets `classified_impact` proceed past its
early return, compute boundaries over the whole graph (virtually always
non-empty on real code, per this program's own extensive prior
findings), and fall into the SAME "every function Unknown" conservative
fallback every other origin already benefits from. `tests()`'s
`is_test` attribute filter (`crates/aether-graph/src/claims.rs`)
already excludes non-test nodes, so including the Module id as an
origin does not leak it into the returned test list.

## What else needed fixing alongside it

A pre-existing test,
`test_impact_uses_baseline_tests_for_removed_functions`, broke: its
"Removed functions were detected; using their baseline test coverage."
message was gated on `origin_ids.is_empty()`, which used to be an
exclusive signal for "nothing but a removal happened." Once a removal's
inevitable side effect on its Module's own source is itself included
as an origin, that condition is no longer exclusive with a real
removal. Fixed by gating the message on `baseline_test_paths` being
non-empty directly (`crates/aether-app/src/project/commands/test_impact.rs`),
independent of whether a Module origin is also present. The actual test
SELECTION in this case was never wrong -- only the display message's
trigger condition was -- confirmed by reading the failing test's own
panic output, which showed the correct `test_old_fn` was still
selected, just under a "Changed functions" header instead of the
removal-specific one.

## Verification, per this program's own discipline

- **A/B verified the regression test actually catches the bug**: ran
  `test_impact_quiet_is_not_empty_for_a_module_level_only_change`
  (new, `crates/aether-app/tests/cli.rs`) against the pre-fix code
  (`git stash` on just `git.rs`) -- it FAILED, with the exact empty
  output the bug predicts (`panicked ... a module-level-only change
  must still conservatively select the known test ...: ""`). Restored
  the fix, re-ran -- PASSED. Not just "added a test and it passes,"
  which proves nothing about whether the test exercises the bug at
  all.
- **All four gates pass**, run serially with `-j1`,
  `CARGO_INCREMENTAL=0`,
  `CARGO_TARGET_DIR=/mnt/chromeos/removable/MOVESPEED/aetherforge-target`:
  `cargo test --workspace -j1 --quiet` (all passed, including the one
  pre-existing test that needed its own fix), `cargo clippy --workspace
  --all-targets -j1 -- -D warnings` (clean), `cargo fmt --all --check`
  (clean), `node --test npm/test/*.test.js` (29 passed, 2 skipped, as
  before -- untouched by this change).
- `CLAUDE.md` and the `impacted_tests` MCP tool description
  (`crates/aether-app/src/project/commands/mcp.rs`) updated so neither
  documents the now-closed gap or the now-inaccurate "reach the
  functions that changed" phrasing (a Module can be an origin now too).

## What this does and does not close

**Closed**: the false-empty `test-impact`/`review` symptom for
Module-only changes, across every cause of it (the collision bug,
ordinary setup-code edits, const/type-only edits -- the previously
documented `CLAUDE.md` gap).

**Still open**: the underlying node-id-collision itself.
`call_evidence_v1` for the LOST occurrence in a collision is still
silently destroyed -- its own Must/Unknown classification fidelity is
still wrong, just no longer silently invisible to `test-impact`'s
conservative selection. Fixing that (path disambiguation or evidence
merging at the `SemanticGraph::upsert_node`/`sync.rs::apply()` layer)
remains a separate, harder, not-yet-attempted piece of work, recorded
in `docs/roadmap.md`'s entry for this finding.

## Scope

This fix touches `crates/aether-app` only -- no change to
`claims::annotate()`, `NodeId` computation, or any `call_evidence_v1`
content. No committed audit (`stage3-rust-audit`, `stage3-python-audit`,
or this round's TypeScript `audit-scored-results.json`) needs
re-scoring: the scorers read `call_evidence_v1` directly from `girder
inspect` output, which this fix does not touch.
