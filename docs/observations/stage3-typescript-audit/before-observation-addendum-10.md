# Stage 3 TypeScript before-observation: tenth addendum -- corrections, a real-graph confirmation, and closing this thread

Follow-up to `before-observation-addendum-9.md` (`5bc7b9c`). Fixes a
real misattribution found in that commit's own doc changes, discloses
a cost implication left out, confirms the combined-origin residual on
a REAL extracted graph (not just a hand-built unit test), and closes
this investigation thread deliberately rather than continuing an
unbounded review cycle. `zero_classification_errors_on_audit` stays
**Met**.

## Corrections to `docs/core-representative-mutations.md` (fixed directly, not just noted)

1. **Misattribution**: the previous wording said the old empty result
   came from "`semantic_changed_impact_with_config`'s bare `test-impact`
   form." Wrong -- `group-invoke`'s origin is `Group.invoke` itself, a
   real, ordinary Function origin; origin resolution was never the
   problem for this mutation. The actual gap was in `tests_for_nodes`
   (resolved-Calls-reachability only), which found no proven edge for
   the unresolved polymorphic dispatch and returned nothing -- fixed
   specifically by `test_impact.rs`'s fallback (addendum-6), not by the
   `git.rs` origin-filter fix (addendum-5). Corrected directly in the
   file.
2. **Scale, previously undisclosed**: precision/recall are computed over
   only the 3 tests this harness declares (its own documented scope),
   but the fallback this mutation triggers is graph-wide. The measured
   result's own `classified.unknown_total_count` is **472** for this
   Click checkout -- a real edit to a function reached only through an
   equally-unresolved dispatch would make `test-impact --quiet` select
   a potentially large fraction of the real test suite, not a small,
   targeted number. This is the designed trade-off (over-select rather
   than miss), but a real cost that was not stated. The actual
   `is_test`-tagged subset of the 472 was not separately measured;
   only the total unknown-classified node count is confirmed.

## `CLAUDE.md` correction

The zero-boundary paragraph still described the Module-origin
fail-closed question as open ("not yet constructed and tested ... in
any language") -- stale since `ba57090` actually closed it for Go.
Rewritten to state the Go case is checked and fixed, name the second
mandatory-check fix (`test_checks.rs`, addendum-9), and point at the
472-node scale disclosure above.

## Retraction

Addendum-8 stated: "No document in this session ever states a
different number [than 52] as the [Rust] scored count." **False**:
addendum-7 section 4 itself contains the string "102 Rust
`not_a_call_site`-excluded" -- introduced in that same document while
restating the Rust audit's scope, never corrected. Retracted here.
The actual Rust `correction-2` audit scores 52 sites (28 exact + 24
conservative), confirmed directly from `audit-after.json`'s own
`cell_counts`/`scored_count` fields, as addendum-8 separately and
correctly verified. No committed Rust audit result is affected by
this stray, never-acted-upon string; it was a documentation slip in a
summary sentence, not a count used anywhere else in this program's
measurements.

## The same-file guard, confirmed on a real extracted graph (not just a hand-built unit test)

`crates/aether-graph/src/claims.rs`'s two new unit tests
(addendum-8) construct nodes with hand-set `file` fields -- correct for
testing the guard's own logic in isolation, but not proof it behaves
the same way against a REAL `GraphBuilder`-extracted Go file. Checked
directly: extended `repro-go2`'s fixture with a second, ordinary
same-file pair (`Other()` / `TestOther`), analyzed, then:

- **Edit only `Other`'s body** (leaving `table`/`Get`/`TestGet`
  untouched): `test-impact . --quiet --classified` selects exactly
  `TestOther` as `must`, zero boundaries, zero mention of `TestGet`.
  Confirms the guard correctly stays silent (adds no spurious boundary)
  when a real same-file Function origin exists and is itself fully
  resolved -- not just in the synthetic unit test.
- **Edit `table` (module-level) AND `Other`'s body together, in the
  same commit**: `test-impact . --quiet --classified` STILL selects
  only `TestOther`, zero boundaries. **`TestGet` is completely
  missing** -- confirmed, concretely, on a real graph: this is exactly
  the "combined-origin residual" addendum-9 described as a theoretical
  risk (a Module-only-origin change silently riding along unflagged
  whenever ANY same-file Function origin also happens to be present in
  the same commit, even one structurally unrelated to the module-level
  change). Not a new defect -- the documented, disclosed boundary of
  the fix's deliberately narrow scope, now verified real rather than
  only reasoned about.

This residual is now a confirmed, not merely disclosed-as-possible,
gap: `docs/roadmap.md`'s entry for this should carry a note that it is
empirically reproduced, with this repro as the reference case for
whatever eventually replaces the current "no same-file Function
origin" heuristic with the more precise "Module diff not explained by
child-function diffs" condition addendum-8 already named as the
correct longer-term fix.

## What remains genuinely open -- stated plainly, not completed here

This thread is being closed deliberately at this point rather than
continuing an unbounded review cycle. The following were raised in the
review that prompted this document and are NOT completed:

- The TypeScript `describe`/`beforeEach`-level (non-collision) scenario
  has not been re-run on any binary built after `ea09ce6`, and no
  `cli.rs` integration test covers it directly (only Rust const-edit
  and Go module-var-edit cases have dedicated CLI tests). Addendum-9's
  claim that this case is "covered by the existing CLI integration
  tests" overstated it -- no TypeScript-specific CLI test for this
  scenario exists.
- Neither the two new `aether-graph` unit tests nor the new
  `aether-app` executor test (`tests_impacted_check_is_not_vacuous_
  for_a_module_level_only_edit`) was A/B-verified against pre-fix code
  the way the `test_impact.rs` fixes were. They were confirmed to pass
  on the fixed code and reasoned through carefully, but not confirmed
  to fail on the prior code.
- After-fix collision-repro outputs (the re-runs done in addendum-9)
  were not committed under `collision-repro/after-fix/` as the prior
  review asked; they exist only in this document's prose.

These are listed here, explicitly, as the correct starting point for
whoever picks this up next -- not silently dropped, not falsely
claimed complete.

## Closing this thread

Starting from Stage 3 TypeScript's before-observation measurement, this
investigation thread found and fixed: a node-id-collision bug that
silently destroys call evidence (still only partially mitigated -- the
`test-impact`/`tests.impacted` SYMPTOM is fixed across every known
cause and every invocation form checked; the underlying evidence loss
itself is not); a false-empty result in `test-impact`'s CLI path, in
both its bare and `--quiet` forms; the identical bug independently
present in Plan Format v2's own mandatory `tests.impacted` gate; and,
as a verified side effect, closed the empty-selection symptom of a
long-documented dynamic-dispatch recall-0.000 defect (the dispatch
itself remains genuinely unresolved). Every fix passed all four gates;
every claim of "fixed" or "verified" in this thread that was later
found to be premature was corrected in a subsequent addendum rather
than left standing, with the exception of the two items listed as
still-open above.

**Next session should**: pick up the still-open items listed here and
in addendum-9, decide on and implement the actual node-collision fix
(path disambiguation or evidence merging) or the "Module diff not
explained by child-function diffs" refinement to the same-file guard,
then re-extract the TypeScript corpus to `docs/stage3-typescript-corpus.json`'s
pinned repos under a persistent path, and only then run the long-deferred
gate-profile over all 46 Must sites (first column: "own claim present" /
lost-node status, with sites 23 and 91 marked).
