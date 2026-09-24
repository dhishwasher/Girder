# Stage 3 TypeScript before-observation

Source commit: `48b6c11` (scorer) + `511fe77` (final labels) +
`a1428b4` (precommitted prediction). Binary measured:
`/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder`.

## A real bug found and fixed mid-measurement, not silently worked around

The first scorer run (against `48b6c11`'s scorer, before the fix below)
produced **24 `unsafe_exclusion` cells** -- a severe deviation from the
precommitted prediction (`a1428b4`: 0 unsound cells expected anywhere).
Per that prediction's own stated rule, this was treated as a stop signal
and investigated before trusting any result, not reported as-is.

**Root cause, found by direct investigation**: `typescript-6.0.3` uses
CRLF line endings throughout (54,434 occurrences in `checker.ts` alone).
`Path.read_text()` performs Python's default universal-newline
translation, silently converting every `\r\n` to `\n` in the returned
string -- but Girder's own reported byte offsets are against the RAW,
untranslated file bytes. The scorer's byte-offset reconstruction (joining
`splitlines()`'s own, translation-blind, always-`\r`-free line list with a
single `\n`) therefore undercounted by exactly one byte per preceding
line. Confirmed directly, not just reasoned about: a real site 29,877
lines into `checker.ts` computed an offset 29,876 bytes short of the
verified ground truth (`bytes.find()` on the raw file) -- an exact match
to the line count.

**Fixed** (commit alongside this document): read the file as bytes and
decode directly (`decode()` never translates newlines, unlike text-mode
file reading), reconstruct the prefix from `text.split("\n")` (which
leaves any `\r` attached to each line, reproducing the file's exact
original bytes when re-joined) instead of from `splitlines()`'s
`\r`-stripped entries. A regression test (three CRLF-terminated lines
before a real call, asserting the exact byte position and that the
LF-only-assuming computation would have landed 3 bytes short) was added
before re-running against real data. Re-verified the fix directly against
the original failing site: computed offset now matches the ground-truth
`bytes.find()` position exactly (`1777900`, both ways).

## Result after the fix: 0 unsound cells, prediction's count was wrong but the underlying measurement is sound

[audit-scored-results.json](audit-scored-results.json): **97 scored, 8
not_a_call_site, 0 site_relocation_failed, 0 failed**.

- **51 Unknown-labeled sites: all 51 score `exact`.** Girder's current
  TypeScript extraction correctly reports `unknown` for every one of
  these -- built-in targets, structural/interface receivers with
  unbounded implementors, `any`-typed receivers, computed-property
  dispatch, and the rest of `labeling-rubric.md`'s Unknown cases are all
  already reported as `unknown` by the unmodified extractor, not silently
  dropped or wrongly claimed.
- **46 Must-labeled sites: all 46 score `conservative`.** Girder's
  current TypeScript extraction proves `must` for **zero** of them.
- **0 unsound cells anywhere** (0 `overclaim`, 0 `unsafe_exclusion`).
  `zero_classification_errors_on_audit` is **Met**.

## The precommitted prediction was wrong (8 exact expected, 0 observed) -- investigated, not silently adjusted

`before-observation-prediction.md` predicted 8 of the 46 Must sites
(same-file, in a decorator-free file) would score `must`. All 8 scored
`unknown` with reason `typescript-binding-or-structural-dispatch-unproven`
(the fallback reason, never `proven-top-level-lexical-binding`).
Investigated directly rather than left as an unexplained miss: every one
of the 8 predicted sites' target functions (`narrowTypeByTypeFacts`,
`popNameGenerationScope`, `emitExpression`, `writePunctuation`, the local
`test`/`verifySolutionScenario` helpers) is declared **nested inside
another function's closure**, not at true module scope --
`narrowTypeByTypeFacts` sits inside `export function createTypeChecker(...)
{ ... }`, which itself spans nearly the entire multi-million-byte file
(confirmed directly: `narrowTypeByTypeFacts`'s own character offset is
well after `createTypeChecker`'s). `claims.rs`'s shared `top_level` check
(`parent.id() == root.id()`) requires a candidate function's PARENT node
to be the module root itself -- a function nested inside a giant factory
closure has the factory function as its parent, not the module, so it was
never eligible for the same-file proof mechanism in the first place,
regardless of decorators.

The prediction's reasoning (same-file + decorator-free is sufficient) was
**incomplete**, not wrong about the mechanism it did check -- it omitted
the `top_level` requirement, which is nearly always trivially satisfied
in Rust/Python/Go code (module-level functions are the norm there) but is
**rarely satisfied in this specific TypeScript corpus**, where the
"export one giant factory function containing everything as nested
closures" pattern dominates the compiler's own internal architecture (and
is a common, broader TypeScript/JavaScript idiom, not unique to this
codebase). This is a genuine, useful finding about TypeScript's own
dispatch-ambiguity landscape, not a defect in the measurement -- recorded
honestly as a missed prediction with its real cause identified, the same
discipline this program's Python round applied to its own
"supplementary count" prediction miss.

## Updated Stage 3 TypeScript criterion status

- `zero_classification_errors_on_audit`: **Met** -- 0/97 unsound cells.
- `nonempty_must_precision_1000_on_real_repository`: **Not Met**. Girder
  currently proves zero Must sites in the frozen 105-site sample --
  precision is undefined on an empty set, not 1.000. No resolver design
  or implementation has been attempted for TypeScript yet; this
  before-observation exists specifically to measure the baseline before
  any such work begins, mirroring exactly where Rust's and Python's own
  Stage 3 rounds started.
- `measured_dispatch_corpus_improvement`: **Not Met** -- no TypeScript
  resolver change has been made this round to measure an improvement
  against; the existing pooled dispatch corpus's TypeScript cells are
  untouched.

**Stage 3 TypeScript remains IN PROGRESS, not DONE** -- this document
reports the honest baseline, exactly as this program's own established
discipline requires (measure and publish, don't chase a better number).

## A concrete lead for the next step (gate-profile)

Unlike Python's own gate-profile round (which found `transformed_scope`
-- any decorator anywhere in a file -- as the dominant blocker),
**this round's before-observation surfaces a different, TypeScript-
specific dominant blocker: nested-function/closure scope**. A proper
gate-profile step (mirroring `dispatch_audit_gate_profile_python.py`'s
role) should check, for each of the 12 real-audit conservative sites and
each Must-labeled site in this sample, whether `top_level`'s
`parent.id() == root.id()` requirement is the SPECIFIC blocker (as it
appears to be for all 8 investigated here) versus `transformed_scope`
(still unmodified for TypeScript, unchecked this round for the other 38
Musts) versus something else entirely -- not assumed from this round's
8-site sample alone.
