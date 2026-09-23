# Stage 3 Python audit: frozen ground-truth labeling rubric

Written and committed BEFORE any of the 105 selected sites
(`audit-sites.json`) is read. Mirrors the discipline Rust's own Stage 3
audit used ("ground truth is read from the source before Girder is run on
any site"): this rubric exists so every site's true class is decided by a
rule fixed in advance, not by whatever looks convenient once the actual
sites are in front of the labeler. Quotes below are from
`docs/call-classification-policy.md`'s frozen Python row:

> **Must**: Proven binding with explicit source-snapshot/no-runtime-rebinding
> assumptions and validated scope; annotations alone do not establish exact
> runtime class.
> **May**: Proven possible inherited/overridden methods, super/MRO targets,
> or bounded receiver candidates.
> **Unknown**: getattr/reflection, monkey patching, decorators with
> unproven effects, unconstrained duck typing, dynamic imports, unresolved
> fixtures and inheritance.

"Source snapshot" means: the three pinned, extracted packages
(click-8.4.1, pydantic-2.13.4, requests-2.34.2) as they exist in the
sdist, nothing else. A target defined in a fourth package, the stdlib, or
a C extension is outside the snapshot by definition.

## Rebinding, defined once

A name is **rebound** (disqualifying an otherwise-Must binding) if the
snapshot contains, anywhere reachable at the call site: a second
assignment to the same name in an enclosing scope that isn't provably
dead code; `global`/`nonlocal` combined with reassignment; `del` of the
name followed by reuse; `setattr`/`exec`/`eval` targeting the same
name or attribute anywhere in the snapshot (unconstrained duck typing
and monkey patching are Unknown per the policy, so ANY `setattr` call
anywhere touching a plausibly-related attribute name is treated as a
rebinding risk for that name, conservatively); or `import x as y` /
`from a import b as c` aliasing that could plausibly shadow the name in
some reachable scope. This is deliberately generous toward disqualifying
Must (erring toward May/Unknown), matching this whole design's
"prefer refusing to answer over answering wrongly" instruction.

## Case-by-case rules

**1. `self.m()` / `cls.m()`.**
- If no class anywhere in the snapshot overrides `m` for any subclass of
  the defining class (checked by reading the whole class hierarchy in the
  snapshot, not assumed): **Must**, target the single definition. Python's
  MRO deterministically resolves to that one implementation for ANY
  instance of ANY subclass when no override exists anywhere reachable,
  regardless of the exact instantiated subtype -- this is the direct
  analogue of Rust's "exactly one inherent impl, no trait competitor"
  Must proof, not a guess.
- If an override exists anywhere in the snapshot for the same method name
  on any class in the hierarchy (a subclass of the defining class, in
  either direction of the call, or an ancestor override for a `cls.m()`
  classmethod call): **May**, with the candidate set being every override
  reachable through the class hierarchy actually present in the snapshot
  -- "proven possible inherited/overridden methods... or bounded receiver
  candidates" per the policy. Never May with an unbounded or unenumerated
  candidate set; if the override set can't be fully enumerated from the
  snapshot (e.g. the base class is imported from outside the snapshot and
  its own subclasses elsewhere are unknown), that's Unknown instead
  ("unresolved fixtures and inheritance").
- A library whose whole purpose is being subclassed by callers outside
  the snapshot (Click's `Command`/`Group`, `click.Context`) does not by
  itself demote an in-snapshot call to Unknown -- the source-snapshot
  assumption is explicit in the policy ("explicit source-snapshot...
  assumptions"), so external subclasses that don't exist in this snapshot
  are out of scope by definition, the same way Rust's audit didn't have
  to consider a downstream crate overriding a trait. This is an assumption
  the claim's evidence should disclose (mirroring Rust's
  `glob-safety-checked-file-local-only`-style assumption strings), not a
  reason to under-classify.

**2. `from x import f; f()` and `mod.f()` (plain/qualified function calls).**
- **Must** only if: the import target resolves, unambiguously, to exactly
  one function/class definition within the snapshot (following the import
  through re-exports within the snapshot, same bare-name-uniqueness
  simplification Rust's design used -- not full transitive resolution),
  AND the name is not rebound (see above) between the import and the call.
- If the import target is outside the snapshot (stdlib, C extension, an
  unpinned third-party dependency): **not a call site for Must/May
  purposes in the way a source-snapshot proof can reach** -- read case 7
  below; this is not automatically Unknown, it may be `not_a_call_site`
  or a distinct labeled category, decided per case 7.
- `mod.f()` (a qualified call through an imported module object) follows
  the same rule once `mod` itself is resolved to a single in-snapshot
  module; `mod` being rebound (reassigned) anywhere reachable disqualifies
  it the same as a plain name.

**3. Receivers known only from a type annotation or a bare parameter.**
- The policy is explicit: "annotations alone do not establish exact
  runtime class." A call on a parameter typed only via an annotation
  (`def f(x: Foo): x.m()`) is **never Must** from the annotation alone.
- It can still be **May** if `Foo`'s own method `m` has a bounded,
  enumerable override set in the snapshot (case 1's May rule applied to
  the annotated type as the base). It is **Unknown** if the actual
  runtime type could differ from the annotation in a way Python doesn't
  enforce (Python annotations are not runtime-checked by default) AND no
  other snapshot evidence narrows the receiver -- which is the common
  case; treat annotation-only receivers as May at best, defaulting to
  Unknown when the override set itself can't be bounded.

**4. `x = Foo(); x.m()` -- direct construction.**
- **Must** requires: `Foo` resolves to a single in-snapshot class
  (case 2's import rule), `Foo.__new__` is not overridden anywhere in
  `Foo`'s hierarchy in the snapshot (an overridden `__new__` can return an
  instance of a different type entirely -- this is exactly the kind of
  "validated scope" the policy requires and a naive reading would miss),
  no metaclass on `Foo` or its bases defines `__call__` in the snapshot
  (same reasoning -- a metaclass can intercept construction and return
  something else), `x` is not rebound before the call, and `m` has no
  override anywhere in `Foo`'s hierarchy in the snapshot (case 1's rule).
- If `Foo.__new__`/a metaclass `__call__` exists in the snapshot but is
  provably the identity/standard behavior (e.g. `__new__` that just calls
  `super().__new__(cls)` with no branching): still Must, since it doesn't
  actually change the constructed type -- read the body, don't just check
  presence.
- Otherwise: May (bounded candidates) or Unknown (unbounded), following
  the same split as case 1.

**5. Decorated names.**
- A decorator **replaces the function object** the decorated name binds
  to (`@click.command() def f(): ...` binds `f` to whatever
  `click.command()(f)` returns, not to the original function). Calling
  the decorated name afterward therefore calls the DECORATOR's return
  value's `__call__`, not the original function directly.
- **Unknown** unless the decorator's effect is traced and proven not to
  change dispatch: e.g. `@staticmethod`/`@classmethod`/`@property`
  (well-defined, in the language, not "unproven effects") can be Must/May
  under the same rules as an undecorated method once their own,
  well-understood transformation is accounted for. A decorator defined in
  the snapshot whose body can be read and shown to return its argument
  unchanged (a passthrough) also doesn't demote to Unknown. Any decorator
  whose return value's actual runtime type/behavior isn't traceable from
  the snapshot (most custom decorators, including click's own `@command`/
  `@option`/`@argument`, which wrap the function in a `Command`/build
  parameter objects) makes the decorated name's later calls **Unknown**
  ("decorators with unproven effects" is explicit in the policy).
- Calling THROUGH the decorator machinery itself (e.g. click's own
  internal `Command.__call__`/`.invoke()` implementation, read directly
  as ordinary in-snapshot code) is scored by the ordinary rules, once
  the receiver's type is itself established (which, per the paragraph
  above, is usually not provable past the decoration boundary).

**6. `super().m()`.**
- **May**, always, per the policy's explicit "super/MRO targets" listing
  in the May column -- never Must, even when the current class's own MRO
  is fully known, because `super()` in general resolves relative to the
  actual runtime class of `self` (which may be a further subclass in a
  cooperative multiple-inheritance chain), not just the lexically
  enclosing class. The candidate set is every method named `m` on every
  class between the current class (exclusive) and any class the MRO could
  place after it, bounded to what's enumerable in the snapshot; if that
  set can't be bounded, Unknown instead.

**7. Targets outside the indexed package (builtins, stdlib, third-party
not in the snapshot).**
- This is exactly Rust's own audit-correction lesson (8 sites mislabeled
  in the first Rust labeling pass by treating an external target
  inconsistently) -- fixed here in advance rather than found by a later
  correction: a call whose resolved target is NOT in the three-package
  snapshot is labeled `not_a_call_site` for scoring purposes if it is not
  a call at all (see case 9), but if it IS a genuine call whose target is
  simply external, it is labeled with `true_class: unknown` and a note
  that the target is external -- the same treatment Rust's audit gives an
  unresolvable call, not a special "excluded" bucket. The scorer must
  still be able to check Girder doesn't claim Must/May against a target
  it can't actually resolve into the graph (an overclaim) for these sites.

