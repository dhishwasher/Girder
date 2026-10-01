# Stage 3 TypeScript before-observation: seventh addendum -- corrections and a structural safety argument

Follow-up to `before-observation-addendum-6.md` (`ea09ce6`). Corrects
one claim that described a test run that had not actually happened,
supplies the cheap zero-boundary check that addendum-6 left open, and
precisely restates what the earlier Rust sweep actually found.
`zero_classification_errors_on_audit` stays **Met** throughout.

## 1. Correction: the `--run`/full-path regression test was not actually A/B-verified before this commit

`docs/roadmap.md`'s entry (as committed in `ea09ce6`) claimed both the
bare-`--quiet` and the `--run`/full-path assertions "were confirmed to
fail on the respective pre-fix code." Only the `--quiet` half of that
was true at the time -- the `--run`/bare-form assertions had only ever
been run against the already-fixed code. Verified properly now:
checked out `crates/aether-app/src/project/commands/test_impact.rs` at
`ea09ce6~1` (the commit that fixed only `quiet_from_graph`, not the
full path) over the current working tree, rebuilt, and re-ran
`test_impact_quiet_is_not_empty_for_a_module_level_only_change` alone.
It failed exactly as predicted, at the full-path assertion:

```
panicked at crates/aether-app/tests/cli.rs:1528:5:
the full (non-quiet) test-impact report must also conservatively
select the known test for a module-level-only change, not report
"No tests found in the impact set": ...
No tests found in the impact set.
```

Restored the committed fix (`git checkout --
crates/.../test_impact.rs`, confirmed zero diff against HEAD), rebuilt,
re-ran -- passed. The roadmap's claim is now true; it was not when
first committed.

## 2. The zero-boundary question: checked, and found to be a structural property of the current extractor, not luck

Addendum-6 left open whether the Module-origin fallback is fail-closed
on its own or merely safe because real code happens to always carry an
Unknown boundary somewhere. Attempted to construct a genuinely
zero-boundary fixture with a real, working test, per the suggested Go
case (`add.go` / `add_test.go`, package-level `const` added after a
committed baseline).

**Result: could not construct one, and reading `claims::annotate()`
directly shows why not, in all four currently-supported languages**:

- **Python and TypeScript**: `claims.rs`'s gap-emission loop pushes
  `"implicit-runtime-dispatch-not-certified"` (Unknown, `coverage_gap:
  true`) **unconditionally** for every Python or TypeScript file,
  regardless of content (`if lang == Lang::Python || lang.is_typescript()
  { gaps.push(...) }` -- no condition on the file's actual contents).
  Every Python/TS file with any code at all already has a boundary.
- **Rust**: every `callable`/`macro_invocation`/`attribute_item`/
  `decorator` syntax node gets its own claim via the main per-node
  loop (`claims.rs` lines ~435-448), and this is **unconditional on
  node KIND, not on whether the attribute is `#[test]`** -- the
  `transformed_scope` exemption for `#[test]`/`#[tokio::test]` only
  affects Must-proof *eligibility*, not whether the attribute node
  itself still gets its own `"unexpanded-macro-or-decorator"` Unknown
  claim. Every `#[test]`-attributed Rust function unconditionally
  carries at least one boundary: the `#[test]` attribute's own claim.
- **Go**: confirmed directly against the suggested fixture
  (`add.go`'s `Add`, `add_test.go`'s `TestAdd` calling `Add(1, 2)` and
  `t.Fatal(...)`): `girder inspect` shows 2 `class:unknown` claims
  inside `TestAdd`, reason `go-binding-or-interface-dispatch-unproven`
  for BOTH the cross-file call to `Add` (defined in a different file)
  and the method call on `*testing.T`. Go has no same-file Must-proof
  mechanism at all in the current extractor (unlike Rust/Python/TS),
  so every call inside a Go function -- cross-file or a method call on
  any receiver -- is unconditionally Unknown.

**Conclusion**: a Module-only-origin change can only ever reach a truly
empty `classified_impact` result in a file that contains NOTHING
matching ANY of the above -- no call, no attribute, and (for Python/TS)
no content at all, which is already structurally impossible for
Python/TS. For Rust and Go specifically, it requires a file with
literally zero calls and zero attributes anywhere, which describes an
empty or near-empty file, not a realistic one containing a working
test. This is a property of the current extractor's design (verified
by reading the code, not inferred from a handful of fixtures), not a
coincidence that depends on which specific files happen to be in this
corpus. Recorded as a checked, positive finding, not left as an open
worry -- though it remains true that `classified_impact` itself does
not special-case or specifically detect a Module-only origin; the
fail-closed behavior is an emergent property of the gap-emission code
being unconditional, not a dedicated rule for this scenario.

