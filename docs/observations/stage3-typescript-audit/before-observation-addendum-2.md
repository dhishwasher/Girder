# Stage 3 TypeScript before-observation: second addendum

**This corrects `before-observation-addendum.md` (commit `c5c5624`)
section 5 rather than re-editing it.** That section concluded the two
paren-balance-flagged sites (23, 91) were explained by an established,
safe, cross-language "containment attribution" pattern shared with
Rust's and Python's own DONE audits. Direct mechanistic investigation
below found that conclusion **understated** what is actually happening:
both sites are explained by a real, previously-undisclosed extractor
bug -- a semantic-path collision silently discards an entire node's
call evidence -- that does NOT occur anywhere in Rust's or Python's
committed audits, checked precisely rather than assumed. The scored
result for both sites is still safe (see "Impact on this round" below),
so `zero_classification_errors_on_audit` stays **Met**, but the earlier
framing ("not a TypeScript-specific gap") is wrong and is withdrawn.

## The mechanism, confirmed by direct reproduction against the real binary

`crates/aether-builder/src/mapper/claims.rs::annotate()` walks every
`call_expression`/`new_expression` node in a file's parse tree
unconditionally (`walk()`, `callable()`) and gives each one its own
`CallClaim`, attributed to the nearest enclosing `Function`-kind graph
node (`owner()`), or to the file's `Module` node if none encloses it.
Reading this code directly (not assumed from behavior) shows no special
case that would ever cause a `callable` node to get NO claim -- every
site should get one.

