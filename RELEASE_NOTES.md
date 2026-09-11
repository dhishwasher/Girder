# Unreleased benchmark work

The competitive benchmark now has a frozen, resource-bounded foundation for
comparing Girder with GitNexus, codebase-memory-mcp, code-review-graph, and
ripwire. It commits the corpus, mutation sequence, independent oracle, scoring
rules, version and artifact pins, adapter contract, resource limits, and narrow
unit tests before any comparative run. Measured results will be added here only
after each adapter and campaign checkpoint is complete; this entry makes no
competitive claim yet.

The first adapter checkpoint now covers both Girder 0.2.6 MCP modes through
the common harness. Its retained tiny-fixture validation publishes the known
dynamic-dispatch losses and keeps invalidated preflight runs separate. These
adapter checks are excluded from the later competitive aggregate.

See [the benchmark protocol](docs/competitor-benchmark/README.md).

# Girder 0.2.6

Girder 0.2.6 ships the opt-in MCP watcher that was merged after the 0.2.5
release. Start it with:

```sh
girder mcp /path/to/project --watch
# or
npx -y girder-mcp /path/to/project --watch
```

The watch-enabled process owns a validated in-memory graph generation,
coalesces filesystem events, reparses changed files, runs full project-wide
resolution, persists the complete candidate atomically, and only then
publishes it to MCP calls. Calls wait while the graph is stale or rebuilding.
The existing invocation without `--watch` keeps its prior behavior.

The frozen watcher campaign matched fresh cold analysis for all 15 initial
graphs and all 45 consecutive mutations across Rust, Python, TypeScript, TSX,
and Go. It reused 4,930 of 4,995 file extractions (98.70%). Full parsing
occurred for 5 of 45 updates (11.11%), all because the dirty set exceeded half
of the owned source files. Small repositories therefore reuse less parsing:
the three-file fixtures fell back on 5 of 15 updates.

This release claims preservation of cold-analysis resolution while parsing is
reused. It does not claim broader semantic coverage or production-repository
latency. Existing intent-search, dynamic-dispatch, and Go function-value
limitations remain.

See the [watcher design, measurements, and limitations](https://github.com/dhishwasher/Girder/blob/v0.2.6/docs/mcp-watching.md)
and the [incremental equivalence record](https://github.com/dhishwasher/Girder/blob/v0.2.6/docs/incremental-updates.md).