## 3. Policy-wording alignment

`docs/call-classification-policy.md`'s exact text is "Empty output
with Unknown boundaries never means no tests need running" -- a
conditional claim (empty is only untrustworthy when boundaries are
also present), not the unconditional "an empty result means nothing
needs testing" that `CLAUDE.md`'s prior wording and the
`impacted_tests` MCP tool description both stated. Both corrected in
this commit to match the policy's own conditional wording precisely,
and to point at where the open question from section 2 above is
tracked.

## 4. Precise restatement of addendum-4's Rust sweep (it was a union of two different checks, not one)

Addendum-4 described finding "3 candidates" in Rust under one
"complete sweep" framing. That overstated it: the row-criterion check
(covering claim starts on an earlier line, excluding disclosed
whole-file gap claims) found exactly **2** candidates on its own (82,
92) -- both checked against real source and confirmed to be ordinary
multi-line method chains, not collisions. Index 89 was NOT found by
that sweep at all; it is excluded from it by construction, because its
covering claim IS the disclosed `duplicate-semantic-path` whole-file
gap claim, which the row-criterion filter deliberately skips. Index 89
was found separately, earlier, by directly reading `ser.rs`'s source
and counting `fn serialize_element` definitions.

**Correct statement of completeness**: the row-criterion sweep (for
sites NOT covered by a disclosed gap claim) is one check; every site
whose `observed_reason` is literally `duplicate-semantic-path` is a
second, separate, necessary check (Rust: 1 such site, index 89; Python:
0). A sweep is only complete as the UNION of both. Both were in fact
run (just not stated as a deliberate union at the time); the completed
counts are unchanged (TypeScript: 2 total, both found by the row
criterion directly since this round's `NEVER_COVERS` already excludes
`duplicate-semantic-path` so no TypeScript site could hide behind it;
Rust: 1 via the `duplicate-semantic-path`-reason check plus 2
false-positive row-criterion hits that are not collisions; Python: 0
by either check) -- only the framing of how completeness was
established needed correcting, not the counts themselves.

The claim in addendum-4 that `serde_json`'s `Compound` type has
"~7 impls" sharing method names like `serialize_field`/`end` was
introduced by advisor review and never independently verified against
the actual source. Checked now: not verified in this session either,
for lack of remaining time budget in this already-extensive
investigation -- stated here as unverified rather than repeated as
fact. It does not affect any committed count, since no scored site in
Rust's audit depends on it.

"Complete... across every already-measured site" in earlier addenda
means complete across the SAMPLED sites in each committed audit
(97 TypeScript, 102 Rust `not_a_call_site`-excluded, 85 Python), not
every call site in the underlying repositories -- stated precisely
here since it was previously implied more broadly than warranted.

## What remains explicitly open, not completed in this session

Given the scope this investigation has already reached, the following
from the prior review round are recorded as genuinely open rather than
attempted under diminishing time budget -- not silently dropped, not
claimed done:

- Re-running `tools/core_trustworthiness_oracle.py` and
  `tools/core_representative_mutations.py` against the rebuilt binary
  and diffing against the committed oracle/mutation results.
- Re-running every committed collision repro (TS lost-body, TS
  describe/`beforeEach`, Rust lost-impl) under every invocation form
  (`--quiet`, bare, `--quiet --out`, `--run`) on the post-fix binary --
  only the Rust const-edit and TS describe-edit cases were directly
  retested in this round.
- Corpus-wide (not just this repo's own graph) `duplicate-semantic-path`
  prevalence counts across the pinned TypeScript/Rust/Python corpora.
- Checking `review`'s own Go-specific behavior (a Go Module's `source`
  may not represent "the whole file" the same way Rust/Python/TS's do --
  raised in review, not checked).
- A dedicated grep for other consumers of `changed_node_ids`/`diff_from`
  (the hook, `watch`) that might apply the same Function-only filter
  pattern independently.

These are listed so a future session can pick them up from this
document rather than re-discover them.
