# Core Representative Benchmark

This benchmark measures a bounded fresh-graph analysis workflow on six pinned
open-source repositories: three Rust crates and three Python source
distributions. It is a reproducible beta gate for the declared corpus and 40
preselected call-edge probes. It is not a claim of general semantic accuracy,
cold-cache performance, incremental-index performance, or representative
affected-test accuracy.

## Checked inputs

[`core-representative-corpus.json`](core-representative-corpus.json) fixes each
archive URL, compressed byte count, SHA-256 digest, exact archive root, license
files, source extensions, and complete source inventory. The harness rejects
redirects, credentials in URLs, links, special files, path traversal, duplicate
or case-colliding paths, invalid UTF-8, and any file/count/byte/digest mismatch.
Extraction occurs in a private temporary directory and never merges into an
existing checkout.

| Language | Repository | Source files | Physical lines |
|---|---|---:|---:|
| Rust | petgraph 0.6.5 | 79 | 27,797 |
| Rust | serde_json 1.0.150 | 69 | 23,116 |
| Rust | regex 1.12.4 | 22 | 11,973 |
| Python | Click 8.4.1 | 49 | 24,956 |
| Python | Pydantic 2.13.4 | 272 | 114,752 |
| Python | Requests 2.34.2 | 35 | 11,526 |

The corpus contains 526 checked source files, 7,048,034 source bytes, and
214,120 physical lines. Its 31 positive and nine negative cases are declared
before execution. Both endpoints must exist in the exported graph; missing
endpoints abort rather than becoming false negatives.

## Protocol

For every repository, the harness creates one fresh private extraction. For
every repetition against that verified extraction, it:

1. inventories every configured source before measurement;
2. deletes any prior graph, launches a private hash-verified Bit Code binary,
   and runs `bitcode analyze <checkout> --json` with
   `RAYON_NUM_THREADS=2`;
3. records bounded wall time, child peak RSS from POSIX `wait4`, analysis phase
   times, graph size, and stdout/stderr hashes;
4. exports the exact graph through `bitcode inspect <graph> --json`, checks
   node/edge counts, evaluates every declared edge, and hashes a canonical
   semantic representation; and
5. re-inventories the source tree and verifies that neither the source nor the
   private Bit Code executable changed.

Each child has a hard timeout, a combined output cap, and POSIX process-group
cleanup. Results are atomically replaced. The observation binds the source
commit and clean-worktree state; binary, manifest, policy, harness, subprocess
support, and RSS-wrapper SHA-256 digests; CPU model; memory; filesystem; kernel;
Python version; and Rayon thread count.

The source inventory necessarily reads every file before timing, and operating
system caches are not cleared. The protocol is therefore named
`fresh-graph-fresh-process-os-cache-uncontrolled`, not cold indexing.

## Precommitted beta policy

[`core-representative-beta-policy.json`](core-representative-beta-policy.json)
requires exactly five runs per repository on the fixed two-thread host
configuration. It fails unless:

- every aggregate and per-language micro/macro precision and recall value is
  `1.000000`, with zero false positives and zero false negatives;
- graph artifact digests, canonical semantic digests, node/edge counts, case
  outcomes, source inventories, and the binary remain identical;
- each of the five smaller repositories has median analyze time at most 15s,
  maximum time at most 30s, and maximum RSS at most 128 MiB;
- Pydantic has median analyze time at most 120s, maximum time at most 180s, and
  maximum RSS at most 384 MiB;
- every inspect completes within 5s, every graph is at most 32 MiB, and the sum
  of the six repository medians is at most 180s; and
- the measured source worktree is clean and all policy/tool bindings are
  present.

These are broad beta usability ceilings for the reference two-core Celeron
host, not a statistical regression budget. Internal build/similarity/save
timings are diagnostic and do not have separate thresholds.

## Reproduce

Build the exact binary with the repository's required Cargo environment, then
run the harness from the repository root:

```sh
python3 -m unittest -v tools.test_core_representative_benchmark
python3 tools/core_representative_benchmark.py \
  --bitcode /mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/bitcode \
  --offline --evaluate-policy \
  --output docs/core-representative-observation.json
```

Omit `--offline` only when the checked cache is not yet populated. A policy
failure still writes its complete observation when `--output` is supplied and
then exits unsuccessfully with the failed check ids.

## Result

The first recorded observation
([`core-representative-observation.json`](core-representative-observation.json))
**passes** the precommitted policy: `beta_pass: true`, zero failed checks.

| Repository | Median analyze | Max analyze | Max RSS | Semantic | Digests |
|---|---:|---:|---:|---|---|
| petgraph-0.6.5 | 2.0s | 2.4s | 50.6 MiB | 3 TP, 0 FP/FN | 1/1 |
| serde_json-1.0.150 | 1.6s | 1.6s | 41.7 MiB | 4 TP, 0 FP/FN | 1/1 |
| regex-1.12.4 | 0.5s | 0.6s | 19.6 MiB | 4 TP, 4 TN, 0 FP/FN | 1/1 |
| click-8.4.1 | 3.9s | 4.2s | 47.6 MiB | 4 TP, 3 TN, 0 FP/FN | 1/1 |
| pydantic-2.13.4 | 46.4s | 47.1s | 206.6 MiB | 3 TP, 1 TN, 0 FP/FN | 1/1 |
| requests-2.34.2 | 2.4s | 2.5s | 28.5 MiB | 13 TP, 1 TN, 0 FP/FN | 1/1 |

Sum of medians: 56.8s (limit 180s). Every repository has exactly one unique
artifact digest and one unique canonical semantic digest across the five
runs — the determinism gate this benchmark exists to enforce (see "Make
.aether serialization canonical" in the git history) passes cleanly.

## Claims that remain open

This gate measures fresh graph creation and exact success on 40 curated call
edges. It does not close the following evidence gaps:

- genuine cold-cache indexing;
- one-file incremental latency and stale-edge removal on the representative
  corpus;
- framework-discovered representative test inventories, dynamic affected-test
  precision/recall, and impact-query latency (the closest evidence is
  `docs/core-representative-mutations.md`'s single declared mutation, not a
  representative sweep);
- randomly or independently sampled semantic edges beyond the declared cases;
  or
- portability of the performance ceilings to other host/storage classes.

Concurrent analysis isolation, bounded product Git/test subprocesses, and
fault-injected recovery are closed separately — see `core-gap-analysis.md`.
