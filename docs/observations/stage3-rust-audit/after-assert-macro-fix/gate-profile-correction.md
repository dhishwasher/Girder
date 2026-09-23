# Correction to the gate-profile design spec's constraint 4

Found on review, before writing any implementation code. `gate-profile.md`
is kept unedited per the resume contract; this corrects it.

## The committed rule was wrong

`gate-profile.md` said: *"An inherent method on the exact concrete type
always wins over a same-named trait method on that type -- a fixed Rust
resolution rule... the inherent method's existence alone settles it."*

This is false in general. Rust's method resolution builds a list of
candidate receiver types in order -- `T`, `&T`, `&mut T`, then each step of
`T`'s deref chain -- and at **each step**, considers every method (inherent
and trait) whose receiver matches that step. The first step with any match
wins; inherent only beats trait **within that same step**. A trait method
whose receiver matches an *earlier* step than the inherent method's own
receiver beats the inherent method outright, unconditionally.

`tests/floyd_warshall.rs:11`'s Must claim survives this correction, but by
verified fact, not by the general rule as originally stated: every
`add_node` definition in petgraph takes `&mut self`
(`grep -rn "fn add_node" src/` -- eleven definitions, every one `&mut self`;
the one `&self`-shaped line the grep also matched, `adj.rs:205`, is
`add_node_from_edges`, a different method name). `Build::add_node` is also
`&mut self` (`src/data.rs:47`). Both the inherent method and the only
in-crate trait method of that name sit at the identical `&mut Graph` step,
so inherent-over-trait applies and settles it -- not because inherent
methods are unconditionally preferred, but because there is no earlier-step
competitor to lose to.

## Corrected constraint 4

Must holds only if no candidate trait method named `m` matches at a step
earlier than the inherent method's own receiver step. Concretely, before
trusting an inherent `fn m(<step>, ...)`:
- Check every in-crate trait that declares a method named `m`, whether or
  not it is actually implemented for `T` (a blanket impl could supply it)
  -- if any such declaration's receiver matches an earlier autoref step
  than the inherent method's, the result is Unknown, not Must.
- The std prelude's trait method names are a small, fixed set (`clone`,
  `into`, `as_ref`, `borrow`, `fmt`, `eq`, `cmp`, ...) -- if `m` is in that
  set, the result is Unknown regardless of what's found in-crate, since an
  unindexed prelude trait could supply an earlier-step candidate this
  extractor has no way to rule out.
- A non-glob, non-prelude import could still bring an external trait's
  same-named method into scope; treat any import this extractor cannot
  positively identify as a known non-trait item (e.g. `HashMap`) as reason
  enough to stay Unknown.
- Also confirmed for this specific site (not just asserted): no other
  type-namespace item named `Graph` exists in petgraph (no `type`/`trait`/
  `enum Graph`), and the crate's one `pub use crate::graph::Graph` at
  `src/lib.rs:122` re-exports the same struct, not a different one --
  bare-name uniqueness holds for this case.

## What this changes in the design

Nothing about the prediction (`tests/floyd_warshall.rs:11` is still the one
frozen site expected to move) or the other four rejected candidates
(unaffected by this correction -- they failed on the explicit-annotation
or reachable-only-through-a-trait constraints, not on receiver-step
ordering). What changes is that any **implementation** of constraint 4 must
encode the step-order check above, not "inherent always wins" -- a resolver
built against the original wording would be unsound the first time it hit
a type with an earlier-step trait method of the same name (e.g. a `&self`
trait method alongside a `&mut self` inherent one), which never happens to
arise for this one verified site but would for others the moment this rule
is applied more broadly.

Guards a real implementation also needs, identified alongside this
correction (not previously written down): the receiver type must come from
a **named** import (not a glob) whose path resolves into the indexed
crate itself, walked through the actual `use`-tree AST rather than the
over-inclusive `use_declaration_names` helper (correct for shadow
detection, wrong for proof); the receiver's declared type must be written
as exactly `T<...>` with no `&`/`Box`/`dyn`/`impl` wrapping (autoderef stays
out of scope entirely); `x` must have exactly one binding in the function,
counting closure/`for`/`match`/`if let` patterns as bindings too; the
unique inherent impl must be generic over all of `T`'s own parameters, so
coherence rules out a hidden second impl through a type alias or macro
expansion; and the produced Must claim must **replace** the existing
Unknown claim for that call, not sit alongside it as a second, contradictory
claim, with a span starting at the call expression itself (matching where
the audit scorer's byte offset actually lands, confirmed against
`gate-profile.json`'s `'.add_node(())'` snippet) rather than at the method
name.
