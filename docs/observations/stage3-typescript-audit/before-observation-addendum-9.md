# Stage 3 TypeScript before-observation: ninth addendum -- a second mandatory gate had the same bug; oracle/mutation re-run

Follow-up to `before-observation-addendum-8.md` (`ba57090`). Covers the
remaining items from the review that prompted addendum-8: re-running
the trustworthiness oracle and representative-mutation harness against
the final binary, and re-checking other consumers of the Function-only
origin-filter pattern for the same bug shape. One of those consumers
had it. `zero_classification_errors_on_audit` stays **Met** throughout;
none of this changes any committed Stage 3 audit's scored cells.

## A second mandatory check had the identical bug: `tests.impacted` in plan execution

`crates/aether-app/src/project/planfile/checks/test_checks.rs::run_tests_impacted`
-- the function the **mandatory** `tests.impacted` plan check gates on
(the one check CLAUDE.md itself describes as something "the model
cannot remove") -- called `graph.tests_for_nodes(changed)` directly,
with no fallback, the identical narrow mechanism `test_impact.rs`'s
own full path used before addendum-6's fix. `changed_node_ids` here
(unlike `git.rs`'s pre-fix version) already includes Module nodes, so
a Module-only-origin step (a const/type-only edit, a collision-lost
occurrence, a Go package-level var change) reaches `run_tests_impacted`
with a Module id in `changed` -- but `tests_for_nodes` still finds
nothing for it, and this function reports `passed: true, "no impacted
tests for this step's changed nodes"` with **no fallback at all**. A
plan step that edits only a module-level const could pass its
mandatory test gate while a real, reachable test silently never ran.

**Fix**: the identical scoped fallback -- when `tests_for_nodes` is
empty and `changed` is non-empty, fall back to
`classified_impact(changed).tests(graph)`'s conservative union before
declaring "no impacted tests."

**Checked against the existing, intentional "Gap 24" vacuous-pass
behavior before assuming this fix was safe everywhere**: a prior test,
`create_only_step_verified_only_by_tests_impacted_passes_while_
verifying_nothing` (`executor.rs`), pins that a step creating a
brand-new, caller-less function with `tests.impacted` as its only
check passes while verifying nothing -- a DIFFERENT, already-accepted
gap (schema.rs's own "Gap 24" comment), handled at parse time for
`plan_version: 2` but deliberately still reachable via `plan_version:
1` to pin the raw executor's behavior independent of that rule. Reran
the full `planfile`-scoped test suite (86 tests) after the fix: this
pinned test still passes unchanged, confirmed not by assumption but by
running it -- the fixture has no test function at all in its graph, so
`.tests()`'s `is_test` filter leaves the fallback's result empty too,
regardless of any boundary escalation.

New regression test:
`tests_impacted_check_is_not_vacuous_for_a_module_level_only_edit`
(`executor.rs`) -- a real plan step whose only edit inserts a `const`
before an existing, tested function (so the function's own node
`source` is untouched, only the module's is), checked kind: "match"
Substitute edit, run through the real executor end-to-end. Confirms
the check no longer reports "no impacted tests."

All four gates pass after this fix (full workspace rebuild, `cargo
test --workspace -j1 --quiet`, `clippy -D warnings`, `fmt --check`,
`node --test`).

## Oracle re-run: unchanged trustworthiness, +2 disclosed boundaries

`python3 tools/core_trustworthiness_oracle.py --bitcode <binary>`
against the binary built with every fix in this thread (sha256
`beadb6c91907bf0e20830319e4cbbeff19eab4172cacc17caea6980cb336113f`):
precision/recall **unchanged** at 1.000/1.000 for Rust, Python, and
combined. The only diff against the previously-committed
`docs/core-trustworthiness-baseline.json`: `coverage_gap` boundary
count +2 (30→32 in the aggregate; isolated to the Python fixture's own
`classified.boundary_by_category.coverage_gap`, 6→8), rolling up into
`boundary_count` 102→104. Diffed field-by-field against the prior
baseline to confirm this is the ONLY change -- no other language, no
must/may count, no precision/recall figure moved. This is the expected,
benign side effect of the Go-case fix in addendum-8 (a Module-origin
boundary now exists in two more cases than before): more honest
disclosure, not a different answer. **Updated
`docs/core-trustworthiness-baseline.json`** to the new, re-verified
measurement (not deleted or bypassed -- the new numbers are checked
into the baseline the oracle compares against, following this
program's own "update what's genuinely re-measured, never silently
loosen a check" discipline). Re-ran the oracle against the updated
baseline: `baseline: matches`.

## Representative-mutation re-run: a long-documented recall-0.000 defect's symptom is now fixed

`python3 tools/core_representative_mutations.py --bitcode <binary>
--json` against the same binary. **This result changed substantively,
not just by a boundary count**: the `group-invoke` mutation (Click's
`Group.invoke` vs `Command.invoke` polymorphic dispatch, reached only
through an untyped pytest fixture parameter) went from `TP=0, FP=0,
FN=2, TN=1, precision=1.000, recall=0.000` to `TP=2, FP=1, FN=0, TN=0,
precision=0.667, recall=1.000`.

**This is the exact, long-standing, documented defect `CLAUDE.md` and
`docs/core-representative-mutations.md`/`docs/core-gap-analysis.md`
cite by name** ("The documented dynamic-dispatch failure remains: a
representative mutation measured recall 0.000"). Traced the mechanism
directly, not assumed: `tools/core_representative_mutations.py` runs
the bare (no-flag) `test-impact <checkout>` form for its "static
prediction" -- exactly the code path addendum-6 fixed in
`test_impact.rs`. This dispatch is genuinely unresolved (Girder cannot
determine `self.invoke`'s receiver type through an untyped fixture
parameter -- confirmed via the new result's own `classified` field:
all three declared tests classify `unknown`, not `must`/`may`, so
Girder is NOT claiming to have solved the dispatch). Previously,
`tests_for_nodes` found zero resolved reachability for this
unresolved-dispatch origin and returned nothing, with no fallback --
recall 0.000. Now the fallback engages and conservatively includes
every test the unresolved boundary could reach, including one that
turns out not to actually need it (`test_other_command_invoke`,
`FP=1`) -- recall improves to 1.000 at the cost of precision dropping
to 0.667, exactly the designed "miss nothing real, over-select instead"
trade-off this whole program's conservative-union philosophy is built
on.

**Updated, not silently**: `docs/core-representative-mutations.json`
(the checked result; also carries forward an unrelated schema v1→v3
format upgrade the script itself had already undergone before this
session, independent of this fix -- confirmed by comparing against
`docs/observations/stage1/representative-mutations-measurement.json`,
already schema v3 with the OLD recall-0.000 numbers, proving the
format change and the metric change are separate, correctly
attributing only the metric change to this fix). `docs/core-
representative-mutations.md`'s result table and "Verified defect"
section rewritten to state plainly: the dispatch-resolution gap itself
is UNCHANGED and still open (recorded in `docs/core-gap-analysis.md`,
also updated) -- what changed is that the selection no longer silently
drops it. `CLAUDE.md`'s own citation corrected to match.

## Collision repros re-confirmed on the final binary, across multiple invocation forms -- not every combination

Re-ran, on the binary with every fix in this thread:

- **TypeScript lost-body** (`onlyInLost`-only edit, the exact
  addendum-5/6 scenario): `--quiet`, bare, `--quiet --out x`, and
  `--run` all now correctly report the 3 real tests (not empty);
  `--run` fails only because this throwaway repo's `girder.toml`
  disables test commands, not because of an empty selection.
- **Rust lost-impl** (`impl A for S`, the exact addendum-3/4 scenario):
  `--quiet`, bare, and `--quiet --out x` all correctly report both
  real tests (`calls_a`, `calls_b`).

**Not separately re-run under every flag in this round**: the TS
describe/`beforeEach`-level (non-collision) scenario under `--out`/
`--run` specifically (already covered under `--quiet`/bare by the
existing CLI integration tests, and structurally identical to the
now-multiply-confirmed Rust/Go const-edit cases); Rust lost-impl under
`--run` specifically. Listed as explicitly open, not assumed covered.

## Still open (unchanged from addendum-8, now more precisely scoped)

- The underlying node-id-collision data loss itself
  (`call_evidence_v1` fidelity for the lost occurrence) -- unfixed.
- `--quiet --out`/`--quiet --run` reach a correct answer via a
  different internal mechanism (narrow-then-fallback) than bare
  `--quiet` (unconditional classified union), with a different
  boundary-notice wording.
- A single commit combining a Module-only-origin change with an
  unrelated, reachable function-body change elsewhere would make the
  narrow selection non-empty overall, so the fallback would not
  engage, potentially still missing the Module-only file's own tests
  from that specific combined selection -- in BOTH `test_impact.rs`
  and now `test_checks.rs`'s `run_tests_impacted`.
- The dispatch-resolution gap itself (fixture-mediated polymorphic
  dispatch through an untyped parameter) remains open future work,
  tracked in `docs/core-gap-analysis.md` -- this thread only fixed
  what happens when that resolution fails, not the resolution itself.

## Still not started

The gate-profile step (over all 46 TypeScript Must sites) has still not
been started. Precondition list unchanged from addendum-8 plus: the
combined-origin residual above should inform the gate-profile's design
if it ever needs to reason about multi-change commits.
