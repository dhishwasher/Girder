# Stage 3 TypeScript before-observation: eighth addendum -- the Go gap, closed; corrections

Follow-up to `before-observation-addendum-7.md` (`7507b5f`). That
document's §2 claimed the Module-origin escalation is "structural ...
in all four languages." Checked against the shared-evidence unit test
`direct_local_bindings_have_evidence_in_all_four_languages`
(`crates/aether-builder/src/mapper/claims.rs`) before trusting that
claim further, per the review that prompted this document: it already
proves a same-file Go call can be classified `Must` with **zero**
Unknown claims -- directly contradicting §2's premise that Go always
carries a boundary. §2 was wrong for Go. This addendum corrects it,
closes the real gap it missed, and settles the remaining precision
items from the last two rounds. `zero_classification_errors_on_audit`
stays **Met**.

## Precommitted prediction, then the result

Before running anything: a Go file with a package-level `var table =
[]int{1, 2, 3}`, `func Get(i int) int { return table[i] }`, and `func
TestGet(t *testing.T) { _ = Get(2) }` all in one file -- same-file, so
`Get`'s call is Must-provable, and Go has no unconditional whole-file
gap (unlike Python/TS) and no attribute-based claim (unlike Rust's
`#[test]`) -- predicted: zero `class:unknown` claims in `inspect`
output; shrinking `table` to `{1, 2}` (a module-level-only edit, no
function body touched) would make `--quiet` return completely empty
stdout AND stderr (no boundary notice at all); the bare form and
`--run` would report "No tests found in the impact set"; `--run` would
execute nothing.

**Confirmed exactly**, on the binary as of `7507b5f` (pre-this-fix):
`inspect` showed 0 `class:unknown` claims anywhere in the file;
`--quiet` returned empty stdout and empty stderr; bare and `--run`
both printed "No tests found in the impact set"; `--run` executed
nothing. This is a more severe instance than the Rust/TypeScript cases
closed in addenda 5-6: those always had `--quiet`'s stderr notice even
when the terminal selection also started empty (TypeScript/Python's
unconditional gap, Rust's `#[test]` claim); Go's case was silent on
every signal in every form.

## The fix

`crates/aether-graph/src/claims.rs::classified_impact`: after the
origin-resolution loop that seeds `best`/`queue`, a second pass checks
every Module-kind origin for whether any Function-kind origin in the
same `origins` slice shares its file. If none does, a
`coverage_gap: true` boundary (`"module-level-change-with-no-function-
origin"`) is pushed for it. This makes `boundaries` non-empty in
exactly the case this document is about, which triggers the EXISTING
"any boundary anywhere escalates every Function node to Unknown" rule
(`claims.rs` lines ~220-224, unchanged) -- no new escalation logic was
written, only the missing trigger for it.

Deliberately scoped to "no Function origin from the same file," not
"any Module origin": an ordinary function-body edit already carries
its own Function origin (and whatever boundaries that function's own
evidence already has), and every edit's Module changes too (a Module's
`source` is the whole file) -- escalating on every Module origin
unconditionally would have disturbed the bounded trustworthiness
fixtures' existing exact-precision measurements for no reason, the
same regression-risk lesson from addendum-6's reverted first attempt
at the `test_impact.rs` fix.

## Verification

- Two new `aether-graph` unit tests
  (`a_module_origin_with_no_same_file_function_origin_still_escalates`,
  `a_module_origin_with_a_same_file_function_origin_is_not_separately_
  escalated`) -- both pass; the second specifically guards against the
  over-escalation risk above.
- End-to-end, against the real rebuilt binary (sha256
  `48553d3a52ce674283fa2c7c11fddaab21bc51fab4523679807e86ce3208ab14`):
  re-ran the exact Go repro. `--quiet` now prints `TestGet` to stdout
  and the boundary notice to stderr. The bare form and `--run` both
  select and -- for `--run` -- actually EXECUTE `TestGet`, which
  **genuinely fails**: `panic: runtime error: index out of range [2]
  with length 2`. This is a real regression this exact mechanism would
  have silently missed before the fix, caught end-to-end, not just in
  a unit test.
