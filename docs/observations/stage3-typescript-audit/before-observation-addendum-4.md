# Stage 3 TypeScript before-observation: fourth addendum

Follow-up to `before-observation-addendum-3.md` (`a1561cb`). Further
review found addendum-3's safety conclusions rested on a
non-discriminating test, and identified an untested but far more
important direction: whether a source edit made only inside a lost
occurrence's body is itself detected. It is not. This is the headline
finding of this whole review thread. `zero_classification_errors_on_audit`
stays **Met** for this round's 97 scored sites regardless (see "Scope"
below) -- this is a different, more serious finding about the
extractor/graph layer generally, not about this round's scored cells.

## Headline finding: a real edit inside a lost node produces an empty `test-impact` result

Addendum-2's Calls-graph check queried `verifySolutionScenario`'s
`callers` -- not discriminating, since the surviving occurrence also
calls it. Addendum-3 fixed that with `onlyInLost`/`onlyInSurvivor`
helper functions and confirmed `orient`'s `callers` still resolves
correctly either way. That showed the FORWARD direction (call FROM the
lost body) is fine. It did not test the direction that actually
matters for this program: does an EDIT made inside the lost body get
detected at all.

**Tested directly, both languages, clean A/B repro** (committed under
`docs/observations/stage3-typescript-audit/collision-repro/`):

- **TypeScript** (`typescript-repro/`): baseline committed to a fresh
  git repo, `girder analyze`'d. Edit A: duplicate the `onlyInLost();`
  call inside the LOST `it("when project is indirectly referenced by
  solution")` occurrence's body only. Edit B (control, from the same
  baseline): duplicate `onlyInSurvivor();` inside the SURVIVING
  occurrence's body only.
  - Edit A: `girder review . --quiet` → `crate::sample` (module only,
    no function-level node). `girder test-impact . --quiet` →
    **empty. Zero tests.**
  - Edit B: `girder review . --quiet` → `crate::sample` AND
    `crate::sample::when project is indirectly referenced by solution`.
    `girder test-impact . --quiet` → all three real test names
    (`disables looking into the child project`, `when project is
    directly referenced by solution`, `when project is indirectly
    referenced by solution`).
- **Rust** (`rust-repro/`): a tiny crate, `struct S`, two traits `A`
  and `B` each declaring `fn go(&self)`, `impl A for S` and `impl B for
  S` each implementing it, each calling a distinct free function
  (`only_via_a`/`only_via_b`), two `#[test]`s (`calls_a`, `calls_b`).
  `girder inspect` confirms only ONE `crate::lib::S::go` Function node
  survives (matching `impl B`'s body -- `impl A`'s own node is gone).
  - Edit inside the LOST `impl A for S { fn go }` body (duplicate the
    `only_via_a()` call): `girder review . --quiet` → `crate::lib`
    only. `girder test-impact . --quiet` → **empty. Zero tests**, not
    even `calls_a`.
  - Control, same edit inside the surviving `impl B for S { fn go }`
    body: `review` → `crate::lib` AND `crate::lib::S::go`.
    `test-impact` → both `calls_a` and `calls_b` (the conservative
    must∪may∪unknown union, exactly as expected).

**This is a false-empty `test-impact` result on an ordinary,
real-code edit**, not a contrived edge case: it reproduces on the
first attempt in both languages, with the SAME simple "two things
share a name at the same nesting level" precondition that is already
confirmed to occur at scale in real code (19 duplicate `it()`
descriptions in one date-fns file; Rust's own `serde_json` has multiple
trait impls sharing method names like `serialize_element`/`end`/
`serialize_field` across ~7 impls per addendum-3). `CLAUDE.md` already
documents an empty-selection gap for `classified_impact(&[])` on
const/type-only edits with "no boundary notice" -- this is the SAME
observable failure (empty selection, no warning) but triggered by an
ordinary function-body edit inside a node the extractor never told
anyone it dropped. This is worse than the documented gap because there
is no way for a caller to know it applies: nothing in `review`'s or
`test-impact`'s output distinguishes "nothing changed here" from "this
function's own node was silently destroyed by a naming collision
elsewhere in the file."

## Root cause, located in source

`crates/aether-graph/src/lib.rs`, `SemanticGraph::upsert_node` (and
`upsert_projection_node`, which calls it):

```rust
pub fn upsert_node(&mut self, node: Node) -> NodeId {
    let id = node.id;
    if let Some(&idx) = self.index.get(&id) {
        self.graph[idx] = node;
    } else {
        let idx = self.graph.add_node(node);
        self.index.insert(id, idx);
    }
    id
}
```

Documented as "insert a node, or update it in place if its id already
exists" -- correct and intended for its actual use case (re-parsing the
SAME logical entity after an edit, where the old and new `Node` really
are the same thing at two points in time). The bug is not here; this
function does exactly what its contract says.

The bug is in how this gets called during extraction:
`crates/aether-builder/src/sync.rs`, `FileSync::apply()`:

```rust
for node in &out.nodes {
    graph.upsert_projection_node(node.clone());
}
```

(called from `load_file_unresolved`/`update_file`, i.e. every time a
file is parsed or re-synced). `out.nodes` -- confirmed directly by
reading `claims::annotate()` (see addendum-2) -- DOES contain both
colliding occurrences as separate entries at this point, each with its
own correctly-computed call evidence. This loop upserts them in
sequence with no collision check. The second one's upsert call
silently overwrites the first one's entire graph entry via the exact
code path quoted above. Nothing downstream of this loop ever sees the
first occurrence again.

**Not yet fixed.** Two directions, not chosen between here: (a)
disambiguate colliding paths at the source (e.g. incorporate an
impl-target or positional discriminator into the semantic path so two
distinct occurrences never compute the same id), or (b) detect the
collision at `apply()`'s call site and merge/preserve both nodes'
evidence (e.g. keep both under distinct internal indices while
`duplicate_paths` already discloses the ambiguity at the path level)
instead of silently dropping one. Fixing this is Stage-3-adjacent but
crosses language and stage boundaries (it affects Rust's already-DONE
audit too), so it is recorded on the roadmap rather than fixed
unilaterally inside this TypeScript-stage document.

## Scope: does not change any committed Met/DONE status

- TypeScript this round: 97 scored, 0 unsound cells, unaffected --
  sites 23 and 91 both score in the safe direction regardless of this
  mechanism (already established in addendum-2/3).
- Rust's DONE audit: unaffected -- index 89's cell is sound under
  Rust's own frozen `NEVER_COVERS` rule regardless of the mechanism
  (addendum-3).
- Python's DONE audit: no instance of this mechanism found in it at
  all (see sweep below).

This finding is about the underlying extractor/graph layer's behavior
on realistic edits, not about any already-scored cell in any
committed audit. It belongs on the roadmap as a cross-cutting priority
item (added in this commit), not as a retraction of any DONE status.

## Completed sweep, correcting addendum-3's "could not be completed"

Addendum-3 used a delta/span heuristic that returned 42 ambiguous
candidates and admitted it could not reliably separate real collisions
from benign scorer-anchor drift. The reliable criterion, once found:
**does the covering claim start on an earlier SOURCE LINE than the
site, excluding disclosed whole-file gap claims** (row data is present
in every committed inspect JSON's `call_evidence_v1` attribute; no
repo checkout needed). Re-ran this against all 97 TypeScript scored
sites and all of Rust's and Python's own committed DONE audits:

- **TypeScript**: exactly 2 (sites 23 and 91) -- matches the
  already-confirmed instances exactly, no others.
- **Rust**: 3 candidates. Index 89 (already confirmed, `ser.rs`'s two
  `serialize_element`s). Indices 82 and 92 checked directly against
  real source and found to be ordinary multi-line method chains
  (`self.it.next().map(...)` in `regex/string.rs`;
  `RegexSetBuilder::new(...).case_insensitive(...)...build()` in
  `tests/suite_string_set.rs`) -- the covering claim is the WHOLE
  chained call expression, the site is a later link in the SAME
  expression, not a different, unrelated scope. Not collisions.
- **Python**: zero candidates.
- Index 53 (petgraph, previously mis-explained in addendum-3 as "all
  four neighbours agree on the target"): re-checked with the row
  criterion -- its covering claim (337-355) starts on the SAME line as
  the site (`start_row: 10` → line 11, matching the site's own `line:
  11`). It is the site's own line's own call
  (`graph.add_node(())`), anchored at the receiver `graph` (5
  characters) rather than the method name -- an ordinary scorer-anchor
  offset, ordinary and uninteresting, not a collision and not evidence
  needing the "four neighbours" argument at all. Corrected here;
  addendum-3's reasoning for this one specific site was right in
  conclusion (safe) but wrong in mechanism.

This is now a complete, confident sweep, not an open item: only sites
23, 91 (TypeScript) and index 89 (Rust) are collision instances across
every already-measured site in this entire program.

## What's committed

`docs/observations/stage3-typescript-audit/collision-repro/`:
`typescript-repro/` (`sample.ts`, baseline `inspect-baseline.json`, the
two `orient` outputs, all four `review`/`test-impact` outputs for both
the lost-body and survivor-body edits) and `rust-repro/` (`Cargo.toml`,
`src/lib.rs`, the same set of outputs). Per the previous review's
instruction, these are committed rather than left in a scratchpad that
does not persist across sessions -- confirmed necessary: the FIRST
repro (addendum-2) was already gone by the time addendum-3 began, and
had to be rebuilt from its written description alone.

## Still not started

The gate-profile step (over all 46 TypeScript Must sites) has still
not been started -- now additionally blocked on relocating/re-pinning
the TypeScript corpus checkouts (the scratchpad holding them does not
persist across sessions) and, per the roadmap entry added alongside
this document, on deciding how to prioritize the node-collision fix
itself against continuing TypeScript's own measurement work.
