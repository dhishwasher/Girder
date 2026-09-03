# Description-based search accuracy

This measurement checks whether `bitcode search` / `search_code` / `get_source
--intent` — all three route through `SemanticGraph::semantic_search`
(`crates/aether-graph/src/similarity.rs`) — return the node a competent human
would expect for a natural-language description, rather than an exact node
path. It records an observed outcome, not an architectural claim, and it
**does not meet the threshold it committed to before measuring.**

## Why this measurement exists

`find_definition` with an exact symbol name is excellent. `get_source
--intent` and `search_code` — the common case, since an agent usually has a
description and not an exact semantic path — were not. Two reproductions:

- `get_source` with intent `"the function that builds the MCP discover
  response"` returned `aether-debugger`'s `Program::function` and
  `aether-app`'s `render_function` instead of `mcp::discover_result`.
- `bitcode search . "static list of MCP tool definitions with names and
  schemas"` scored every hit 0.07–0.09, indistinguishable from noise.

Reading `similarity.rs` showed why: `semantic_search` was plain Jaccard over
an unweighted token set built from `{name} {source}}` — a common token like
"function" (which appears in dozens of bodies via `NodeKind::Function`
pattern matches) counted exactly as much as a distinctive one like
"discover". It also only ever considered `NodeKind::Function` nodes. The
second reproduction turned out not to be fixable by re-weighting alone: there
is no `Const`/`Static` `NodeKind`, so the `TOOLS` static itself was never a
graph node at all. The closest a competent human could expect is the `Tool`
struct — a `NodeKind::Type` node whose fields are literally `name`,
`description`, `schema` — which the Function-only scope excluded outright.
That is a real, independently-confirmed second gap, not a restatement of the
first.

## Precommitted method

- Policy: `docs/description-search-accuracy-policy.json`, committed **before**
  any measurement, including the threshold below.
- Harness: `tools/description_search_accuracy.py` (reproducible — see below).
- Source commit: `c24016bf904e61d321eed49932570cb6458dae9e`.
- 31 (description, expected semantic path) pairs, drawn from real nodes
  across all seven crates plus Python (`tools/`, `demo-project/`,
  `sample-project/`), including both reproductions above as ordinary corpus
  entries rather than a separate gate.
- Metric: top-1 and top-5 accuracy of `bitcode search <root> "<description>"`
  against the expected path.
- Threshold, stated before measuring: **top-1 ≥ 0.75, top-5 ≥ 0.90.** The
  policy's own rationale flagged this as deliberately short of 1.0 (some
  descriptions paraphrase intent in words that never appear in the node's
  name, path, or body — a token-overlap method is not expected to win those)
  but did not anticipate how far short of it the implementation would land.

## Observation

| | top-1 | top-5 |
|---|---:|---:|
| Baseline (unweighted Jaccard, Function-only) | 16.1% (5/31) | 25.8% (8/31) |
| Fixed (see below) | **41.9% (13/31)** | **77.4% (24/31)** |
| Threshold | 75.0% | 90.0% |
| Policy result | | **FAIL** |

Full per-item baseline and per-item final results, including every returned
hit list, are in `docs/description-search-accuracy-baseline-observation.json`
and `docs/description-search-accuracy-observation.json`.

## What changed, and what each change actually did

Five changes to `semantic_search`, applied in this order, each verified
against the corpus before moving to the next (all still present; none was
reverted):

1. **IDF-weighted cosine similarity, replacing plain Jaccard**, with a 3×
   boost for tokens that also match the node's own name or semantic path.
   This alone fixed both required reproductions (`discover_result` and
   `Tool` both moved from unranked to rank 1) but left overall accuracy low
   (top-1 32.3%, top-5 61.3%) — most remaining misses were not a weighting
   problem.
2. **Widened candidate scope from `NodeKind::Function` to `Function ∪
   Type`.** Necessary for the `Tool` struct reproduction; also introduced a
   new failure class (struct field-name vocabulary winning over the correct
   function — see below).
3. **Excluded test-marked nodes** (`attr("is_test")`) from candidates. This
   repository names tests as near-full-sentence descriptions of the behavior
   they cover (e.g. `loads_a_directory_and_resolves_across_files`), which is
   exactly the shape of a natural-language query, so an unrelated but
   verbosely-named test regularly out-scored the real implementation, whose
   short identifier can only ever share a few tokens with a query. Net effect
   was small (top-1 32.3% → 35.5%) — plausible but not the dominant issue.