**Reproduced directly with the actual `girder` binary** (scratchpad
`repro-gap2/sample.ts`, not committed -- a 40-line synthetic file): two
`it("when project is indirectly referenced by solution", () => {...})`
blocks, in different `describe(...)` scopes, with genuinely different
bodies. `girder inspect --json` shows only ONE
`crate::sample::when project is indirectly referenced by solution`
Function node in the final graph -- the second (later-in-file)
occurrence. The first occurrence's Function node does not exist
anywhere in the output, under any index, with any content. Its own
call evidence (the `it(...)` call's own self-referencing claim, AND
every call made inside its body) is not attached to the module, not
merged into the surviving node, not marked as a coverage gap -- it is
simply gone. The surviving occurrence's own claims look completely
normal (compare `crate::sample::when project is directly referenced by
solution`, a non-colliding sibling, whose claims list is intact and
whose first entry's span matches its own node's span exactly, the same
shape the lost occurrence's claims would have had).

Read together with `claims.rs:87`'s existing `duplicate_paths` computed
via `out.nodes.iter().any(|n| !seen.insert(n.id))` (a whole-file,
Must-proof-blocking gate that IS disclosed, tested, and already
accounted for everywhere in this program): **that check only detects
that a collision happened somewhere in the file; it says nothing about,
and does nothing to disclose, which specific node's call evidence was
silently destroyed by the collision.** The two effects are different in
kind: `duplicate_paths` degrades Must-proof eligibility file-wide (a
disclosed, understood, already-measured effect); the node-loss found
here destroys per-call evidence for the LOSING occurrence specifically,
with no disclosure at all -- not even an honest `unknown` claim for
that node's own calls.

## Both flagged sites confirmed to be real instances, by direct source inspection

- **Site 91** (`typescript-6.0.3`,
  `src/testRunner/unittests/tsserver/projectReferences.ts`): `grep -n`
  confirms `it("when project is indirectly referenced by solution", ...)`
  appears at lines 1186 and 1242 -- two genuinely different test bodies
  with the identical description string. Site 91 (line 1189) is inside
  the first (lost) occurrence. The graph's surviving
  `...projectReferences::when project is indirectly referenced by
  solution` Function node has span 53592-54656, matching the SECOND
  occurrence, not the first -- confirming which one was kept.
- **Site 23** (`date-fns-4.1.0`, `src/intlFormatDistance/test.ts`):
  `grep -n 'it("works with future"'` finds **19 occurrences** of this
  exact description string in this one file (a common pattern:
  identical assertion wording reused across many `describe(...)` blocks
  that vary only the input values). Site 23 (line 109) is inside one of
  these. Given the same mechanism confirmed above, only one of the 19
  survives as a distinct node; the rest lose their own call evidence the
  same way -- the small covering-claim span this specific site showed
  (508 bytes, versus site 91's 16,709) reflects that this particular
  `it()` body is short, not that a different mechanism is at work.

## Rust's and Python's own DONE audits checked precisely, not assumed clean

`c5c5624`'s addendum used a loose test (`claim.start_byte !=
site.byte_offset`) to count "borrowed claim" sites in Rust (24/52) and
Python (25/85), and treated the pattern as uniformly safe. That test
conflates two different things, as raised in review: a genuinely lost
node (this bug) versus the scorer's own site-offset anchor landing a
few bytes away from a claim that IS the site's real, intact evidence
(e.g. a method-call receiver prefix). Re-checked properly: for every
non-exact site in both audits, compute `delta` (site offset minus the
covering claim's own start) and the covering claim's span.

- **Rust** (23 non-exact sites excluding index 89): every one has
  `delta` in **1-476 bytes** and covering-claim span in **10-623
  bytes**. Spot-checked index 3 directly against the real source
  (`petgraph-0.6.5/benches/bellman_ford.rs:46`): the claim's own
  `start_byte` (1372) is exactly where `min(NODE_COUNT, j_from +
  neighbour_count)` begins -- Girder's evidence for this call is
  complete and correct; the scorer's own recorded site offset (1388) is
  16 bytes later, inside the argument list, a scorer-side anchoring
  detail, not a Girder gap. All 23 match this shape: small delta, small
  span, real intact claim just anchored a bit before the scorer's own
  offset.
- **Rust index 89** (the one confirmed `duplicate-semantic-path` hit,
  `serde_json/src/ser.rs:504`): delta 13558, span 63877 -- structurally
  different from the 23 above, but ALSO different from this document's
  finding: its covering claim's `reason` is literally
  `"duplicate-semantic-path"`, the DISCLOSED whole-file `coverage_gap:
  true` claim `claims.rs`'s gap loop always emits at `(0, file_length)`
  when `duplicate_paths` is true. This is a legitimate, disclosed,
  intentional gap claim doing its job -- not a silent node loss.
- **Python** (all 25 non-exact sites): every one has `delta` in **1-90
  bytes** and covering-claim span in **8-339 bytes** -- the same benign
  scorer-anchor shape as Rust's 23, none matching the large-delta,
  large-span signature this document's finding produces.
- **Swept all 97 TypeScript scored sites** with the same delta/span
  check (not just the 2 paren-flagged ones): exactly one,
  index 91, exceeds a delta>500-and-span>2000 threshold. Site 23's
  raw numbers (delta 176, span 508) fall under that threshold despite
  being confirmed the same mechanism by direct source inspection (see
  above) -- **the delta/span heuristic is not a reliable detector on
  its own**; it caught the one large instance but would miss a short
  lost `it()` block. Source-level duplicate-description grepping is the
  reliable check, not a byte-distance threshold.

**Conclusion**: this node-loss bug, as verified, is specific to files
in this TypeScript corpus with duplicate test-description strings. It
was checked directly against Rust's and Python's own committed,
DONE, gate-passed audits and does not appear in either -- their
"borrowed claim" sites are uniformly explained by benign scorer-anchor
deltas, a genuinely different and harmless phenomenon. Neither
language's DONE status is threatened by this finding.

## Impact on this round's measurement: still `Met`, checked not assumed

Both flagged sites' scored cells remain correct under the frozen
scoring rule: site 91 (`true_class: must`) scores `conservative`
(observed `unknown`, borrowed from the enclosing `describe(...)`
claim); site 23 (`true_class: unknown`) scores `exact` (observed
`unknown`). Checked specifically for the dangerous case raised in
review -- a lost-node site whose borrowed claim happens to be `must`,
which would score `exact` with nobody noticing it was never really
proven: **no such site exists in this round.** Only 91 and 23 are
confirmed lost-node sites in the 97 scored, and neither's borrowed
class is `must`. `zero_classification_errors_on_audit` stays **Met**.

## The production `Calls` graph (used by `orient`/`test-impact`) checked separately -- not affected

`call_evidence_v1` (this document's whole subject) is distinct from the
graph's resolved `Calls` edges, built by a separate, project-wide pass
(`aether-builder::sync::resolve_calls`), which is what `orient`'s
`callers`/`callees` and `test-impact` actually consume for reachability.
Checked directly against the repro: `girder orient . --nodes
crate::sample::verifySolutionScenario --json` lists
`crate::sample::when project is indirectly referenced by solution` in
`callers` -- present, not dropped. The path collision merges the two
real call sites (the surviving occurrence and the lost one) under one
caller identity rather than losing the caller entirely, because
`resolve_calls` operates on function paths/names, which are
collision-tolerant by construction (two occurrences sharing a path
simply appear once). **This program's primary reachability guarantee
(the one `test-impact`/`impacted_tests` actually rely on) is not
undermined by this finding** -- the loss is confined to the finer-grained
per-call Must/May/Unknown evidence this Stage 3 program's own
classification is built on, which is a real gap worth disclosing on its
own terms, not inflated into a claim about broader correctness it
doesn't touch.

## Correction to the "own claim present" gate-profile column proposed in `c5c5624`

That proposal defined "own claim present" as exact `start_byte`
equality with the scorer's recorded offset. Per the Rust/Python check
above, that definition would flag 47 clearly-benign anchor-delta sites
across the two languages' already-DONE audits alongside genuine losses,
and per the date-fns sweep above, a delta/span threshold on its own
would MISS a short lost block. **Corrected recommendation**: the
gate-profile's first column should instead check, for each Must site's
enclosing scope, whether its own semantic path is shared with another
node in the same file (the same `duplicate_paths` computation
`claims.rs` already does, but reported per-candidate rather than only
as a whole-file boolean) -- not a byte-distance heuristic on the scored
claim.

## Still not started

The gate-profile step itself (over all 46 Must sites) has still not
been started. This document and `c5c5624` together are its
precondition.
