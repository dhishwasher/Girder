# Addendum to methodology.md: corrections found by a further review

An `advisor` review of the committed `methodology.md` (`0b2ba8c`) found one
real coverage gap, one real selector bug, and several inaccurate claims.
Per this session's practice, `methodology.md` is not edited in place;
corrections and the resulting re-selection are recorded here.

## 1. Zero decorator coverage -- a fourth repository added, selection regenerated

`shape_counts_pool` (added to the selector's own output as part of this
fix) showed **zero** `decorator`-shaped matches across all three
originally-pinned repositories, despite `methodology.md` §6 claiming
stratification across "seven shapes." Investigated directly, not assumed:
grepping for `^\s*@[A-Za-z]` in `typescript-6.0.3` found 8 files, but every
hit is inside a **template-literal test fixture** -- the compiler's own
test suite embeds TypeScript source snippets (including ones using
decorators) as backtick strings fed to an evaluator, which `mask_ts_source`
correctly treats as string content, not executing code. Genuine,
directly-executing decorator usage is truly absent from all three original
repositories (zod and date-fns: 0 files each, confirmed by the same grep).

This also invalidated `methodology.md` §9's listed "known lead" that a
future gate-profile step would find `transformed_scope` (any decorator
anywhere in a file) blocking real sites -- with zero decorator-using files
in the corpus, that gate is vacuously false everywhere and could never
have been exercised.

**Fixed by adding a fourth repository**, not by lowering the shape count
or leaving the gap disclosed-but-unfixed: `class-validator-0.15.1`
(MIT-licensed, pinned in `docs/stage3-typescript-corpus.json`,
`sha256 100a22e5...`). Checked before pinning, not assumed suitable:
26 files with genuinely-applied (not template-literal-embedded) decorator
usage (`@IsEmail()`, `@IsLongerThan(...)`, etc. on real class properties in
`sample/` and `test/`), 236 archive members (no risk of repeating the
member-count problem `typescript-6.0.3`'s full tarball hit), memory
feasibility checked directly (`girder analyze` peaked at 23MB RSS / 1.6s).

Re-ran the selector with the fourth repository and the `KEYWORDS_NOT_CALLS`
fix below: pool grew from 132,578 to 136,471 (class-validator alone
contributes 315 `decorator`-shaped matches to the pool). The regenerated
105-site selection now has all seven shapes represented, 15 each exactly
(105 = 7 × 15) -- `docs/observations/stage3-typescript-audit/audit-sites.json`
was overwritten with this result (not a new file; the original, published
before this addendum, is superseded in place since no site had been
labeled yet -- see §4 below for exactly what had already been read).

## 2. `super`/`import` wrongly excluded real call sites

`KEYWORDS_NOT_CALLS` included `super` and `import`. Both are wrong:
`super(args)` is a real parent-constructor call, and `import("x")` is a
real dynamic-import call expression -- both extremely common TypeScript/
JavaScript patterns, unlike Python's `super()` (not a Python keyword at
all, so this exact collision never arose in that audit). Removed both from
the set; two regression tests added
(`test_super_call_is_a_real_plain_call`,
`test_dynamic_import_is_a_real_plain_call`). Static `import { x } from
'y'` is unaffected: it has no `identifier(` shape at all, so removing
`import` from the reject set doesn't cause it to be picked up.

## 3. Masker spot-check: no over-masking found

The 12%/18,141-line masking reduction (methodology.md §4) showed the
masker removes something, not that it removes only the right things. Ran
a dedicated check: every line that matched a shape pattern RAW but not
AFTER masking (18,212 lines, count differs slightly from methodology.md's
18,141 because this was re-run after the `super`/`import` fix changed
what counts as a raw match), sampled 20 with a fixed seed (`20260924`),
and read each one directly against its source. All 20 are genuinely
correct maskings: line comments, JSDoc block-comment lines
(`* @param`/`* [MDN Reference]`), a regex literal, and -- the two that
looked suspicious on first read (`let [{ [order(1)]: x } = order(0)] =
[{}];` and `constructor() {`, neither containing any visible string/
comment/regex on the line itself) -- both confirmed, by reading the
surrounding file context, to be inside a multi-line template-literal test
fixture (the same "compiler evaluates TypeScript-source-as-a-string"
pattern from §1), correctly masked as string content spanning multiple
lines. No over-masking bug found; the masker's conservative behavior on
template-literal-embedded test fixtures is doing exactly what it's
supposed to.

## 4. Already-read sites, disclosed honestly

