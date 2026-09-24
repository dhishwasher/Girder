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
  This one's content WAS read, via one script
  (`python3 -c` inline, not saved) that walked every `call_evidence_v1`
  attribute in the file, filtered to `coverage_gap: true` claims, and
  printed, per distinct `reason` string: a count, the max span size, and
  ONE sample `(file, start_byte, end_byte, span)` tuple. The exact
  printed output was:
  `implicit-runtime-dispatch-not-certified count: 176 max_span: 162878`,
  `unexpanded-macro-or-decorator count: 315 max_span: 208`,
  `duplicate-semantic-path count: 4 max_span: 162878` (each with one
  sample). This was read from class-validator-0.15.1 only -- not from
  any of the other three pinned repos, and not from typescript-6.0.3
  specifically, where the 6 same-file predicted sites actually live.
  **What this was and was not blind to, stated precisely rather than
  asserted as fully blind**: the script never printed or read any
  per-site `class` (must/may/unknown) value, any `targets`, or any
  `caller` -- only reason-string aggregate counts and spans restricted
  to gap claims. It therefore could not have previewed any specific
  site's Must/Unknown answer. But it DID reveal, before the prediction
  was committed, that `unexpanded-macro-or-decorator` gap claims are
  narrow (max 208 bytes) while the other two reasons are whole-module --
  a structural fact about this corpus's claim-span distribution that
  directly shaped the scorer's `NEVER_COVERS` design one commit before
  the prediction. Recorded here because the prediction document's
  specific claim ("no measurement output anywhere") is factually wrong
  regardless of how much this content mattered to the prediction's
  substance -- the honest disclosure is exactly what was read, not a
  characterization of how blind it left the process.

## 4. `duplicate-semantic-path` / Rust-audit docstring correction

`dispatch_audit_scorer_typescript.py`'s module docstring previously
claimed "neither the Rust nor the Python scorer's own `NEVER_COVERS`
includes it, a latent gap in both already-DONE audits that happened not
to matter because their specific sampled files never tripped
`duplicate_paths`." The first half is true (confirmed by grepping both
scorers' `NEVER_COVERS` definitions: Rust's is
`{"implicit-drop-or-operator-dispatch-not-certified"}`, Python's is a
single unrelated entry; neither includes `duplicate-semantic-path`). The
second half is false: Rust's own committed, final DONE audit
(`docs/observations/stage3-rust-audit/after-method-call-fix/correction-2/audit-after.json`
-- `correction-2` is the latest cited state per `docs/roadmap.md`)
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

## 5. Paren-balance / enclosing-claim check: 2 flagged in TypeScript -- investigated further, found to be an established, safe, cross-language pattern, not a TypeScript-specific gap

A proper enclosing-claim check (does each scored site's own covering
claim actually start at the site's own byte offset, or does the site
sit inside a larger claim's span with unbalanced open-parens between the
claim's start and the site -- a pattern that would indicate the site is
being scored against a claim that isn't its own) was run across all 97
scored sites. Exactly 2 were flagged:

- **Site 23** (date-fns-4.1.0, `src/intlFormatDistance/test.ts:109`):
  covering claim starts at byte 3093, site's own offset is 3269 (3
  unbalanced open-parens between them). The site itself is a
  `new_expression` (`new Date(1986, 3, 4, 10, 30, 0)`) nested as an
  argument inside a multi-line `intlFormatDistance(...)` call whose own
  claim spans 3093-3601.
- **Site 91** (typescript-6.0.3,
  `src/testRunner/unittests/tsserver/projectReferences.ts:1189`, the
  `verifySolutionScenario` Must site): covering claim starts at byte
  41250 (a `describe(...)` call's own claim, spanning its entire
  callback body to byte 57959), site's own offset is 50479.

**Both were checked for an offset bug first, and ruled out.** Reading
the raw bytes at each site's own computed offset confirms it lands
exactly on the call's own callee token (`verifySolutionScenario({` at
50479; inside `new Date(1986, 3, 4, 11, 30, 0),\n new Date(1986, 3, 4,
10, 30, 0)` at site 23's line) -- not a masker or column-math error.
**A search for any claim starting within +-1200 bytes of each site's own
offset confirms neither call gets a claim starting at its own position**
-- for site 91 specifically, claims exist ending at byte 50205 (a
different, immediately-preceding call) and nothing starts again until
inside the `describe(...)` claim's own later sub-claims; verified there
are 56 other claims genuinely nested inside the `describe(...)` claim's
41250-57959 span (i.e. many OTHER calls in that same callback body DO
get their own claim) -- `verifySolutionScenario`'s specific call is a
real, individual miss, not evidence that the whole region lacks
per-call granularity.

