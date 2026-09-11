# codebase-memory-mcp adapter checkpoint

The codebase-memory-mcp 0.10.8 adapter uses the checksum-verified official portable Linux release through its native MCP server. This checkpoint freezes the concrete v0.10.8 schemas and normalization before any scored codebase-memory campaign. It is preflight evidence, not a competitive result.

The official archive SHA-256 is `6eef49652bc0c7820f43114125044d40bf7f4d97c11b2592f6b0f6a307702325`. The extracted executable reports `codebase-memory-mcp 0.10.8` and has SHA-256 `1175645cb30560e7e47d78611cd1bcb509478eaf6d4e51f72fe18327ee9c1351`. The [adapter manifest](codebase-memory-adapter-manifest.json) pins every retained preflight artifact.

## Native mapping

The adapter starts one isolated MCP session, calls `index_repository` in `full` mode with `persistence=false`, and keeps that session open through the mutation sequence. It uses:

| Benchmark query | Native operation | Projection |
|---|---|---|
| Definition | exact paginated `search_graph`, then `get_code_snippet` | unique native file and qualified name become `path::symbol`; snippet source proves body freshness |
| Callers | paginated `trace_path`, inbound, depth 1, calls mode | direct inbound rows |
| Callees | paginated `trace_path`, outbound, depth 1, calls mode | direct outbound rows |
| Impact | paginated `trace_path`, inbound, depth 32, calls mode | every returned inbound row |
| Tests | the same inbound depth-32 trace | rows in language-native test-shaped paths |

Pagination follows the native `next` cursor with all other arguments unchanged. Qualified-name prefixes map to a repository path only when the mapping is unique. The adapter does not read `oracle.json`, add grep results, or force a reindex after mutations. The product-owned watcher and evaluator probe loop expose current, stale, rebuilding, or unchanged answers.

The base-state validation returned the exact `calculate_total` definition and source, `normalize_value` callee, `render_summary` caller, `render_summary` plus `test_render_summary` impact, and `test_render_summary` test. It did not return the function-parameter callback. That expected loss remains in the frozen oracle.

## Low-resource placement failures

The first startup placed the private cache on the ChromeOS removable mount. The pinned product rejected that ancestry during its cache-private identity check. Moving the cache and home to a mode-0700 local directory allowed its daemon to start, but executing the 293 MB binary from the removable mount still exceeded the product's fixed 30-second frontend-to-daemon admission window. The daemon exited cleanly.

Copying the already verified executable once to a mode-0700 local temporary runtime directory resolved admission. Subsequent schema, response, and adapter probes completed with about 1.7 GB `MemAvailable`; observed process-tree RSS stayed below 6 MB. Both failed placements and every successful native response remain under [preflight artifacts](preflight-artifacts/codebase-memory), and [preflight-log.json](preflight-log.json) excludes them from scores and timing aggregates.

## Reproduce the next campaign

After verifying and extracting the policy-pinned archive, make one local runtime copy and run a fresh isolated campaign:

```sh
runtime=$(mktemp -d /tmp/cbm-benchmark-runtime.XXXXXX)
chmod 700 "$runtime"
cp /external/acquisition/codebase-memory-mcp "$runtime/codebase-memory-mcp"
chmod 700 "$runtime/codebase-memory-mcp"

PYTHONPATH=. python3 -m tools.competitor_benchmark.campaign \
  --product codebase-memory-mcp \
  --fixture tiny-python \
  --binary "$runtime/codebase-memory-mcp" \
  --work-root /external/work/codebase-memory-tiny \
  --output /external/runs/codebase-memory-tiny
```

The adapter creates a fresh mode-0700 local `HOME` and `CBM_CACHE_DIR`; the external work path does not hold its live index. Run no other competitor at the same time.
