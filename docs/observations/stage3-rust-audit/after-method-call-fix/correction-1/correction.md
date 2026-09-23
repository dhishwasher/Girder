# Correction to the Stage 3 Rust "DONE" observation (committed `46a5633`)

A third `advisor` review, run against the just-committed `46a5633`/`b9ec773`
"DONE" state specifically to sanity-check the milestone before treating it
as settled, found one more genuine unsoundness plus several accuracy
problems in the committed documentation. Fixed in `facc21c`. **This does
not revoke DONE** --
re-measurement after the fix shows the audit/corpus/oracle/supplementary
numbers are byte-identical to `46a5633`'s own numbers. It is recorded here,
as a correction, rather than by editing the already-committed
`after-observation.md`/`supplementary-hand-verification.md` files, per this
session's established practice of not softening or silently rewriting a
prior observation.

## 1. `enclosing_mod_scope` stopped only at `mod_item`, not at `block` -- unsound

Round 5 (committed `2452bc1`) added module-scoping to `in_crate`, fixing a
crate-wide-then-file-wide unsoundness. But `enclosing_mod_scope`'s ancestor
walk stopped only at `mod_item`, not at `block`. A `mod` declared *inside a
function body* (block-scoped in real Rust, not module-scoped) computed
scope `0` -- the same scope id as the file's own top-level module -- so a
completely unrelated, module-level `use` elsewhere in the SAME file could
wrongly be recognized as naming that block-local `mod`.

**Concrete false Must, verified live before fixing** (not assumed): a file
containing both
```rust
use quickcheck::Gen;
fn helper() { mod quickcheck {} }
#[test]
fn t() {
    let mut g: Gen = Gen::new();
    g.size();
}
```
(with an in-crate `struct Gen { fn size(&mut self) -> usize }` defined
elsewhere) produced a Must claim against the in-crate `Gen::size`, when
`quickcheck::Gen` in real Rust names the external crate -- a block-local
`mod` is not visible to a module-level `use` in the same file. Confirmed by
writing this exact fixture as a test, observing it fail under the
then-current code, then fixing it.

**Fix**: `enclosing_mod_scope` now also stops at `block`, matching real
Rust's scoping (a function/closure/`unsafe`/`async`/`const` body is its own
scope). Two items directly in the same immediate module body or the same
immediate block are always mutually visible in Rust regardless of
declaration order; anything with a different nearest enclosing
module-or-block is conservatively treated as not sharing scope. This is
not exhaustively verified against every corner of Rust's resolution (e.g.
an item in an outer block referenced from a nested inner block of the same
function, which this conservatively rejects even where real Rust would
allow it) -- disclosed rather than asserted as complete, unlike the two
prior rounds' "safe" claims about this same rule.

**Verified by mutation**: reverted to the `mod_item`-only walk with the
fixture above as a permanent regression test
(`a_mod_declared_inside_a_function_body_does_not_make_its_bare_name_in_crate_at_module_level`);
confirmed it fails under the reverted code and passes under the fix.
`tests/floyd_warshall.rs:11`'s own basis (`src/algo/mod.rs`'s `pub mod
floyd_warshall;` and `pub use floyd_warshall::floyd_warshall;`, both at
that file's own top level, scope 0) is unaffected -- confirmed by
re-running the full measurement suite below, not just by reasoning that it
should be unaffected.

## 2. Corrected counts

- **Test count**: `2452bc1`'s commit message says "12 new tests (49 total,
  up from 37)". The 49 was a copy-confusion with the unrelated "49
  supplementary Must claims in petgraph" figure. The actual, directly
  counted (`grep -c '^    #\[test\]$'`, which excludes `#[test]` strings
  embedded inside fixture source literals) numbers: `62141a9`'s own
  committed baseline had **28** tests; this correction's final state has
  **40** -- **12 new tests**, which is the one part of that message that
  was right. `62141a9`'s own "11 new tests" is also corrected here: it was
  actually 12 (16 → 28), a miscount carried in that commit's message ever
  since.
- **Mutation-test coverage claim**: the roadmap checkpoint said "every fix
  in the final round verified by mutation." The glob-scope narrowing (item
  in round 5) and the `STD_PRELUDE_METHOD_NAMES` additions have no
  dedicated mutation test -- the glob narrowing is a straightforward
  removal of a code path (mutation would just be re-adding mod-name
  matching for globs, which round 5's document already explains the risk
  of without a fixture proving it live), and the prelude additions are
  data, not logic, so "mutation" doesn't apply the same way. The roadmap
  checkpoint entry is corrected to name specifically which changes were
  mutation-verified (the module-scope rule, the destructured-pattern
  check, and the three previously-vacuous tests) rather than claiming
  blanket coverage.
- **Dropped plan item, now recorded rather than silently absent**: the
  implementation-plan addendum's `#[ignore]`d real-petgraph verification
  test (reading a checkout path from an env var, reproducing every fact in
  the two correction documents, kept out of the four common gates) was
  never written. The supplementary hand-verification documents in this
  directory serve the same verification purpose via a different mechanism
  (grep/decode against a real checkout, run manually each round rather
  than as an ignored test), but the specific test artifact the plan named
  does not exist. Recorded as not done, not implied complete.

## 3. Re-measurement: all numbers hold, unchanged from `46a5633`

Binary: `/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder`,
sha256 `1537b460a8eb3f2b8949c518a134415800fa99ed05f50341094c5bfe8e0f99e9`.

- [audit-after.json](audit-after.json): **52 scored, 28 exact, 24
  conservative, 0 unsound** -- identical to `46a5633`.
- [corpus-after.json](corpus-after.json): pooled 21 exact / 35
  conservative, `must_true_positives: 4`, `must_false_positives: 0` --
  identical.
- [oracle-after.json](oracle-after.json): Rust and Python both
  `precision: 1.0`, `recall: 1.0` -- identical, stderr empty.
- Supplementary claims, recounted from fresh
  [petgraph-0.6.5-inspect.json](petgraph-0.6.5-inspect.json) /
  [serde_json-1.0.150-inspect.json](serde_json-1.0.150-inspect.json) /
  [regex-1.12.4-inspect.json](regex-1.12.4-inspect.json): **49** in
  petgraph (23/14/12 across the same three files), **0** in serde_json,
  **0** in regex -- identical split to `46a5633`.
- All four common gates and the final build pass, chained in a single
  sequential background job (not run concurrently this time -- an earlier
  round in this session ran `cargo test` and `cargo clippy` as two
  separate concurrent background jobs, which cargo's own build-directory
  lock serialized safely but which violated this repository's "never a
  second cargo job running concurrently" constraint; not repeated here):
  [all-gates-and-build.log](all-gates-and-build.log).

## Conclusion

Stage 3 (Rust) remains **DONE**. This is the sixth round of review across
this whole resolver's lifetime (`d0ce300` → `c436dbd` → `62141a9` →
`2452bc1` → this correction), the third of which (rounds 4, 5, and this
one) each found a genuine unsoundness in the immediately prior round's own
fix. None of the three findings changed any measured number on the three
audited crates -- every fix narrowed an over-broad `in_crate` recognition
rule that none of this sample's actual sites happened to depend on. That
is a property of this specific sample, not a guarantee the rule is now
exhaustively correct; the disclosed limitations in `after-observation.md`
and this document should be read as a live list, not a closed one.
