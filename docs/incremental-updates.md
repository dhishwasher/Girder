# Incremental parsing reuse and cold-analysis equivalence

The implementation reuses clean-file parsing and runs full project-wide
resolution. Its claim is limited to preserving cold-analysis resolution;
it does not improve semantic coverage.

**Validation is in progress.** All 68 frozen mutation steps and the project
integration cases passed. The mandatory three-file Rust facade test removed
the old callee edge, matched complete cold node/edge records, and parsed one
file while reusing two. Measurements and workspace/platform gates must also
pass before the incremental release gate can be claimed or watching added.

## Frozen contract

The [policy](incremental-update-policy.json) and
[mutation corpus](incremental-mutation-corpus.json) were committed before
implementation. Seven language fixtures contain 68 consecutive mutation
steps. Five project integration cases cover configuration, source ownership,
manifest metadata, persisted reconciliation, and normalized batches.
Scaling uses 3, 30, and 300 owned source files for Rust, Python,
TypeScript, TSX, and Go, with three mutations per dataset.

The builder accepts a batch of source replacements, creations, and
deletions. A move is a deletion followed by a creation in that batch.
It normalizes relative paths and coalesces repeated paths before changing
state; the last operation wins. Absolute paths, traversal, and unsupported
source paths are rejected without changing the cache or graph.

Updates have three stages:

1. Collect dirty paths and structural-change reasons.
2. Invalidate every source-owned node and its incident edges for resolution.
   Every source file is conservatively a resolution dependent. This accounts
   for imports, facades, receiver hints, reverse dependencies, and changes to
   the set of same-named candidates without relying on incomplete old edges.
3. Parse changed files and reuse complete extraction outputs from clean
   files. Reconstruct the source graph in the cold loader's sorted path
   order, resolve the whole project, and apply the existing persisted-graph
   reconciliation rules before replacing the complete graph.

Changed files are parsed from fresh syntax trees, as in cold analysis.
The cache retains complete extraction outputs, including unresolved
references and other evidence that is absent from graph edges. It does not
attempt to preserve incoming edges from an earlier resolution pass.

Full parsing is required for ownership/configuration changes, uncertain
event mappings, cache inconsistency, or a dirty set greater than half the
owned source files. The denominator is the larger of the before/after
owned file counts; exactly 50% does not trigger the threshold. Full
project-wide resolution with parsing reuse is the normal path, not a
fallback. Updates report dirty, parsed, reused, and invalidated files,
phase timing, and all full-rebuild reasons.

The cached project loader builds a private candidate and uses the same
source inventory, Cargo binary metadata, Go module mapping, and durable
reconciliation as cold loading. Changed persisted graph bytes force full
reconciliation so another writer's metadata cannot be hidden by an older
cached merge. The caller remains responsible for validating file stability
and publishing the complete candidate.

## Equality and measurement

After every mutation, the oracle is a fresh cold analysis of the current
sources and configuration. Comparison includes every node field, exact
path-derived IDs, complete edge records and weights, Calls, and test
relationships. Only graph allocation order is ignored. Any mismatch fails
the gate, regardless of speed.

The fixtures preserve existing cold misses, including Go function-value
callbacks. The [Go limitation](go-support.md), dynamic-dispatch failure,
and intent-search limitations remain in scope as limitations, not fixes.

The measurement worker runs the builder's cold and incremental analysis in
separate processes, with source strings loaded from fixture JSON before
timing. These timings exclude project filesystem inventory, candidate-cache
cloning, and persistence. It records elapsed time, peak RSS, parsing reuse, fallback
counts/reasons, and canonical graph digests. Mismatches retain complete
graph artifacts. The update process's peak RSS includes its initial cold
cache construction and preceding mutations. These are synthetic scaling
fixtures, not evidence of production-repository performance.

Fallback counts, their denominator and percentage, latency, and parsing
reuse will be reported with the observation. Frequent full rebuilds or
cost regressions remain visible even if equality passes. The measurement
does not impose a speed threshold or permit speed to excuse inequality.

Routine CI runs the Rust equivalence regressions and Python harness tests.
The recorded timing campaign runs separately with no concurrent Cargo work.
