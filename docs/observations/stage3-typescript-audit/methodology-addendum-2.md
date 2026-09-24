# Addendum 2 to methodology.md: decorator-label facts, corrected claims

A further review of `methodology-addendum.md` (`f0cf901`) asked two
questions that need answering before the rubric can handle decorator
sites correctly, and found two more claims worth correcting. Per this
session's practice, neither prior document is edited in place.

## 1. Decorator mode and cross-file resolution, checked directly

`class-validator-0.15.1/tsconfig.json:14`: `"experimentalDecorators":
true` -- **legacy decorators**, not the TC39 stage-3 decorators
TypeScript also supports. This matters for the rubric: a legacy decorator
factory's returned function receives `(target, propertyKey)` (or
`(target, propertyKey, descriptor)` for methods/accessors); a TC39
decorator's returned function receives `(value, context)` instead. Every
sampled `decorator` site in this corpus is a legacy-decorator factory call
(`@IsEmail(...)`, `@IsLongerThan(...)`, etc.), so the rubric only needs to
handle the legacy shape for this corpus, though a future round adding a
TC39-decorator repository would need the other shape too.

Checked, not assumed: `sample/sample2-using-groups/Post.ts:1` imports
`IsEmail` via `'../../src/decorator/decorators'` (a relative path), and
`test/functional/custom-decorators.spec.ts` imports similarly
(`'../../src/validation/Validator'`, `'../../src/register-decorator'`,
etc.) -- both resolve **cross-file, within the snapshot**, to real
definitions in `src/`. This means the two calls every decorated line
actually contains (per Python's rubric case 8's model, which applies here
too) are:
1. **The factory call itself** (`IsEmail(undefined, {...})`) -- a real,
   in-snapshot call, resolvable the same way any other cross-file
   TypeScript call is (once cross-file resolution exists for this
   language's resolver, which it does not yet).
2. **The application of whatever the factory returns** to the decorated
   property/method -- this is dynamic (the returned value's identity
   isn't known statically without tracing the factory's own body), and by
   the same reasoning Python's rubric case 5 gives for a Python decorator
   ("a decorator replaces the function object... Unknown unless the
   decorator's effect is traced and proven"), this is **Unknown** unless
   the specific factory's body is read and shown to have no
   dispatch-relevant effect.

## 2. `verify_inventory` for `class-validator-0.15.1`, run (was missing)

`methodology-addendum.md` did not show this check for the fourth
repository, unlike the other three. Run now: passes, confirming
`docs/stage3-typescript-corpus.json`'s recorded `sources.{files, bytes,
physical_lines, manifest_sha256}` for `class-validator-0.15.1` match the
extracted tree exactly.

## 3. Correction: the "known lead" claim was backwards

`docs/roadmap.md`'s checkpoint entry (`73e9699`) implied the decorator fix
made `transformed_scope` newly exercisable against **production** code.
Checked directly against the actual selected sites: all 15 sampled
`decorator` sites are factory calls in `class-validator`'s `sample/` or
`test/` files -- **none** are inside `src/`, and no `src/`-internal
same-file Must proof depends on `transformed_scope` being false in any of
those 26 decorator-using files (`src/` itself has zero decorator usage,
confirmed in `methodology-addendum.md` §1). `transformed_scope` will trip
for all 26 decorator-using files, correctly gating **other**, non-decorator
call sites inside those same test/sample files (e.g. a plain helper call
elsewhere in `custom-decorators.spec.ts`) -- so the gate IS exercised, just
not against production dispatch code the way the roadmap entry implied.
Corrected here rather than in the roadmap entry itself.

## 4. Correction: the masking-percentage comparison used mismatched inputs

`methodology-addendum.md` §5 compared TypeScript's 11.8% (four
repositories, corrected `KEYWORDS_NOT_CALLS`) against Python's 9.3%
(`49,953 -> 45,321`) directly -- an apples-to-oranges comparison, since
the TypeScript figure includes both the repository-count change and the
keyword fix, neither of which Python's own baseline reflects. The correct,
isolated comparison uses TypeScript's *original* three-repository,
*original*-keyword figure, already computed in `methodology.md` itself
before either later change: **150,719 raw &rarr; 132,578 masked, 12.0%**.
Python: 49,953 &rarr; 45,321, **9.3%** (methodology.md rounded this to
"~9%"; the exact figure is 9.3%). The conclusion stands --
TypeScript's masking effect (12.0%) is larger than Python's (9.3%) -- but
on the correct, isolated inputs, not the four-repository figure.

## 5. `inspect --json` size and peak RSS for `typescript-6.0.3`, measured

Per `methodology-addendum.md` §6's own stated need, before any scorer
script is written: `girder inspect --json` against the full
`typescript-6.0.3` graph (709 files, 36,566 nodes, 78,631 edges) took
**4 seconds**, peaked at **229MB RSS**, and produced a **50.5MB** JSON
file -- larger than pydantic's 22MB (as expected from a much larger
source tree) but well within this VM's budget for a plain `json.load` in
the scorer, alongside whatever else is running. No streaming parser is
needed.

## 6. Not yet done

The labeling rubric (`labeling-rubric.md`) is next, quoting
`docs/call-classification-policy.md`'s TypeScript row directly. It must
resolve, using the facts above: whether `obj?.m()` on a provably-bound
receiver is Must (the call may simply not execute, but Python's own
rubric already treats a call inside an ordinary conditional as still
Must -- the same reasoning plausibly applies to optional chaining, to be
decided explicitly, quoting the policy row, not asserted by analogy) or
whether it needs its own explicit carve-out; how overloads resolve to a
single implementation body (Must if that implementation is unique, not
demoted to May merely because multiple signatures exist); the two-call
model for decorator lines from §1 above; rebinding shapes with no Python
analogue (`Foo.prototype.m = ...`, `Object.defineProperty`, module
augmentation, namespace/declaration merging); `declare`/ambient targets
(outside the snapshot); and `.d.ts` declaration-only lines
(`not_a_call_site`).
