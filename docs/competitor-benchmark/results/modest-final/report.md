# Modest cross-language competitive benchmark

This is the final aggregate of 20 complete modest-fixture campaigns: twelve external-product campaigns under frozen revision 11 and eight corrected Girder campaigns under revision 12. The eight Girder revision 11 campaigns are preserved and excluded. Tiny-fixture adapter gates are reported separately.

`COMPLETE` means the campaign executed to the end. It does not mean its answers were correct. Base results count 20 distinct tasks per runnable product: five query kinds across four languages. Final mutation-query status counts use the last probe for 120 distinct tasks per product: five query kinds after each of six mutations across four languages. Precision/recall is micro-aggregated from the four base tasks for that query kind. Operational totals include every warmup, measured query, retry, and error response. Response sizes are UTF-8 bytes; no tokenizer was run.

## Overall comparison

| Product | Install | Cold mean (s) | Warm five-query mean (s) | Base exact PASS/20 | Definition PASS/4 | Callers P/R | Callees P/R | Impact P/R | Tests P/R | Final mutation queries | Mutation summaries | Update to all-correct | Query bytes | Calls/query record | Peak RSS (bytes) |
|---|---|---:|---:|---:|---:|---|---|---|---|---|---|---:|---:|---:|---:|
| girder 0.2.6 | PASS | 2.142 | 0.603 | 5/20 | 3/4 | 0.750/0.375 | 0.667/0.500 | 0.750/0.375 | 0.750/0.375 | PASS 36, WRONG 84 | WRONG 24 | unavailable | 168084 | 1.400 | 3407872 |
| girder-watch 0.2.6 | PASS | 2.270 | 0.582 | 5/20 | 3/4 | 0.750/0.375 | 0.667/0.500 | 0.750/0.375 | 0.750/0.375 | PASS 36, WRONG 84 | WRONG 24 | unavailable | 182885 | 1.400 | 4718592 |
| ripwire 0.5.0 | PASS | 1.030 | 1.950 | 7/20 | 4/4 | 1.000/0.500 | 1.000/0.750 | 1.000/0.438 | 0.667/0.750 | PASS 48, WRONG 72 | WRONG 24 | unavailable | 494062 | 1.200 | 16142336 |
| codebase-memory-mcp 0.10.8 | PASS | 23.491 | 0.197 | 6/20 | 4/4 | 0.750/0.375 | 0.667/0.500 | 0.750/0.375 | 1.000/0.375 | ERROR 80, PASS 6, WRONG 34 | ERROR 20, WRONG 4 | unavailable | 366959 | 1.029 | 5767168 |
| code-review-graph 2.3.8 | PASS | 6.290 | 0.851 | 5/20 | 4/4 | 1.000/0.500 | 0.333/0.750 | 1.000/0.438 | unavailable/0.000 | PASS 28, WRONG 92 | WRONG 24 | unavailable | 3187974 | 1.400 | 198860800 |
| gitnexus 1.6.11 | RESOURCE_BLOCKED | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | unavailable | 1076068352 |

No product reached a fully correct answer after any mutation. Update-to-all-correct latency is therefore unavailable for every runnable product. Codebase-memory-mcp's COMPLETE campaigns include terminal native errors after rename; those errors remain separate from wrong parseable answers.

## Per-language results

