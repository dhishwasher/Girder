# Stage 6 fresh comparative campaign (Girder 0.4.0, policy revision 13)

Fresh serial rerun on the frozen pins in stage6-freeze-v2.json; Girder 0.4.0 candidate versus ripwire, codebase-memory-mcp and code-review-graph. GitNexus stays RESOURCE_BLOCKED from the baseline and was not rerun.

Every precision/recall pair is shown as `precision/recall`. Base correctness uses exactly one warmed query of each kind. Operational byte and call totals include warmup, the measured warm query, and every freshness probe; failed probes remain in those totals. No tokenizer was run.

| Product | Cold setup (s) | Warm five-query total (s) | Definition | Callers P/R | Callees P/R | Impact P/R | Tests P/R | Mutation terminal | Query bytes | Calls | Peak RSS (bytes) |
|---|---:|---:|---|---|---|---|---|---|---:|---:|---:|
| codebase-memory-mcp 0.10.8 | 26.602 | 0.143 | PASS | WRONG; 0.000/0.000 | WRONG; 0.000/0.000 | WRONG; 0.000/0.000 | WRONG; unavailable/0.000 | ERROR 5, WRONG 1 | 91631 | 180 | 5767168 |
| codebase-memory-mcp 0.10.8 | 32.339 | 0.182 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 1.000/0.500 | ERROR 5, WRONG 1 | 93434 | 180 | 5767168 |
| codebase-memory-mcp 0.10.8 | 33.266 | 0.201 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 1.000/0.500 | ERROR 5, WRONG 1 | 74317 | 140 | 5767168 |
| codebase-memory-mcp 0.10.8 | 35.162 | 0.199 | PASS | WRONG; 1.000/0.500 | WRONG; unavailable/0.000 | WRONG; 1.000/0.500 | WRONG; 1.000/0.500 | ERROR 5, WRONG 1 | 92587 | 180 | 5767168 |
| code-review-graph 2.3.8 | 3.823 | 0.517 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; unavailable/0.000 | WRONG; unavailable/0.000 | WRONG 6 | 574724 | 140 | 193822720 |
| code-review-graph 2.3.8 | 5.424 | 0.514 | PASS | WRONG; 1.000/0.500 | WRONG; 0.500/1.000 | WRONG; 1.000/0.750 | WRONG; unavailable/0.000 | WRONG 6 | 872770 | 140 | 109096960 |
| code-review-graph 2.3.8 | 8.013 | 0.492 | PASS | WRONG; 1.000/0.500 | WRONG; 0.250/1.000 | WRONG; 1.000/0.250 | WRONG; unavailable/0.000 | WRONG 6 | 778664 | 140 | 109359104 |
| code-review-graph 2.3.8 | 4.206 | 0.511 | PASS | WRONG; 1.000/0.500 | WRONG; 0.000/0.000 | WRONG; 1.000/0.750 | WRONG; unavailable/0.000 | WRONG 6 | 870054 | 140 | 109625344 |
| girder 0.4.0 | 5.222 | 2.264 | WRONG | WRONG; 0.000/0.000 | WRONG; 0.000/0.000 | WRONG; 0.000/0.000 | WRONG; 0.667/1.000 | WRONG 6 | 41191 | 140 | 3014656 |
| girder 0.4.0 | 4.102 | 1.134 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 43199 | 140 | 3276800 |
| girder 0.4.0 | 7.440 | 1.735 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 42818 | 140 | 3407872 |
| girder 0.4.0 | 4.170 | 1.360 | PASS | WRONG; 1.000/0.500 | WRONG; unavailable/0.000 | WRONG; 1.000/0.500 | WRONG; 0.000/0.000 | WRONG 6 | 43161 | 140 | 3670016 |
| girder-watch 0.4.0 | 4.479 | 2.427 | WRONG | WRONG; 0.000/0.000 | WRONG; 0.000/0.000 | WRONG; 0.000/0.000 | WRONG; 0.667/1.000 | WRONG 6 | 49472 | 140 | 4169728 |
| girder-watch 0.4.0 | 2.227 | 0.644 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 51530 | 140 | 4440064 |
| girder-watch 0.4.0 | 3.203 | 0.975 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 51551 | 140 | 4702208 |
| girder-watch 0.4.0 | 2.209 | 1.566 | PASS | WRONG; 1.000/0.500 | WRONG; unavailable/0.000 | WRONG; 1.000/0.500 | WRONG; 0.000/0.000 | WRONG 6 | 51486 | 140 | 5099520 |
| ripwire 0.5.0 | 1.295 | 2.094 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 119961 | 120 | 12947456 |
| ripwire 0.5.0 | 1.321 | 3.430 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 124276 | 120 | 13529088 |
| ripwire 0.5.0 | 2.740 | 4.298 | PASS | WRONG; 1.000/0.500 | PASS; 1.000/1.000 | WRONG; 1.000/0.250 | WRONG; unavailable/0.000 | WRONG 6 | 121199 | 120 | 14327808 |
| ripwire 0.5.0 | 2.201 | 2.912 | PASS | WRONG; 1.000/0.500 | WRONG; unavailable/0.000 | WRONG; 1.000/0.500 | WRONG; 0.667/1.000 | WRONG 6 | 124090 | 120 | 16650240 |

