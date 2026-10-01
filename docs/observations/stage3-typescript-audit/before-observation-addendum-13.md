# Stage 3 TypeScript before-observation: thirteenth addendum -- the combined-origin residual, fixed and verified

Follow-up to `before-observation-addendum-12.md` (`67dfae9`), which
closed the Rust trait-impl node-collision root cause. This document
closes the OTHER item from the user's standing directive: the
combined-origin false-empty residual confirmed on a real Go graph in
addendum-10 (editing a module-level var together with an unrelated,
ordinary, same-file function in one commit silently dropped the
module-affected test, with zero boundaries reported).

## The bug, re-confirmed, and why the first attempt at this fix failed before landing

Implemented a masked-text-comparison check
(`aether_graph::origins_excluding_explained_modules`, in the new
`crates/aether-graph/src/diff.rs` function): a Module origin is
excluded from `origin_ids` only when its own source, with every
sibling Function origin's span (from the same file) masked out, is
byte-identical between the git baseline and the current graph --
i.e. every part of the module that changed is already covered by a
Function origin already in the list. Applied this identically in both
callers that build origins from a before/after graph pair:
`crates/aether-app/src/project/git.rs::semantic_changed_impact_with_config`
(the `test-impact` CLI path) and
`crates/aether-app/src/project/planfile/checks/test_checks.rs::changed_node_ids`
(the `tests.impacted` plan check) -- both now call the identical
shared function instead of each re-deriving an approximation.

**The first attempt (committed only to this session's working tree,
never pushed) failed its own new regression test.** Editing `table`
and `Other` together still selected only `TestOther`; `TestGet` stayed
missing. Traced directly rather than guessed at: `classified_impact`
(`claims.rs`) had its OWN, SEPARATE "does this Module origin have ANY
same-file Function origin" guard, added independently in the earlier
Go fix (addendum-8) -- the exact same coarse approximation this
addendum's new upstream filtering was built to replace. Since `Other`
(a real, resolved Function origin) was present in `origin_ids`
alongside the Module, `classified_impact`'s own internal guard ALSO
saw "a same-file Function origin exists" and independently decided not
to escalate -- undoing the more precise upstream decision before it
ever mattered. Two layers were independently implementing the same
wrong heuristic.

**Fix for that**: simplified `classified_impact`'s own guard back to
unconditional -- ANY Module-kind origin that still reaches this
function now always gets a boundary, no same-file-Function exception.
This is safe specifically because BOTH callers that build origins from
a diff now pre-filter "explained" modules upstream before calling it;
a Module origin that survives that filtering is, by construction,
genuinely unexplained. (`orient`'s and `test-impact --nodes`'s
explicit single-id queries bypass this filtering entirely, but those
pass exactly one user-named id with no sibling-origin ambiguity to
begin with, so the unconditional rule is correct there too.)

## New tests

- `crates/aether-graph/src/diff.rs`:
  `module_origin_explained_by_its_sole_function_origin_is_excluded`
  (ordinary case: module fully covered by its one function origin's
  span → excluded) and
  `module_origin_with_unexplained_content_alongside_an_edited_function_is_kept`
  (the combined-origin case: a `const` changes outside the edited
  function's own span → module stays).
- `crates/aether-graph/src/claims.rs`: rewrote
  `a_module_origin_with_a_same_file_function_origin_is_not_separately_
  escalated` (which asserted the NOW-WRONG old behavior) into
  `any_module_origin_unconditionally_escalates_in_classified_impact_
  itself`, which also corrects a wrong assumption caught by the test
  itself on first write: a boundary does NOT downgrade an already-
  verified Must-path (the BFS seeds `best` with Must *before* the
  boundary fallback's `.or_insert(Unknown)` runs, which cannot
  override an existing entry) -- it only reaches tests that have no
  resolved path of their own. Fixed the test's assertions to match
  this actual, correct, and arguably more desirable behavior (a
  genuine Must-proof is never weakened by an unrelated boundary)
  rather than changing the implementation to match a wrong assumption.
- `crates/aether-app/tests/cli.rs`:
  `test_impact_still_selects_a_module_level_test_when_an_unrelated_
  function_is_also_edited`, the real end-to-end scenario through the
  CLI.

## Verification

- All four gates pass: `cargo test --workspace -j1 --quiet` (full
  pass -- `aether-graph` 78→80 tests, `aether-app` 72→73 tests, zero
  regressions elsewhere), `clippy -D warnings` (clean), `fmt --check`
  (clean), `node --test` (unchanged).
- End-to-end against the rebuilt binary (sha256
  `2861fe7cf70d016a025a4bc1e385447a62fdb9fdc1dd8b7edbfbb9bb6f327092`):
  editing `table` and `Other` together now selects BOTH `TestGet`
  (`unknown`, reached via the boundary) and `TestOther` (`must`, its
  own direct resolved call) -- confirmed via `--quiet` and
  `--quiet --classified`. The ordinary case (editing only `Other`)
  re-confirmed unaffected: zero boundaries, only `TestOther` selected,
  exactly as before this fix. Outputs committed under
  `docs/observations/stage3-typescript-audit/collision-repro/
  combined-origin-after-fix/`.
- `core_trustworthiness_oracle.py`: `baseline: matches` exactly, no
  diff at all against the current committed baseline.
- `core_representative_mutations.py`: one benign `+1 coverage_gap`
  boundary-count change (10027→10028) in the `group-invoke` mutation,
  with precision/recall, true/false positive sets all UNCHANGED --
  the mutation's own probe-insertion commit apparently also touches
  module content outside `Group.invoke`'s own span, now more precisely
  disclosed as an additional boundary rather than silently merged into
  the one already there. Updated the committed
  `docs/core-representative-mutations.json` to match; no narrative
  change needed since the measured OUTCOME (precision/recall) didn't
  move.

## Status: both items from the standing directive's "first priority" are now closed

- The node-identity/collision problem (Rust trait-impl path collisions,
  addendum-12): fixed, verified, all gates pass, oracle/mutations
  unchanged.
- The combined-origin false-empty case (this document): fixed,
  verified, all gates pass, oracle/mutations unchanged (one disclosed,
  benign boundary-count increase).

Both were A/B-verified (or in this document's case, verified by first
confirming the fix DIDN'T work as intended, tracing the actual cause
rather than assuming, and fixing that instead of adjusting the test to
match broken behavior) and checked against the existing Rust/Python
trustworthiness measurements with no criterion weakened.

## Still open (unaffected by either fix, listed for completeness)

- TypeScript's own distinct collision mechanism (duplicate `it()`/
  `describe()` description strings, sites 23/91) -- a different
  mechanism from the Rust trait-impl case, not addressed.
- Rust's DONE audit's `correction-3` re-score against a fresh `inspect`
  of the pinned corpus -- not done (checkouts not present in this
  session's scratchpad).
- The TypeScript corpus re-extraction and the long-deferred
  gate-profile step itself -- not started.

Per the standing directive recorded in `docs/roadmap.md`
(2026-10-01), the next step is confirming `main` is green (done above)
and then pivoting to the monetization-readiness phase, not continuing
further resolver/correctness expansion.