| Product | Fixture | Definition | Callers P/R | Callees P/R | Impact P/R | Tests P/R | Base status | Final mutation queries | Mutation summaries | Query bytes | Calls | Peak RSS |
|---|---|---|---|---|---|---|---|---|---|---:|---:|---:|
| girder | modest-rust | PASS | 1.000/0.500 | 1.000/1.000 | 1.000/0.500 | 1.000/0.500 | PASS 2, WRONG 3 | PASS 12, WRONG 18 | WRONG 6 | 42057 | 140 | 3014656 |
| girder | modest-python | PASS | 1.000/0.500 | 1.000/1.000 | 1.000/0.500 | 1.000/0.500 | PASS 2, WRONG 3 | PASS 12, WRONG 18 | WRONG 6 | 42438 | 140 | 2752512 |
| girder | modest-typescript-tsx | PASS | 1.000/0.500 | unavailable/0.000 | 1.000/0.500 | 0.000/0.000 | PASS 1, WRONG 4 | PASS 12, WRONG 18 | WRONG 6 | 43101 | 140 | 3407872 |
| girder | modest-go | WRONG | 0.000/0.000 | 0.000/0.000 | 0.000/0.000 | 1.000/0.500 | WRONG 5 | WRONG 30 | WRONG 6 | 40488 | 140 | 2621440 |
| girder-watch | modest-rust | PASS | 1.000/0.500 | 1.000/1.000 | 1.000/0.500 | 1.000/0.500 | PASS 2, WRONG 3 | PASS 12, WRONG 18 | WRONG 6 | 46062 | 140 | 4427776 |
| girder-watch | modest-python | PASS | 1.000/0.500 | 1.000/1.000 | 1.000/0.500 | 1.000/0.500 | PASS 2, WRONG 3 | PASS 12, WRONG 18 | WRONG 6 | 46042 | 140 | 4169728 |
| girder-watch | modest-typescript-tsx | PASS | 1.000/0.500 | unavailable/0.000 | 1.000/0.500 | 0.000/0.000 | PASS 1, WRONG 4 | PASS 12, WRONG 18 | WRONG 6 | 46713 | 140 | 4718592 |
| girder-watch | modest-go | WRONG | 0.000/0.000 | 0.000/0.000 | 0.000/0.000 | 1.000/0.500 | WRONG 5 | WRONG 30 | WRONG 6 | 44068 | 140 | 3923968 |
| ripwire | modest-rust | PASS | 1.000/0.500 | 1.000/1.000 | 1.000/0.250 | unavailable/0.000 | PASS 2, WRONG 3 | PASS 12, WRONG 18 | WRONG 6 | 122311 | 120 | 13852672 |
| ripwire | modest-python | PASS | 1.000/0.500 | 1.000/1.000 | 1.000/0.500 | 0.667/1.000 | PASS 2, WRONG 3 | PASS 12, WRONG 18 | WRONG 6 | 125476 | 120 | 13279232 |
| ripwire | modest-typescript-tsx | PASS | 1.000/0.500 | unavailable/0.000 | 1.000/0.500 | 0.667/1.000 | PASS 1, WRONG 4 | PASS 12, WRONG 18 | WRONG 6 | 125202 | 120 | 16142336 |
| ripwire | modest-go | PASS | 1.000/0.500 | 1.000/1.000 | 1.000/0.500 | 0.667/1.000 | PASS 2, WRONG 3 | PASS 12, WRONG 18 | WRONG 6 | 121073 | 120 | 12754944 |
| codebase-memory-mcp | modest-rust | PASS | 1.000/0.500 | 1.000/1.000 | 1.000/0.500 | 1.000/0.500 | PASS 2, WRONG 3 | ERROR 20, PASS 2, WRONG 8 | ERROR 5, WRONG 1 | 91512 | 175 | 5767168 |
| codebase-memory-mcp | modest-python | PASS | 1.000/0.500 | 1.000/1.000 | 1.000/0.500 | 1.000/0.500 | PASS 2, WRONG 3 | ERROR 20, PASS 2, WRONG 8 | ERROR 5, WRONG 1 | 91069 | 175 | 5767168 |
| codebase-memory-mcp | modest-typescript-tsx | PASS | 1.000/0.500 | unavailable/0.000 | 1.000/0.500 | 1.000/0.500 | PASS 1, WRONG 4 | ERROR 20, PASS 1, WRONG 9 | ERROR 5, WRONG 1 | 92667 | 180 | 5767168 |
| codebase-memory-mcp | modest-go | PASS | 0.000/0.000 | 0.000/0.000 | 0.000/0.000 | unavailable/0.000 | PASS 1, WRONG 4 | ERROR 20, PASS 1, WRONG 9 | ERROR 5, WRONG 1 | 91711 | 180 | 5636096 |
| code-review-graph | modest-rust | PASS | 1.000/0.500 | 0.250/1.000 | 1.000/0.250 | unavailable/0.000 | PASS 1, WRONG 4 | PASS 6, WRONG 24 | WRONG 6 | 802152 | 140 | 109056000 |
| code-review-graph | modest-python | PASS | 1.000/0.500 | 0.500/1.000 | 1.000/0.750 | unavailable/0.000 | PASS 1, WRONG 4 | PASS 6, WRONG 24 | WRONG 6 | 900050 | 140 | 108740608 |
| code-review-graph | modest-typescript-tsx | PASS | 1.000/0.500 | 0.000/0.000 | 1.000/0.750 | unavailable/0.000 | PASS 1, WRONG 4 | PASS 10, WRONG 20 | WRONG 6 | 895976 | 140 | 198860800 |
| code-review-graph | modest-go | PASS | 1.000/0.500 | 1.000/1.000 | unavailable/0.000 | unavailable/0.000 | PASS 2, WRONG 3 | PASS 6, WRONG 24 | WRONG 6 | 589796 | 140 | 108777472 |

## Setup friction

