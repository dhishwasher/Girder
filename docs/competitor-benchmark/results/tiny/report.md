# First external campaign: tiny Python adapter gate

This is the first valid external small-fixture comparison under frozen policy revision 5. The corpus marks tiny-python as adapter_smoke and excludes it from the final aggregate. The frozen function-parameter callback remains in the oracle; every product missed it in callers and reverse impact. Revision 3 is invalid because its evaluator mislabeled one wrong whole-file test answer as stale; revision 4 is incomplete because the resource sampler did not handle a normal process-exit race. Both invalid runs remain preserved.

Every precision/recall pair is shown as `precision/recall`. Base correctness uses exactly one warmed query of each kind. Operational byte and call totals include warmup, the measured warm query, and every freshness probe; failed probes remain in those totals. No tokenizer was run.

| Product | Cold setup (s) | Warm five-query total (s) | Definition | Callers P/R | Callees P/R | Impact P/R | Tests P/R | Mutation terminal | Query bytes | Calls | Peak RSS (bytes) |
|---|---:|---:|---|---|---|---|---|---|---:|---:|---:|
| girder 0.2.6 | 4.331 | 0.525 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 1.000/0.500 | WRONG 6 | 41958 | 140 | 2752512 |
| girder-watch 0.2.6 | 4.060 | 0.529 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 1.000/0.500 | WRONG 6 | 45563 | 140 | 3932160 |
| ripwire 0.5.0 | 1.060 | 1.946 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 122926 | 120 | 13291520 |

Campaign states: girder=COMPLETE, girder-watch=COMPLETE, ripwire=COMPLETE. Every base warmed definition query passed. Every base warmed direct-callee query passed.

The 18 mutation summaries ended with WRONG 18. Update-to-all-correct latency is unavailable wherever the terminal status was not PASS.

## Where Girder Lost

- Ripwire's base test selection recalled 1.000 versus Girder's 0.500. Ripwire selected the whole test file, so that recall came with an unrelated false positive.
- Girder normal used 140 tool calls across the recorded query attempts; Ripwire used 120.
- Girder watch took 57.025 seconds for the campaign versus 42.683 seconds for normal Girder on this tiny fixture.

## Where Girder Won

- Girder normal returned 41958 query-response bytes versus Ripwire's 122926 (65.9% fewer). Both had the same PASS 40, WRONG 60 query-status counts.
- Girder's base test answer had precision 1.000; Ripwire's was 0.667.

## What This Benchmark Does NOT Establish

- The tiny Python fixture is an adapter and scoring gate. Its corpus entry excludes it from the final competitive aggregate.
- These results do not establish behavior on the committed modest Rust, Python, TypeScript/TSX, or Go fixtures, or on large repositories.
- They do not establish universal speed, memory, context cost, or semantic coverage. They measure one low-resource Chromebook container.
- Response sizes are UTF-8 bytes actually delivered by native tools. They are not token counts or complete coding-agent session costs.
- A matching PASS/WRONG count does not mean the products returned the same wrong answers; the raw records retain each answer and oracle comparison.
- The result does not turn `UNSUPPORTED`, `TIMEOUT`, `RESOURCE_BLOCKED`, `INSTALL_FAILED`, or `ERROR` into a win. None occurred in the supplied runs.

## Reproduction

The frozen commands and limits are in [`../../README.md`](../../README.md) and [`../../policy.json`](../../policy.json). Each campaign row in `summary.json` pins its product version and commit, harness commit, policy hashes, result SHA-256, and raw archive.