**8. Are decorator lines and their invocations call sites?**
- `@property` -- **yes**, a real call: `property(func)`. Label per the
  ordinary function/class-call rules (case 2/4) applied to `property`
  itself (a builtin, so case 7 applies: target outside snapshot, labeled
  `unknown` with an external-target note, never Must).
- `@click.command()` -- **two** calls: `click.command()` (constructs a
  decorator) and then the returned decorator called with `f` as its
  argument. Both are real calls; the selector's `decorator` shape may
  land its regex match on either sub-expression depending on the line --
  read the actual line to determine which call the matched span
  corresponds to before labeling, don't assume it's always the outer one.
- Bare `@some_name` (no parens) -- **one** call: `some_name(f)`.

**9. `not_a_call_site`.**
- `def f(`, `class C(`, `except (A, B):`, generic subscripts (`Dict[str,
  int]`), match-statement patterns (`case Point(x=0):`), f-string
  format-spec braces that regex-match a paren incidentally, and -- now
  that masking is in place -- text that survives masking only because
  `tokenize` failed on that file (none observed across the three
  packages, per `audit-sites.json`'s `masking.tokenize_failure_count: 0`,
  but the rule is recorded in case a future re-run of the selector hits
  one). Text genuinely inside a string/docstring/comment should already
  be filtered by the mask; if a selected site's `text` field shows content
  that is obviously inside a string despite the mask (a masking bug), that
  site is `not_a_call_site` and the masking bug is reported and fixed
  before scoring, not silently relabeled around.

## What this rubric deliberately does not resolve

- Full alias-tracking through arbitrary levels of indirection (`a = b; a
  = c; c()`) -- read the actual snapshot's control flow for each specific
  site rather than trying to generalize a rule here; if it's genuinely
  ambiguous after reading, it's Unknown.
- Whether a specific decorator's effect can be "traced" is a per-decorator
  judgment call applied while labeling, not enumerated here -- record the
  reasoning inline in the labeled-sites file for each decorated site so
  it's auditable later, the same way Rust's `audit-sites-labeled-v2.json`
  carries a rationale per site.