| Product | Outcome | Distribution/build | Network/account | Measured friction |
|---|---|---|---|---|
| girder 0.2.6 | PASS | Checksum-verified official prebuilt Linux release archive; no compiler | Network for acquisition; no API or account | Installed and ran from the external acquisition cache |
| gitnexus 1.6.11 | RESOURCE_BLOCKED | Exact npm lock; installation did not complete, so compiler need is unknown | npm registry for acquisition; no API or product account | 9p symlink EACCES, then corrected local install exceeded the frozen 1 GiB process-tree RSS cap by 2,326,528 bytes |
| codebase-memory-mcp 0.10.8 | PASS after local placement | Checksum-verified official portable Linux binary; no compiler | Network for acquisition; no API or account | Required a mode-0700 local runtime and cache; removable-mount placement failed private-cache and executable-admission checks |
| code-review-graph 2.3.8 | PASS after lock and placement corrections | Hash-locked binary-only Python wheels; no compiler fallback | Python package index for acquisition; no API or account | Fresh install took 59.308 seconds and 214,331,392 bytes peak RSS; SQLite data required a local CRG_DATA_DIR |
| ripwire 0.5.0 | PASS | Checksum-verified official prebuilt Linux release archive; no compiler | Network for acquisition; no API or account | Installed and ran from the external acquisition cache |

Each of the 20 included campaigns archived an `environment.json`. They agree on 2 CPUs, 2881343488 bytes total memory, 0 bytes swap, Python 3.11.2, and `Linux-6.6.135-09383-g1140e4f27e24-x86_64-with-glibc2.36`. Starting available memory ranged from 1717723136 to 1750597632 bytes. Every record contains the frozen single-job environment.

GitNexus is a host-specific `RESOURCE_BLOCKED` setup result. Its corrected exact-lock installation peaked at 1076068352 bytes, above the frozen 1073741824 byte cap, and was stopped. It receives no correctness, cost, freshness, or comparative score and is not counted as a Girder win.

## Where Girder Lost

- Girder normal and watch each passed 5 of 20 warmed base exact-set tasks. Ripwire passed 7 of 20 and codebase-memory-mcp passed 6 of 20.
- On Go, Girder's native response returned the correct declaration source but only the root identity `crate::calculateTotal`; without a file component, the frozen adapter could not produce the required file-qualified identity. The Go definition was therefore WRONG. Ripwire, codebase-memory-mcp, and code-review-graph passed that base definition task.
- Girder's TypeScript/TSX base test answer had precision/recall 0.000/0.000. Ripwire's was 0.667/1.000 because its whole-file selection found both relevant tests plus one unrelated test.
- Girder normal used 560 calls across its recorded query attempts, versus Ripwire's 480. Watch mode used 560 calls and more response bytes than normal mode.
- Neither Girder mode produced an all-correct mutation summary. The frozen function-parameter callback remained absent from callers, reverse impact, and test selection.

## Where Girder Won

- For exact-definition attempts where both Girder normal and Ripwire were correct throughout, the matched Rust, Python, and TypeScript/TSX pairs contained 60 query records per product. Girder returned 35764 bytes versus Ripwire's 158514 (77.4% fewer), with 120 calls each. The Go pair is excluded because Girder's file-qualified identity was wrong.
- Girder normal's measured peak process-tree RSS was 3407872 bytes across the four campaigns, lower than every successfully tested external product in this matrix. This is host- and adapter-specific.
- Girder and Girder watch returned parseable results after every mutation. Codebase-memory-mcp ended 80 of 120 terminal mutation queries with ERROR. Parseable does not imply correct: Girder still had 84 WRONG terminal queries.

## What This Benchmark Does NOT Establish

- It does not establish an overall best product. No runnable product achieved complete base or mutation correctness.
- It does not establish production-repository behavior. The corpus contains deterministic modest fixtures on one low-resource Chromebook container.
- It does not establish token savings or full coding-agent session cost. Only delivered UTF-8 tool-response bytes, calls, and wall time were measured.
- It does not establish that GitNexus would fail on a machine with more memory; GitNexus never reached semantic measurement here.
- It does not establish that Girder failed to retrieve the Go declaration source. It failed the stricter file-qualified identity requirement through this interface.
- It does not establish complete caller, test, or blast-radius correctness. The frozen dynamic-dispatch case defeated every tested product in at least one required answer.
- Total campaign bytes and times are operational counters. They are not standalone efficiency wins when correctness, retries, or error termination differ.

## Integrity and provenance

The inclusion manifest names all 20 included campaign archives, source-result hashes, policy IDs, freeze-manifest hashes, and harness commits. Every raw artifact byte count was checked before aggregation. All archives were created from local file-by-file copies, avoiding the removable 9p mount's observed direct-tar corruption. The twelve external rows retain revision 11 because their adapters and results were unaffected; Girder rows use revision 12 after the Rust identity correction. The preserved invalid archive and preflight log disclose the reason for exclusion.

## Reproduction

Run the commands in `../../README.md` one product at a time with fresh work and output paths. Then invoke `python3 -m tools.competitor_benchmark.matrix_reporting` with the 20 `--result` and matching `--artifact-root` arguments recorded in `reproduce-report.sh`. `artifacts.sha256` verifies each raw archive. Extracting those archives and running `reproduce-report.sh` regenerates `summary.json`, `summary.csv`, `aggregate.csv`, `inclusion-manifest.json`, and this report byte-for-byte.
