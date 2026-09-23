# Stage 3 Rust after-observation: trusted-assert-macro resolver fix

Source commit: `47c3a06` ("Stage 3 Rust resolver: prove calls inside trusted
assert macros"). Binary measured: `/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder`,
sha256 `e8ab37337968d8e32a390f8caac4e147b6dcfb28a315f4aaffaa5e618ec54dc0`,
built from that commit with `cargo build -p aether-app --bin girder -j1`.

## Result up front: two of three criterion legs met, one not. Rust stays IN PROGRESS.

The roadmap's frozen Stage 3 criterion (docs/roadmap.md lines 279-283 and
369-374) requires, per language: *"measured dispatch-corpus improvement,
nonempty Must precision 1.000, and zero classification errors on the frozen
real-repository audit,"* and explicitly spells out for the after-observation
that *"the audit must show fewer conservative / more exact cells"* alongside
no regression on all three measurements. This fix satisfies the dispatch
corpus and zero-unsound legs, but **the audit is unchanged** -- 27 exact / 25
conservative, identical to the before-observation. Nonempty Must precision
"on a real repository, not on fixtures" (the standard this program's own
`audit-correction-2.md` set) is therefore also not met: the audit's own Must
set is still empty. Stage 3 (Rust) is **not DONE**. It is also not FAILED --
a concrete next resolver step is identified below, not a dead end -- so it
stays **IN PROGRESS**.

## 1. Dispatch corpus: measured improvement, confirmed against the committed baseline

Before (committed `docs/observations/stage2-dispatch-corpus/scoring-summary.json`,
candidate `e35c298`):
- Pooled: 20 exact / 36 conservative, `must_true_positives: 3`,
  `must_precision_on_corpus: 1.0`.
- Rust: 4 exact / 10 conservative.

After (this commit, [corpus-after.json](corpus-after.json), command
`python3 tools/dispatch_corpus_scorer.py --bitcode <binary> --output corpus-after.json`,
exit 0):
- Pooled: **21 exact / 35 conservative**, `must_true_positives: 4`,
  `must_false_positives: 0`, `must_precision_on_corpus: 1.0`.
- Rust: **5 exact / 9 conservative**.

The single cell that moved: `rust-direct-same-file`'s `test_direct`
(`assert_eq!(target(), 42)`), `expected: must` / `observed: must` /
`cell: exact` -- previously `conservative`. No other case in any language
changed. Zero unsound cells before or after (`unsafe_exclusion: 0`,
`overclaim: 0`, both pooled and per-language, both runs). This is exactly
the single-cell move predicted in the already-committed
[audit-correction-2.md](../audit-correction-2.md)'s multi-gate tally: "every
Rust corpus test's call to its origin sits inside `assert_eq!`... A
method/path-call fix alone moves zero corpus cells."

