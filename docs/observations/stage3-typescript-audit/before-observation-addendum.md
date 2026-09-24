# Stage 3 TypeScript before-observation: correction addendum

This addendum corrects and extends `before-observation.md` (commit
`2c7f6e8`) and `before-observation-prediction.md` (commit `a1428b4`)
following a further advisor review of the already-published before-
observation. Per this program's own established practice, neither of
those documents is edited in place — this is a separate, additive
record. Nothing in `before-observation.md`'s headline result changes:
**97 scored, 8 not_a_call_site, 0 unsound cells** stands. Binary
measured, explicitly recorded here for the first time:
sha256 `f87d1d058bb91418e817af35efb4956096a34e486ea39d7346a17477bb50d96f`
(built from commit `86fd130`; confirmed unchanged across the entire
labeling and measurement round -- only Python tooling/docs were touched
in between).

## 1. Corrected same-file count: 6, not 8

`before-observation.md` states "every one of the 8 investigated here is
nested" and `before-observation-prediction.md` predicted 8 same-file,
decorator-free Must sites as provable. Re-checking each of the 8 against
its own `true_target` field in `audit-sites-labeled.json`: **sites 29 and
34 are actually cross-file** -- both have `true_target` pointing at
`tracing.ts:126` / `tracing.ts:160`, not at the same file as their call
site. Only 6 of the 8 listed sites are genuinely same-file. The
prediction's own reasoning was internally inconsistent: it should not
have classified sites 29 and 34 as "same-file, decorator-free provable"
in the first place, independent of the `top_level`/nesting finding that
explains the other 6. This does not change the observed result (all 8
still scored `unknown`, 0 still scored `must`) -- it corrects the
prediction document's own bookkeeping.

## 2. Corrected "12 conservative sites" references: should be 46

`docs/roadmap.md` (the Stage 3 TypeScript "Next step" bullet, in the
checkpoint added at `408791b`) and `before-observation.md`'s own
"concrete lead for the next step" section both say to gate-profile "the
12 real-audit conservative sites." **12 is Python's number, carried over
by mistake when TypeScript's checkpoint text was drafted from Python's
template.** TypeScript's real-repository audit measured in this round
has **46 conservative sites** (every one of the 46 Must-labeled sites).
Any future gate-profile step (still not started, per advisor's explicit
instruction) must run over all 46, not 12.

## 3. Disclosure: two scratchpad measurement files predated the precommitted prediction

`before-observation-prediction.md` states there was "no measurement
output anywhere in the repo or scratchpad" before the prediction was
committed (`a1428b4`, 2026-09-24 01:50:46 -0400). This is false. Two
files existed on disk first, confirmed by mtime:

- `scratchpad/ts-check/TypeScript-6.0.3/inspect-out.json` (mtime
  2026-09-24 01:08:28). This was produced by an earlier, unrelated
  `girder inspect --json` size/RSS check (measuring output size and
  memory behavior on the largest pinned repo) -- its per-claim content
  (classes, reasons, targets) was not read or used for any
  classification, labeling, or prediction reasoning.
- `scratchpad/class-validator-inspect.json` (mtime 2026-09-24 01:43:56).
  This one's content WAS read, for the `NEVER_COVERS` investigation
  documented in `dispatch_audit_scorer_typescript.py`'s own module
  docstring: specifically, the reason strings and byte spans of
  `implicit-runtime-dispatch-not-certified` and
  `duplicate-semantic-path` claims (both whole-module), and the ~315
  `unexpanded-macro-or-decorator` claims' narrow, per-decorator byte
  spans, used to decide `NEVER_COVERS`'s membership. No site-level Must/
  Unknown/May classification result from this file was read or used to
  inform the prediction itself -- the investigation was scoped to
  designing the scorer's coverage-gap exclusion set, not to previewing
  real-repository answers. Recorded here because the prediction
  document's specific claim ("no measurement output anywhere") is
  factually wrong regardless of whether the content mattered to the
  prediction's substance; the honest disclosure is the file existed and
  what was actually read from it, not a claim that it didn't exist.

## 4. `duplicate-semantic-path` / Rust-audit docstring correction

`dispatch_audit_scorer_typescript.py`'s module docstring previously
claimed "neither the Rust nor the Python scorer's own `NEVER_COVERS`
includes it, a latent gap in both already-DONE audits that happened not
to matter because their specific sampled files never tripped
`duplicate_paths`." The first half is true (confirmed by grepping both
scorers' `NEVER_COVERS` definitions: Rust's is
`{"implicit-drop-or-operator-dispatch-not-certified"}`, Python's is a
single unrelated entry; neither includes `duplicate-semantic-path`). The
second half is false: Rust's own committed DONE audit
(`docs/observations/stage3-rust-audit/after-method-call-fix/correction-1/audit-after.json`)
has exactly one scored site (index 89, serde_json `ser.rs:504`) with
`observed_reason: duplicate-semantic-path`. That site's cell is
`conservative` (`true_class: may`, `observed_class: unknown`) -- sound,
not a threat to Rust's DONE status. Python's committed audit has zero
such hits. The docstring has been corrected in place
(`tools/dispatch_audit_scorer_typescript.py`, this commit) to state the
accurate fact: the omission from both prior scorers happened to land on
a site that was safe either way, not an untested gap that "never
tripped."

