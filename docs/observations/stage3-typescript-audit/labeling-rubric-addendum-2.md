# Addendum 2 to labeling-rubric.md: a structural-typing rule for class receivers

Written AFTER the 105 sites were labeled but BEFORE any of them was
rescored, per a further review's request. Recorded honestly as such,
not backdated into the rubric itself.

## Case 1 addition: private/protected members close the structural-substitution gap

Case 1's rule for `this.m()`/instance receivers assumed that "no override
anywhere in the snapshot" is sufficient for Must. A further review pointed
out a gap specific to TypeScript: classes are compared **structurally**
unless they declare at least one `private`, `protected`, or `#private`
member -- without one, ANY object with the same public shape is
type-assignable to a class-typed parameter, even if it never went through
that class's own constructor. A class-typed receiver coming from an
external, caller-supplied parameter could therefore, in principle, be
satisfied by an unrelated structurally-compatible object, not just a
genuine instance or subclass instance.

**Rule, added to case 1**: before labeling Must for a class-typed
receiver, check whether the class declares any `private`/`protected`/
`#private` member. If it does, the class is effectively nominal (no
unrelated object can satisfy it), and the existing "no subclass override"
check is sufficient on its own. If it does NOT, an additional check is
needed: is the receiver's actual value CALLER-SUPPLIED (an external
parameter, where an adversarial or merely different caller could pass a
structurally-compatible-but-unrelated object) or INTERNALLY-CONTROLLED
(e.g. returned by one of the class's own lookup/factory methods querying
its own private state, where the runtime value is provably always a
genuine instance regardless of the type's structural permissiveness)? Must
requires the latter, or a private/protected member closing the gap
outright; a purely-structural class type on a genuinely externally-
supplied parameter is Unknown.

Applied retroactively to the three sites this matters for in the
committed labeling (`675528d`, corrected in the same round as this
addendum): `TestState` (sites 54, 55) has four `private` members,
closing the gap outright. `ScriptInfo` (site 65) has none, but its
receiver traces to an internal `ProjectService` lookup method, not a
caller-supplied parameter -- Must holds under the second branch of this
rule, not the first.

## Case 2/4's "bare-name-uniqueness simplification", made precise

Case 2's own text calls this a "simplification" but never states exactly
what it simplifies. A whole-package duplicate-name check across all 47
Must targets (run after the first correction round, before any label was
changed further) found several targets with more than one same-named
declaration SOMEWHERE in the same pinned repository:
`IsLongerThan`/`IsLonger` (class-validator, additional declarations in
unrelated sample/test files), `MySubClass` (class-validator, 12
declarations across 6 spec files), `emitExpression`/`writePunctuation`
(typescript-6.0.3, a second same-named function in a different compiler
file), `ScriptInfo` (typescript-6.0.3, a second, unrelated class in the
harness-only `harnessLanguageService.ts`), and most seriously
`TestSession` (typescript-6.0.3, FOUR separate `class TestSession`
declarations -- one exported from `helpers/tsserver.ts`, three
module-private or locally-scoped inside `tsserver/session.ts`).

**None of these collisions actually affect any labeled site's own
resolution**, but only because each site's own file was checked directly
against its own import statements or its own enclosing block scope, not
assumed safe from the bare name alone. The rule, stated precisely:
**uniqueness for case 2/4 purposes is evaluated over definitions
reachable from the call site's own lexical scope and import graph, not
over every same-named declaration anywhere in the package.** A same-name
declaration in a sibling block scope (same file, different `it()`/
`describe()` callback), an unrelated file the call site's own file never
imports, or a module-private declaration in a different file, does not
count as a competing candidate -- but each of those three conditions must
be CHECKED (the actual import statement read, or the actual enclosing
scope traced), not assumed from "the name only appears once in this
directory" or any similarly narrow search.

## `tracing` vs `IO`: the same rebinding rule, two different outcomes, both correct

Two same-file-typed mutable variables in the snapshot illustrate the
rebinding definition applied correctly in two directions: `tracing:
typeof tracingEnabled | undefined` (`compiler/tracing.ts:30`, sites 29,
34) is assigned exactly twice in the file -- once to `tracingEnabled`
itself, once to `undefined` -- and `?.` already excludes the `undefined`
case, leaving only one real assigned value once optional chaining is
accounted for (case 6). `IO: IO` (`harness/harnessIO.ts:46`, site 53,
labeled Unknown) is assigned to two DIFFERENT concrete, non-`undefined`
implementations in different branches (`IO = io` and `IO = createNodeIO()`)
-- excluding `undefined` via `?.` (which this site doesn't even use)
would not collapse this to one candidate, since both real assignments are
live alternatives. The rebinding definition disqualifies a rebound name
regardless of how many times it's assigned; what changes the outcome here
is whether excluding the falsy/undefined branch (via `?.` or an explicit
guard) leaves exactly one remaining candidate, not the raw assignment
count.

## What the whole-package duplicate check actually covered, stated precisely

The duplicate-name script above matched `function NAME`, `class NAME`, and
`namespace NAME` declarations only -- it would not have found a
`const NAME = ...` or arrow-function binding of the same name, an
`export { x as y }` alias, or (checked separately, not by that script) a
second parallel module tree the way zod's `src/`/`deno/lib/` split turned
out to be. Every site this round's review specifically raised (the eleven
decorator-factory Musts' barrel-export paths, `TestSession`'s four
declarations, `ScriptInfo`'s two) was verified by reading the actual
import statement or barrel `export * from` line directly, not solely by
this script's pattern match -- but the script itself, taken alone, is not
a complete duplicate-declaration search across every binding form
TypeScript allows. Recorded as a real scope limit, not silently expanded
into a stronger claim than what was run.
