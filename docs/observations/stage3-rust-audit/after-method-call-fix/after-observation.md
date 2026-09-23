# Stage 3 Rust after-observation: inherent-method-on-annotated-receiver resolver

Source commits: `d0ce300` (the resolver), `c436dbd` (wiring in guards the
first commit collected but never used), `62141a9` (seven -- that commit's
own message says six, a miscount of its own bullet list -- more soundness
gaps found before trusting a measurement, plus two implementation bugs
those fixes themselves introduced, found by re-measuring rather than
assumed fixed), and `2452bc1` (this round's fix): two further genuine
unsoundnesses found by two separate `advisor` reviews
of this document's own prior drafts, each catching a real gap the previous
review's fix introduced or missed. Binary measured:
`/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder`, sha256
`3ca620e62ac81d537863490dd535248e9971605a1cd9f144292e284aeae32de5`.

## Result up front: all four criterion legs met, after five correction rounds. Stage 3 (Rust): DONE.

This is the second of two resolver changes measured against Stage 3's
per-language criterion. The first change (`47c3a06`) satisfied the
corpus-improvement and zero-error legs but left the audit unchanged and the
real-repository Must set empty. This one closes both remaining gaps -- but
only after five rounds of finding real gaps before trusting the result,
documented here in full rather than smoothed over. Rounds 4 and 5 each
found a genuine unsoundness in the immediately prior round's own fix for a
*different* unsoundness -- not just missing test coverage. That distinction
matters and is not softened here: this design was wrong, in the specific
sense of admitting a false Must under a constructible input, twice in a
row, after being reviewed and believed fixed both times.

## Why this took five rounds

1. **`d0ce300`** implemented the design spec. First measurement missed the
   predicted site entirely (0 `add_node` claims, 5 `extend_with_edges`
   claims) -- traced to `inherent_wins` checking `cfg_gated` on every impl
   of a competing trait crate-wide, including impls covering unrelated
   types. Fixed within the same commit cycle.
2. **`c436dbd`**, found on review of the resulting observation *before it
   was committed*: several fields the extractor collected were never read
   by the evaluation (caller-side `transformed_scope`/`duplicate_paths`/
   parse-error/`macro_owners`, the target's cfg chain through ancestor
   `mod` declarations, target-side path-collision detection, and a
   staleness reset). Wired in; re-measured; the predicted site held
   (28/24).
3. **`62141a9`**, found on a further review specifically hunting for
   realistic false-Must paths: seven gaps -- destructured receiver
   patterns invisible to the binding-uniqueness check, no
   let-precedes-call/same-block position check, no
   generic-type-parameter-shadowing check, an incomplete
   prelude-method-name list, a visibility check that accepted
   `pub(crate)`/`pub(super)`, a safe-import check that matched on local
   name alone, and a caller-import-aliasing check that only looked at the
   receiver type's own name. Fixing these **regressed the verified site**
   (27/25, not the predicted 28/24) -- two real implementation bugs in the
   new guards, both traced directly: an over-broad "externally aliased"
   classification that flagged an already-verified-safe `std::HashMap`
   import, and an `in_crate` check that didn't recognize a bare `use` path
   naming a sibling `mod` in the same file (petgraph's own
   `src/algo/mod.rs`). **The fix for the second bug was itself unsound**
   -- see round 4.
4. **Round 4**, found by an `advisor` review of the previous "DONE" draft
   of this document before it was trusted or committed: `62141a9`'s fix
   for the sibling-`mod` bug recognized a bare `use` segment as in-crate
   if *any* `mod` declared *anywhere in the crate* had that name --
   described in that commit's own message as "a safe over-approximation."
   That direction is backwards: it makes every downstream check that
   consumes `in_crate` *more* permissive, the unsafe direction.
   Concretely: petgraph's own `src/lib.rs` declares `mod quickcheck;`; an
   unrelated file's `use quickcheck::Gen;` (the external crate's `Gen`)
   would be wrongly treated as in-crate under the crate-wide rule. Fixed
   by scoping `in_crate` per FILE. Same review also strengthened several
   under-tested guards and expanded the prelude method list (see
   [supplementary-hand-verification.md](supplementary-hand-verification.md)
   for the full list).
