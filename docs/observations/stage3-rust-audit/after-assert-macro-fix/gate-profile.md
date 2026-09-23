# Per-site gate profile of the 25 frozen conservative sites, and a design spec for the next resolver change

Built per the roadmap's corrected "Next" entry (`f137cad`): before choosing
what to implement, profile every conservative site against every gate,
using Girder's own evidence, not a hand sample.

## Tool and raw output

[`tools/dispatch_audit_gate_profile.py`](../../../../tools/dispatch_audit_gate_profile.py),
committed alongside this document. Reads `extract_call_claims` (the same
parser `dispatch_audit_scorer.py` uses) against the three crates'
`girder inspect --json` output and reports, per conservative site: the
identifier-only filter result, every non-`#[test]`/`#[tokio::test]`
attribute in the file (correctly including inner `#![...]` attributes --
see "bug fixed" below), whether any of those are `cfg`/`cfg_attr`/
`macro_use` specifically, `duplicate_paths`, parse-error, whether the
site's own caller function owns an untrusted macro invocation
(`macro_owners`), and whether the site sits inside a trusted assert
macro's token tree. Raw output: [gate-profile.json](gate-profile.json).
Command:
```
python3 tools/dispatch_audit_gate_profile.py \
  --results docs/observations/stage3-rust-audit/audit-scored-results-v3.json \
  --crate-root petgraph-0.6.5=<extracted petgraph> \
  --crate-root serde_json-1.0.150=<extracted serde_json> \
  --crate-root regex-1.12.4=<extracted regex> \
  --inspect petgraph-0.6.5=<inspect.json from analyze+inspect> \
  --inspect serde_json-1.0.150=<...> --inspect regex-1.12.4=<...> \
  --output gate-profile.json
```
No build is required -- the `inspect.json` files come from the same
`girder analyze --json` + `girder inspect --json` run already used for
`audit-after.json`.

**Bug fixed before trusting this profile:** an earlier, uncommitted ad hoc
version of this check matched only text starting with `#[`, which misses
inner attributes (`#![...]`) entirely -- silently dropping e.g.
`#![feature(test)]` at the top of a benchmark file. The committed tool
matches both `#[` and `#![`.

## What the frozen policy allows (checked before designing anything)

`docs/call-classification-policy.md`'s Rust row: *"Must requires: Proven
lexical/item binding or **exact concrete dispatch**, no shadowing/alternate
implementation/configuration ambiguity."* Method dispatch on a receiver
whose concrete type is known is explicitly in scope for Must -- it is not
something the policy reserves for May. *"Unknown includes: ... unresolved
trait bounds, unknown receiver adjustment ..."* -- so a call whose receiver
type is generic, a trait object, or otherwise not concretely known stays
Unknown, and a method resolvable only by consulting an unindexed/external
trait also stays Unknown.

## Result: no single gate is the constraint. All 25 sites are blocked by one of two primary gates, independent of `transformed_scope`

| Primary blocker | Sites | Fix required |
| --- | --- | --- |
| Identifier-only filter (method/path shape excluded from Must-proof outright) | 23 | Method/path dispatch proof |
| Same-file-only restriction (bare identifier, but target imported from another file) | 2 | Cross-file import resolution |

**`transformed_scope`, narrowed or not, moves zero of the 25 on its own.**
This corrects a wrong conclusion from an earlier, uncommitted-then-corrected
pass (`f137cad`) that reasoned from a single supplementary example. Checked
properly here: of the 23 identifier-filter-blocked sites, narrowing
`transformed_scope` doesn't touch the identifier filter at all -- that gate
fires before `transformed_scope` is even consulted for a method/path call.
Of the 2 same-file-only sites (`tests/graph.rs:2092`, `tests/quickcheck.rs:547`),
both are *also* independently blocked by real `#[cfg(feature = ...)]`
attributes in their files (confirmed in `gate-profile.json`), so even a
correctly-scoped narrowing (keeping `cfg`/`cfg_attr`/`macro_use` disqualifying,
per the policy's own "configuration ambiguity" language, while excluding
`#[derive]`/`#[inline]`/`#[allow]`/`#[bench]`/`#[deprecated]`/`#[doc]`/
`#[should_panic]`/`#[repr]` -- none of which can introduce a same-named
top-level bare-callable binding, since a derive only ever produces trait-impl
methods, reached via `.method()` syntax, which the identifier-only filter
already excludes regardless) would not unblock either site: they fail on
`cfg` specifically, which stays disqualifying under any policy-consistent
design.

