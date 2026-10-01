# Stage 3 TypeScript before-observation: twelfth addendum -- the node-collision fix, implemented and verified

Follow-up to `before-observation-addendum-11.md` (`a7cd815`), which
recorded the design decision and a precommitted prediction before
writing any code. This document implements the fix and checks the
prediction against the real binary. All three predicted outcomes
confirmed exactly, including the one honestly flagged as uncertain.
`zero_classification_errors_on_audit` stays **Met**.

## The fix

`crates/aether-builder/src/mapper.rs`:

- `collect_rust_method_collisions`: a new pre-pass (Rust only) scanning
  every `impl_item` in a file, grouping its methods by
  `(type_name, method_name)`, and returning the set of pairs provided
  by more than one `impl` block -- a same-file collision group.
- `qualified_method_name`: returns `{name}@{trait}` for a trait-impl
  method whose `(type_name, name)` is in the collision set, and the
  plain `name` otherwise. An inherent method is NEVER qualified
  (`trait_name: None` always short-circuits to the plain name), and a
  non-colliding trait method keeps its plain name too -- only an
  actually-colliding trait-impl method's path changes.
  The `@` separator was chosen for simplicity (no escaping needed
  through CLI args, JSON, or shell), not Rust's own `<Type as Trait>`
  turbofish-adjacent syntax, to keep round-tripping (`find_by_path`,
  plan `node` addressing) straightforward -- the qualified path is an
  internal disambiguator, not meant to be typed by hand to match
  Rust's own UFCS syntax.
- `Scope` (used by `collect_defs`, the Node-creation pass) gained a
  `trait_name: Option<&'a str>` field, set only when entering an
  `impl_item` with a `trait` field, reset to `None` whenever entering
  any OTHER scope kind (a type definition, a function body) so a
  trait's scope never leaks past its own `impl` block.
  `collect_defs`'s function-kind branch calls `qualified_method_name`
  to build the final path segment when `scope.kind == Type` and
  `scope.trait_name.is_some()`; the TYPE's own `scope.path`/`scope.id`/
  `scope.ty` are completely untouched by this, so `Contains` edges and
  `extract_field_flows`'s owner lookup are unaffected -- exactly the
  constraint the prior design review required.
- `collect_calls`'s nested `walk` function (the SEPARATE pass that
  computes a CALLER's own id for attributing calls inside its body)
  needed the identical `enclosing_type`/`enclosing_trait` tracking and
  the same `qualified_method_name` call at its own path-building point,
  so a call made inside a qualified method's body is attributed to the
  SAME node id `collect_defs` gave that method. Threading this required
  widening `scope` from a 2-tuple to a 3-tuple
  (`(Option<type>, Option<trait>, Option<NodeId>)`) and moving the new
  `collisions` parameter into the existing `WalkContext` struct rather
  than adding it as a ninth loose function argument (clippy's
  `too_many_arguments` fired at 8; folding it into the context that
  already carries `source`/`lang`/`module` was the natural fix, not a
  lint suppression).
- The node's own `name` field (display, and `cargo test --workspace
  {name}` filtering) always stays the PLAIN method name -- only the
  `path`/`id` (used for Contains edges, Calls-edge targets, and
  uniqueness) are qualified. A qualified method being a test function
  was never a realistic concern (trait-impl methods are not where
  `#[test]` lives), but this keeps the design correct either way.

## New regression test

`colliding_trait_impl_methods_get_distinct_nodes_and_consistent_
caller_ids` (`crates/aether-builder/src/mapper.rs`): parses the exact
two-trait-impl collision shape, confirms both `go` methods get
distinct nodes with distinct ids/paths (and the plain, unqualified
`name` field on both), and -- the specific consistency check the prior
design review required -- confirms `collect_defs`'s node id for each
method equals the `caller` id `collect_calls`/`walk` recorded for the
call inside that exact method's own body. Passed on first run.

## Verification

- All four gates pass: `cargo test --workspace -j1 --quiet` (full
  pass, including the new test and all 130 `aether-builder` tests,
  zero regressions), `cargo clippy --workspace --all-targets -j1 -- -D
  warnings` (clean after moving `collisions` into `WalkContext`),
  `cargo fmt --all --check` (clean after one auto-format pass), `node
  --test npm/test/*.test.js` (unchanged).
