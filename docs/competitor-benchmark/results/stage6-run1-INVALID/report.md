# Stage 6 fresh comparative campaign (Girder 0.4.0)

Fresh serial rerun on the frozen pins in stage6-freeze.json; Girder 0.4.0 candidate versus ripwire, codebase-memory-mcp and code-review-graph. GitNexus stays RESOURCE_BLOCKED from the baseline and was not rerun.

Every precision/recall pair is shown as `precision/recall`. Base correctness uses exactly one warmed query of each kind. Operational byte and call totals include warmup, the measured warm query, and every freshness probe; failed probes remain in those totals. No tokenizer was run.

| Product | Cold setup (s) | Warm five-query total (s) | Definition | Callers P/R | Callees P/R | Impact P/R | Tests P/R | Mutation terminal | Query bytes | Calls | Peak RSS (bytes) |
|---|---:|---:|---|---|---|---|---|---|---:|---:|---:|
| codebase-memory-mcp 0.10.8 | 21.977 | 0.162 | PASS | WRONG; 0.000/0.000 | WRONG; 0.000/0.000 | WRONG; 0.000/0.000 | WRONG; unavailable/0.000 | ERROR 5, WRONG 1 | 84344 | 165 | 5767168 |
| codebase-memory-mcp 0.10.8 | 22.701 | 0.166 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 1.000/0.500 | ERROR 5, WRONG 1 | 86099 | 165 | 5767168 |
| codebase-memory-mcp 0.10.8 | 25.467 | 0.131 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 1.000/0.500 | ERROR 5, WRONG 1 | 93877 | 180 | 5767168 |
| codebase-memory-mcp 0.10.8 | 25.455 | 0.156 | PASS | WRONG; 1.000/0.500 | WRONG; unavailable/0.000 | WRONG; 1.000/0.500 | WRONG; 1.000/0.500 | ERROR 5, WRONG 1 | 92587 | 180 | 5767168 |
| code-review-graph 2.3.8 | 5.689 | 0.961 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; unavailable/0.000 | WRONG; unavailable/0.000 | WRONG 6 | 574724 | 140 | 109244416 |
| code-review-graph 2.3.8 | 8.560 | 1.217 | PASS | WRONG; 1.000/0.500 | WRONG; 0.500/1.000 | WRONG; 1.000/0.750 | WRONG; unavailable/0.000 | WRONG 6 | 872770 | 140 | 109494272 |
| code-review-graph 2.3.8 | 17.384 | 1.062 | PASS | WRONG; 1.000/0.500 | WRONG; 0.250/1.000 | WRONG; 1.000/0.250 | WRONG; unavailable/0.000 | WRONG 6 | 778664 | 140 | 196026368 |
| code-review-graph 2.3.8 | 6.536 | 1.129 | PASS | WRONG; 1.000/0.500 | WRONG; 0.000/0.000 | WRONG; 1.000/0.750 | WRONG; unavailable/0.000 | WRONG 6 | 870054 | 140 | 198811648 |
| girder 0.2.6 | 1.683 | 0.428 | WRONG | WRONG; 0.000/0.000 | WRONG; 0.000/0.000 | WRONG; 0.000/0.000 | WRONG; 0.667/1.000 | WRONG 6 | 41191 | 140 | 3014656 |
| girder 0.2.6 | 2.283 | 0.512 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 43199 | 140 | 3276800 |
| girder 0.2.6 | 7.118 | 0.981 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 42818 | 140 | 3407872 |
| girder 0.2.6 | 1.721 | 0.430 | PASS | WRONG; 1.000/0.500 | WRONG; unavailable/0.000 | WRONG; 1.000/0.500 | WRONG; 0.000/0.000 | WRONG 6 | 43161 | 140 | 3801088 |
| girder-watch 0.2.6 | 1.661 | 0.350 | WRONG | WRONG; 0.000/0.000 | WRONG; 0.000/0.000 | WRONG; 0.000/0.000 | WRONG; 0.667/1.000 | WRONG 6 | 49479 | 140 | 4055040 |
| girder-watch 0.2.6 | 1.773 | 0.334 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 51531 | 140 | 4431872 |
| girder-watch 0.2.6 | 1.901 | 0.433 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 51549 | 140 | 4694016 |
| girder-watch 0.2.6 | 1.830 | 0.639 | PASS | WRONG; 1.000/0.500 | WRONG; unavailable/0.000 | WRONG; 1.000/0.500 | WRONG; 0.000/0.000 | WRONG 6 | 51608 | 140 | 5074944 |
| ripwire 0.5.0 | 0.556 | 1.215 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 119961 | 120 | 13205504 |
| ripwire 0.5.0 | 0.648 | 1.304 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 124276 | 120 | 13619200 |
| ripwire 0.5.0 | 0.880 | 1.340 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.250 | WRONG; unavailable/0.000 | WRONG 6 | 121199 | 120 | 14188544 |
| ripwire 0.5.0 | 0.708 | 1.457 | PASS | WRONG; 1.000/0.500 | WRONG; unavailable/0.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 124090 | 120 | 16654336 |

Campaign states: codebase-memory-mcp=COMPLETE, codebase-memory-mcp=COMPLETE, codebase-memory-mcp=COMPLETE, codebase-memory-mcp=COMPLETE, code-review-graph=COMPLETE, code-review-graph=COMPLETE, code-review-graph=COMPLETE, code-review-graph=COMPLETE, girder=COMPLETE, girder=COMPLETE, girder=COMPLETE, girder=COMPLETE, girder-watch=COMPLETE, girder-watch=COMPLETE, girder-watch=COMPLETE, girder-watch=COMPLETE, ripwire=COMPLETE, ripwire=COMPLETE, ripwire=COMPLETE, ripwire=COMPLETE.

The 120 mutation summaries ended with ERROR 20, WRONG 100. Update-to-all-correct latency is unavailable wherever the terminal status was not PASS.

## Where Girder Lost

- Ripwire's base test selection recalled 1.000 versus Girder's 1.000. Ripwire selected the whole test file, so that recall came with an unrelated false positive.
- Girder normal used 140 tool calls across the recorded query attempts; Ripwire used 120.
- Girder watch took 34.714 seconds for the campaign versus 23.227 seconds for normal Girder on this tiny fixture.

## Where Girder Won

- Girder normal returned 41191 query-response bytes versus Ripwire's 119961 (65.7% fewer). Their query-status counts differed: Girder WRONG 100; Ripwire PASS 40, WRONG 60.
- Girder's base test answer had precision 0.667; Ripwire's was 0.667.

## What This Benchmark Does NOT Establish

- These results do not establish behavior on fixtures or repository scales outside the supplied campaigns.
- They do not establish universal speed, memory, context cost, or semantic coverage. They measure one low-resource Chromebook container.
- Response sizes are UTF-8 bytes actually delivered by native tools. They are not token counts or complete coding-agent session costs.
- A matching PASS/WRONG count does not mean the products returned the same wrong answers; the raw records retain each answer and oracle comparison.
- The supplied runs recorded ERROR 456 exceptional query statuses; none is converted into a win.

## Reproduction

[`../../README.md`](../../README.md) and the current [`../../policy.json`](../../policy.json) describe the protocol and its revision history. Each campaign row in `summary.json` pins its product version and commit, harness commit, policy hashes, result SHA-256, and raw archive. Recover the exact policy used by these inputs with `git show 37e4d5b3eba6d3cd94a797ce3703506966b92d2f:docs/competitor-benchmark/policy.json` and verify the recorded policy SHA-256.