**This is not a coverage-granularity story specific to TypeScript's test
framework blocks -- it is corrected here from the earlier framing.**
Re-running the identical "does this site's own covering claim start at
its own recorded byte offset" check against Rust's and Python's own
final, committed, DONE, gate-passed audits (per advisor's specific
instruction) found the same "no own claim, attributed via containment to
an enclosing claim" pattern is common, not rare, across both languages:

- **Rust** (`docs/observations/stage3-rust-audit/after-method-call-fix/correction-2/audit-after.json`,
  the currently-cited final state): **24 of 52 scored sites (46%)** have
  no claim starting at their own recorded byte offset -- e.g. index 3
  (`benches/bellman_ford.rs:46`), index 89 (`src/ser.rs:504`, the
  `duplicate-semantic-path` site from section 4), and 22 others. All 24
  score `exact` or `conservative`; **none score `overclaim` or
  `unsafe_exclusion`**. Rust's own `cell_counts` for the whole audit is
  `{"exact": 28, "conservative": 24}` -- zero unsound cells overall,
  containment-attributed sites included.
- **Python** (`docs/observations/stage3-python-audit/after-transformed-scope-fix/correction-1/audit-scored-results.json`):
  **25 of 85 scored sites (29%)** show the same pattern. `cell_counts` is
  `{"exact": 73, "conservative": 12}` -- again zero unsound cells, and
  every one of the 25 containment-attributed sites is `exact` or
  `conservative`.
- **TypeScript** (this round): 2 of 97 (2%) -- the smallest share of the
  three, not the only instance.

**Conclusion, corrected from the original framing**: `find_covering_claim`'s
containment-based attribution (score a site against whichever claim's
byte range contains it, even when that claim's own `start_byte` differs
from the site's) is an established, already-relied-upon part of this
program's entire Stage 3 scoring methodology, used identically by all
three language scorers, and empirically safe everywhere it has been
checked (51 total containment-attributed sites across Rust, Python, and
TypeScript combined, zero of them unsound). It is not a defect
introduced by or unique to this round, and it does not retroactively
threaten any of the three languages' "0 unsound cells" results --
`zero_classification_errors_on_audit` stays **Met** for TypeScript, on
the evidence above, not merely by asserting the two flagged sites are
individually benign.

**What remains a genuinely open, disclosed question, not resolved
here**: whether containment-based attribution is the *right* long-term
model for what "Girder's actual answer for this specific call" means,
as opposed to a design that would instead report "no evidence" (a
distinct outcome from `unknown`) when a call gets no claim of its own.
This has not mattered to any of the 51 sites checked because in every
case the enclosing claim's class happened to match the direction the
call's own (non-existent) claim would very likely have carried too
(driven by the same whole-file/whole-region gates, e.g.
`duplicate_paths`, that would apply to a hypothetical own-claim just as
much as to the enclosing one). A future resolver round that changes
per-call claim granularity, or that scores a site whose enclosing claim's
class does NOT match what a real own-claim would report, would need to
revisit this -- flagged here as a precondition to watch for, not
designed around now.

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

## 7. Two small corrections

- The commit that introduced this addendum's first draft (`7550683`)
  says in its own message that the scorer's pytest suite has "28 tests."
  The actual run showed **27 passed**. Not amended (this program commits
  forward, not `--amend`); corrected here instead.
- That same commit's addendum text originally said `parse-error` belongs
  in `NEVER_COVERS` "for the same reason" as
  `implicit-runtime-dispatch-not-certified` and `duplicate-semantic-path`.
  That overstates the similarity: the other two are emitted on every
  file (structural, unconditional coverage gaps), while `parse-error`
  only appears on genuinely malformed/unparseable files -- arguably
  itself a form of real, visible uncertainty rather than a pure coverage
  artifact. Excluding it from scoring is still safe (it only makes the
  scorer stricter about what counts as "covering," never looser), but
  the rationale is "whole-module and therefore unusable as meaningful
  per-site evidence," not "same reason as the other two."

## Still not started

Per advisor's explicit instruction, the gate-profile step (over all 46
Must sites) has not been started. This addendum is the precondition for
it, not the gate-profile itself. Its column list is extended by one
entry from what was originally specified, per section 5 above: **"own
claim present"** (does a claim start at this site's own byte offset, or
is it only covered via containment) should be the FIRST column checked,
since section 5 established a site without its own claim cannot be
meaningfully attributed to `transformed_scope`, nesting, or
`duplicate_paths` specifically -- those gates describe why a claim came
out `unknown`, not why a claim exists at all. The remaining columns are
unchanged: `transformed_scope`, `duplicate_paths` (with cause), target
node path depth, cross-file, class construction, `parse-error`.
