# Stage 3 TypeScript before-observation: sixth addendum -- the fix in addendum-5 was incomplete

Addendum-5's "fixed"/"closed" framing was premature. Checked directly
rather than assumed: `crates/aether-app/src/project/commands/
test_impact.rs`'s own dispatch (`test_impact.rs:69-72`) routes ONLY the
bare `--quiet` invocation (no `--run`, no `--out`) to `quiet_from_graph`
-- the function addendum-5 actually fixed. Every other invocation form
(`test-impact .` with no flags, `--out`, `--run`) uses `tests_for_nodes`
instead, a resolved-Calls-reachability-only query with no incoming
edges into a Module node and no "any Unknown boundary escalates
everyone" safety net. Addendum-5's own regression test only exercised
the bare `--quiet` path, so it could not have caught this.

## Confirmed empty, before this commit's fix

Against the `repro-nocoll` describe-level edit (no collision involved
at all, same scenario as addendum-4's finding):

```
$ girder test-impact .
...
Changed functions (1):
  · crate::sample
No tests found in the impact set.
  The changed functions have no test coverage reachable via the call graph.
  Consider adding tests, or run the full suite to be safe.

$ girder test-impact . --run
[identical -- "No tests found", nothing executed]

$ girder test-impact . --quiet --out x
test-impact: 0 impacted test(s) -> x
```

`--run` would execute **zero tests** on a real, committed change. This
is a more severe form of the same bug addendum-5 only partly closed.

## The fix, and why it's narrower than it first looked

The obvious fix -- make `tests_for_nodes` unconditionally give way to
`classified_impact`'s conservative union in this function too -- was
tried first and reverted after it broke 3 pre-existing tests
(`test_impact_does_not_count_cfg_test_helpers_as_tests`,
`test_impact_follows_cargo_binary_subprocess_entrypoint`,
`test_impact_follows_unambiguous_python_nullable_receivers`). Those
tests assert a specific, narrow set of impacted tests on an ordinary
(non-empty) change; the broader union pulled in unrelated tests too,
because a whole-file Unknown boundary (common, and present in all
three fixtures) escalates every test once it's consulted at all. This
confirmed `tests_for_nodes`'s narrower, resolved-reachability-only
behavior is a deliberate, already-tested design choice for the full
command's *common* case, not an oversight -- consistent with
`CLAUDE.md`'s existing note that `tests_for_nodes`/`impact_of`'s return
semantics are relied on elsewhere unchanged.

**Final fix**: keep `tests_for_nodes` as the primary selection
(preserving all existing, tested behavior for a non-empty result).
Only when that selection is empty AND there is at least one origin
(i.e. exactly the false-empty condition, not "no change happened at
all"), fall back to the classified must∪may∪unknown union as a
conservative backstop, with a notice explaining the fallback happened.
This is the minimal change that closes the specific gap without
touching the common case at all.

## Verification

- Extended the existing regression test
  (`test_impact_quiet_is_not_empty_for_a_module_level_only_change`,
  `crates/aether-app/tests/cli.rs`) to also assert on the bare (no
  flag) form and `--run`, not just `--quiet`. `--run` uses
  `run_girder_output` rather than `run_girder` since the throwaway test
  repo has no real `Cargo.toml` for the subprocess to build against --
  the assertion only needs the printed selection, not a successful
  subprocess exit.
- Ran the fix against the SAME 3 tests that broke under the first,
  reverted attempt -- all 3 pass with the narrower, fallback-only fix.
- All four gates pass, run serially with `-j1`: `cargo test --workspace`
  (full pass, including all `aether-app` CLI tests), `cargo clippy
  --workspace --all-targets -- -D warnings` (clean), `cargo fmt --all
  --check` (clean after one auto-formatting pass), `node --test
  npm/test/*.test.js` (unchanged, 29 passed/2 skipped).

## Corrected framing

Addendum-5 and `docs/roadmap.md`'s entry for this finding described
the false-empty symptom as "FIXED"/"closed" based on verifying only the
bare `--quiet` path. That claim is corrected here, not retroactively
edited into the earlier commit: the bare `--quiet` form (the one
`CLAUDE.md`'s own documented safe usage pattern,
`T=$(girder test-impact . --quiet); ...`, already used) was genuinely
fixed by addendum-5's change alone. `--run`, `--out`, and the bare
no-flag form were NOT fixed by that commit and remained silently
broken until this one.

## Still open, not addressed by either fix

- The underlying node-id-collision data loss itself (per addendum-4):
  `call_evidence_v1` fidelity for a lost occurrence is still silently
  wrong.
- Whether a Module-origin fallback is fail-closed independent of
  boundaries existing elsewhere in the graph, or merely fail-closed by
  coincidence because real code almost always has at least one Unknown
  boundary somewhere -- not constructed and tested with a genuinely
  zero-boundary fixture in any language. Left as an explicitly open
  question for a future round, not assumed either way.
- Re-running the dispatch-corpus trustworthiness oracle and
  representative-mutation harness against the rebuilt binary to check
  whether their own `test-impact` invocations are affected.
- Prevalence counts (this repo's own graph measured 65/282 modules
  carrying `duplicate-semantic-path` in addendum-4's investigation;
  corpus-wide counts not yet taken).
- Re-running every committed collision repro with `--run`/full-path
  flags specifically, now that this fix exists, to confirm the fallback
  engages for the ORIGINAL collision-driven cases too (only the
  const-edit/describe-edit case was directly tested here).
- `cargo install --path crates/aether-app --target-dir
  /mnt/chromeos/removable/MOVESPEED/aetherforge-install` has not been
  run since this fix -- the installed `~/.cargo/bin/girder` (if on
  `PATH`) is still the pre-fix binary, per `CLAUDE.md`'s own standing
  warning about exactly this staleness trap.

These are recorded as open, not resolved by assumption, following this
whole investigation's own discipline so far.
