# Supplementary check: every new Must claim across all three audited crates

Per the design spec's own instruction ("hand-verify a random sample of every
new Must claim across all three crates... publish it as a supplementary
check, kept out of the frozen audit"). **Not part of the frozen audit** --
Girder selected these sites itself, which the audit methodology forbids for
the frozen sample.

Measured against the final commit (see [after-observation.md](after-observation.md)
for the exact hash and sha256), after five correction rounds -- two of
which (rounds 3 and 4) each found a genuine unsoundness in the immediately
prior round's own "fix" for a different unsoundness, not just a missing
test. All found via `advisor` reviews asked to check this document and its
code for accuracy before either was trusted, run twice this session because
the first review's own fix needed a second review.

## Count, by crate and file

Grepped `proven-inherent-method-on-annotated-receiver` across each crate's
complete `call_evidence_v1` output (`girder inspect --json`), parsed
programmatically (not eyeballed) from the RON attribute text:

- **petgraph-0.6.5: 49**, split `tests/floyd_warshall.rs` 23,
  `tests/k_shortest_path.rs` 14, `tests/operator.rs` 12.
- **serde_json-1.0.150: 0.**
- **regex-1.12.4: 0.**

## Targets, decoded and resolved programmatically across all 49

Every claim's target `NodeId` (RON decimal) converted to the inspect
output's hex node-id form and looked up against the node list directly (not
trusted from the claim's own reason string): **all 49 resolve to exactly
three targets**, all inherent methods on
`crate::graph_impl::mod::Graph<N, E, Ty, Ix>` (`src/graph_impl/mod.rs`):
`add_node` (40 claims), `extend_with_edges` (7 claims), `node_indices` (2
claims). No claim points at `Build::add_node`, at any sibling type's own
differently-typed methods of the same name, or at any method on a type
other than the one actually named in the receiver's annotation.

## Hand-verified against source, all three files read in full this session

- **`tests/floyd_warshall.rs`** (23 claims): single explicit
  `let graph: Graph<...> = Graph::new();` per test function, no rebinding,
  only macro invocation is the trusted `assert_eq!`/`assert!`. This file's
  own import is `use crate::algo::floyd_warshall;` (fully qualified) --
  the bare, unprefixed re-export that matters for the sibling-`mod`
  recognition rule lives in `src/algo/mod.rs` (`pub mod floyd_warshall;`
  plus `pub use floyd_warshall::floyd_warshall;`, both at that file's own
  top level), not in `tests/floyd_warshall.rs` itself -- corrected from an
  earlier draft of this document that attributed the bare re-export to the
  test file.
- **`tests/k_shortest_path.rs`** (14 claims): single
  `let mut graph: Graph<(), (), Directed> = Graph::new();`, no rebinding.
  13 `graph.add_node(())` calls plus one `graph.extend_with_edges(...)`
  call = 14, matching the count exactly.
