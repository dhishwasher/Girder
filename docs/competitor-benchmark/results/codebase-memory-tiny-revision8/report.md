# codebase-memory-mcp tiny Python campaign under revision 8

This is the first valid scored codebase-memory-mcp campaign. It uses policy revision 8 and remains an excluded tiny-python adapter gate. Revision 7 stopped on the first post-rename error and is retained as invalid; revision 8 used the already frozen three-identical-probe and two-second stability bounds for both wrong answers and errors.

Every precision/recall pair is shown as `precision/recall`. Base correctness uses exactly one warmed query of each kind. Operational byte and call totals include warmup, the measured warm query, and every freshness probe; failed probes remain in those totals. No tokenizer was run.

| Product | Cold setup (s) | Warm five-query total (s) | Definition | Callers P/R | Callees P/R | Impact P/R | Tests P/R | Mutation terminal | Query bytes | Calls | Peak RSS (bytes) |
|---|---:|---:|---|---|---|---|---|---|---:|---:|---:|
| codebase-memory-mcp 0.10.8 | 21.930 | 0.234 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 1.000/0.500 | ERROR 5, WRONG 1 | 88414 | 170 | 5767168 |

Campaign states: codebase-memory-mcp=COMPLETE. Every base warmed definition query passed. Every base warmed direct-callee query passed.

The 6 mutation summaries ended with ERROR 5, WRONG 1. Update-to-all-correct latency is unavailable wherever the terminal status was not PASS.

## Where Girder Lost

- No Girder comparison pair was supplied.

## Where Girder Won

- No Girder comparison pair was supplied.

## What This Benchmark Does NOT Establish

- The tiny Python fixture is an adapter and scoring gate. Its corpus entry excludes it from the final competitive aggregate.
- These results do not establish behavior on the committed modest Rust, Python, TypeScript/TSX, or Go fixtures, or on large repositories.
- They do not establish universal speed, memory, context cost, or semantic coverage. They measure one low-resource Chromebook container.
- Response sizes are UTF-8 bytes actually delivered by native tools. They are not token counts or complete coding-agent session costs.
- A matching PASS/WRONG count does not mean the products returned the same wrong answers; the raw records retain each answer and oracle comparison.
- The supplied runs recorded ERROR 112 exceptional query statuses; none is converted into a win.

## Reproduction

[`../../README.md`](../../README.md) and the current [`../../policy.json`](../../policy.json) describe the protocol and its revision history. Each campaign row in `summary.json` pins its product version and commit, harness commit, policy hashes, result SHA-256, and raw archive. Recover the exact policy used by these inputs with `git show a7b652819f37115742601e9dc1d5d58702bde34b:docs/competitor-benchmark/policy.json` and verify the recorded policy SHA-256.