- New `aether-app` integration test
  (`test_impact_quiet_is_not_empty_for_a_go_module_level_only_change`,
  `crates/aether-app/tests/cli.rs`) covers the same scenario through
  the CLI.
- All four gates pass: `cargo test --workspace -j1 --quiet` (all
  passed, including both new unit tests and the new integration test),
  `cargo clippy --workspace --all-targets -j1 -- -D warnings` (clean),
  `cargo fmt --all --check` (clean), `node --test npm/test/*.test.js`
  (unchanged).

## §2's corrected claim

Python and TypeScript: still structural (the unconditional whole-file
gap, verified by reading the unconditional `if lang == Lang::Python ||
lang.is_typescript()` check). Rust: still structural for any file
containing a REAL `#[test]`/`#[tokio::test]`-attributed function (the
attribute node's own claim, computed globally over the whole file, not
scoped to a specific origin). **Go: NOT structural** -- confirmed by
the existing cross-language unit test and this round's own repro, and
why the fix above exists.

## Corrections owed from addenda 4 and 7, now settled

- **Rust's `audit-after.json` (`correction-2`) scored count is 52** (28
  `exact` + 24 `conservative`), confirmed again directly from the file's
  own `cell_counts`/`scored_count` fields. No document in this session
  ever states a different number as the scored count; flagged here
  only because a stray "102" was raised in review as a risk to guard
  against in this document specifically, and this confirms it does not
  appear.
- **The "~7 impls" claim about `serde_json`'s `Compound` type, left
  unverified in addendum-7, is now verified**: `grep -n "^impl.*for
  Compound" src/ser.rs` against the real crate
  (`~/.cargo/registry/.../serde_json-1.0.150/src/ser.rs`) finds exactly
  **7** `impl ... for Compound` blocks (`SerializeSeq`, `SerializeTuple`,
  `SerializeTupleStruct`, `SerializeTupleVariant`, `SerializeMap`,
  `SerializeStruct`, `SerializeStructVariant`). Within them, `fn end`
  appears 7 times (once per impl -- every one of these traits requires
  an `end` method), `fn serialize_field` 4 times, `fn serialize_element`
  2 times (the pair addendum-4's investigation already examined
  directly). The claim is correct.
- **Two items from addendum-6's "still open" list, reconfirmed still
  open, not newly discovered**: `--quiet --out` and `--quiet --run`
  route through the narrow `tests_for_nodes`-then-fallback path (the
  SAME code path the bare form uses, verified by reading
  `test_impact.rs`'s own dispatch: only a bare `--quiet` with no other
  flag and no `--out` reaches `quiet_from_graph` at all), while plain
  `--quiet` alone uses `quiet_from_graph`'s unconditional classified
  union. Both now reach a correct, non-empty answer for a Module-only
  origin (the fallback engages), but by a different mechanism with a
  different boundary-notice message than bare `--quiet`'s. Not unified
  in this round; recorded as a roadmap item, not a defect requiring an
  immediate further fix (both now terminate at correct, conservative,
  non-empty selections). The fallback firing only when the narrow
  result is empty also still means: a single commit touching BOTH a
  lost/Module-only-origin file AND an ordinary, reachable function
  elsewhere would have a non-empty `tests_for_nodes` result overall
  (from the reachable function), so the fallback would not engage, and
  the lost/Module-only file's own tests could still be missing from
  that specific combined selection. This is the same category of
  residual gap as the node-id-collision's own un-fixed call-evidence
  loss (addendum-4) -- recorded, not fixed, in this already-large
  thread.

## Still not started

The gate-profile step (over all 46 TypeScript Must sites) has still
not been started. Per this thread's own accumulated findings, this is
now its precondition list: re-extract the pinned TypeScript corpus to
a persistent path; the node-id-collision data-loss fix itself (distinct
from the test-impact/review symptom fixed across addenda 5, 6, and this
document); re-running the trustworthiness oracle and representative-
mutation harness against the final binary; the residual same-commit
combined-origin gap noted above. All are now roadmap-tracked rather
than scattered across addenda.
