# Stage 3 TypeScript before-observation: third addendum

Follow-up to `before-observation-addendum-2.md` (`7f2eb0d`), which found
a real node-id-collision bug that silently discards a losing node's
call evidence. Further review found that document's own safety
conclusions needed stronger, more discriminating tests than it had
run. This document supplies them. **None of the results below change
`zero_classification_errors_on_audit`'s Met status.**

## 1. The `Calls` graph claim, re-tested with a proper discriminator

Addendum-2 checked `verifySolutionScenario`'s `callers` in the repro
and found the (path-collapsed) caller present -- but that test wasn't
discriminating: the SURVIVING occurrence also calls
`verifySolutionScenario`, so its caller path would appear in `callers`
regardless of whether the lost occurrence's own edge exists at all.

**Re-tested properly**: added `onlyInLost()`, called exclusively inside
the lost `it("when project is indirectly referenced by solution")`
block, and a control `onlyInSurvivor()`, called exclusively inside the
surviving occurrence (`repro-gap2/sample.ts`, scratchpad, not
committed -- see "what's not committed" below). Ran
`girder orient . --nodes crate::sample::onlyInLost --json`:

```
"callers": { "count": 1, "paths": ["crate::sample::when project is indirectly referenced by solution"] }
```

Identical result for `onlyInSurvivor`. **This is the discriminating
result**: a function called ONLY from inside the lost occurrence's body
still gets a correctly-resolved caller edge. `girder test-impact .
--quiet crate::sample::onlyInLost` also lists
`"when project is indirectly referenced by solution"` among the
impacted tests. `resolve_calls` (the separate, project-wide pass behind
`Calls` edges, distinct from `call_evidence_v1`) is confirmed NOT
vulnerable to this collision -- addendum-2's conclusion holds, now on
real evidence rather than an untested inference from a non-discriminating
check.

## 2. Rust index 89 is the SAME bug, not a different, benign one

Addendum-2 described index 89 (`serde_json/src/ser.rs:504`) as "a
legitimate, disclosed, intentional gap claim doing its job -- not a
silent node loss," distinguishing it from the TypeScript sites. That
distinction was wrong, found by actually checking the source and graph
instead of reasoning from the claim's `reason` string alone.

`grep -n "fn serialize_element\b" src/ser.rs` (real crate, fetched via
`~/.cargo/registry`) finds **two** definitions: line 491 (contains the
audited site at line 504) and line 538. The committed
`correction-2/serde_json-1.0.150-inspect.json` has exactly **one**
`crate::ser::Compound<'a, W, F>::serialize_element` Function node, span
starting at row 537 -- matching the SECOND definition. The first
definition's Function node does not exist in the graph at all. This is
mechanistically identical to the TypeScript case: a semantic-path
collision (here, two `impl` blocks providing the same trait method
name, rather than two identically-worded `it()` blocks) silently drops
the earlier occurrence's own node and all its call evidence.

**What this does and doesn't change**: the cell is still `conservative`
(sound) under Rust's own frozen scoring rule, because
`duplicate-semantic-path` is not in Rust's `NEVER_COVERS`, so the
disclosed whole-file gap claim legitimately covers the site regardless
of the per-node loss underneath it. That does not make the underlying
situation "safe either way," as the scorer's docstring previously (as
of `7550683`/`c5c5624`) claimed -- under THIS (TypeScript) file's own
`NEVER_COVERS`, which does include `duplicate-semantic-path`, the
identical situation would score `unsafe_exclusion`. Both scorers are
correct under their own frozen, already-established rules; the "safe
either way" framing was the error, now corrected directly in
`tools/dispatch_audit_scorer_typescript.py`'s own docstring (this
commit), not just in a dated addendum.

**Conclusion**: this bug is confirmed present in at least two of the
three already-measured languages (TypeScript, Rust). It was not found
in Python only because no Python borrowed site happened to hit it in
the specific sampled files (not because Python's extractor is immune --
the collision mechanism lives in shared, language-agnostic code in
`claims.rs`/graph node insertion).

## 3. No masked "must" among any language's borrowed sites -- checked directly

The dangerous case raised in review: a lost-node site whose borrowed
(enclosing) claim happens to be `must`, scoring `exact` with nobody
noticing it was never actually proven, since Rust's and Python's
scorers have no `true_target` check to catch a wrong-target Must the
way TypeScript's does.