- Rebuilt the real `girder` binary (sha256
  `1a502ab70162bf5897f810515384dec86c09b5058f9574b53ac0cb44cea7d8d4`)
  and re-ran the exact `repro-rust2` fixture from addenda 3/4/9/11.

## Precommitted predictions (addendum-11), checked against the real binary

1. **"Two `go` nodes will exist."** Confirmed exactly:
   `crate::lib::S::go@A` (span matching `impl A`'s body, containing
   `only_via_a`) and `crate::lib::S::go@B` (span matching `impl B`'s
   body, containing `only_via_b`), both present in `inspect` output
   with distinct ids.
2. **"`review` will list the specific qualified node."** Confirmed
   exactly: editing only `impl A`'s body, `review . --quiet` now prints
   `crate::lib` AND `crate::lib::S::go@A` -- not just the module, the
   outcome addenda 4-9 spent this entire thread working around for
   every OTHER symptom while the root cause stayed unfixed. This is
   the first point in the whole thread where the actual lost
   occurrence itself, not just its downstream silence, is visible.
3. **"Genuinely uncertain whether `A::go(&S)`'s explicit trait-qualified
   call syntax resolves to a specific candidate."** Confirmed exactly
   as the uncertainty predicted: `orient . --nodes crate::lib::S::go@A
   --json` and the `@B` equivalent BOTH report zero callers -- the call
   remains unresolved; `sync.rs::select_candidate`'s `candidate.owner`-
   based disambiguation (deliberately unchanged by this fix, per the
   design constraint against touching the type's own path) genuinely
   cannot distinguish the two same-owner candidates by trait identity,
   exactly as reasoned through in advance. Confirmed the predicted
   safety net too: `test-impact . --quiet` still correctly selects
   BOTH `calls_a` and `calls_b` regardless, via the conservative
   escalation already in place -- the call's own `unknown`
   classification (trait methods were never Must-eligible, per
   addendum-11's design-decision research) means this remains safe
   even though the specific `Calls` edge isn't gained.

## Downstream measurements re-run on the new binary

- **`core_trustworthiness_oracle.py`**: `baseline: matches`
  (`docs/core-trustworthiness-baseline.json`, unchanged from addendum-9's
  update) -- precision/recall unchanged at 1.000/1.000. Specifically
  confirmed `rust_raii_drop_selected` is STILL correctly selected --
  the exact fixture that would have broken had the design decision
  chosen "qualify every trait-impl method" instead of "qualify only
  colliding ones" (the `sync.rs:967` `{owner}::drop` lookup this
  session's design review found and specifically avoided breaking).
- **`core_representative_mutations.py`**: byte-identical result to the
  currently-committed `docs/core-representative-mutations.json` (diffed
  field-by-field, zero differences) -- expected, since `group-invoke`
  is a Python/Click mutation, entirely untouched by a Rust-only fix.

## Still not done in this round

- **Rust's DONE audit's own `correction-3` re-score** against a fresh
  `inspect` of the pinned corpus (`petgraph`/`regex`/`serde_json`) is
  NOT done here. The pinned checkouts used for that audit are not
  present in this session's scratchpad (same persistence problem
  addenda 4/8/9 already flagged for the TypeScript corpus). Given
  `ser.rs`'s two `serialize_element` methods are the confirmed,
  concrete real-world instance of this exact collision (addendum-4),
  this fix should change that specific site from `duplicate-semantic-
  path` (a whole-file gap claim) to a real, individually-attributed
  claim once re-scored -- recorded as a roadmap item, not assumed or
  silently skipped.
- TypeScript's own collision mechanism (duplicate `it()`/`describe()`
  description strings) is UNCHANGED by this Rust-only fix. Sites 23 and
  91 remain lost-node in the TypeScript corpus; this fix does not
  extend to TypeScript's distinct collision shape (a JS/TS-specific
  qualification scheme -- e.g. a positional/sibling-index discriminator
  for identically-worded test descriptions -- would be separate,
  not-yet-designed work).
- The combined-origin residual (addendum-10, confirmed on a real Go
  graph) is unrelated to and unaffected by this fix -- still open.

## Scope

This fix changes Rust `NodeId`s for trait-impl methods that ACTUALLY
collide with a same-named sibling in the same file -- a narrow,
previously-broken set (every instance was already either completely
missing from the graph, per addendum-4's finding, or silently wrong).
No currently-published, non-colliding Rust path changes. No Python,
TypeScript, or Go path changes at all.
