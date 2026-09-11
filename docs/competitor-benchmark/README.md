# Competitive code-intelligence benchmark

This directory defines the frozen, resource-bounded comparison of Girder 0.2.6 with GitNexus 1.6.11, codebase-memory-mcp 0.10.8, code-review-graph 2.3.8, and ripwire 0.5.0. It asks whether each pinned tool retrieves exact definitions, direct calls, reverse-transitive impact, and relevant tests, and whether those answers become current after a fixed refactor sequence.

Revisions 1 and 2 were invalidated before the first external comparative campaign. Adversarial review found a rename-staleness scoring edge case, the first Girder response-shape probe exposed an impact phrase that selected concept search, and Ripwire preflight established its source and symbol-scoped test response formats. The first revision 3 external run exposed a status-only bug in prior-state comparison for whole-test-file predictions. A revision 4 rerun was interrupted by a normal `/proc` process-disappearance race that the resource sampler failed to catch. Revision 5 fixed that race and produced the separately preserved Girder/Ripwire tiny gate. Before any codebase-memory score, native preflight established that v0.10.8 uses inbound/outbound directions, finite depth 32, boolean persistence, and a local security-sensitive runtime/cache placement; revision 6 corrected that adapter mapping. Revision 7 prevented a known wrong answer from ending the wait while another answer was stale. Its first codebase-memory run then exposed first-error termination before the product's multi-second watcher could recover. Revision 8 applies the existing bounded stability window to errors and wrong answers and produced the separately preserved codebase-memory result. The first code-review-graph install exposed an unhashed extras expression in its not-yet-used dependency lock; revision 9 pins the same PyJWT version and hash with the required `crypto` extra. Successful installation and native response preflight then established its local SQLite placement, complete MCP initialization, source-bearing definition, file-scoped impact, and synchronous incremental-update mappings. Revision 10 freezes those mappings and the concrete adapter before its first score. Every invalid or interrupted run remains in [preflight-log.json](preflight-log.json) with its raw artifact. No aggregate uses them.

The benchmark is intentionally able to publish Girder losses. `oracle.json` is hand-authored from the source corpus before any competitor run. Adapters cannot read it. `corpus.json` fixes a tiny Python fixture followed by Rust, Python, TypeScript/TSX, and Go fixtures. The function-parameter call remains in the oracle so Girder's documented dynamic-dispatch miss cannot be hidden.

## Frozen execution order

1. Commit `policy.json`, `corpus.json`, `oracle.json`, the common adapter API, scoring code, resource guard, and unit tests.
2. Run only the unit tests, then complete both Girder modes end to end.
3. Complete ripwire and the entire tiny-fixture campaign before integrating another external tool.
4. Run the other pinned products serially, cleaning up and checking memory before every expensive step.
5. Generate raw artifacts, environment metadata, JSON and CSV results, and the final report without changing the frozen inputs.

The exact acquisition commands, native operations and field projections, dependency locks, hashes, timeouts, memory thresholds, byte accounting, polling rules, status rules, and scoring definitions are in [policy.json](policy.json). Mutations apply cumulatively. Definition checks include a frozen source marker, which makes the body-only edit observable without changing the call graph. Each completed query is classified as `PASS`, `WRONG`, `STALE`, `UNSUPPORTED`, `TIMEOUT`, `RESOURCE_BLOCKED`, `INSTALL_FAILED`, or `ERROR`; none of the last five categories is converted into a product win.

## Reproduction

The Girder, Ripwire, codebase-memory-mcp, and code-review-graph adapters and serial campaign runner are now available. The first valid external small-fixture campaign uses revision 5. Its [generated report](results/tiny/report.md), [JSON](results/tiny/summary.json), [CSV](results/tiny/summary.csv), and checksum-pinned [raw archives](results/tiny/artifacts.sha256) are committed. The [codebase-memory adapter checkpoint and revision 8 result](codebase-memory-adapter.md) preserve its separate policy revision, failed placements, invalid first run, and valid scored rerun. The [code-review-graph adapter checkpoint and revision 10 result](code-review-graph-adapter.md) preserve its installation and placement failures, native mappings, scored tiny campaign, and invalid direct-9p packaging attempt. The `tiny-python` corpus entry is an adapter gate and remains excluded from the final competitive aggregate.