## Method/path candidates: 5 pass every existing gate independent of the identifier filter

Filtering to `identifier_filter_pass: false`, `transformed_scope_trips_narrowly: false`
(no `cfg`/`cfg_attr`/`macro_use`), `duplicate_paths: false`,
`macro_owners_blocks_caller: false`, `true_class: must` (Rust's May set is
empty, so a `true_class: may` site -- `src/graph_impl/serialization.rs:321`
-- can never score exact under a Must-only extension and is excluded from
consideration here, not just from this design):

1. `tests/floyd_warshall.rs:11` -- `graph.add_node(())`
2. `src/lexical/bhcomp.rs:146` -- `Bigint::from_u64(theor.mant)`
3. `src/algo/mod.rs:335` -- `dfs.move_to(i)`
4. `benches/bellman_ford.rs:53` -- `g.add_edge(n1, n2, distance)`
5. `benches/matrix_graph.rs:166` -- `gr.add_node(())`

Checked each against a design restricted to **no inference** (explicit
`let x: T<...>` binding only, matching the smallest, most defensible
version of "exact concrete dispatch"):

- **#1 (`floyd_warshall.rs`):** `graph` has an explicit
  `let mut graph: Graph<(), (), Directed> = Graph::new();` in the same
  function, no rebinding. `Graph` resolves via the file's own
  `use petgraph::{prelude::*, Directed, Graph, Undirected};` to the crate's
  one `Graph<N,E,Ty,Ix>` struct. `add_node` has exactly one inherent impl
  on `Graph` (`src/graph_impl/mod.rs:525`, confirmed by grep). **Checked
  the receiver-adjustment risk directly, not assumed:** `petgraph::data::Build`
  *also* declares `fn add_node(&mut self, ...)` (`src/data.rs:47`), and
  `Graph` implements `Build` (`src/data.rs:135`) -- so two candidates exist
  by name. Rust's own method resolution always prefers an inherent method
  over a trait method on the same concrete type at the same autoref step;
  this is a fixed language rule, not a per-crate fact to re-verify, so the
  inherent method's existence alone settles it. **Survives every
  constraint.**
- **#2 (`bhcomp.rs`):** `Bigint::from_u64` is a *path* call, not a method
  call on an annotated variable -- no receiver binding to check. But
  `from_u64` is not an inherent method on `Bigint` at all: it's defined
  inside `pub(crate) trait Math: Clone + Sized + Default { ... }`
  (`src/lexical/math.rs:789`, method at line 826), reached via
  `impl Math for Bigint`. A bare `Type::method()` syntax with no inherent
  candidate requires resolving through the trait bound, which is exactly
  the "receiver adjustment" the policy lists under Unknown -- there is no
  fixed-priority rule analogous to inherent-over-trait when *only* a trait
  supplies the method. **Does not survive** under a design restricted to
  inherent impls only (the only kind of impl this fix's target-uniqueness
  check could soundly reason about without a trait-implementor index).
- **#3 (`algo/mod.rs`):** `dfs` has no explicit type annotation at its
  binding site in this function; its type is only available by inference
  through an earlier `Dfs::from_parts(...)` call. **Does not survive** the
  no-inference constraint.
- **#4, #5 (both benches):** `g`/`gr` are bound via `Graph::new()` /
  `MatrixGraph::default()` with no explicit annotation; their types are
  only available by inference from later usage or the function's return
  type. **Do not survive** the no-inference constraint.

**Only `tests/floyd_warshall.rs:11` survives every gate under a design the
policy permits.** One frozen site is enough to satisfy "measured...
improvement" -- moving it would produce 28 exact / 24 conservative and a
nonempty audit Must set (currently zero), closing the
`nonempty_must_precision_1000_on_real_repository` gap this stage's criterion
also requires.

## Guard against overclaiming on the 27 currently-exact sites

7 of the 27 exact sites are method/path-shaped, all `true_class: unknown`
(the currently-correct answer). Checked each against the same design so it
doesn't newly overclaim any of them:
- `tests/suite_string.rs:16` (`crate::suite()?.iter()`) -- receiver is an
  inline expression result, no `let` binding to read a type from. Fails the
  explicit-annotation requirement; stays Unknown.