- **Rust**: printed `true_class`/`observed_class`/`observed_reason` for
  all 24 non-exact sites (indices 3, 8, 21, 23, 24, 37, 53, 56, 59, 62,
  65, 66, 69, 74, 81, 82, 85, 89, 91, 92, 93, 94, 98, 101). **One does
  have `observed_class == "must"`: index 53**
  (`petgraph-0.6.5/tests/floyd_warshall.rs:11`, `true_class: must`,
  `cell: exact`). Checked directly rather than dismissed: the source
  (`graph.add_node(())` called ~8 times in a row, one per line) and the
  inspect data show FOUR claims within 100 bytes of the site's offset
  (342), at 305, 337, 369, 401 -- none starting exactly at 342, but
  **all four** are `proven-inherent-method-on-annotated-receiver` and
  **all four** target the identical node id, which resolves to
  `crate::graph_impl::mod::Graph<N, E, Ty, Ix>::add_node` -- the
  obviously-correct target for this test. This is the same small-delta
  scorer-anchor pattern as the other 22 benign sites (not the
  node-collision bug: no missing Function node, no lost evidence,
  every nearby occurrence of this repeated call correctly proven), just
  occurring on a site whose `true_class` happens to also be `must` --
  so its `observed_class == must` is CORRECT, not a masked false
  proof. Every other borrowed site in Rust's audit has
  `observed_class != must`.
- **Python**: same check across all 25 non-exact sites. **Zero have
  `observed_class == "must"`.**
- **TypeScript**: Girder observed `must` for 0 of the 97 scored sites
  this entire round (see `before-observation.md`: "all 46 Must-labeled
  sites score conservative"). A borrowed-must false `exact` is
  therefore structurally impossible in this round regardless of how
  many lost-node sites exist -- a stronger, simpler argument than
  auditing sites 23/91 individually, and one that covers every other
  possibly-affected site in the sample even without identifying them
  all.

No language's DONE/Met status is threatened by this finding.

## 4. Full sweep for other TypeScript lost-node sites: partially completed, limits disclosed

Re-ran the search using the criterion from review (covering claim
starts on an earlier line, reason not a disclosed gap) against all 97
committed scored sites, using the already-recorded `byte_offset` field
(no repo checkout needed for this part): **42 of 97** sites have a
covering claim that doesn't start at their own offset and isn't a
disclosed whole-file gap claim. Of these, only **1** (site 91) has a
large delta (>300 bytes) and large covering span (>1000 bytes); the
other 41, including site 23, have small deltas -- consistent with, but
not proof of, the same benign scorer-anchor pattern found throughout
Rust's and Python's audits.

**This sweep could not be completed with full confirmation.** The
pinned repo checkouts used to build the original measurement lived in a
scratchpad directory that no longer exists in this session (a new
session/scratchpad began partway through this review). Site 23 was
independently confirmed a real lost-node instance by directly grepping
its (separately re-fetched) source for duplicate `it()` descriptions --
19 occurrences of `"works with future"` in the same file -- proving the
delta/span heuristic alone is not a reliable detector (it missed 23,
whose block is short). A post-hoc duplicate-name count over the
COMMITTED (already-deduplicated) node list was tried and found
unreliable in the other direction: a real collision always reduces to
one surviving name, invisible as a "duplicate" after the fact, while
files with many same-named class methods (e.g. many classes' own
`constructor`/`close`) produce false positives when only the name's
trailing path segment is compared, not the full path.

**Honest status**: sites 23 and 91 are the only ones CONFIRMED (via
direct source inspection, independent of the scorer's own heuristics)
to be genuine lost-node instances. The other 40 flagged-but-small-delta
sites are neither confirmed nor ruled out as further instances -- a
full confirmation would require re-fetching all four pinned repos and
grepping each candidate's file for duplicate test descriptions or
duplicate method names at the relevant scope, not done here. This is
recorded as an open item for the gate-profile step, not resolved by
assumption in either direction. Per point 3 above, this open question
does not bear on `zero_classification_errors_on_audit`'s Met status
either way, since no borrowed-must case is possible in this round
regardless of how many of the 40 are eventually confirmed.

## What's not committed

`repro-gap2/sample.ts` and its `girder analyze`/`orient`/`test-impact`
output live only in this session's scratchpad
(`/tmp/claude-1000/.../scratchpad/repro-gap2/`), which does not persist
across sessions (confirmed directly: the PREVIOUS session's scratchpad,
containing the first repro used in addendum-2, was already gone when
this review began, requiring the onlyInLost/onlyInSurvivor repro to be
rebuilt from scratch). The repro is simple enough to reproduce from the
description in addendum-2 and this document; it is not being committed
into the repo itself since it exists only to demonstrate extractor
behavior on synthetic input, not as project source or a fixture other
tooling depends on.

## Still not started

The gate-profile step itself (over all 46 Must sites) has still not
been started. This document, `before-observation-addendum-2.md`, and
`before-observation-addendum.md` together are its precondition.
