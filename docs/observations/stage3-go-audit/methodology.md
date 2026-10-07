# Go real-source audit methodology (frozen before any site is labeled)

Status: **FROZEN** (the commit that sets this line is the freeze; nothing below changes after it except by a new, separately named version).
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
sampled corpus equals what Girder's default walk indexes. The extracted tree contains only
`.go` files and no `go.mod`. The audited scope is
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
labeling and drafted by the second agent, reviewed by the lead) scans the
extracted tree with regular expressions only.

- **Candidates:** an identifier or selector chain immediately followed by `(`,
  excluding `func` declaration headers, `//` and `/* */` comments, and string or
  raw-string literals.
- **Order:** files sorted by path, candidates by byte offset, then shuffled with
  `random.Random(20261006)` within each stratum.
- **Strata** (shape detected by regex alone, first match wins, in this order):
  `go_defer`; immediately-invoked function literal; generic instantiation
  (`f[T](`); **builtin** (a bare identifier in the fixed builtin-name list
  below, decidable by name); **bare_cross_file** (a bare identifier declared as a
  top-level `func NAME(` in another file of the same directory, from a regex scan
  of declarations); **bare_other** (any other bare identifier, including
  same-file functions, conversions, and local closures); **pkg_qualified**
  (`pkg.F(` where `pkg` is an import name in that file); **selector** (any
  other dotted chain). `bare_cross_file` and the builtin split are
  preregistered here because they are decidable without Girder and target the
  rule under test; they are not tuning.
- **Selection is mechanical** (see `quotas.json` and `sites-selection.json`): the
  first `quota[k]` candidates of each stratum in `--emit --seed 20261006` order
  for the initial draw; fixed continuation rounds of 40
  (11/7/8/6/2/1/1/4 across `bare_cross_file`, `bare_other`, `selector`,
  `pkg_qualified`, `go_defer`, `iife`, `generic`, `builtin`); a stratum that runs
  out sends its shortfall to `bare_cross_file`, then `bare_other`, then
  `selector`. The selected site list is generated and its hash committed before
  any labeling.
- **Quotas are set from population counts, in a separate precommit.**
  `tools/go_audit_sites.py --counts` is run first, before any labeling, and
  reports only per-stratum and per-package population counts (no labels, no
  Girder). The quotas for the initial 140 candidates are then frozen in a commit
  of their own, with a floor so that `bare_cross_file` and `bare_other` together
  supply at least 40 candidates and the dispatch-relevant strata (`selector`,
  `pkg_qualified`, `go_defer`, `iife`, `generic`) each supply at least 5 where
  the population allows, so the sample is not dominated by builtins and
  conversions. If a stratum's population is below its floor, all of it is taken
  and the shortfall is disclosed.
- **Reaching 100 actual calls (fixed rule, set now):** label candidates in the
  seeded order. If fewer than 100 are actual calls after the first 140, draw the
  next 40 from the same seeded order (strata unchanged) and repeat until at
  least 100 actual calls exist. No hand-picked replacement, and no site dropped
  after seeing a Girder answer.
- **Not-a-call sites** are labeled `not_a_call_site` and scored separately,
  never counted as calls. Categories: type conversion `T(x)`; builtin
  (`len`, `cap`, `append`, `make`, `new`, `copy`, `delete`, `close`, `panic`,
  `recover`, `print`, `println`, `min`, `max`, `clear`, `complex`, `real`,
  `imag`); an interface method specification inside `type X interface { ... }`;
  a function type literal (`func(...)` in a type, field, or parameter); a
  function declaration header; a type assertion form; and any other non-call
  match. The enumerator also blanks comments and string, rune, and raw-string
  literals before matching, and reports the excluded matches by reason.

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

**Fallback if the second agent fails.** Each batch gets at most two attempts. After
that, the lead drafts the batch. Those sites are flagged `single_agent` in the
provenance and audited at 100%, and every observation that uses the labels
reports how many sites took this path. This is fixed now so the method does not
change after the freeze.

**Bodyless functions.** `sync`, `bytes`, and `strings` contain assembly-backed and
`//go:linkname` declarations without bodies. A call to one has language truth
`must` (one static binding); label it `target_in_snapshot: false` when the body
lives outside the snapshot. G1-b requires a body, so a Girder Unknown there is a
conservative miss, not an error.

The label set is therefore drafted by one agent and audited by another; this is
disclosed in every observation that uses it.

## Matching sites to Girder claims

Girder records each call claim's site as the whole `call_expression` node span
(`span_of` on the call node in `mapper/claims.rs`), which starts at the leftmost
operand for chained calls such as `f(x).g(y)`, `d.data[i].M()`, or `v.(T).M()`.
The enumerator therefore emits `paren_byte` and `call_end_byte` for every site,
and **a site matches a claim when `claim.site.end_byte == call_end_byte`**: each
call node ends at its own closing parenthesis, so the end byte identifies exactly
one call. The Go scorer extends `tools/dispatch_audit_scorer.py` with this exact
end-byte match; its containment rule (offset within `[start_byte, end_byte)`) is
not used because nested calls such as `f(g(x))` satisfy it for more than one
claim. A site with no matching claim is reported as unmatched and scored as
`unsafe_exclusion` (a reachable call outside any classified surface), as in the
frozen Rust and Python audits.

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
   validation, quotas, and the selected initial site list, frozen together after
   advisor review.
3. The context tool and the labeling batches.
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
