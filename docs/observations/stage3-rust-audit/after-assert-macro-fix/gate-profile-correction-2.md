# Second correction to the gate-profile design spec, before any implementation code

Found on review of `bac5544`'s correction, before writing any Rust. Kept as
a new document per the resume contract; nothing earlier is edited.

## Gaps in the first correction

The first correction (`bac5544`) fixed the "inherent always wins" claim but
left three things unaddressed, and had two factual slips. All checked
directly against the petgraph checkout below, not assumed.

**1. Applicability (bounds) -- the same-step rule needs it, and it wasn't stated.**
Inherent-wins-at-the-same-step only holds if the inherent impl actually
*applies* to the receiver. Checked directly: the inherent impl
(`src/graph_impl/mod.rs:483`, `impl<N, E, Ty, Ix> Graph<N, E, Ty, Ix> where
Ty: EdgeType, Ix: IndexType`) and the competing `Build for Graph` impl
(`src/data.rs:135`, `impl<N, E, Ty, Ix> Build for Graph<N, E, Ty, Ix> where
Ty: EdgeType, Ix: IndexType`) have **identical** bounds -- so whenever one
applies, both do; there is no case where `Build::add_node` is reachable but
the inherent one isn't. Also checked: no blanket `impl<X> Build for X`
exists anywhere (`grep -rn "Build for" --include=*.rs .` finds exactly four
hits, all for named concrete types -- `List`, `Graph`, `StableGraph`,
`GraphMap` -- never an unconstrained type parameter), and no `Deref`/
`DerefMut` impl exists for `Graph` anywhere in the checkout (the three
`Deref` impls found are for unrelated types `Ptr`, `Frozen`, `Small`), so
there is no deref-chain fallback to worry about either.

Rule for the implementation: before trusting inherent-over-trait at a
shared step, every bound on the inherent impl must also appear on each
competing trait impl covering the same type (normalize inline and
`where`-clause bounds into (param, bound) pairs and check inherent-bounds
⊆ trait-impl-bounds); any blanket impl of the competing trait over a bare
type parameter makes the result Unknown; and no in-crate `Deref`/`DerefMut`
impl may exist for the receiver type (the orphan rules make an in-crate-only
check sufficient, since an external crate cannot add a `Deref` impl for an
in-crate type). What this cannot settle -- whether the bounds themselves are
actually satisfied by the caller's concrete arguments (e.g. that `Directed:
EdgeType` and the default `Ix` truly hold) -- is not attempted; disclose it
as an assumption string on the evidence rather than solving trait
satisfaction.

**2. Target-side configuration -- entirely missing from the original spec.**
The spec's constraint 5 only reused the *caller*-side gates
(`transformed_scope` etc. on the file containing the call). If the
inherent `add_node`, its impl block, the `Graph` struct itself, or the
`graph_impl`/`graph` module declarations were `#[cfg]`-gated, the call
could resolve to `Build::add_node` under a different feature
configuration than the one this extractor happened to index.

Checked directly for this site: no `#[cfg(...)]`/`#[cfg_attr(...)]`
attribute appears immediately above the `Graph` struct (`src/graph_impl/
mod.rs:347`), the inherent impl (`:483`), the `Build for Graph` impl
(`src/data.rs:135`), or either module declaration in `src/lib.rs`
(`mod graph_impl;` at line 142, `pub mod graph { ... }` at line 161 --
`pub mod data;` at line 134 carries `#[macro_use]`, not `#[cfg]`). Clean.

Rule for the implementation: none of the struct, the inherent impl block,
any competing trait impl, or any enclosing module declaration on the path
from the crate root to any of these may carry `cfg`/`cfg_attr`.

**3. External globs -- the first correction's rule ("non-glob, non-prelude
import") left `use other_crate::*` unaddressed, and named it "prelude" as
if petgraph's own `prelude::*` were a special case rather than an ordinary
in-crate glob.** An external glob is the most dangerous unhandled case: it
can silently bring a same-named trait method into scope from a crate this
extractor never indexes.

Checked directly: `floyd_warshall.rs` uses `petgraph::prelude::*`.
`src/prelude.rs`'s own `pub use` lines are all `crate::...` paths -- no
external re-export anywhere in it (read the whole 19-line file). The one
trait it re-exports, `visit::EdgeRef`, declares `source`/`target`/`weight`/
`id`, no `add_node`. Clean, but the rule that makes this checkable at all
was missing from the first correction.

