# Correction to the assert-macro-fix after-observation

Found on review before starting the next resolver step. `after-observation.md`
and the roadmap's matching checkpoint entry (`3f5ed5e`, `29029d7`) are kept
unedited per the resume contract; this corrects two overclaims and adds one
verification that was computed but never written down.

## 1. "Confirmed as the tightest constraint... across every proof category" overclaimed an n=1

`after-observation.md` said `transformed_scope`'s whole-file gate was
"confirmed as the tightest constraint on this audit's real-repository
numbers across every proof category examined so far" and the roadmap's
"Next" section proposed narrowing it as the concrete next step. That rested
on exactly one candidate (`serde_json/src/lexical/math.rs:632`), and that
one candidate is **also independently blocked**, not just by
`transformed_scope`: `greater_equal` and `isub` are defined inside `mod
large { }` (confirmed: `mod large {` opens at line 541 and both functions
sit inside it at 4-space indent, before the file's next `mod` boundary), so
their parent is not the root `source_file` node and the pre-existing
`top_level` check in `crates/aether-builder/src/mapper/claims.rs` would
reject them independently of `transformed_scope`, even narrowed to nothing.

**Narrowing `transformed_scope` alone cannot move any of the 25 frozen
audit conservative cells either.** Checked directly against the frozen
sample, not the one supplementary candidate: of the 25, 23 are
method/path-shaped (excluded categorically by the identifier-only filter,
`crates/aether-builder/src/mapper/claims.rs:122`, regardless of
`transformed_scope`) and 2 are bare-identifier calls to a target imported
from another file (excluded by the pre-existing same-file-only restriction
on `proven`, also regardless of `transformed_scope`). No path through the
25 frozen sites runs through `transformed_scope` at all as the *sole*
blocker.

The roadmap's "Next" section is corrected in this commit to remove the
narrow-`transformed_scope`-first plan and replace it with: build a
per-site gate profile for all 25 conservative sites (which gate blocks
each one, and whether more than one gate blocks it) before choosing what
to implement next.

## 2. "Nearly all matches are doctest comments or cross-file calls" was a 5-of-18 sample, stated as if exhaustive

The real-repository root-cause search (`after-observation.md` section 3)
grepped 18 files across the three crates for the assert-macro-wrapped bare-
call pattern (`grep -rlnE 'assert(_eq|_ne)?!\([a-zA-Z_][a-zA-Z0-9_]*\('`
--include=*.rs . | wc -l` => 18), but the writeup only named specific
findings from 5 of them (`bellman_ford.rs`, `page_rank.rs`, `builders.rs`,
`tests/graph.rs`, `lexical/math.rs`) and phrased the conclusion ("the large
majority... are inside doc comments... the clear real-code hits are almost
all cross-file") as if the full 18 had been checked. They had not.

Correcting the claim to what was actually verified, no more: **the fix's
own evidence output is the load-bearing fact, and that one is exhaustive**
-- grepped the new claim reason (`proven-call-inside-trusted-assertion-
macro`) across all three crates' complete `call_evidence_v1` output
(every node, not a sample): **zero occurrences.** The fix provably never
fires on any of the three audited crates, full stop, independent of the
5-of-18 textual investigation. The textual investigation was root-cause
color for *why*, not itself a completeness claim; also worth noting, its
regex only matches a callee in a macro's first-argument position, so it
would miss e.g. `assert_eq!(x, f())` entirely -- a further reason not to
treat it as exhaustive.

## 3. Verification that was computed but not written down: corpus diff is exactly the one cell

Diffed every one of the 56 test cells between the committed Stage 2
baseline (`docs/observations/stage2-dispatch-corpus/scoring-results.json`,
scorer `e35c298`, sha256 `8ddd746526c64406363a0fbff67ffd7f9c70c0feccd93e18fa6a2c7fd9a6b4c7`)
and this fix's after-run (`corpus-after.json`, scorer `706b344`, sha256
`0f776f407a5b475bec7e8329bf251e607a14b6b1d4e8397529a074c9257a99a5`), by
`(case_id, test_id) -> cell`, not by matching per-language totals (which
could hide offsetting moves). Result: **exactly one diff**,
`('rust-direct-same-file', 'test_direct'): conservative -> exact`. No
other case in any language moved in either direction.

The scorer's sha256 differs between the two runs because `706b344`
(committed as part of Stage 2's own disclosure corrections, before this
fix cycle started) fixed `resolve_symbol`'s qualifier handling: a qualifier
that matched nothing used to silently fall through to an unqualified
candidate when exactly one remained, which could return the wrong node
without raising an error; it now correctly returns no match instead. That
change affects only symbol resolution when a qualifier is given and stale
candidates are involved -- it does not touch `cell_label` or the confusion-
matrix logic the "measured improvement" claim rests on, and the full
56-cell diff above confirms empirically that it introduced no hidden
change to this particular measurement.

Additional hashes computed at measurement time but not recorded in
`after-observation.md`: binary `/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder`
sha256 `e8ab37337968d8e32a390f8caac4e147b6dcfb28a315f4aaffaa5e618ec54dc0`
(also stated in the header); `docs/dispatch-corpus.json` sha256
`9e3208a8f3fdc8faaee55cece63c1b5e25526322ebebbba915652f4f526ccd0a`
(unchanged since Stage 2); `docs/call-classification-policy.md` sha256
`d6d686a9033c5a9e44665e8134707a8074296352ec3af28507286d925d96befb`
(unchanged since Stage 1); `tools/dispatch_audit_scorer.py` sha256
`be46062f0ae0d22a55ab9254da59aea669881259e50d1dcc86a9fb4c4d2e42ab`;
`tools/core_trustworthiness_oracle.py` sha256
`677127c1317ae9239cb0e6408b4ca33d2a6cec9e11d670f85e5cf9d63fc35493`;
`docs/observations/stage3-rust-audit/audit-sites-labeled-v2.json` sha256
`ea5c653bc1f59478d02bcc9b55c9ab5cfa499af756c67ac70be530887eacadb2`.

## What still holds, unchanged

Everything else in `after-observation.md` and the roadmap checkpoint is
unaffected: the dispatch-corpus improvement (now doubly confirmed by the
exhaustive 56-cell diff above), the probe-verification of
`rust-direct-same-file`, the zero-unsound and oracle-no-regression results,
the two disclosed-but-unfixed holes, and the corrected criterion status
(two of three legs met, Stage 3 Rust IN PROGRESS, not DONE, not FAILED).
Only the specific "next resolver step" recommendation is wrong and is
corrected in this commit's roadmap edit.
