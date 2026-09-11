# Unreleased benchmark work

The competitive benchmark now has a frozen, resource-bounded foundation for
comparing Girder with GitNexus, codebase-memory-mcp, code-review-graph, and
ripwire. It commits the corpus, mutation sequence, independent oracle, scoring
rules, version and artifact pins, adapter contract, resource limits, and narrow
unit tests before any comparative run. Measured results are added here only
after each adapter and campaign checkpoint is complete.

The first adapter checkpoint now covers both Girder 0.2.6 MCP modes through
the common harness. Its retained tiny-fixture validation publishes the known
dynamic-dispatch losses and keeps invalidated preflight runs separate. These
adapter checks are excluded from the later competitive aggregate.

Ripwire 0.5.0 is the first external adapter. Its checksum-pinned CLI mapping,
body-source check, symbol-scoped test-file projection, pagination boundaries,
and raw preflight failures are frozen before the scored small-fixture run.

The first valid external small-fixture comparison is now recorded under frozen
policy revision 5. Girder normal, Girder watch, and Ripwire each completed the
tiny Python campaign with 40 PASS and 60 WRONG query records. All found the
static definition and direct callee; all missed the frozen function-parameter
callback in callers and reverse impact. Girder normal returned 41,958 query
response bytes versus Ripwire's 122,926, while using 140 calls versus 120.
Ripwire recalled both relevant tests by selecting their whole file, with one
unrelated false positive; Girder returned one relevant test and no unrelated
test in the base warm query. This fixture is an adapter gate and is excluded
from the final competitive aggregate.

The codebase-memory-mcp 0.10.8 adapter is now frozen before its first scored
run. Native preflight corrected the not-yet-measured mapping to the pinned
server's inbound/outbound trace directions, finite depth 32, boolean
persistence, cursor pagination, and JSON group shapes. Two low-resource
placement failures remain published: the removable mount could not satisfy the
product's private-cache ancestry check, and a removable-drive executable missed
its fixed daemon admission window. A local mode-0700 copy of the verified
binary with a fresh local private cache passed all five adapter operations.
Before measurement, the freshness loop was also corrected so a stable known
wrong answer cannot terminate probing while another applicable answer remains
stale. Any stale result now resets the wrong-answer stability window.

See [the benchmark protocol](docs/competitor-benchmark/README.md) and the
[generated first-external result](docs/competitor-benchmark/results/tiny/report.md).

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