Before `methodology.md` was committed, roughly 22 site texts (10 from an
early full-file dump plus 3 each for 4 shapes) were printed and read from
the **original, 3-repository** selection -- contradicting that document's
implicit "no site had been opened" framing, which was carried over
uncritically from Python's own methodology template rather than checked
against this session's own actions. Only site **text** was viewed, never a
`true_class`/label, so this does not compromise labeling validity, but it
should have been disclosed rather than silently true.

After §1's re-selection (a different repository set, different pool,
different sample), checked directly which of those 22 previously-viewed
`(package, file, line)` triples still appear in the final selection:
**exactly one** -- `typescript-6.0.3 src/compiler/checker.ts:8561`
(`if (context.typeParameterSymbolList?.has(symbolId)) {`, an
`optional_chaining_call` site). This one site's text was seen before the
rubric existed; its eventual label should be treated with that in mind
during any future audit of the audit, though seeing call-shaped line text
in isolation (no surrounding function/class context, no cross-referenced
definition) is a weak form of exposure compared to the deliberate,
rubric-guided reading labeling itself requires.

## 5. Corrected claims

- **"12% ... smaller than Python's 33%" compared different measures.**
  Python's 33% counts LINES touching a string or comment; TypeScript's
  reduction counts the change in MATCHED-LINE POOL SIZE, a different
  measure -- and it was also recomputed with the final code (four
  repositories, corrected `KEYWORDS_NOT_CALLS`): raw 154,762 &rarr;
  masked 136,471, an **11.8%** reduction (per-repository: typescript-6.0.3
  120,062 &rarr; 103,711; zod-3.23.8 10,461 &rarr; 9,808; date-fns-4.1.0
  20,311 &rarr; 19,103; class-validator-0.15.1 3,928 &rarr; 3,849).
  Python's own matched-line-pool reduction (the comparable measure) was
  49,953 &rarr; 45,321, about **9%**. By the correct, comparable measure,
  TypeScript's masking effect (11.8%) is actually *larger* than Python's
  (9%), not smaller as originally stated.
- **"TypeScript 7.x ... ships no `.ts` compiler source" was asserted,
  not checked.** Verified directly this round with the GitHub contents
  API against `v7.0.2`: `src/compiler/checker.ts` exists and is
  **exactly the same size** (3,151,774 bytes) as the v6.0.3 file this
  audit uses. The "TypeScript 7 is a Go rewrite with no `.ts` source"
  premise was simply wrong. This does not change the choice of
  `typescript-6.0.3` (still a reasonable, verified, stable pin), only
  the stated justification, which is corrected here rather than left
  standing as a false claim.
- **`.d.ts` files are in scope and were not mentioned.** `typescript-6.0.3`
  has 108 `.d.ts` files among its 709 `.ts`-matching files (the selector's
  `rglob("*.ts")` matches `.d.ts` too, since `.d.ts` ends in `.ts`); zod
  has 1, date-fns has 2, class-validator has 0. `.d.ts` files are
  declaration-only (no executable bodies), so any call-shaped match inside
  one is a real ambiguity the rubric needs its own rule for (labeled
  `not_a_call_site` in the common case of a bare signature, per the same
  reasoning Python's rubric case 9 already uses for `def f(`-shaped
  declarations) -- this will be made explicit in the rubric itself
  (`labeling-rubric.md`, not yet written), not resolved here.
- **`.tsx` scope was "not verified empty".** Checked directly:
  `typescript-6.0.3` 0 `.tsx` files, `zod-3.23.8` 0, `class-validator-0.15.1`
  0, `date-fns-4.1.0` **2**. The selector's `rglob("*.ts")` genuinely
  never sees these 2 files, confirmed rather than assumed -- `.tsx` remains
  entirely out of scope for this round, with the actual affected-file
  count now stated instead of "not verified."

## 6. Not yet done

The scorer (`dispatch_audit_scorer_typescript.py`) has not been written.
Per this review's own guidance, it must reuse the Python scorer's
corrected byte-offset logic (mask the file, find the match column on the
masked line, read the offset from the raw line, fail closed via a
`line_text_matches`-equivalent check if they diverge) and must compute
UTF-8 byte offsets, not character offsets, tested against a non-ASCII file
before being trusted -- the TypeScript compiler source contains non-ASCII
text (locale strings, comments crediting international contributors, seen
directly in date-fns's `src/locale/*` files during the spot-check above).
`inspect --json` output size and peak RSS for `typescript-6.0.3` should be
measured before any scorer script loads that output wholesale with
`json.load` -- pydantic's own inspect output was already 22MB at a much
smaller source size; the TypeScript compiler's could be substantially
larger.