Rule for the implementation: any glob whose path resolves outside the
indexed crate makes the result Unknown, no exception for a crate's own
"prelude" module by name. An in-crate glob's target module must itself
`pub use` nothing from outside the indexed crate -- checked recursively,
not just one level.

**4. Type-resolution choice, pinned in writing.** The roadmap's "Next" entry
still says types are resolved "keyed by semantic path, not bare name",
which implies full re-export/path resolution. The design actually
implemented uses **bare-name uniqueness** instead: a receiver's declared
type name resolves soundly if it is the *only* type-namespace item
(struct/enum/union/trait/type-alias) of that bare name anywhere in the
indexed crate, reached through a **named, non-glob** import (or an
in-file definition) whose first path segment is `crate`/`self`/`super` or
the crate's own package/`[lib]` name -- combined with the rule above (no
external glob), and the further requirement that nothing anywhere in the
indexed crate `use`s or `pub use`s an external item of that same bare
name (which would make the bare name itself ambiguous with an unindexed
type, not just multiply-defined in-crate). This is a deliberate,
documented simplification of the original "resolve through imports to a
semantic path" framing, not an oversight -- checked to still hold for this
site: `grep -rnE "^\s*(pub(\([^)]*\))? )?(struct|union) Graph\b" --include=*.rs .`
across the whole checkout returns exactly one hit
(`src/graph_impl/mod.rs:347`), and no `type Graph`/`trait Graph`/`enum Graph`
exists anywhere either (checked in the prior correction).

**5. Factual slips in the first correction, fixed here:**
- There are **12** `fn add_node(` definitions in `src/`, not 11: `adj.rs:189`,
  `adj.rs:327`, `data.rs:47,140,167,194`, `matrix_graph.rs:297,1248`,
  `graphmap.rs:280`, `csr.rs:268`, `graph_impl/mod.rs:525`,
  `graph_impl/stable_graph/mod.rs:264`. All 12 take `&mut self`, re-verified
  by grepping the full, un-truncated signature of each (not just the first
  line). `adj.rs:205`'s earlier match against `-v "&mut self"` was
  `add_node_from_edges`, a different method name entirely, not a `&self`-
  shaped `add_node` -- it only matched because its multi-line signature
  puts `&mut self` on a separate line from `fn add_node_from_edges(`.
- The first correction's greps were scoped to `src/` only. Girder's index
  also covers `tests/`, `benches/`, and `examples/`. Re-ran across the
  whole checkout for both `fn add_node(` and the struct search: no
  additional definitions found in any of those directories.

## Net result: `tests/floyd_warshall.rs:11` still survives every check

All five gaps above are now checked, not assumed, and all resolve in favor
of Must for this specific site. The prediction is unchanged: implementing
the corrected design moves exactly `tests/floyd_warshall.rs:11` and nothing
else among the 52 scored sites.

## A blind spot in the audit scorer itself, for measurement time

`tools/dispatch_audit_scorer.py`'s `cell_label` compares only `class`
(must/may/unknown/excluded), never `targets`. A Must claim whose `targets`
pointed at the wrong function -- e.g. `Build::add_node` or
`StableGraph::add_node` instead of `graph_impl/mod.rs:525`'s inherent
method -- would still score `exact` against a `true_class: must` site,
silently. This is a scorer limitation to be aware of, not something to fix
before implementing: when the after-observation is written, explicitly
assert that the produced claim's `targets` NodeId matches the inherent
`function_item` at `graph_impl/mod.rs:525` (found the same way `annotate()`
already matches a node's span, not by reconstructing a path string --
impl-method semantic paths are keyed by the impl block's own file/module
scope, which does not necessarily match the struct's defining location).
The same check belongs in the implementation's own unit tests and in the
hand-verified sample of new Must claims across all three crates.

## Staging note for the implementation (unchanged from the design spec, restated for continuity)

Two pieces, landed together once all four gates pass (an unwired
crate-wide index would trip `dead_code` under `clippy -D warnings`):
**(A)** the crate-wide index itself -- type-namespace items, inherent
impls with their normalized bounds, trait method declarations with
receiver kind, trait impls with their bounds, `Deref` impls, and `cfg`
marks -- with its own unit tests, verified against the real petgraph
checkout to reproduce every fact established in this document and its
predecessor before moving on; **(B)** the post-`resolve_calls` pass that
replaces the existing Unknown claim with the new Must claim (never appends
a second, contradictory claim), with the full adversarial test list from
the design spec plus the incremental-staleness flip.
