# Go real-source audit methodology (frozen before any site is labeled)

Status: **drafted for review; becomes frozen only in the commit that marks it so.**
Applies the roadmap's Stage 3 audit contract to Go: at least 100 independently
audited real call sites, precommitted before any resolver change, covering direct
calls, alternate dispatch, and unknown boundaries. The dispatch rule under test
is [policy.md](policy.md).

## Corpus

The pinned Go 1.27.1 standard-library subset in
[`docs/stage3-go-corpus.json`](../../stage3-go-corpus.json) (manifest sha256
`5873656e02c6d250ff4c5a96ec6d69c44562ed03c253df048faf34229c588ce5`): 15
packages, 70 non-test `.go` files, 26,925 lines, reproduced by
`tools/go_audit_inventory.py`. The extraction filter physically removes
`*_test.go`, `testdata/`, `vendor/`, and `//go:build ignore` files, so the
sampled corpus equals what Girder's default walk indexes. The audited scope is
**non-test library source of those packages only**; test files and other
packages are out of scope and disclosed as such.

## Girder stays away from the audit tree

No `girder` command (analyze, inspect, query, context, orient, search, MCP) may
be run on the extracted audit tree, and no labeling brief may suggest one, until
the labels are committed and hash-frozen. Corpus size is judged from file and
line counts only. This is the same discipline as the dispatch corpus: ground
truth is read from source, not from the product.

## Site enumeration (independent of Girder's parser)

A plain-Python enumerator (`tools/go_audit_sites.py`, pinned by hash before
labeling) scans the extracted tree with regular expressions only.

- **Candidates:** an identifier or selector chain immediately followed by `(`,
  excluding `func` declaration headers, `//` and `/* */` comments, and string or
  raw-string literals.
- **Order:** files sorted by path, candidates by byte offset, then shuffled with
  `random.Random(20261006)` within each stratum.
- **Strata** (shape detected by regex): bare-identifier call; package-qualified
  call (`pkg.F(`, where `pkg` is an import name in that file); other
  selector call (`x.M(`); `go`/`defer` call; immediately-invoked function
  literal; generic instantiation (`f[T](`). Initial targets out of 140
  candidates: 40, 25, 40, 10, 10, 5, with the shortfall of any stratum
  redistributed in stratum order.
- **Reaching 100 actual calls (fixed rule, set now):** label candidates in the
  seeded order. If fewer than 100 are actual calls after the first 140, draw the
  next 40 from the same seeded order (strata unchanged) and repeat until at
  least 100 actual calls exist. No hand-picked replacement, and no site dropped
  after seeing a Girder answer.
- **Not-a-call sites** are labeled `not_a_call_site` and scored separately,
  never counted as calls. Categories: type conversion `T(x)`, builtin
  (`len`, `cap`, `append`, `make`, `new`, `copy`, `delete`, `close`, `panic`,
  `recover`, `print`, `println`, `min`, `max`, `clear`, `complex`, `real`,
  `imag`), a parameter or struct-field declaration, a type assertion form, and
  any other non-call match.

## Ground-truth labels

Each actual call gets `must`, `may`, or `unknown` by language semantics, using
the classification vocabulary, plus a one-line reason and the evidence lines
that decide it. A call that statically names one function declared outside the
indexed subset is labeled `must` with `target_in_snapshot: false` (language
truth), and is scored for soundness only.

**Process.**
1. A plain script (`tools/go_audit_context.py`, hash-pinned) precomputes per-site
   context: the site line, the enclosing function and its parameters, local
   declarations and short variable declarations before the site in that
   function, and the file's imports and package clause.
2. A second agent drafts labels in batches of about 20 sites, returning JSON per
   site. Its raw outputs are committed as provenance. It is told not to run
   `girder`.
3. The lead audits **100% of `must` and `may` labels and a fixed seeded share
   (30%, `random.Random(20261006)`) of `unknown` and `not_a_call_site`
   labels**, re-deriving each from source. Every disagreement is relabeled from
   source and recorded in a disagreement log; the final label is the lead's.
4. Labels, evidence, disagreement log, and raw drafts are committed and their
   hashes pinned before the first Girder run on the tree.

The label set is therefore drafted by one agent and audited by another; this is
disclosed in every observation that uses it.

## Scoring

An audit cell is **unsound** (an error) if Girder's answer asserts more
certainty than the label (a false Must, or a `may` claim with no viable
candidate set: `overclaim`), or excludes a reachable call from reasoning
(`unsafe_exclusion`). An honest Unknown for a true `must`/`may` is
**conservative**, not an error. A claimed Must must name the labeled target
(when `target_in_snapshot` is true); a Must with a wrong target is an
overclaim. Per-site answers are read from the node `call_evidence_v1`
attribute exposed by `analyze` then `inspect`, matched by byte offset, using the
committed `tools/dispatch_audit_scorer.py` conventions. Unmatched or unindexed
sites are reported, never dropped.

## Order of commits

1. Snapshot pin and inventory tool: done (`a950897`).
2. This methodology, the [policy](policy.md), its fixtures and runtime
   validation, frozen together after advisor review.
3. Enumerator and context tools, then the enumerated site list.
4. Drafted labels, audit log, frozen labels.
5. Baseline observation: the fixture contract, the unchanged 49-case corpus,
   and the audit on the current binary. Failures published as-is.
6. Resolver implementation, after-observation, then all common gates on the
   committed candidate.

## Disclosed limits

- Standard-library code is unusually uniform and low in interface dispatch
  across the audited packages; it is not a substitute for application code.
  Pinned Afero 1.11.0 and Gorilla WebSocket 1.5.3 (existing `go-support-v1`
  repositories) are a possible second stratum, to be decided by a later
  precommitted extension rather than mid-audit.
- The audit cannot exercise package-qualified resolution (`module std`), which
  the policy leaves Unknown.
- Labels were drafted by an agent and audited at the rates above; unaudited
  Unknown/not-a-call labels carry residual error risk, quantified by the
  disagreement rate on the audited share.
