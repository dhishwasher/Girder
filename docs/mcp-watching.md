# Opt-in MCP watching

Watch mode is implemented and verified in the source tree at `b00489d`. It is
not included in the published npm 0.2.4 package. The
[incremental equality and platform gates](incremental-update-validation.json)
passed before watcher implementation began; the watcher has its own
[policy and gates](mcp-watch-policy.json) and a completed
[observation](mcp-watch-observation.json).

Start a watch-enabled server with:

```sh
girder mcp /path/to/project --watch
```

The invocation without `--watch` keeps the existing per-call behavior. There
is no scheduled default-on change. Watching keeps extraction data and a
complete published graph in the MCP process. Its seven tool schemas and
free/paid split stay the same. Tools use the shared CLI query handlers with
the published source graph; durable graph metadata is reconciled separately
for persistence. Historical Git baselines are still extracted when a review
or automatic test-impact request needs them.

Events wait for 250 ms of quiet. Source and configuration inputs must match
two checks 100 ms apart, including mtime and size. Content fingerprints also
check that the cached extraction describes the bytes read, including a file
that changed and reverted during parsing. Validation repeats before and
after persistence. Native notifications are supplemented by periodic input
checks and validation before tool response emission, because notifications
can be delayed or lost. These checks read source bytes even when parsing is
reused; their CPU and I/O cost belongs in watch measurements.

Every update resolves the whole project using changed extraction plus cached
clean extraction. Paired renames remove the old path and create the new one.
Unknown renames, directory deletion, overflow, uncertain ownership, or
unstable reads require full reconciliation. Configuration changes, cache
inconsistency, and dirty sets above 50% retain the incremental policy's full
parsing behavior. All supported languages keep cold analysis's existing
resolution misses. Watching does not improve semantic coverage.

Source configuration applies along with mandatory exclusions for `target`,
`.git`, `node_modules`, and `__pycache__`. The process ignores its own journal
and graph persistence traffic. An OS file lock identifies the graph owner
outside the watched root, so removing and recreating the root does not
admit a second watcher. Lock identities do not depend on a launcher's
`TMPDIR`, `TEMP`, or `TMP`. Watch mode rejects a root that contains its own
ownership storage. Lock-file identity is checked around ownership-sensitive
operations; storage access failures fail closed.

Graph persistence shares the existing writers' journal lock, recovery,
baseline checks, staged writes, and atomic replacement. A candidate becomes
visible only after complete persistence and input validation. Graph-dependent
calls wait while stale or rebuilding, within the existing tool timeout.
A response checks the generation again under the invalidation/emission
lock, so invalidated paid output is retried or returned as a tool error.

Transactions also lock their normalized output paths, so nested project roots
that target the same graph coordinate with each other. Those locks cover
baseline reads, commit or rollback, and watcher publication or response emission.
New journals record replacement fingerprints. Recovery checks every target
before restoring any target and retains a conflicting journal instead of
overwriting a later writer's data. Journals remain local to each project root;
conflicting foreign journals require explicit recovery. An older journal with
missing staged bytes and no replacement fingerprint also stays unresolved
when its applied value cannot be verified. The watcher remains stale in these
cases and graph calls return a tool error when their timeout expires.

Queries use one worker with one queued request. A timed-out result is
discarded. The worker may finish its current computation afterward; it
cannot emit that result later, and repeated timeouts cannot create an
unbounded set of query threads. Tool text remains subject to the existing
4 MiB limit. Protocol responses and watcher diagnostics use stdout and
stderr respectively.

## Recorded result

The one recorded campaign passed complete source and persisted-record equality
for all 15 initial graphs and all 45 consecutive mutations. Every update was
published; none was discarded or failed. Across those updates, 65 file
extractions were parsed and 4,930 were reused, so parsing reuse was 98.70%.
Full parsing occurred for 5 of 45 completed candidate builds (11.11%), all with
the frozen `dirty_set_exceeds_half` reason. The 15 initial builds took 10.550
seconds in total (0.543-second median; 1.099-second maximum).

| Initial files | Mutations equal | Full parsing | Parsed / reused | Reuse | Median update | Maximum update |
|---:|---:|---:|---:|---:|---:|---:|
| 3 | 15/15 | 5/15 (33.33%) | 25 / 20 | 44.44% | 0.448 s | 0.510 s |
| 30 | 15/15 | 0/15 (0%) | 20 / 430 | 95.56% | 0.514 s | 0.616 s |
| 300 | 15/15 | 0/15 (0%) | 20 / 4,480 | 99.56% | 0.987 s | 1.088 s |
| **All** | **45/45** | **5/45 (11.11%)** | **65 / 4,930** | **98.70%** | **0.514 s** | **1.088 s** |

The high full-parsing rate and low reuse in the three-file fixtures are a
delivery limitation of the conservative greater-than-50% dirty-set rule. The
larger fixtures avoided full parsing, but update latency rose because integrity
checks still read the configured source inventory. This measurement supports
only preservation of cold-analysis resolution while parsing is reused. It does
not claim improved semantic coverage or production-repository latency.

Repository CI passed at the measured commit. The native watcher safety and MCP
restart gates also passed on Linux, Windows, Intel macOS, and Apple Silicon in
[the four-platform run](https://github.com/dhishwasher/Girder/actions/runs/34524122557).

## Measurement contract

The [watch corpus](mcp-watch-measurement-corpus.json) reuses the incremental
corpus's 3-, 30-, and 300-file scaling fixtures for Rust, Python,
TypeScript, TSX, and Go: 15 initial builds and 45 consecutive mutations.
The explicitly invoked probe compares complete source node/edge records and
complete persisted records with fresh cold extraction and reconciliation.
The persistence oracle carries its own preceding cold graph through the
sequence; the watcher's saved output never becomes the expected prior graph.
Exact IDs, endpoints, edge weights, Calls, and test relationships are included;
only internal allocation order is ignored. Any mismatch fails correctness.

The mutation-to-publication timer includes filesystem writes, event delivery,
coalescing, stability checks, source inventory, cache cloning, resolution,
journal persistence, and publication. The observer waits on publication;
it runs the cold oracle after stopping that timer. Background integrity
checks remain active during the cold oracle. Initial construction is recorded
separately. These synthetic fixtures do not establish production latency.

Watcher diagnostics report published updates, attempted updates, completed
candidate builds, discarded candidates, failed attempts, full parsing,
parsed/reused files, reasons, and latency. The fallback denominator is
completed candidate builds, including candidates later discarded; initial
construction is excluded. The percentage is full-parsing builds divided by
that denominator. Reasons may overlap. Measurement rows retain retry costs,
and raw events retain emitted counter checkpoints for interrupted or incomplete
mutations. Work after the last emitted checkpoint is unquantified. The harness
saves stdout and stderr as they arrive, including before a timeout or interruption.
Frequent full parsing remains a delivery limitation even if equality passes.

The recorded harness used committed inputs and a separately built probe,
refused an existing output, and preserved the raw probe and diagnostic streams
alongside the observation. The timing probe remains ignored in routine CI; CI
runs its assessment unit tests. Native filesystem, ownership, MCP, timeout,
and restart gates execute on Linux, Windows, Intel macOS, and Apple Silicon
macOS through the platform verification workflow.
