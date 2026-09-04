# Core Representative Benchmark

This benchmark measures a bounded fresh-graph analysis workflow on six pinned
open-source repositories: three Rust crates and three Python source
distributions. It is a reproducible beta gate for the declared corpus and 43
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

Three declared cases were added after gap 22 (docs/core-gap-analysis.md item
22) found that a declared-case benchmark reading 1.0 says nothing about a
call shape the declared cases never exercise: `regexset-new-chained-builder-call`
and `from-slice-nested-argument-call` pin the two real chained/nested-argument
shapes gap 22 fixed in Rust, and `getchar-not-testing-isolation-mock` is a
declared-negative case pinning the real, still-open Python-side local/global
name-shadow misattribution gap 22 explicitly left unfixed (see gap 23). All
three are real code in the pinned archives, not synthetic fixtures.

| Language | Repository | Source files | Physical lines |
|---|---|---:|---:|
| Rust | petgraph 0.6.5 | 79 | 27,797 |
| Rust | serde_json 1.0.150 | 69 | 23,116 |
| Rust | regex 1.12.4 | 22 | 11,973 |
| Python | Click 8.4.1 | 49 | 24,956 |
| Python | Pydantic 2.13.4 | 272 | 114,752 |
| Python | Requests 2.34.2 | 35 | 11,526 |

The corpus contains 526 checked source files, 7,048,034 source bytes, and
214,120 physical lines. Its 33 positive and 10 negative cases are declared
before execution. Both endpoints must exist in the exported graph; missing
endpoints abort rather than becoming false negatives.

## Protocol

For every repository, the harness creates one fresh private extraction. For
every repetition against that verified extraction, it:

1. inventories every configured source before measurement;
2. deletes any prior graph, launches a private hash-verified Girder binary,
   and runs `girder analyze <checkout> --json` with
   `RAYON_NUM_THREADS=2`;
3. records bounded wall time, child peak RSS from POSIX `wait4`, analysis phase
   times, graph size, and stdout/stderr hashes;
4. exports the exact graph through `girder inspect <graph> --json`, checks
   node/edge counts, evaluates every declared edge, and hashes a canonical
   semantic representation; and
5. re-inventories the source tree and verifies that neither the source nor the
   private Girder executable changed.

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
  --girder /mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder \
  --offline --evaluate-policy \
  --output docs/core-representative-observation.json
```

Omit `--offline` only when the checked cache is not yet populated. A policy
failure still writes its complete observation when `--output` is supplied and
then exits unsuccessfully with the failed check ids.

## Result

The corpus's first observation, before gap 22 (docs/core-gap-analysis.md)
added the three cases below, **passed** the precommitted policy on 40
declared cases: `beta_pass: true`, zero failed checks, 1.000/1.000
precision and recall throughout. It did not catch gap 22's chained-call,
nested-argument, or Python local-shadow-misattribution defects, because no
declared case exercised any of those shapes — the same failure mode gap 11
established first. Gap 22 added three declared cases from real code already
inside the pinned archives (not synthetic fixtures) to close that blind
spot, and the corpus was re-measured
([`core-representative-observation.json`](core-representative-observation.json)):
`beta_pass: false`, driven entirely by one deliberately-declared, genuinely
unfixed Python case — not tuned back to 1.0.

| Repository | Median analyze | Max analyze | Max RSS | Semantic | Digests |
|---|---:|---:|---:|---|---|
| petgraph-0.6.5 | 1.8s | 2.6s | 48.2 MiB | 3 TP, 0 FP/FN | 1/1 |
| serde_json-1.0.150 | 1.3s | 1.3s | 38.8 MiB | 5 TP, 0 FP/FN | 1/1 |
| regex-1.12.4 | 0.5s | 0.5s | 17.3 MiB | 5 TP, 4 TN, 0 FP/FN | 1/1 |
| click-8.4.1 | 3.1s | 3.2s | 44.8 MiB | 4 TP, 3 TN, **1 FP**, 0 FN | 1/1 |
| pydantic-2.13.4 | 38.0s | 40.0s | 201.8 MiB | 3 TP, 1 TN, 0 FP/FN | 1/1 |
| requests-2.34.2 | 2.0s | 2.2s | 25.9 MiB | 13 TP, 1 TN, 0 FP/FN | 1/1 |

`serde_json-1.0.150` (was 4 TP) and `regex-1.12.4` (was 4 TP) each gained one
true positive: `from-slice-nested-argument-call` and
`regexset-new-chained-builder-call`, both real chained/nested-argument call
sites gap 22 fixed, both now resolving correctly — the fix holds on
previously-uncurated third-party code, not only the unit tests written
against it. `click-8.4.1`'s new false positive is
`getchar-not-testing-isolation-mock`: `click.termui.getchar()`'s reassigned
`_getchar` global gets wrongly linked to
`click.testing.CliRunner.isolation`'s unrelated, same-named nested mock —
a real instance of the Python-side misattribution gap 22 left open (gap 23),
declared rather than left silent.

By language: Rust is 13 TP, 4 TN, 0 FP, 0 FN — **1.000/1.000**, unchanged.
Python is 20 TP, 5 TN, **1 FP**, 0 FN — micro precision 0.952381, macro
precision 0.933333, recall still 1.000/1.000. Aggregate: 33 TP, 9 TN, 1 FP,
0 FN — micro precision 0.970588, macro precision 0.966667, recall
1.000/1.000. Sum of medians: 46.6s (limit 180s), inside every performance
ceiling — only the semantic checks and, incidentally, worktree cleanliness
at measurement time failed. Every repository still has exactly one unique
artifact digest and one unique canonical semantic digest across the five
runs — the determinism gate this benchmark exists to enforce (see "Make
.aether serialization canonical" in the git history) still passes cleanly;
determinism and precision/recall are independent properties, and this run
shows the corpus can fail the second while holding the first perfectly.

## Claims that remain open

This gate measures fresh graph creation and exact success on 43 curated call
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
