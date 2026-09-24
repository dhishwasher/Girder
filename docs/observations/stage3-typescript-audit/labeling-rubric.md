# Stage 3 TypeScript audit: frozen ground-truth labeling rubric

Written and committed BEFORE any of the 105 selected sites
(`audit-sites.json`) is read. Mirrors the discipline Rust's and Python's
own audits used: every site's true class is decided by a rule fixed in
advance, not by whatever looks convenient once the actual sites are in
front of the labeler. Quotes below are from
`docs/call-classification-policy.md`'s frozen TypeScript row:

> **Must**: Proven lexical binding/concrete implementation without viable
> structural/overload alternatives.
> **May**: Proven compatible structural/interface implementors, union
> alternatives, overload candidates.
> **Unknown**: any/unknown receivers, computed properties, eval/
> reflection, unproven narrowing, unresolved declarations/generated/
> runtime code.

"Source snapshot" means: the four pinned, extracted repositories
(`typescript-6.0.3`'s `src/` only, `zod-3.23.8`, `date-fns-4.1.0`,
`class-validator-0.15.1`) as pinned in `docs/stage3-typescript-corpus.json`,
nothing else. A target defined outside the snapshot (Node's stdlib, a
third-party npm dependency not itself pinned, the DOM lib types) is
outside the snapshot by definition.

## Rebinding, defined once

A name is **rebound** (disqualifying an otherwise-Must binding) if the
snapshot contains, anywhere reachable at the call site: a second
assignment to the same binding in an enclosing scope that isn't provably
dead code; `Foo.prototype.m = ...` or `Object.defineProperty(Foo.prototype,
'm', ...)` reassigning a method after its class body; module augmentation
(`declare module 'x' { interface Foo { ... } }`) or namespace/declaration
merging that adds or shadows a member after the fact; or an `import { x as
y }` aliasing that could plausibly shadow the name in some reachable
scope. This is deliberately generous toward disqualifying Must (erring
toward May/Unknown), matching this whole design's "prefer refusing to
answer over answering wrongly" instruction -- the same standard Rust's
and Python's own rubrics use.

## Case-by-case rules

**1. `this.m()` / instance method calls.**
- If no class anywhere in the snapshot overrides `m` for any subclass of
  the defining class (checked by reading the whole class hierarchy in the
  snapshot, not assumed), AND no *interface* in the snapshot that the
  receiver's static type could also satisfy has a different, snapshot-
  present implementation of `m` reachable through that interface: **Must**,
  target the single definition. This is the direct TypeScript analogue of
  Rust's "exactly one inherent impl, no trait competitor" and Python's
  "no override anywhere in the snapshot" Must proofs.
- If an override exists anywhere in the snapshot (a subclass overriding
  `m`, or a sibling class satisfying the same *structural* interface with
  its own `m`), **May**, candidate set = every override/implementor
  reachable through the class hierarchy AND every structurally-compatible
  type actually present in the snapshot -- "proven compatible structural/
  interface implementors... or overload candidates" per the policy. If
  the candidate set can't be fully enumerated from the snapshot (e.g. the
  receiver's type is an interface exported for external implementation,
  and TypeScript's structural typing means ANY object shape satisfying it
  could be a receiver, not just snapshot-present classes), that's
  **Unknown** instead ("unresolved... runtime code").
- **Structural typing is the key TypeScript-specific risk case 1's Rust/
  Python analogues don't have**: a receiver typed as an `interface` (or a
  type alias to an object shape) is satisfiable by ANY object with a
  compatible shape, not just classes that `implements` it explicitly --
  TypeScript has no nominal "this satisfies this interface" declaration
  requirement the way Rust's trait `impl` blocks do. A call through a
  bare `interface`-typed receiver is therefore **never Must from the
  interface type alone** ("any/unknown receivers" in the policy's Unknown
  column reads naturally to include this) -- it is May at best, and only
  if the actual, concrete set of snapshot types assignable to that
  interface can be enumerated; otherwise Unknown.

**2. `import { f } from 'x'; f()` and `mod.f()` (plain/qualified calls).**
- **Must** only if: the import target resolves, unambiguously, to exactly
  one function/class definition within the snapshot (following the import
  through re-exports within the snapshot, same bare-name-uniqueness
  simplification Rust's and Python's designs used, not full transitive
  resolution), AND the name is not rebound (see above) between the import
  and the call.
