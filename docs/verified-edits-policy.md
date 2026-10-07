# Verified edits policy v1 (frozen before implementation)

Status: **FROZEN** by the commit that adds this file together with
`fixtures/verified-edits/v1/` and its manifest. Nothing here changes after that commit except by a
new, separately named version. Stage 5 of [roadmap.md](roadmap.md) extends the existing Plan Format v2
graph-addressed edits and journaled projection; it is not a separate editor.

## Scope

- A **certified step** is a Plan Format v2 step that carries a `verify` block. A step without one
  runs exactly as it does today and is reported `certified: false` ("uncertified"). v1 plans are
  untouched. `plan_version` stays 2; because the parser rejects unknown keys, an older binary rejects a
  plan that carries `verify` and fails closed.
- Certified edit kind in v1: **`replace_node` only**, on Rust and Python nodes (the languages graph
  edits support). `rename_node`, `delete_node`, `insert_into_module`, and text edits inside a certified
  step are outside certified scope and are refused as `insufficient_evidence`. This is a deliberate
  narrow scope: a hole that cannot be certified is refused, never half-supported.

## Plan fields

```json
"verify": {
  "baseline": { "<semantic path>": "<fingerprint>" },
  "delta": {
    "nodes": { "changed": ["<path>"], "added": ["<path>"], "removed": ["<path>"] },
    "edges": { "added": [["<from path>", "<to path>", "<Kind>"]], "removed": [] }
  }
}
```

All of `baseline`, `delta.nodes.{changed,added,removed}` and `delta.edges.{added,removed}` must be
present (empty arrays are explicit statements). A missing key is `insufficient_evidence`.

## Fingerprint

`fingerprint(node) = lowercase hex sha256( b"girder-node-fingerprint-v1\0" + path + b"\0" + source )`,
where `path` is the node's semantic path and `source` is its `Node.source` text exactly. Binding the
path makes two same-named overloads with **identical bodies** still differ. One function computes it
for both the authoring output and the verifier. `girder context` without `--source-only` adds a
`fingerprint` field to each node; `--source-only` output is unchanged (it returns exactly `{path,
language, source}`, a measured contract). In a multi-step plan a later step's baseline is checked
against the candidate state after earlier steps, and a successful `replace_node` leaves the node's
source exactly equal to the replacement text (otherwise `insufficient_evidence`: projection mismatch),
so an author can compute the post-edit fingerprint.

## Actual delta

Computed per step from a **cold rebuild** of the candidate workspace before and after the step's edits
(not from the incremental graph alone); if the incremental graph differs from the cold one the step is
refused as `insufficient_evidence`. All comparisons are set comparisons keyed by semantic path:

- **nodes:** over every node whose kind is **not `Module`**: `added` = path only in after; `removed` =
  path only in before; `changed` = path in both with different `Node.source` text. Positions, spans, and
  derived attributes are ignored (a body edit shifts every later span; that is not a change). A `Module`
  node's `source` is the whole file's text, so it would change on every edit; Module nodes are therefore
  excluded from the node comparison. This is sound because of the **projection-exactness** requirement
  below, which guarantees nothing outside the edited node's span changes. (Measured: type nodes such as
  a struct hold only their own declaration, so a method-body edit does not change them.)
- **edges:** the set of `(source path, target path, kind)` over **all** edge kinds (including
  `Contains`, `Inherits`, `Calls`). `added`/`removed` are differences of those sets.
- **call-evidence classes** (Must/May/Unknown) are reported, never compared.

The declared delta must equal the actual delta exactly. An identical replacement has an empty delta.

**Projection exactness.** After the step, the edited file's bytes must equal the original file with
exactly the addressed node's span replaced by the replacement text; the node's new `source` must equal
that text. Anything else is `insufficient_evidence` (projection mismatch). Together with the Module
exclusion this makes the node delta complete for `replace_node`.

**Incremental versus cold comparison.** The incremental graph (kept by the v2 pipeline) and the cold
rebuild are compared on the same canonical form as the delta: for every non-Module node, the mapping
`path -> source`; and the set of `(source path, target path, kind)` edge triples. Any difference between
the two is `insufficient_evidence`. Derived attributes and ordering are not compared.

