# The `orient` MCP tool

## Why this measurement exists

Girder's MCP server exposed six single-purpose tools (`get_source`,
`find_definition`, `search_code`, `ask_codebase`, `impacted_tests`,
`review_changes`). Answering "what am I about to touch and what does it
reach" for one starting node required chaining several of them: `get_source`
for the source, `ask_codebase` twice for callers and callees, `impacted_tests`
for coverage, and `ask_codebase` again for the impact set — each a separate
subprocess round trip (`crates/aether-app/src/project/commands/mcp.rs`'s
`run_tool` re-execs the `girder` binary per call). `orient` bundles all of
that into one call. This measures whether that bundling is actually worth
it, in round trips and bytes, against the chain it replaces.

## Precommitted method

`docs/orient-tool-policy.json`, committed before `orient` was implemented,
declares 15 orientation tasks: 12 with an exact starting symbol and 3 with
only a natural-language intent, spanning every repository already pinned by
`core-representative-v1`, `typescript-support-v1`, and `go-support-v1`
(petgraph, serde_json, regex, click, pydantic, requests, zod, type-fest,
afero, gorilla/websocket — no new repository was pinned for this
measurement). For each task the policy fixes the baseline: the exact chain
of existing-tool calls (`get_source`, `ask_codebase` × 3, `impacted_tests`,
plus `search_code` first for intent tasks) that answers the same question.

Thresholds fixed in advance: `orient` must answer in **1** round trip
(trivially true by construction — the interesting number is the baseline's,
which ranges 5–6 per task); its response bytes must be at most **1.2×** the
chained baseline's; and it must return the **same node set** the chain
returns, with a truncated section (see below) allowed to list fewer paths
than it counts as long as the count is honest and nothing listed is wrong.

`tools/orient_benchmark.py` runs each task exactly once: it extracts a
disposable copy of the pinned repository from the already-cached, already
-verified tarball, runs the baseline chain and the composite call, and
compares them. It was run to a scratch file twice while its own
correctness-comparison logic was being fixed (see Honest limits); the
version that produced the committed `docs/orient-tool-observation.json` was
run once and is not rerun.

## Observation

From `docs/orient-tool-observation.json`: **36 of 37 gated checks pass**.

- **Round trips**: 78 baseline calls across the 15 tasks (5 or 6 per task)
  collapse to 15 `orient` calls — one per task, always.
- **Bytes**: aggregate composite bytes (45,986) are *smaller* than the
  aggregate chained-baseline bytes (101,283) — a **0.45** ratio against the
  1.2 ceiling, not merely under it.
- **Correctness**: 11 of 12 exact-symbol tasks match the chained baseline's
  node set exactly (two of them, `click-command-invoke` and
  `click-parser-parse_args`, have impact/test sets in the hundreds and
  exercise `orient`'s truncation — see below). One task,
  `websocket-writejson`, fails, and the cause is disclosed rather than
  patched over: `test-impact --quiet` only prints test names for
  `"rust" | "python"` (`crates/aether-app/src/project/commands/test_impact.rs`),
  so the baseline silently drops the Go test `TestDeprecatedJSON` that
  `orient`'s `tests` section (built directly on
  `SemanticGraph::tests_for`, with no language filter) correctly reports.
  This is a real, pre-existing gap in `impacted_tests --quiet` for
  non-Rust/Python languages, not a defect in `orient`.
- **Intent tasks**: all three (`intent-shortest-path`,
  `intent-mock-filesystem-read`, `intent-serialize-value-to-string`) resolved
  to the wrong node — 0/3. This is not gated (declared
  `reported_but_not_gated` in the policy) and is consistent with
  `description-search-accuracy-v1`'s measured 41.9% top-1 accuracy on this
  codebase's own search. Byte ratio and correctness are marked "not
  applicable" for these three, since the baseline is anchored to the
  intended node and the composite to whatever it actually resolved — a
  ratio between answers to two different questions would not be honest
  either way.

## What the numbers say

Bundling the chain into one call is a clear win on both axes this policy
gates: fewer round trips always, and — in this sample — fewer bytes too,
not just an acceptable overhead. The one real correctness failure is more
informative than a clean pass would have been: it surfaces that
`impacted_tests --quiet`'s test-name filter has a blind spot for non-Rust/Python
languages that predates `orient` and that `orient`'s own `tests` section does
not share.

## Honest limits

- **Confidence is not a correctness signal here.** All three intent-task
  misses were reported with `"confidence": "high"` (scores 0.33, 0.27, 0.39
  — all above the 0.12 floor in `orient.rs`'s `LOW_CONFIDENCE_SCORE_FLOOR`).
  The floor was picked from `docs/core-gap-analysis.md`'s single recorded
  real-hit/noise split (0.17 vs 0.05–0.09); this measurement is the first
  evidence that it does not generalize to catching a wrong top-1 match, only
  to catching a near-zero one. Treat `orient`'s `confidence` field as a weak
  signal, not a guarantee, until it has been calibrated against a larger
  sample.
- **Test identity granularity differs by tool.** `impacted_tests` identifies
  tests by bare name (for feeding a test runner); `orient` identifies them by
  full semantic path. Two distinct test functions with the same name in
  different files are one baseline entry but two `orient` entries — this
  measurement's harness accounts for it by comparing name sets rather than
  counts wherever `orient` truncates, but the underlying tools still speak
  two different identity languages for tests specifically.
- **Ten repositories, one commit each.** Like every other measurement in
  this family, the direction (fewer round trips, comparable-or-fewer bytes)
  should generalize; the exact 0.45 ratio and 36/37 pass rate are this
  sample's numbers, not a portable percentage.
- **This does not re-verify graph correctness.** A task's correctness check
  only compares `orient`'s output to what the existing, separately-gated
  tools already return. It does not re-check whether those tools' underlying
  `Calls`/`IsTest` edges are themselves correct — `go-support-v1`'s own
  observation already recorded a missed edge for a function-value reference,
  which is why the two Go exact-symbol tasks' byte ratios are reported but
  not gated in the policy.
- **`--depth` beyond 2, cross-language call edges, and dynamic dispatch are
  explicit extension points** (`crates/aether-app/src/project/commands/orient.rs`),
  not measured by this corpus at all.

## Reproducing

```
cargo build -p aether-app -j1 --target-dir <target-dir>
python3 tools/orient_benchmark.py \
    --girder <target-dir>/debug/girder \
    --output docs/orient-tool-observation.json
```

Every pinned repository is extracted from the tarballs already cached
offline by `core-representative-v1`, `typescript-support-v1`, and
`go-support-v1` — no network access and no new pin required. The script
exits non-zero when a gated check fails, matching every other benchmark in
this family.

`tools/test_orient_benchmark.py` unit-tests the harness's pure logic (argv
construction, truncation-tolerant comparison, gating) with no binary and no
network, and is not affected by the recorded FAIL above. It is **not** added
to `.github/workflows/ci.yml`'s measurement-harness step, the same choice
already made for `tools/test_go_support_benchmark.py`: the committed
observation here is a known FAIL, and CI should not gate on rerunning a
measurement whose recorded result is deliberately left failing.