```sh
python3 -m unittest -v tools.competitor_benchmark.test_foundation
python3 -m unittest -v tools.competitor_benchmark.test_girder_adapter
python3 -m unittest -v tools.competitor_benchmark.test_ripwire_adapter
python3 -m unittest -v tools.competitor_benchmark.test_codebase_memory_adapter
python3 -m unittest -v tools.competitor_benchmark.test_code_review_graph_adapter
python3 -m unittest -v tools.competitor_benchmark.test_campaign
python3 -m unittest -v tools.competitor_benchmark.test_reporting
```

See [the Girder adapter checkpoint](girder-adapter.md) for its earlier retained validation. The valid comparison was reproduced one product at a time with fresh work and output paths:

```sh
PYTHONPATH=. python3 -m tools.competitor_benchmark.campaign \
  --product girder --fixture tiny-python --binary /path/to/girder-0.2.6 \
  --work-root /external/work/girder-tiny --output /external/runs/girder-tiny

PYTHONPATH=. python3 -m tools.competitor_benchmark.campaign \
  --product girder-watch --fixture tiny-python --binary /path/to/girder-0.2.6 \
  --work-root /external/work/girder-watch-tiny --output /external/runs/girder-watch-tiny

PYTHONPATH=. python3 -m tools.competitor_benchmark.campaign \
  --product ripwire --fixture tiny-python --binary /path/to/ripwire-0.5.0 \
  --work-root /external/work/ripwire-tiny --output /external/runs/ripwire-tiny

PYTHONPATH=. python3 -m tools.competitor_benchmark.campaign \
  --product code-review-graph --fixture tiny-python \
  --binary /path/to/venv/bin/code-review-graph \
  --work-root /external/work/code-review-graph-tiny \
  --output /external/runs/code-review-graph-tiny
```

The generator verifies every recorded raw byte count before it writes results:

```sh
PYTHONPATH=. python3 -m tools.competitor_benchmark.reporting \
  --result raw/girder.tar.gz=/external/runs/girder-tiny/result.json \
  --result raw/girder-watch.tar.gz=/external/runs/girder-watch-tiny/result.json \
  --result raw/ripwire.tar.gz=/external/runs/ripwire-tiny/result.json \
  --output /path/to/report-output \
  --title "First external campaign: tiny Python adapter gate" \
  --scope-note "Excluded adapter gate."
```

Preserved `result.json` files retain the original absolute raw-artifact paths. To replay from an extracted archive on another machine, pass its extracted campaign directory explicitly:

```sh
PYTHONPATH=. python3 -m tools.competitor_benchmark.reporting \
  --result raw/girder.tar.gz=/tmp/campaign-girder/result.json \
  --artifact-root raw/girder.tar.gz=/tmp/campaign-girder \
  --output /tmp/replayed-report --title "Replayed result" \
  --scope-note "Archive replay."
```

Every campaign command sets `CARGO_BUILD_JOBS=1`, `CMAKE_BUILD_PARALLEL_LEVEL=1`, `MAKEFLAGS=-j1`, `RAYON_NUM_THREADS=1`, and `npm_config_jobs=1`. Measured work runs offline after pinned acquisition. A tested supervisor checks preflight memory, samples process-tree RSS and system headroom, bounds time and output, and kills descendants. The runner refuses an existing output directory or a concurrent campaign lock.

## Reporting boundaries

The final report will contain `Where Girder Lost`, `Where Girder Won`, and `What This Benchmark Does NOT Establish`. Costs remain attached to correctness, and response sizes are labeled bytes because no tokenizer is used. These measurements describe one constrained Chromebook and the committed fixtures; they do not establish universal performance, behavior on large repositories, or capabilities beyond the native interfaces exercised here.