**Probe-verified** (not just trusted from Girder's own answer): ran
`cargo test test_direct` directly against `fixtures/dispatch-corpus/rust/direct-same-file`
([probe-direct-same-file.log](probe-direct-same-file.log)) -- `test_direct`
passes and its body is exactly `assert_eq!(target(), 42)` calling the
same-file top-level `pub fn target() -> i32 { 42 }`, confirmed by reading
the fixture source directly. The claimed Must edge is real.

## 2. Stage 1 oracle: no regression

[oracle-after.json](oracle-after.json), command
`python3 tools/core_trustworthiness_oracle.py --bitcode <binary> --no-check --json`,
exit 0, stderr empty. Rust and Python both: `precision: 1.0`, `recall: 1.0`,
`false_positives: []`, `false_negatives: []` -- unchanged from the committed
Stage 1 baseline. This mutation corpus's `rust_direct_selected` case does not
happen to use the assert-wrapped same-file pattern, so its own `classified.must`
stays empty; that is a property of this separate, independently-designed
corpus, not a regression.

## 3. Real-repository audit: unchanged, root cause investigated (not guessed)

[audit-after.json](audit-after.json), command
`python3 tools/dispatch_audit_scorer.py --bitcode <binary> --sites audit-sites-labeled-v2.json --crate-root ... --output audit-after.json`,
exit 0: **52 scored, 27 exact, 25 conservative, 0 unsound** -- byte-identical
cell counts to the before-observation (`audit-scored-results-v3.json`).

Per advisor's explicit instruction, checked *why* rather than assuming: grepped
the three inspected crates' `call_evidence_v1` output
(`petgraph-0.6.5-inspect.json`, `regex-1.12.4-inspect.json`,
`serde_json-1.0.150-inspect.json`, all written alongside `audit-after.json` by
the scorer) for the new claim reason `proven-call-inside-trusted-assertion-macro`:
**zero occurrences in all three crates.** The fix never fires on this sample,
which is why the audit didn't move -- not a scorer artifact, not a labeling
issue.

Searched the three crates' real (non-doctest) source directly for the pattern
this fix targets -- a bare identifier call as an assert-macro's first
argument -- via `grep -rnE 'assert(_eq|_ne)?!\([a-zA-Z_][a-zA-Z0-9_]*\('`.
Of the matches:
- The large majority (e.g. `petgraph`'s `bellman_ford.rs:79`,
  `page_rank.rs:28`, most of `regex`'s `builders.rs` hits) are inside `///`
  doc comments (doctest examples), not real code -- tree-sitter never emits a
  `macro_invocation` node for comment text, so these were never candidates.
- The clear real-code hits are almost all cross-file: e.g.
  `petgraph`'s `tests/graph.rs:202` calls `is_cyclic_undirected(&gr)`, but
  `is_cyclic_undirected` is `pub fn`-defined in `src/algo/mod.rs`, not in the
  test file itself (confirmed by grep) -- the pre-existing, already-documented
  same-file-only restriction excludes it regardless of this fix, matching the
  "2/25 (8%)" root-cause category from `audit-correction.md`.
- The one clear same-file, non-doctest, non-cross-file candidate found:
  `serde_json/src/lexical/math.rs:632`, `debug_assert!(greater_equal(x, y));`,
  where `greater_equal` is `pub fn`-defined at line 575 of the same file.
  This file carries eleven `#[cfg(fast_arithmetic = "32"|"64")]` attributes
  elsewhere (confirmed by grep) -- none of them `#[test]`/`#[tokio::test]` --
  which trips the pre-existing, unmodified `transformed_scope` gate for the
  **whole file**, blocking `proven` from ever being populated, independently
  of this fix's own logic.

**Conclusion:** the audit leg didn't move because the audit sample contains
essentially no genuinely eligible sites for this specific fix (same-file,
non-doctest, bare-call-in-trusted-assert-macro is rare in real non-test-harness
code), and the one clear candidate is blocked by the *same* whole-file
`transformed_scope` gate already identified in `audit-correction-2.md` as
independently blocking 18 of the 23 method/path-call misses (`#[cfg]` there).
`transformed_scope`'s file-wide blast radius -- not the specific proof
category (identifier vs. method/path vs. assert-macro-wrapped) -- is now
confirmed as the tightest constraint on this audit's real-repository numbers
across every proof category examined so far. Narrowing it (e.g. scoping the
gate to the enclosing item rather than the whole file, or excluding attributes
demonstrably free of expression-rewriting semantics like `#[cfg]` combined
with `#[inline]`) is the next resolver design question, not a method/path-call
filter extension alone.

## 4. Disclosed remaining holes

**Crate-wide macro shadowing.** The shadow check added by this fix
(`shadowed_assert_macros`) is whole-*file*, not whole-*crate*: a
`macro_rules!` redefinition of `assert`/`assert_eq`/etc. in a different file,
brought into scope via `#[macro_use] extern crate` or 2018-edition path
scoping, leaves no trace this file's own syntax tree can see. Checked
empirically rather than assumed: grepped all three audit crates and the
dispatch corpus for `macro_rules!\s*(debug_)?assert` and `#[macro_use]` --
no trusted name is redefined anywhere in either sample; the only
assert-like local macro found is `petgraph`'s `assert_one_of!` (a distinct
name, not one of the six trusted names, so it does not interact with this
check at all). This residual risk is real but not live in either measured
corpus, and is disclosed on the evidence itself: every Must claim produced by
this fix carries the new assumption string `assert-macro-not-shadowed-crate-wide`.

**Control-flow insensitivity.** Checked against the frozen policy
(`docs/call-classification-policy.md:16`): *"Must: the callee binding is
proven and has no viable alternate target"* -- this is a statement about
**binding certainty**, not runtime execution guarantee. The pre-existing
direct-call proof already grants Must to a call inside an `if` branch or
loop body that may never execute; this fix is consistent with that existing
semantics, not a new category of imprecision. Concretely, three sub-cases
get Must even though they may not run: an `assert!`/`assert_eq!` message
argument (evaluated only on failure), the right operand of an assert
condition's `&&`, and any `debug_assert*` call when debug-assertions are
compiled out. All three are consistent with the policy's own definition of
Must as read above, so no code change follows from this -- but it is
disclosed here since "Must" read informally could otherwise be mistaken for
"will execute."

## Corrected Stage 3 Rust criterion status

- `measured_dispatch_corpus_improvement`: **Met.** 20/36 -> 21/35 pooled,
  4/10 -> 5/9 Rust, `rust-direct-same-file` conservative -> exact,
  probe-verified.
- `zero_classification_errors_on_audit`: **Met.** 0/52 unsound, unchanged;
  oracle 1.0/1.0 precision and recall, unchanged.
- `nonempty_must_precision_1000_on_real_repository`: **Not met.** The
  audit's own Must set is still empty -- 0 fires of the new claim reason
  across all three crates, root-caused above (not guessed).
- `audit_shows_fewer_conservative_more_exact_cells`: **Not met.** 27/25
  before and after, byte-identical.

Stage 3 (Rust) status: **IN PROGRESS**, not DONE, not FAILED. Next resolver
step: narrow `transformed_scope`'s whole-file blast radius (the confirmed
tightest constraint across both the method/path-call and assert-macro-wrapped
proof categories on this audit), rather than adding further proof categories
without addressing the gate that independently blocks them.
