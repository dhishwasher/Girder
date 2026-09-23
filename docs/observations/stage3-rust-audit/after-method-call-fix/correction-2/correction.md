# Second correction to the Stage 3 Rust "DONE" observation

A fourth `advisor` review -- this one run alongside the start of Stage 3
Python work, specifically re-checking `correction-1`'s own "can only
under-recognize, never over-recognize" claim about `enclosing_mod_scope`
-- found a third distinct collision source in that same function, on top
of the two `correction-1` already found and fixed. **This does not revoke
DONE**: re-measurement after the fix shows the audit/corpus/oracle/
supplementary numbers are byte-identical to `correction-1`'s own numbers.
Recorded as its own correction, per this session's practice of not editing
a previously-committed observation.

## The bug: `enclosing_mod_scope`'s "no container" sentinel collided with a real container's own start byte

`enclosing_mod_scope` returned `0` both for "walked all the way up, found
no enclosing `mod_item`/`block`" (true file-root scope) AND for a real
enclosing container whose own `start_byte()` happens to be `0` -- which
happens whenever a `mod`/`block` is the very first thing in a file (no
leading attribute, no leading whitespace, nothing else before it). A `use`
genuinely at file-root and an unrelated `mod`'s own child (nested inside a
DIFFERENT `mod` that itself starts at byte 0) could then wrongly compute
the identical scope id purely from this coincidence of file position, not
from actually sharing lexical scope.

**Concrete false Must, verified live before fixing**:
```rust
mod m {
    use quickcheck::Gen;
    #[test]
    fn t() {
        let mut g: Gen = Gen::new();
        g.size();
    }
}
mod quickcheck {}
pub mod gens;
```
(`gens.rs` defines an in-crate `struct Gen { fn size(&mut self) -> usize }`.)
`mod m { ... }` is the very first thing in the file, so its `start_byte()`
is `0`; `use quickcheck::Gen;`, declared directly inside `mod m`, therefore
computed scope `0` too -- the SAME id `enclosing_mod_scope` returns for the
sibling, truly-top-level `mod quickcheck {}` declared after `mod m`
closes. These are different scopes in real Rust (a `use` inside `mod m`
cannot see `mod m`'s own sibling `mod quickcheck` one level up without
`super::`/`crate::`), but the collision made them compute equal, producing
a false Must against the unrelated in-crate `Gen::size`.

**Fix**: use `u64::MAX` (never a real tree-sitter byte offset in any file
this program will ever index) as the "no enclosing container" sentinel,
instead of `0`. A real container's `start_byte()` can legitimately be
`0`; the sentinel now cannot collide with it.

**Verified by mutation**: reverted to `0` with the fixture above as a new
permanent regression test
(`a_mod_starting_at_byte_zero_does_not_collide_with_the_file_root_scope_sentinel`);
confirmed it fails under the reverted code, and passes under the fix.

## Why this wasn't caught in `correction-1`

`correction-1`'s own fix (stopping the ancestor walk at `block` as well as
`mod_item`) was itself correct and is unaffected by this bug -- this is a
genuinely separate, third collision source in the same function (the first
two: crate-wide vs. per-file scope in round 4/5, block vs. module scope in
`correction-1`). `correction-1`'s document restated the "can only
under-recognize, never over-recognize" invariant as settled after fixing
the second collision, without specifically stress-testing the sentinel
value itself for a collision with a real container's own position. This is
the third time this exact safety-direction claim about this function has
needed correcting; the code comment above `enclosing_mod_scope` now says so
explicitly, rather than restating the invariant as if newly and finally
settled.

## Re-measurement: all numbers hold, unchanged from `correction-1`

Binary: `/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder`,
sha256 `dd5ae79a222d2a98b42e6b9221a263ceed5dbd85d4b83f10e2deedf14c9ac17f`,
built from `777c2a7`.

- [audit-after.json](audit-after.json): **52 scored, 28 exact, 24
  conservative, 0 unsound** -- identical to `correction-1`.
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
  **0** in regex -- identical split.
- All four common gates and the final build pass, chained sequentially in
  a single background job:
  [all-gates-and-build.log](all-gates-and-build.log).

## Conclusion

Stage 3 (Rust) remains **DONE**. This is the seventh round of review
across this resolver's whole lifetime, and the fourth in which a review
found a genuine unsoundness in an immediately prior round's own fix or its
safety-direction claim. As with `correction-1`: none of these findings
changed any measured number on the three audited crates, which is a
property of this specific sample, not a guarantee the rule is now
exhaustively correct. Given this specific function (`enclosing_mod_scope`
and the `in_crate` logic it feeds) has now needed four separate corrections
across four review rounds, any future change to it should be treated with
particular suspicion and re-reviewed specifically for scope-collision and
safety-direction claims before being trusted, not assumed sound because it
compiles and passes its own tests.
