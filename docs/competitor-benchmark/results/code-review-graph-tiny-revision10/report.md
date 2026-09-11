# code-review-graph tiny Python campaign under revision 10

This is the first valid scored code-review-graph campaign. It uses policy revision 10 and remains an excluded tiny-python adapter gate.

Every precision/recall pair is shown as `precision/recall`. Base correctness uses exactly one warmed query of each kind. Operational byte and call totals include warmup, the measured warm query, and every freshness probe; failed probes remain in those totals. No tokenizer was run.

| Product | Cold setup (s) | Warm five-query total (s) | Definition | Callers P/R | Callees P/R | Impact P/R | Tests P/R | Mutation terminal | Query bytes | Calls | Peak RSS (bytes) |
|---|---:|---:|---|---|---|---|---|---|---:|---:|---:|
| code-review-graph 2.3.8 | 8.427 | 0.776 | PASS | WRONG; 1.000/0.500 | WRONG; 0.500/1.000 | WRONG; 1.000/0.750 | WRONG; unavailable/0.000 | WRONG 6 | 869360 | 140 | 193798144 |

Campaign states: code-review-graph=COMPLETE. Every base warmed definition query passed.

The 6 mutation summaries ended with WRONG 6. Update-to-all-correct latency is unavailable wherever the terminal status was not PASS.

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
- The result does not turn `UNSUPPORTED`, `TIMEOUT`, `RESOURCE_BLOCKED`, `INSTALL_FAILED`, or `ERROR` into a win. None occurred in the supplied runs.

## Reproduction

[`../../README.md`](../../README.md) and the current [`../../policy.json`](../../policy.json) describe the protocol and its revision history. Each campaign row in `summary.json` pins its product version and commit, harness commit, policy hashes, result SHA-256, and raw archive. Recover the exact policy used by these inputs with `git show 2a5641ccc19ff2846d0f5a2ea37555955b31df99:docs/competitor-benchmark/policy.json` and verify the recorded policy SHA-256.
