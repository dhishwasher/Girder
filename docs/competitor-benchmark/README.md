# Competitive code-intelligence benchmark

This directory defines the frozen, resource-bounded comparison of Girder 0.2.6 with GitNexus 1.6.11, codebase-memory-mcp 0.10.8, code-review-graph 2.3.8, and ripwire 0.5.0. It asks whether each pinned tool retrieves exact definitions, direct calls, reverse-transitive impact, and relevant tests, and whether those answers become current after a fixed refactor sequence.

Revision 1 was invalidated before the comparative campaign. Adversarial review found a rename-staleness scoring edge case, and the first Girder response-shape probe exposed an impact phrase that selected concept search. Revision 2 fixes both, freezes the concrete prompt, and retains the invalid probe and raw response in [preflight-log.json](preflight-log.json). No score or timing uses that probe.

The benchmark is intentionally able to publish Girder losses. `oracle.json` is hand-authored from the source corpus before any competitor run. Adapters cannot read it. `corpus.json` fixes a tiny Python fixture followed by Rust, Python, TypeScript/TSX, and Go fixtures. The function-parameter call remains in the oracle so Girder's documented dynamic-dispatch miss cannot be hidden.

## Frozen execution order

1. Commit `policy.json`, `corpus.json`, `oracle.json`, the common adapter API, scoring code, resource guard, and unit tests.
2. Run only the unit tests, then complete both Girder modes end to end.
3. Complete ripwire and the entire tiny-fixture campaign before integrating another external tool.
4. Run the other pinned products serially, cleaning up and checking memory before every expensive step.
5. Generate raw artifacts, environment metadata, JSON and CSV results, and the final report without changing the frozen inputs.

The exact acquisition commands, native operations and field projections, dependency locks, hashes, timeouts, memory thresholds, byte accounting, polling rules, status rules, and scoring definitions are in [policy.json](policy.json). Mutations apply cumulatively. Definition checks include a frozen source marker, which makes the body-only edit observable without changing the call graph. Each completed query is classified as `PASS`, `WRONG`, `STALE`, `UNSUPPORTED`, `TIMEOUT`, `RESOURCE_BLOCKED`, `INSTALL_FAILED`, or `ERROR`; none of the last five categories is converted into a product win.

## Reproduction

The campaign runner and concrete adapters will be added in the next checkpoints. Until then, only the frozen-foundation tests are valid:

```sh
python3 -m unittest -v tools.competitor_benchmark.test_foundation
```

Every campaign command will set `CARGO_BUILD_JOBS=1`, `CMAKE_BUILD_PARALLEL_LEVEL=1`, `MAKEFLAGS=-j1`, `RAYON_NUM_THREADS=1`, and `npm_config_jobs=1`. Measured work runs offline after pinned acquisition. A tested supervisor checks preflight memory, samples process-tree RSS and system headroom, bounds time and output, and kills descendants. The final reproduction command will be recorded here and in the generated report after the runner itself is committed.

## Reporting boundaries

The final report will contain `Where Girder Lost`, `Where Girder Won`, and `What This Benchmark Does NOT Establish`. Costs remain attached to correctness, and response sizes are labeled bytes because no tokenizer is used. These measurements describe one constrained Chromebook and the committed fixtures; they do not establish universal performance, behavior on large repositories, or capabilities beyond the native interfaces exercised here.