- **`tests/operator.rs`** (12 claims): read in full (`cat -n`, not
  excerpted). Three `Graph`-typed bindings in one function -- `graph`,
  `output`, `expected_res` -- each its own single, non-rebound
  `let ...: Graph<(), (), Directed> = Graph::new();`. `output` is not
  unused: it is passed by `&mut` reference into
  `complement(&graph, &mut output, ())` and then read via
  `output.contains_edge(x, y)`. The 12 claims are 4 `graph.add_node(())` +
  4 `expected_res.add_node(())` + 1 `graph.extend_with_edges(...)` + 1
  `expected_res.extend_with_edges(...)` + 2 `graph.node_indices()` (inside
  the nested
  `for x in graph.node_indices() { for y in graph.node_indices() { ... } } }`
  loop) = 12.
  - **Why this file's sites are new relative to `c436dbd`, confirmed by
    reading that commit's actual code** (`git show
    c436dbd:crates/aether-builder/src/mapper/method_index.rs`): its
    `binding_type_if_unique` handled `"for_expression" | "match_pattern" |
    "let_condition"` identically, incrementing the binding count if the
    *entire for-expression's text* (via a naive word-split, not the
    grammar's `pattern` field) contained the receiver's name anywhere --
    which wrongly counted "graph" appearing inside the iterator expression
    `graph.node_indices()` itself as a second binding of "graph",
    over-conservatively blocking the proof (never unsoundly). A later
    round replaced this with a `pattern`-field-only check for
    `for_expression`, correctly recognizing that only the loop variable is
    bound, newly enabling this file's proofs. `tests/floyd_warshall.rs`
    and `tests/k_shortest_path.rs` have no `for` loop over a
    receiver-typed expression and were unaffected by this specific change.
- **`extend_with_edges`**: `grep -rn "fn extend_with_edges" --include=*.rs .`
  against the full checkout: inherent-only, on **three** types --
  `Graph` (`src/graph_impl/mod.rs`), `StableGraph`
  (`src/graph_impl/stable_graph/mod.rs`), and `MatrixGraph`
  (`src/matrix_graph.rs`) -- corrected from an earlier draft of this
  document, which claimed "one other type" without having run this half
  of the grep.
- **`node_indices`**: `grep -rn "fn node_indices" --include=*.rs .`:
  inherent-only, on three types -- `Graph`, `StableGraph`, and `List`
  (`src/adj.rs:302`, confirmed by reading the file's own `impl` blocks
  directly: line 302 falls inside `impl<E, Ix: IndexType> List<E, Ix> {`,
  opened at line 162 -- corrected from an earlier draft's unverified guess
  of "Adjacency"). No trait anywhere declares either `extend_with_edges`
  or `node_indices`, so `inherent_wins` returns true immediately for both
  -- and since the crate-wide inherent-method index is keyed by
  `(type_name, method_name)`, the other types' same-named methods never
  collide with `Graph`'s own entry.
- **`add_node`**: verified in the design-spec documents from an earlier
  round -- the competing `Build::add_node` sits at the identical
  `&mut self` step with textually identical bounds on both impls, and
  neither the struct, either impl, nor either module declaration carries
  `cfg`.
- **`tests/floyd_warshall.rs:11`'s own target, decoded**: the frozen
  audit's scored byte offset (342) falls inside the claim at
  `(start_byte:337,end_byte:355,...)` in
  `crate::tests::floyd_warshall::floyd_warshall_uniform_weight`'s
  evidence, target `NodeId(1266861872763132360)` -> hex `1194cc29418859c8`
  -> `crate::graph_impl::mod::Graph<N, E, Ty, Ix>::add_node`. Confirmed
  directly from the raw RON text this round, not inferred.
- **Audit diff, run programmatically this round** (not asserted): loaded
  both `after-assert-macro-fix/audit-after.json` (the prior resolver
  change's result, 27 exact / 25 conservative) and this round's
  `audit-after.json`, keyed every result by `(crate, file, line)`, and
  diffed `cell` for every key present in both. **Exactly one entry
  changed**: `(petgraph-0.6.5, tests/floyd_warshall.rs, 11)`,
  `conservative -> exact`, `true_class: must`, `observed_class: must`.

## Round 4: a genuine unsoundness found in round 3's own "safe" fix, plus test-vacuity findings

An `advisor` review of round 3's claimed-final state (`62141a9`) found a
real false-Must path before this document was trusted:

**`in_crate`'s "any `mod` declared anywhere in the crate" rule was
unsound, not merely a safe over-approximation as `62141a9`'s own commit
message claimed.** Round 3's fix for the sibling-`mod` recognition problem
(needed so `src/algo/mod.rs`'s own bare `pub use
floyd_warshall::floyd_warshall;` is recognized as in-crate) was implemented
crate-wide instead of scoped: any bare first segment matching a `mod`
declared *anywhere* in the crate was treated as in-crate, everywhere. This
makes every downstream check that consumes `in_crate` *more* permissive,
the unsafe direction. Concretely: petgraph's own `src/lib.rs` declares
`mod quickcheck;` (confirmed live: `grep -n "mod quickcheck" src/lib.rs`);
under the crate-wide rule, an unrelated file's `use quickcheck::Gen;` (the
external crate's `Gen`) would be wrongly treated as in-crate, and if that
file also had its own in-crate `struct Gen`, the call could be falsely
proven Must against the wrong target.

**Round 4 fix**: scoped `in_crate` per FILE (`file_in_crate_check`): a bare
segment in-crate only if the same file declares a `mod` of that name.
Verified by mutation (reverted to crate-wide, confirmed a new negative test
failed; restored, confirmed it and the rewritten sibling-mod positive test
both pass -- the earlier version of that positive test used a
`crate::`-prefixed import and an inline `mod`, and would have passed even
under the never-widened original rule, so it never actually reproduced the
bug it claimed to).

Round 4 also: expanded `STD_PRELUDE_METHOD_NAMES`
(`is_sorted*`/`DoubleEndedIterator`/`ExactSizeIterator`/`Fn*`/`IntoFuture`/
`Future` methods); replaced a vacuous destructured-receiver test (which
only exercised a tuple-annotated destructure that `bare_type_name` already
rejects for an unrelated reason) with three real shadowing reproductions
(`for`-loop, closure-param, struct-shorthand patterns), each confirmed by
mutation to fail if `pattern_binds_name` regresses to identifier-only;
added direct tests for two previously-untested guards (an inline
`#[cfg] mod` wrapping the target impl; a file-level `#![cfg]` on the
target's file); added a standalone unit test of `path_occurrences` (the
target-side duplicate-path guard, unreachable through the full pipeline
since two identically-named inherent methods trip the earlier
candidate-count check first).

## Round 5: file-scoping was ALSO still unsound (needed module-scoping), plus three tests found vacuous on their own terms

A second `advisor` review, of round 4's own "DONE" draft, found:

**`in_crate`'s file-scoped rule was still unsound: a bare `use` path
resolves against the current MODULE's own scope, not merely the current
file.** `mod m { use foo::Gen; }` next to a top-level `mod foo;` in the
SAME FILE names the external crate `foo` -- `foo` and the `use` sit in
different modules even though they share a file; Rust's actual resolution
does not look outward from a nested module to a sibling of an ancestor
module for a bare `use` segment. **Round 5 fix**: `ModDeclFact` and
`ImportFact` each gained a `scope: u64` field (the start byte of the
nearest enclosing `mod_item`, or 0 for file-root, via a new
`enclosing_mod_scope` helper); `in_crate` now requires the segment AND the
scope to match. Verified by mutation with the exact fixture advisor
specified (`mod m { use foo::Gen; }` beside top-level `mod foo;` and
`mod gens;`, `Gen` defined in `gens.rs`): confirmed a false Must fires
under the file-scoped (not module-scoped) rule, and does not fire under
the module-scoped fix. `src/algo/mod.rs`'s own `mod`/`pub use` pair are
both declared at that file's own top level (scope 0), so this narrowing
does not affect `tests/floyd_warshall.rs:11` -- confirmed by re-running
the full audit/corpus/oracle/supplementary measurement after the fix (all
numbers held, see [after-observation.md](after-observation.md)).

`file_globs_are_in_crate` (glob imports have no per-glob scope tracked)
was narrowed to recognize only `crate`/`self`/`super`/the package name for
globs -- never matching a bare glob segment against any `mod` name at
all -- rather than risk the same class of bug at file- or crate-scope for
globs specifically. None of this design's currently-measured claims
depend on glob mod-name matching (petgraph's own `use
petgraph::prelude::*;` matches via the package-name branch).

**The reset-loop "not load-bearing" claim from round 4 was also wrong, for
a different reason than a regular test gap: it was verified against only
one of the two incremental-update code paths.** Round 4 checked
`update.rs` (used by `update_files`) and found the loop provably inert
there (that path always reconstructs a fresh graph from pristine per-file
extraction before `resolve_calls` runs). It did not check `load_file`/
`load_files` (`sync.rs:739-754`), which apply each file's extraction
directly onto the SAME live graph passed in and then call `resolve_calls`
on it -- no fresh reconstruction. A new test using sequential `load_file`
calls (load a struct+impl and a caller, confirm Must, then `load_file` a
third file adding a second inherent impl) confirms the loop IS load-bearing
on this path: mutation-disabling the reset loop makes this new test fail
while leaving the `update_files`-based test (which exercises the other
path) passing, exactly distinguishing the two. The code comment and this
document are corrected to say the loop matters for `load_file`/
`load_files`, not for `update_files`.

**Three tests didn't isolate the guard they were named for; each was
rewritten and reverified by a combined single-compile mutation** (forcing
`file_has_inner_cfg` to always return `false`, forcing
`cfg_gated_including_inline_mod_ancestors` to always return `false`, and
forcing `reaches_call` to always be `true`, all at once):
- `a_file_level_inner_cfg_attribute_blocks_a_proof_in_that_file` originally
  put the `#![cfg]` on the CALLER's file, where the unrelated caller-side
  `transformed_scope` gate blocks the call for a different reason,
  regardless of whether `file_has_inner_cfg` works. Rewritten to put the
  target struct+impl in their own file (`src/graph_impl.rs`, declared via
  `pub mod graph_impl;`) with the `#![cfg]` on that file instead.
- `a_let_declared_after_the_call_does_not_prove_it` originally wrapped the
  call in a nested `fn helper(graph: &mut Graph)`, where the parameter
  binding (not the let-position check) is what blocks it. Rewritten to a
  plain text-level case: the call precedes the `let` in the same block.
- A new `a_let_in_a_sibling_block_does_not_prove_a_call_outside_it` test
  covers the other position case: the `let`'s own enclosing block ends
  before the call's sibling block begins.
- All four (including `an_inline_cfg_gated_mod_wrapping_the_inherent_impl_blocks_proof`,
  confirmed non-vacuous as originally written) fail under the combined
  mutation and pass under the real code -- verified in one compile each
  way, not asserted.

## The two bugs round 3's own count caught (background)

Both found by re-measuring after round 3's soundness-gap fixes regressed
the verified site (27/25, not the predicted 28/24) instead of holding it --
traced directly, not assumed:

1. **`externally_aliased_names` treated a verified-safe import as unsafe.**
   Inserted every non-in-crate named import's bare local name, including
   `std::collections::HashMap` -- which a separate check had already
   verified safe -- making "HashMap" crate-wide-unsafe and re-disqualifying
   every file that safely imports it. Fixed by sharing one
   `import_is_verified_safe` predicate between both checks.
2. **A bare `use` path naming a sibling `mod` wasn't recognized as
   in-crate at all.** Originally fixed by widening to crate-wide scope,
   which introduced round 4's finding; round 4 narrowed to file scope,
   which introduced round 5's finding; round 5 narrowed to module scope.