**Baselines.** A `baseline` entry for a node the step does not edit is `insufficient_evidence`.

## Refusal categories and order

Every decision is made on the disposable candidate **before** `commit_project_writes`; a refusal is a
step failure, so `on_failure` behaves as it does today and the real source and any saved graph are
untouched. Checks run in this order and the first failing one names the category:

1. `insufficient_evidence`: an incomplete `verify` block; an edited node with no baseline; a text edit
   or an unsupported edit kind in the step; an unsupported language; a parse error or duplicate-path
   gap in a touched file after the edit; a projection mismatch; incremental and cold graphs differ.
2. `ambiguity`: the addressed path does not resolve to exactly one node (including an unknown path).
3. `wrong_overload`, **pre-apply**: the supplied baseline for the addressed node is not its current
   fingerprint but equals the current fingerprint of a different node with the same bare `name`.
4. `stale_input`: the supplied baseline matches neither the addressed node nor any same-named sibling;
   also, the on-disk bytes changed between planning and commit (the existing commit check).
5. `wrong_overload`, **post-apply**: the actual changed node set differs from the declared one and a
   changed or declared node shares its bare `name` with a node that the other set names.
6. `delta_mismatch`: the actual node delta differs from the declared one.
7. `unexpected_edge`: the actual edge delta differs from the declared one.

## Reporting

Each step in the report carries `certification` (`certified`, `refusal` category or null, and the
actual delta). Separately, the report has a **"predicted impact, not execution evidence"** section:
Must/May/Unknown counts for the changed nodes and the tests reachable before and after per class, with
bounded lists (20 each) and a truncation flag, plus tests newly or no longer reachable. Executed
`tests.impacted` results stay in their own section; predicted reachability is never presented as
execution evidence.

## Fixtures and criterion

[`fixtures/verified-edits/v1/manifest.json`](../fixtures/verified-edits/v1/manifest.json) (sha256 `92694481535ebdc1399cf4913cee756e47513afae0f278abb47092e38c2ac86c`; generated by
`tools/make_verified_edit_fixtures.py`, fingerprints computed independently of the product) lists 19
plans over four fixture projects, each with its expected outcome and refusal category:
wrong-overload by fingerprint and by delta; the correct-target counterpart; the same trio with
**identical bodies**; stale baseline; unexpected edge and its declared-edge counterpart; a declared
change that does not happen; a no-op replacement (empty delta); four insufficient-evidence cases; an
uncertified plan that still runs; a two-step plan whose second step is refused and rolled back; and a
Python correct and stale pair. The Rust fixtures compile and their tests prove which overload each
caller reaches.

**Precommitted criterion:** the committed wrong-overload fixture is refused without any change to the
source files, a saved graph, or the transaction journal (all hashed before and after), while its
correct-target counterpart succeeds; stale-input, unexpected-edge, and rollback checks pass; then all
four common gates.

## Before-observation (measured with the unmodified product)

[`observations/stage5-verified-edits/before-probe.json`](observations/stage5-verified-edits/before-probe.json),
produced by `tools/verified_edit_baseline_probe.py` against today's `girder`:
- For every plan that must commit, the declared delta equals the actual delta computed under this
  policy (nodes by `Node.source` read through `context`, Module nodes excluded, edge triples over all
  kinds). The Module node changes on every edit (its source is the whole file), which is why it is
  excluded.
- Today's pipeline, with `verify` removed, **accepts all 19 plans** (exit 0), including both
  wrong-overload plans: the existing pipeline silently edits the wrong overload. Every refusal this
  stage adds is therefore attributable to the new verification layer.
- Fingerprints are hashed outside the product, but the `source` text they hash is the one `girder context`
  reports, so they are independent of the verifier's hashing, not of the product's source extraction.

## Known limits (disclosed in advance)

- Certified scope is `replace_node` on Rust and Python only; a rename or delete cannot be certified yet.
- The delta is structural (nodes and edges). It does not prove behavior; it proves the edit changed
  exactly what the author declared.
- Call edges depend on Girder's own resolution: an edit that changes an unresolved call changes no
  `Calls` edge and so is invisible to the delta, though call-evidence changes are reported.
- Predicted test reachability is conservative and, under the whole-graph Unknown flood, is dominated by
  Unknown on real code.