- If the import target is outside the snapshot (a third-party npm
  package, Node/DOM built-ins): **not a call site for Must/May purposes
  in the way a source-snapshot proof can reach** -- read case 8 below;
  not automatically Unknown, may be `not_a_call_site` or a distinct
  labeled category depending on shape.
- `mod.f()` (a qualified call through an imported namespace/module
  object) follows the same rule once `mod` itself resolves to a single
  in-snapshot module.

**3. Receivers known only from a type annotation, generic parameter, or
`any`/`unknown`.**
- The policy is explicit: "any/unknown receivers" are named directly in
  the Unknown column. A call on a parameter typed `any`, or whose type
  resolves to `unknown` without a narrowing check that provably concludes
  before the call, is **never Must, never May** -- straight to Unknown.
- A call on a generic type parameter (`function f<T>(x: T) { x.m(); }`
  with no `extends` bound narrowing `T` to something whose implementors
  are enumerable in the snapshot) is likewise Unknown -- `T` could be
  instantiated with anything at the call site, unbounded.
- A bounded generic (`<T extends Foo>`) reduces to case 1's rule applied
  to `Foo` as the receiver's effective type.

**4. `new Foo()` / `new Foo<T>()` -- direct construction.**
- **Must** requires: `Foo` resolves to a single in-snapshot class (case
  2's import rule), no snapshot-present subclass or structurally-
  compatible alternative is assignable where `Foo` is expected at that
  call site (TypeScript has no `__new__`/metaclass equivalent, but DOES
  have factory-pattern static methods and the possibility that `Foo` is
  itself reassigned -- covered by the rebinding definition above), and
  `Foo` is not rebound before the call.
- Otherwise: May (bounded candidates, e.g. a known small set of
  subclasses) or Unknown (unbounded), following case 1's split.

**5. Decorated names (legacy `experimentalDecorators`, the only mode this
corpus's pinned repositories use -- confirmed via
`class-validator-0.15.1/tsconfig.json`, see
`methodology-addendum-2.md` \S1).**
- A decorator line is (at minimum) **two calls**, per the same reasoning
  Python's rubric case 5/8 give for a Python decorator: the **factory
  call** itself (`IsEmail(undefined, {...})`) is a real, ordinary
  in-snapshot call, scored by cases 1-4 above depending on its own shape
  -- it is NOT automatically Unknown just for being on a decorator line.
  The **application of the factory's return value** to the decorated
  property/method (an implicit call TypeScript inserts, not written as
  source text at all under legacy decorators, so it never becomes its
  own selected site) is Unknown by construction: the returned function's
  identity isn't statically known without tracing the specific factory's
  body, and is out of scope for THIS audit's site-level scoring since it
  has no selectable line/column of its own.
- `@property`-shaped built-in accessor decorators do not apply to this
  corpus (that's a Python-specific idiom); no equivalent case needed here.

**6. `obj?.method()` -- optional chaining.**
- Decided here, quoting the policy directly rather than asserting by
  analogy: the policy's Must/May/Unknown definitions describe **which
  target is called if the call happens**, not whether the call happens at
  all -- the same distinction Python's own rubric relies on implicitly
  (a call inside an `if` block is still scored by its own target-binding
  proof, not demoted to Unknown merely for being conditional; Rust's
  rubric makes the same assumption for calls inside `if`/`match` arms).
  Optional chaining's short-circuit is exactly this kind of ordinary
  control-flow conditionality, not a distinct binding-uncertainty the
  policy's Unknown column names (it lists "any/unknown receivers,"
  "unproven narrowing" -- neither describes "the call might not run,"
  both describe "the target might not be provable"). **Therefore: `obj?.
  m()` is scored by the SAME rule as `obj.m()`** (case 1/2 above applied
  to whatever `obj`'s type is) -- Must if the target is otherwise provably
  unique and `obj`'s own optionality doesn't itself introduce a receiver-
  type ambiguity (i.e., `obj: Foo | null` still has a single concrete
  `Foo` receiver type once the null case is chain-excluded, same as
  Python's `Optional[Foo]` receivers under an `if x is not None:` guard
  would be). The `?.` itself is never, on its own, a reason to downgrade
  to May or Unknown.

**7. Overloaded functions/methods (multiple signature declarations, one
implementation body).**
- **Must** if the single implementation body is otherwise provably
  unique by cases 1-4's rules -- multiple TYPE signatures sharing one
  implementation is a compile-time-only distinction; at runtime there is
  exactly one function body every overloaded call actually reaches. The
  policy's "overload candidates" language in the May column describes a
  *different* situation: a call whose overload resolution genuinely can't
  be narrowed to one signature's *implementation* target because the
  candidates are actually separate implementations (e.g. an interface's
  several call signatures each satisfied by a different snapshot-present
  object) -- not the ordinary "one function, several declared
  signatures" TypeScript idiom, which stays Must.

**8. Targets outside the indexed snapshot (Node/DOM built-ins,
third-party npm dependencies not pinned in this corpus).**
- Same as Rust's and Python's own audit-correction lesson, applied here
  in advance rather than found by a later correction: a call whose
  resolved target is NOT in the four-repository snapshot is labeled
  `not_a_call_site` for scoring purposes if it is not a call at all (see
  case 9), but if it IS a genuine call whose target is simply external,
  it is labeled with `true_class: unknown` and a note that the target is
  external -- the same treatment every other language's audit gives an
  unresolvable external call.
- `declare`/ambient declarations (`declare module`, `declare function`,
  `declare global`) name symbols with NO body in the snapshot by
  definition (an ambient declaration is a type-only assertion about
  something implemented elsewhere, often literally nothing -- pure
  compile-time information). A call resolving only to a `declare`d
  signature is external-target `unknown`, same as case 8's general rule,
  never Must/May regardless of how "unique" the declaration looks.

**9. `.d.ts` files and declaration-only lines.**
- `typescript-6.0.3` alone has 108 `.d.ts` files among its 709
  `.ts`-matching files (the selector's `*.ts` glob matches `.d.ts` too).
  A `.d.ts` file contains ONLY type declarations -- no executable bodies,
  no real calls, ever. Any call-shaped regex match inside one (a method
  signature `foo(x: string): void;` that superficially looks like
  `plain_call`/`method_call`) is **`not_a_call_site`** unconditionally --
  there is no runtime call there at all, the same reasoning as case 9's
  Python analogue (`def f(` is a declaration, not a call) but stronger:
  a `.d.ts` file can never contain a real call under any circumstance,
  so this rule needs no further judgment once the file extension is
  confirmed.

**10. Are decorator lines and their invocations call sites?**
- The bare decorator factory call (`@IsEmail()`) IS a real call -- see
  case 5. A bare `@name` with no parens (a decorator that's a plain
  function reference, not a factory call) is **not a selectable call
  site under this corpus's shape patterns** (the `decorator` shape's
  regex requires only `^@\s*[A-Za-z_][A-Za-z0-9_.]*`, which matches both
  the parenthesized-factory and bare forms, but a bare `@name` with no
  following `(` has no call syntax at all on that line -- if selected,
  it is `not_a_call_site`, the implicit-application call TypeScript
  inserts has no source position of its own, same as the second call in
  case 5).

**11. `not_a_call_site`.**
- `function f(`, `class C(` (invalid TS syntax, listed defensively --
  TypeScript's `extends`/`implements` have no parens, see
  `methodology.md` \S5's own note on this), `catch (e)`, generic type
  parameter lists that happen to regex-match a paren incidentally
  (uncommon given the shape patterns' own generic-aware design, but
  possible), interface/type member signatures (case 9's `.d.ts` rule
  applies identically to a `.ts` file's own top-level `interface`/`type`
  body, not just `.d.ts` files), and text that survives masking only
  because `mask_ts_source` has a gap the methodology's own spot-check
  didn't find (none observed across the four-repository pool in that
  spot-check, but the rule is recorded in case a labeling session finds
  one the spot-check missed -- report it as a masking bug, fix it, and
  do not silently relabel around it, mirroring Python's identical rule).

## What this rubric deliberately does not resolve

- Full alias-tracking through arbitrary levels of indirection (`const a =
  b; const c = a; c()`) -- read the actual snapshot's control flow for
  each specific site rather than trying to generalize a rule here; if
  genuinely ambiguous after reading, it's Unknown.
- Whether a specific decorator factory's returned function has a
  "provably no dispatch effect" body (case 5's own carve-out, mirroring
  Python's case 5) is a per-decorator judgment call applied while
  labeling, not enumerated here -- record the reasoning inline in the
  labeled-sites file for each such site, the same way Python's own
  `audit-sites-labeled.json` carries a rationale per site.
- TC39 (stage-3) decorators' different call signature (`(value,
  context)` instead of `(target, propertyKey)`) is not resolved here
  since no pinned repository uses that mode -- a future round adding one
  would need its own rubric addition, not an assumption that case 5
  transfers unchanged.