- `tests/quickcheck.rs:418` (`quickcheck::quickcheck(...)`) -- `quickcheck`
  is an external dev-dependency crate, not indexed. Fails the
  resolves-to-an-indexed-crate-local-struct requirement; stays Unknown.
- `tests/stable_graph.rs:327` (`.insert(...)`) -- `insert` is not a
  crate-locally-unique inherent method name (common across std collections
  and this crate's own types). Fails the single-inherent-impl requirement;
  stays Unknown.
- `src/de.rs:949` (`.parse()`) -- `str::parse`/`FromStr::parse`, a std
  trait method, not a crate-indexed struct's inherent method. Fails the
  resolves-to-indexed-crate-local-struct requirement; stays Unknown.
- `src/regex/string.rs:2204` (`.next()`) -- `Iterator::next`, a std trait
  method with no crate-local inherent candidate of that exact name on the
  relevant type. Stays Unknown.
- `src/visit/traversal.rs:188` (`.clear()`) -- a common std-collection-style
  method name, not crate-locally unique. Stays Unknown.
- `tests/test.rs:2387` (`RawMapKey::ref_cast(...)`) -- `ref_cast` comes from
  the external `ref-cast` crate's trait, not an inherent impl in this
  crate. Stays Unknown.

All 7 correctly stay Unknown under the proposed design; none is at risk of
becoming a false Must.

## Design spec for the next resolver change (not yet implemented)

Scope: prove `x.m()` where all of the following hold, reusing every
existing gate (`transformed_scope`, `duplicate_paths`, parse-error,
`macro_owners`) unchanged:

1. **Receiver.** `x` has exactly one `let x: T<...> = ...;` binding in the
   same function, with no rebinding or shadowing of `x` anywhere in that
   function. No type inference -- an unannotated `let` does not qualify.
2. **Type resolution.** Resolve `T` through the file's named imports, glob
   imports, and re-exports to exactly one indexed struct's semantic path.
   Key the crate-wide index by semantic path, not bare name (two same-named
   structs in different modules, e.g. `regex`'s two `RegexSetBuilder`s
   across `regex::` and `regex::bytes::`, must not collide). Integration
   tests reference the crate under its published name (`petgraph::`), not
   `crate::` -- map that via the crate's own `Cargo.toml` package name.
3. **Method.** Exactly one inherent `m` exists across all `impl T { ... }`
   blocks for that resolved semantic path, crate-wide. If a
   `duplicate-semantic-path` gap touches either the type or the method, the
   result is Unknown, not Must.
4. **Receiver adjustment.** An inherent method on the exact concrete type
   always wins over a same-named trait method on that type -- a fixed Rust
   resolution rule, safe to encode directly (confirmed against
   `petgraph::data::Build::add_node` above). A method reachable *only*
   through a trait (no inherent candidate) is Unknown, not Must -- `Type::m()`
   syntax with no inherent `m` needs a trait-implementor index this design
   does not build. If a trait supplying `m` is in scope from an unindexed
   (external) crate and could apply to `T`, the result is Unknown; the std
   prelude's own method names (a small, fixed set) are the one case worth
   naming explicitly so common std methods (`clone`, `next`, `parse`,
   `insert`, `clear`, ...) aren't mistaken for a crate-local unique method.
5. **Where it lives.** In or after `resolve_calls` (project-wide), since it
   needs the crate-wide inherent-impl index -- `annotate()`'s per-file pass
   cannot build this alone.

**Prediction, stated before implementing:** moves exactly
`tests/floyd_warshall.rs:11` and nothing else among the 52 scored sites
(guard-checked against all 7 at-risk exact sites above; the other 4
method/path candidates that pass every existing gate independently fail
constraint 1 or 4, per the per-candidate analysis above).

**After implementing:** hand-verify a random sample of every new Must claim
this produces across all three crates (it will fire on more than the 52
audited sites -- the audit only samples 105 of however many call sites
exist). Publish that as a supplementary check, kept out of the frozen audit
itself, matching how this program has always treated supplementary
findings.

## If constraint 4 turns out unsound, or #3/#5 fail on verification

Per the roadmap's decision rule: if implementing this shows constraint 4
cannot admit `floyd_warshall.rs:11` soundly after all, or steps 2/3 fail
once actually built and no other frozen site survives, that is the point to
apply FAILED-AND-PUBLISHED once and stop -- not to keep searching for a
different design.
