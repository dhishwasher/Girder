# Girder adapter checkpoint

The Girder 0.2.6 adapter now runs both normal MCP and `mcp --watch` through the common interface. It uses the checksum-verified official Linux release binary, an isolated home and cache, the existing local license file without logging it, JSON-lines MCP, and the frozen process-tree supervisor. Every native response is retained before normalization.

This checkpoint is adapter validation on `tiny-python`, not a competitive result. The later [final modest-language aggregate](results/modest-final/report.md) reran Girder after every external adapter was frozen. The two checkpoint runs exercised cold preparation, a warm-up, one measured warm query of each kind, and all six cumulative mutations.

| Mode | Definition | Callers | Callees | Impact | Tests |
| --- | ---: | ---: | ---: | ---: | ---: |
| normal MCP | PASS, 1.000/1.000 | WRONG, 1.000/0.500 | PASS, 1.000/1.000 | WRONG, 1.000/0.500 | WRONG, 1.000/0.500 |
| MCP `--watch` | PASS, 1.000/1.000 | WRONG, 1.000/0.500 | PASS, 1.000/1.000 | WRONG, 1.000/0.500 | WRONG, 1.000/0.500 |

Each cell reports status and precision/recall for the measured base-state warm query. The misses are the frozen callback path: Girder finds `render_summary -> calculate_total`, but it does not resolve `invoke_callback`'s function parameter to `calculate_total`. The resulting callback test is absent from callers, reverse impact, and test selection. This is the documented dynamic-dispatch limitation, and the benchmark retains it as a loss.

The normal run recorded 50,470 response bytes and 140 logical native tool calls across setup, warm-up, measured queries, and readiness probes. The watch run recorded 53,681 bytes and 140 calls. Those totals are operational audit counters for different executions; they are not a token comparison or a normal-versus-watch efficiency claim. Each record's byte count matched the sizes of its referenced raw artifacts, and no license material was present.

For each mutation, definition source and callee results reached the current oracle. Overall update-to-correct latency is null because caller, impact, and test answers never reached the full dynamic-dispatch oracle. Per-query first-pass times remain in [girder-adapter-observation.json](girder-adapter-observation.json), along with exact warm timings, byte counts, call counts, status counts, peak observed RSS, result hashes, and archive hashes.

The earlier response-shape probe and the first two accounting-invalidated runs remain in [preflight-log.json](preflight-log.json). Their compressed raw artifacts are retained and excluded from all later aggregates.

## Reproduce this checkpoint

After acquiring and verifying the pinned binary described by `policy.json`, run one mode at a time with fresh paths:

```sh
PYTHONPATH=. python3 -m tools.competitor_benchmark.campaign \
  --product girder \
  --fixture tiny-python \
  --binary /path/to/girder-0.2.6 \
  --work-root /external/path/work/girder-normal \
  --output /external/path/runs/girder-normal

PYTHONPATH=. python3 -m tools.competitor_benchmark.campaign \
  --product girder-watch \
  --fixture tiny-python \
  --binary /path/to/girder-0.2.6 \
  --work-root /external/path/work/girder-watch \
  --output /external/path/runs/girder-watch
```

The runner refuses an existing output path and a concurrent campaign lock. It validates the frozen manifest before starting.
