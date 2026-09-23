# Correction to the Rust audit before-observation

Corrects `audit-scoring-summary.json` and `audit-scored-results.json`
(candidate `f3e9f56`), found on review before any resolver work began. Per
the resume contract, those files are preserved unedited; this is a new,
separate correction, and the numbers below are the current, superseding
ones. `audit-sites-labeled-v2.json`, `tools/dispatch_audit_scorer.py`, and
`audit-scored-results-v2.json` are the corrected artifacts.

## 1. Eight `must`/`may` labels were wrong under the frozen policy

`docs/call-classification-policy.md`: "External/generated code, unindexed
source, parser errors, and stale evidence are boundaries, not proof of
absence." Each crate is analyzed in isolation (`girder analyze <crate>`),
so anything defined outside that one crate's own source tree — the
standard library, a dev-dependency, or a derive-generated method from an
external crate — is unindexed by construction. True ground truth for a
call into such a target is Unknown, not Must/May, regardless of how
unambiguous the call looks syntactically.

Eight sites were mislabeled `must` or `may` with a target outside the
indexed snapshot, each verified by reading the actual import/definition,
not assumed:

| Site | Call | True target location |
| --- | --- | --- |
| 3 | `min(...)` | `std::cmp::min` |
| 9 | `.test_iter(...)` | `regex_test` dev-dependency crate |
| 46 | `quickcheck::quickcheck(...)` | `quickcheck` dev-dependency crate |
| 65 | `connecting_edges.insert(...)` | `std::collections::HashSet::insert` |
| 69 | `buf.parse()` | `std::str::FromStr::from_str` |
| 82 | `self.it.next()` | `regex-automata` dependency (`meta::FindMatches`, confirmed no `mod meta` in regex's own `src/`) |
| 94 | `self.stack.clear()` | `std::vec::Vec::clear` |
| 102 | `RawMapKey::ref_cast(...)` | the external `ref-cast` crate's derive macro |

All eight corrected to `unknown` in `audit-sites-labeled-v2.json`. This
moves all eight from `conservative` to `exact` (Girder already observed
Unknown at every one of them), which is a real change in Girder's favor —
stated explicitly, since a correction that happens to help the tool being
measured deserves the same scrutiny as one that hurts it.

## 2. The scorer's row-based matching was imprecise; rewritten byte-precise

The original scoring used scratch scripts (not committed) that matched a
site to a `CallClaim` by searching ±2 rows of the sampled line. This risks
picking the wrong claim on a line with more than one call, or missing a
claim whose `call_expression` starts on a different row than where the
regex sampler's match text appeared (a multi-line receiver chain, for
example).

`tools/dispatch_audit_scorer.py` (now committed, with
`tools/test_dispatch_audit_scorer.py`) recomputes each site's exact byte
offset from its original shape-pattern match and looks up the `CallClaim`
whose `[start_byte, end_byte)` span contains that offset — exact containment,
not proximity.

## 3. The three "no evidence found" sites are not a neutral third category

The original run filed sites 24, 59, and 89 as `no_evidence_found`, treated
as neither scored nor an error. That is not a status the frozen Stage 3
policy has room for: `unsafe_exclusion` is defined as "a reachable call
site excluded from the graph's reasoning entirely," and a call with no
covering evidence at all is exactly that unless something else in the same
file honestly discloses the gap.

Investigated properly:

- **Sites 24 and 59** are inside `quickcheck! { ... }` macro blocks.
  Confirmed their enclosing functions (`mst_undirected`,
  `graphmap_reverse_sccs`) are absent from the graph's node list entirely —
  `quickcheck!`'s macro argument is opaque to the extractor, so nothing
  inside it is indexed as a distinct function. But the byte-precise scorer
  found that both sites ARE covered by a claim: the enclosing module
  (`crate::tests::quickcheck`) itself carries an `unexpanded-macro-or-
  decorator` gap whose recorded span covers the whole macro invocation,
  including these lines. This is an honest disclosure, not a silent
  drop — an agent querying this region is told "there is unexpanded macro
  content here," even though the specific call inside cannot be pointed to.
  Both score `conservative`/`exact` (per true class), not `unsafe_exclusion`.
- **Site 89** sits inside one of *two* colliding `Compound<'a, W,
  F>::serialize_element` definitions in `serde_json/src/ser.rs` (one for
  `SerializeSeq`, one for a different trait) — confirmed two `fn
  serialize_element` bodies matching that same semantic path in the file.
  One of the two Function nodes is not present in the graph output at all
  (an upsert/path-collision effect downstream of the file-level
  `duplicate-semantic-path` gap `crates/aether-builder/src/mapper/
  claims.rs` already records for this exact file). The byte-precise scorer
  found the site IS covered: the module (`crate::ser`) carries that
  `duplicate-semantic-path` gap with a whole-file span. Same reasoning as
  above — disclosed, not silent. Scores `conservative` (true class `may`).

All three are legitimately scored, not excluded. `unsafe_exclusion` is 0,
confirmed by the byte-precise matcher, not asserted.

## 4. The root-cause claim ("100% of misses") was wrong; corrected tally

`unresolved_reason()` (`crates/aether-builder/src/mapper/claims.rs`) returns
the same string, `rust-binding-or-dispatch-unproven`, for every Rust call
site that fails proof, for whatever reason. The reason string alone proves
nothing about *why* a specific site failed — two of the original
conservative sites are bare-identifier calls that pass the identifier
filter (`min`, since corrected to `unknown`; and `n(0)`/`kosaraju_scc(...)`,
still true `must`), yet still failed, for unrelated reasons.

Corrected tally, derived from source facts per site (not from Girder's
output), over the 25 conservative cells that remain after the label
correction:

- **23 of 25 (92%)**: the callee is a method call or a path/associated
  call (`crates/aether-builder/src/mapper/claims.rs:122`,
  `function.filter(|f| f.kind() == "identifier")`) — categorically
  ineligible for Must certification regardless of actual resolvability.
  This is real and still the dominant cause.
- **2 of 25 (8%)**: identifier-shaped calls (`n(0)` at
  `petgraph/tests/graph.rs:2092`; `kosaraju_scc(...)` at
  `petgraph/tests/quickcheck.rs:547`) whose target is imported
  (`use petgraph::graph::node_index as n;` and
  `use petgraph::algo::{..., kosaraju_scc, ...};` respectively), not
  defined top-level in the calling file — the already-documented
  same-file/top-level-only limitation (Stage 1, Stage 2), not a new cause.

Both root causes were already named before this audit; what the audit adds
is a *measured proportion* on real code (92% vs. 8%), not a new discovery,
and definitely not "100% attributable to one cause."

## Corrected confusion matrix

| | Original (`f3e9f56`) | Corrected |
| --- | --- | --- |
| exact | 18 | 27 |
| conservative | 31 | 25 |
| unsafe_exclusion | 0 (3 unscored) | 0 |
| overclaim | 0 | 0 |
| scored total | 49 | 52 |

`must_or_may` recall on the audit: 0/25 exact-must-or-may... no true
must/may site scored exact (all 25 remaining true must/may sites scored
conservative) — recall is still 0%, unchanged by this correction. Zero
classification errors (0 overclaim + 0 unsafe_exclusion) is confirmed, not
merely asserted, on the corrected data.
