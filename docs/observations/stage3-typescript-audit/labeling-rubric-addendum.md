# Addendum to labeling-rubric.md: two contradictions fixed before labeling

A further review found two internal contradictions in the committed
rubric (`a20afd8`) and one unstated dependency. Fixed here rather than by
editing the rubric in place, per this session's practice. None of the 105
sites had been labeled when this was found.

## 1. Case 5 contradicted itself on what "the site" is

Case 5 said the factory's return-value application "never becomes its own
selected site," implying the selected decorator LINE is scored as the
factory call. But it never said this explicitly, and a labeler reading
case 5's "two calls" framing in isolation could reasonably score the
SAME selected site (e.g. `@IsEmail(undefined, {`) against either call.

**Resolved**: under legacy decorators, TypeScript emits
`__decorate([IsEmail(...)], target, key)` -- the factory call
(`IsEmail(...)`) has real source text and a real byte position (the
decorator expression itself); the application of its return value to the
property (the implicit call `__decorate` performs) has NO source text of
its own anywhere in the original file. **The selected site IS the factory
call, full stop** -- scored by cases 1-4 like any other call, no special
decorator-specific downgrade. The return-value application is not a
separate scoreable site; it is recorded only in the rationale, as
context, never given its own `true_class`.

## 2. Case 10 was wrong about bare `@name`

Case 10 claimed a bare `@name` (no parens) is `not_a_call_site`. This
directly contradicts Python's own rubric case 8, which case 10 was
supposed to mirror: Python's bare `@some_name` **is** a real call
(`some_name(f)`), and the same holds here -- under legacy decorators,
`@dec` on its own emits `__decorate([dec], target, key)`, a real call to
`dec(target, key)`. If `dec` is a single, unambiguous, unrebound
in-snapshot binding, that call is scoreable by cases 1-4 exactly like a
factory call.

**Checked directly against this corpus, not left theoretical**: all 15
sampled `decorator` sites have an opening paren on the same line
(`@IsEmail(...)`, `@ValidateNested()`, etc. -- none are bare `@name`).
This specific contradiction therefore mislabels **zero** sites in this
round's actual sample, but the rule itself was wrong and is corrected here
so a future re-selection (or a differently-composed decorator-heavy
repository) doesn't inherit it. Corrected rule: a bare `@name` IS a real
call to `name`, scored the same as a parenthesized factory call, not
`not_a_call_site`.

## 3. Case 6's unstated dependency, made explicit

Case 6 said `obj: Foo | null` "still has a single concrete `Foo` receiver
type once the null case is chain-excluded" without stating that this
conclusion itself depends on case 1's rule (no override/structural
competitor for `Foo` anywhere in the snapshot). Made explicit: optional
chaining removes the null/undefined branch from consideration, but
whether the remaining `Foo`-typed call is Must, May, or Unknown is decided
by applying case 1 to `Foo` exactly as if `?.` were `.` -- `?.` never
changes the answer to that separate question, it only decides whether
case 1's answer gets applied at all (versus the call simply not
executing).

## Labeling process notes, not rubric changes

Per this review, before labeling: grep each repository once, up front,
for `\.prototype\.\w+\s*=` and `Object\.defineProperty\(` (the rebinding
definition's two most consequential shapes) rather than re-checking per
site. For every eventual Must label, open the target definition and
confirm both uniqueness (case 2) and absence of either rebinding pattern
reachable from it, not just absence at the call site itself.