Campaign states: codebase-memory-mcp=COMPLETE, codebase-memory-mcp=COMPLETE, codebase-memory-mcp=COMPLETE, codebase-memory-mcp=COMPLETE, code-review-graph=COMPLETE, code-review-graph=COMPLETE, code-review-graph=COMPLETE, code-review-graph=COMPLETE, girder=COMPLETE, girder=COMPLETE, girder=COMPLETE, girder=COMPLETE, girder-watch=COMPLETE, girder-watch=COMPLETE, girder-watch=COMPLETE, girder-watch=COMPLETE, ripwire=COMPLETE, ripwire=COMPLETE, ripwire=COMPLETE, ripwire=COMPLETE.

The 120 mutation summaries ended with ERROR 20, WRONG 100. Update-to-all-correct latency is unavailable wherever the terminal status was not PASS.

## Where Girder Lost

- Ripwire's base test selection recalled 1.000 versus Girder's 1.000. Ripwire selected the whole test file, so that recall came with an unrelated false positive.
- Girder normal used 140 tool calls across the recorded query attempts; Ripwire used 120.
- Girder watch took 87.123 seconds for the campaign versus 57.313 seconds for normal Girder on this tiny fixture.

## Where Girder Won

- Girder normal returned 41191 query-response bytes versus Ripwire's 119961 (65.7% fewer). Their query-status counts differed: Girder WRONG 100; Ripwire PASS 40, WRONG 60.
- Girder's base test answer had precision 0.667; Ripwire's was 0.667.

## What This Benchmark Does NOT Establish

- These results do not establish behavior on fixtures or repository scales outside the supplied campaigns.
- They do not establish universal speed, memory, context cost, or semantic coverage. They measure one low-resource Chromebook container.
- Response sizes are UTF-8 bytes actually delivered by native tools. They are not token counts or complete coding-agent session costs.
- A matching PASS/WRONG count does not mean the products returned the same wrong answers; the raw records retain each answer and oracle comparison.
- The supplied runs recorded ERROR 448 exceptional query statuses; none is converted into a win.

## Reproduction

[`../../README.md`](../../README.md) and the current [`../../policy.json`](../../policy.json) describe the protocol and its revision history. Each campaign row in `summary.json` pins its product version and commit, harness commit, policy hashes, result SHA-256, and raw archive. Recover the exact policy used by these inputs with `git show e443952bf97e8aaa3f4e99a3bd5d1c15df086276:docs/competitor-benchmark/policy.json` and verify the recorded policy SHA-256.