5. **Round 5**, found by a SECOND `advisor` review, of round 4's own "DONE"
   draft: the file-scoped fix from round 4 was **also still unsound**. A
   bare `use` path resolves against the current MODULE's own scope, not
   merely the current file -- `mod m { use foo::Gen; }` next to a
   top-level `mod foo;` in the SAME FILE names the external crate `foo`,
   not the sibling module. Fixed by adding a `scope` field (the nearest
   enclosing `mod_item`'s start byte, or 0 for file-root) to both
   `ModDeclFact` and `ImportFact`, and requiring both the segment name AND
   the scope to match. The same review also found the round-4 claim that
   the in-module evidence-reset loop is "not load-bearing on any current
   call path" was verified against only one of two incremental-update code
   paths (`update.rs`, via `update_files`) and is actually false for the
   other (`load_file`/`load_files`, which apply directly onto the live
   graph without reconstruction) -- corrected, with a new test that
   isolates exactly that path. It also found three tests that didn't
   isolate the guard they were named for (blocked by an unrelated gate
   instead) and several unverified documentation claims (a
   `extend_with_edges` grep that was only half-run; a wrong guess at which
   type `adj.rs:302`'s `node_indices` belongs to; an "exactly one site
   changed" claim that hadn't actually been diffed programmatically). All
   fixed and reverified this round -- full detail, including every
   mutation-test result, in
   [supplementary-hand-verification.md](supplementary-hand-verification.md).

Re-measured after all of the above: the predicted site is restored, with
the same 49 supplementary claims as every prior round since `62141a9` --
each round's fix narrows *which* imports are treated as in-crate, and none
of the three audited crates' actually-measured sites ever depended on the
unsound widenings, so this crate's own numbers are unchanged even though
the underlying rule was wrong twice.

## 1. Real-repository audit: moved exactly as predicted, target verified, diff run programmatically

[audit-after.json](audit-after.json): **52 scored, 28 exact, 24
conservative, 0 unsound** (`cell_counts: {"exact": 28, "conservative": 24}`,
`scored_count: 52`, no `unsafe_exclusion`/`overclaim` cells present).

**Diffed programmatically** against
[after-assert-macro-fix/audit-after.json](../after-assert-macro-fix/audit-after.json)
(the prior resolver change's result, 27 exact / 25 conservative) by
`(crate, file, line)` key: **exactly one entry's `cell` changed**,
`(petgraph-0.6.5, tests/floyd_warshall.rs, 11)`, `conservative -> exact`,
`true_class: must`, `observed_class: must`, reason
`proven-inherent-method-on-annotated-receiver`.

**Target verified, not just class**: the scored byte offset (342) falls
inside the claim `(start_byte:337,end_byte:355,...)` in
`crate::tests::floyd_warshall::floyd_warshall_uniform_weight`'s evidence,
target `NodeId(1266861872763132360)` -> hex `1194cc29418859c8` ->
`crate::graph_impl::mod::Graph<N, E, Ty, Ix>::add_node` -- the correct
inherent method, decoded and resolved against the node list directly.

## 2. Dispatch corpus and Stage 1 oracle: unchanged, no regression

[corpus-after.json](corpus-after.json): pooled 21 exact / 35 conservative,
`must_true_positives: 4`, `must_false_positives: 0` -- unchanged from the
first resolver change's after-numbers.

[oracle-after.json](oracle-after.json): Rust and Python both `precision:
1.0`, `recall: 1.0`, no false positives or negatives -- unchanged. stderr
empty ([oracle-after.stderr.log](oracle-after.stderr.log)), exit 0.

## 3. Supplementary check: every new Must claim, not just the frozen site

Full detail, including all five rounds' findings and every mutation-test
result, in
[supplementary-hand-verification.md](supplementary-hand-verification.md).
Summary: **49** new Must claims in petgraph (23 `tests/floyd_warshall.rs`,
14 `tests/k_shortest_path.rs`, 12 `tests/operator.rs`; 0 in serde_json; 0 in
regex), all 49 programmatically confirmed to resolve to exactly three
correct targets (`Graph::add_node`, `Graph::extend_with_edges`,
`Graph::node_indices`), all three source files read in full this session.

## 4. Common gates

All four, run once each against this round's final commit:
[gate-01-cargo-test-workspace.log](gate-01-cargo-test-workspace.log) (every
crate, 0 failed, 1 pre-existing ignored test, exit 0),
`cargo clippy --workspace --all-targets -j1 -- -D warnings` and
`cargo fmt --all --check` both exit 0
([gate-02-03-clippy-fmt.log](gate-02-03-clippy-fmt.log)),
[gate-04-node-test.log](gate-04-node-test.log) (31 tests, 29 pass, 2
skipped -- no vendored binary in this environment).

## Corrected Stage 3 Rust criterion status -- all four legs

- `measured_dispatch_corpus_improvement`: **Met** (first resolver change,
  unaffected by this one).
- `zero_classification_errors_on_audit`: **Met.** 0/52 unsound; oracle
  1.0/1.0.
- `audit_shows_fewer_conservative_more_exact_cells`: **Met.** 27/25 ->
  28/24, the predicted site, target verified, diff run programmatically.
- `nonempty_must_precision_1000_on_real_repository`: **Met.** 1 Must site
  in the frozen sample, scored exact, precision 1/1 = 1.000.

**Stage 3 (Rust): DONE.** All four criterion legs hold with committed
evidence, measured against a binary whose implementation was reviewed five
times before trusting any number from it -- four of those reviews each
found and fixed real gaps, and two of those fixes were themselves unsound
in ways only found by a subsequent review that specifically re-checked the
previous fix's own safety-direction claim rather than trusting that it
compiled and passed its own test. Per the roadmap's language order ("do not
start Python before Rust is trustworthy on a real repository"), Python may
now begin.

## Remaining, disclosed limitations (not blocking DONE, carried forward)

- **Glob-import safety is a deviation from the frozen design spec's own
  constraint 6, not merely a scoped-out simplification.**
  `gate-profile-correction-2.md`'s constraint 6 requires an in-crate glob's
  target module to itself `pub use` nothing from outside the indexed
  crate, "checked recursively, not just one level." The implementation
  checks only the caller's own file's glob imports (and, after round 5,
  recognizes only `crate`/`self`/`super`/the package name for a glob's
  bare segment -- never a `mod` name, to avoid the same class of
  scope-confusion bug this round found for named imports). Every claim
  carries the `glob-safety-checked-file-local-only` assumption string
  disclosing this, but it should be named plainly as an unmet piece of the
  committed spec, not softened to a residual simplification.
- `STD_PRELUDE_METHOD_NAMES` is hand-compiled, not generated from the
  toolchain's own std source (`rust-src` isn't installed in this
  environment) -- erred broad where checked, but not exhaustively verified
  against the actual standard library.
- No type inference: only an explicit `let x: T<...>` binding is provable.
- Bare-name type-namespace uniqueness and bare-name `mod`/import-aliasing
  checks, not full semantic-path/re-export resolution -- sound (fails
  closed) wherever checked in this session, but a crate with genuinely
  ambiguous same-named items reachable under different re-export paths
  would correctly, conservatively fall back to Unknown rather than attempt
  disambiguation.
- Trait-competitor bounds are compared for exact textual equality, not
  solved for satisfaction.
- The macro-token-tree binding rule an earlier correction round sketched
  (a name bound inside a trusted macro's own token tree) is not
  implemented; the existing `macro_owner_function_indices` guard instead
  blocks proof entirely for any function containing an untrusted macro
  invocation, which is conservative (never unsound) but coarser than the
  sketch. Recorded as a deviation, not implemented.
- Module-scoping (round 5) is itself approximate: it identifies a module by
  the byte offset of its nearest enclosing `mod_item`, which is correct
  within one file but does not attempt to unify the same logical module
  when it's split across multiple `mod`-declared files (e.g. two `mod`
  declarations pointing, via `#[path]`, at content that Rust would treat
  as one module) -- an edge case not observed in any of the three audited
  crates, and conservatively safe (can only under-recognize, never
  over-recognize, in-crate status) rather than unsound.