`parse-error` has also been added to `NEVER_COVERS` (was previously
absent from the set entirely, not just under-documented). Confirmed
whole-module: 4 instances in typescript-6.0.3
(`transformers/utilities.ts`, `types.ts`,
`es2015.symbol.wellknown.d.ts`, `services/exportInfoMap.ts`), each with
`start_byte == 0` spanning the file's full length. Checking the
`observed_reason` distribution across all 97 scored sites in this
round's actual measurement confirms only two reasons ever became a
scored site's covering claim
(`typescript-binding-or-structural-dispatch-unproven` x83,
`unresolved-call-syntax` x14) -- `parse-error` never mattered to this
round's result, but its absence from `NEVER_COVERS` was a latent
correctness gap for any future round where it might. A regression test
(`test_parse_error_also_never_covers`,
`tools/test_dispatch_audit_scorer_typescript.py`) was added alongside
the fix; full suite re-run clean (27 passed).

## 5. Paren-balance / enclosing-claim-masking check: 2 flagged, both confirmed benign

A proper enclosing-claim check (does each scored site's own covering
claim actually start at the site's own byte offset, or does the site
sit inside a larger claim's span with unbalanced open-parens between the
claim's start and the site -- a pattern that would indicate the site is
being scored against someone else's claim rather than its own) was run
across all 97 scored sites. Exactly 2 were flagged:

- **Site 23** (date-fns-4.1.0, `src/intlFormatDistance/test.ts:109`):
  covering claim starts at byte 3093, site's own offset is 3269 (3
  unbalanced open-parens between them).
- **Site 91** (typescript-6.0.3,
  `src/testRunner/unittests/tsserver/projectReferences.ts:1189`, the
  `verifySolutionScenario` Must site): covering claim starts at byte
  41250, site's own offset is 50479 (2 unbalanced open-parens between
  them). Direct inspection of nearby claims confirmed no narrower
  per-call claim exists anywhere near this site's byte position -- the
  covering claim is a single large (~16.7 KB) region.

Both were investigated directly, not just flagged and left. Neither
threatens `zero_classification_errors_on_audit`'s "Met" status: in both
cases the enclosing claim's class matches the safe/conservative
direction already recorded (site 91 is a Must site scored
`conservative`, i.e. Girder reports `unknown` -- a large enclosing
"unproven" claim covering it is still the conservative, correct-
direction answer, not an overclaim). No false Must, no false May, and no
genuinely reachable site was actually excluded by this.

Site 91 does reveal a genuine, previously-undisclosed **extractor
coverage-granularity gap**, worth recording as a real, non-blocking
limitation rather than a soundness defect: Girder's TypeScript extractor
does not emit a narrow per-call claim for calls nested deep inside test-
framework callback structures (`describe()`/`it()` blocks, in this
corpus's test-runner-heavy files); instead it covers a large region with
one coarse "unproven" claim. This is conservative (never wrong-
direction) but coarse -- a future resolver round targeting this file
would need finer-grained claim boundaries before it could ever prove
anything inside such a block, independent of whatever else blocks the
proof (see section 6).

## 6. Nesting vs. `duplicate_paths`: competing hypotheses, not collapsed into one

The 6 genuinely same-file predicted sites' target functions were checked
against BOTH candidate explanations for their Must-proof failure,
following advisor's instruction to record them as competing hypotheses
rather than picking one without evidence:

- **`checker.ts`** (`narrowTypeByTypeFacts`): mechanically confirmed
  nested via its own `node.path` field --
  `crate::compiler::checker::createTypeChecker::getFlowTypeOfReference::narrowTypeByTypeFacts`
  (nested three levels under `createTypeChecker`). Also confirmed
  `checker.ts` IS in the `duplicate-semantic-path`-affected file set.
  Both explanations apply; this file alone cannot isolate which one is
  doing the work.
- **`compileOnSave.ts`** (`test`): its `node.path` has two entries, one
  clearly nested under `should respect line endings::test`. Also
  confirmed in the `duplicate-semantic-path`-affected set. Same
  situation -- both explanations co-occur.
- **`projectReferences.ts`** (`verifySolutionScenario`): `node.path` is
  `crate::testRunner::unittests::tsserver::projectReferences::verifySolutionScenario`
  -- only ONE level of nesting shown in the path string, unlike the
  clearly multi-level nesting seen for the other three functions. This
  is genuinely ambiguous as a standalone test of the `top_level`
  hypothesis specifically. Also confirmed in the `duplicate-semantic-
  path`-affected set, which is a SUFFICIENT explanation for this file's
  non-Must status on its own regardless of what `top_level` alone would
  have decided. **Left explicitly unresolved** whether `top_level` would
  independently reject this site -- not guessed at, since `duplicate_paths`
  already fully accounts for the observed result here.
- **`emitter.ts`** (`emitExpression`, `popNameGenerationScope`,
  `writePunctuation`): all three mechanically confirmed nested via
  `node.path` -- `crate::compiler::emitter::createPrinter::<name>` for
  each. Confirmed `emitter.ts` is **NOT** in the `duplicate-semantic-
  path`-affected file set. This is the one clean, isolated test case
  among the four: `emitter.ts`'s Must-proof failure is attributable to
  the `top_level` nesting check alone, with `duplicate_paths` ruled out
  as a co-explanation for this specific file.

Net: the nesting hypothesis is confirmed as a real, independently-
sufficient blocker for at least `emitter.ts`. For the other three files,
nesting and `duplicate_paths` are both present and neither has been
isolated as the sole cause -- correctly recorded as competing,
unresolved hypotheses for those three, not collapsed into a single claim
the evidence doesn't support.

## Still not started

Per advisor's explicit instruction, the gate-profile step (over all 46
Must sites, columns: `transformed_scope`, `duplicate_paths` with cause,
target node path depth, cross-file, class construction, `parse-error`)
has not been started. This addendum is the precondition for it, not the
gate-profile itself.
