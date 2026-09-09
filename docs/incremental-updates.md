# Incremental parsing reuse and cold-analysis equivalence

The implementation reuses clean-file parsing and runs full project-wide
resolution. Its claim is limited to preserving cold-analysis resolution;
it does not improve semantic coverage.

**The recorded equality measurement passed: 113/113 comparisons matched.**
All 68 frozen mutation steps and the project integration cases passed their
regressions. The mandatory three-file Rust facade test removed the old callee
edge and parsed one file while reusing two. Workspace/platform gates remain
pending before the incremental release gate can be claimed or watching added.

Full parsing occurred in **11/113 measured updates (9.73%)**: seven exceeded
the 50% dirty-file threshold, and one each exercised configuration changes,
uncertain event mapping, ownership changes, and cache inconsistency. The
updates parsed 149 files and reused 5,149 extraction results (**97.19% reuse**).
Peak process memory increased in **71/113 comparisons**. These costs remain
part of the result even though equality passed.

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

The [observation](incremental-update-observation.json) records the single
campaign against implementation commit
`ac6fb2752ef907dc5d8a52f298b4bd80f9d1adab`, including frozen input and binary
hashes. No mutations were changed or excluded. The following rows aggregate
the five languages' three scaling mutations (15 comparisons per size):

| Owned files | Cold time, total (s) | Update time, total (s) | Cold peak RSS, max (KiB) | Update peak RSS, max (KiB) | Parsed / reused files | Full parsing |
|---:|---:|---:|---:|---:|---:|---:|
| 3 | 0.550772 | 0.020431 | 2,688 | 2,816 | 25 / 20 | 5/15 (33.33%) |
| 30 | 0.577484 | 0.027554 | 3,072 | 3,200 | 20 / 430 | 0/15 (0%) |
| 300 | 1.245402 | 0.161655 | 6,784 | 8,120 | 20 / 4,480 | 0/15 (0%) |

Across all 113 comparisons, timed cold analysis totaled 5.193717 seconds and
timed updates 0.261836 seconds. No timed update was slower than its paired
cold analysis in this run. The cold process pays first-use parser startup;
the update process already paid for its initial cold cache construction
(5.126729 seconds in total, recorded separately). The difference therefore
cannot be attributed solely to parsing reuse. Including process startup,
initial cache construction, preceding mutations, and output serialization,
the cold processes totaled 10.450484 seconds and update processes 10.780067
seconds. Maximum process peak RSS was 6,784 KiB cold and 8,120 KiB update.

Frequent full parsing is visible in the small scaling fixtures: a two-file
batch exceeds half of a three-file project. This limits reuse on those
updates. There is no speed threshold, and speed cannot excuse inequality.

Routine CI runs the Rust equivalence regressions and Python harness tests.
The recorded timing campaign runs separately with no concurrent Cargo work.