4. **Added plural/3rd-person "-s" stemming** (`saves` → `save`, `resolves` →
   `resolve`, `runs` → `run`). This was the single largest win (top-5 58.1% →
   80.6%): the original tokenizer did exact string comparison with no
   stemming at all, so a query written as "resolves a name" could not match
   a function literally named `resolve` — not a scoring defect, a strict
   token-equality bug.
5. **Added camelCase/PascalCase word-splitting**, matching the existing
   snake_case split. Rust type names in this codebase are PascalCase
   (`SemanticGraph`, `AnthropicProvider`, `GraphReplica`), and the tokenizer
   only ever split on `_`, so `SemanticGraph` indexed as one 13-character
   token that could never equal query words "semantic" or "graph"
   individually. This raised top-1 to 41.9% but *lowered* top-5 to 77.4% —
   splitting also gave `Type` nodes with descriptive PascalCase field-holder
   names more matchable vocabulary, and some of that vocabulary won over the
   correct function on unrelated queries (see below).

## What did not work, honestly

The threshold was not met, and further hand-tuning against this 31-item
corpus was deliberately stopped rather than continued, because the remaining
failures are not weighting problems the current lexical approach can safely
fix without overfitting to this specific corpus:

- **Cross-crate name collisions.** `resolve` is a real, differently-behaved
  function in both `aether-graph::knowledge::SemanticGraph::resolve` (the
  intended target for "resolves a partial or suffix name to the matching
  node id") and `aether-app::project::planfile::checks::graph::resolve`. Both
  are named identically; nothing in a token-overlap score can prefer one
  `resolve` over another `resolve` when the query's other words don't
  discriminate between them.
- **IDF over-rewarding a single rare token against multiple common ones.**
  "computes a content hash for a file" (expected: `hash_file`, which matches
  *two* name tokens, "hash" and "file") lost to `compute_mac` (matches only
  "compute") because "hash" and "file" are common enough in this codebase
  that their IDF is low, while "compute" is rare enough that one match
  outweighed two. This is a known pathology of raw IDF weighting, not
  specific to this repository — a real fix (e.g. capping per-token weight,
  or blending in an absolute match-count term) needs its own measurement
  against this same corpus and was out of scope for this pass.
- **Python test classes are `Type` nodes not caught by the `is_test`
  exclusion.** `is_test` is set on individual test *functions*; a
  `unittest.TestCase` subclass like `FindDuplicatesTests` carries no such
  attribute on the class node itself, so widening search to `Type` let test
  *classes* back in through the side door step 3 was meant to close.
- **The 0.75/0.90 threshold itself may simply have been too optimistic**,
  set from reasoning about the two required reproductions rather than from
  any prior data about this corpus. That is a real possibility worth naming
  rather than papering over by lowering it now that the number is in.

## Honest limits

- **31 items, hand-selected by one person for lexical variety and crate
  coverage**, not sampled by a stated mechanical rule the way
  `context-vs-read-cost`'s node list was. A different, equally reasonable
  corpus could score differently in either direction.
- **Descriptions are paraphrases written by the same person who then graded
  them**, which is the best available proxy for "what a competent human
  would expect" but is not independent human-labeled data.
- **This measures `bitcode search`'s stdout**, not `get_source --intent` or
  `search_code` over the MCP JSON-RPC transport directly — justified because
  all three call the same `semantic_search`, verified by reading
  `crates/aether-app/src/project/commands/authoring_context.rs` and
  `crates/aether-app/src/project/commands/graph.rs`.
- **The EXTENSION POINT this module has documented from the start** —
  swapping `tokenize`+lexical scoring for a real embedding model and cosine
  similarity — remains the most likely way to close the rest of this gap.
  Everything in this pass stayed inside the "lightweight, dependency-free"
  design the module committed to; it was not enough.

## Reproducing

```bash
cargo build -p aether-app
python3 tools/description_search_accuracy.py --bitcode target/debug/bitcode \
    --output docs/description-search-accuracy-observation.json
# pre-fix comparison, not gated:
python3 tools/description_search_accuracy.py --bitcode <pre-fix-binary> \
    --baseline --output docs/description-search-accuracy-baseline-observation.json
```

It exits non-zero when gated accuracy misses the threshold — it does today.
The complete per-item record, including every ranked hit list returned, is in
`docs/description-search-accuracy-observation.json`.
