# Edit blast-radius advisory policy

This policy is precommitted before implementation of the post-edit advisory.
It defines the meaning of reached, the report bound, the latency ceiling, and
the observation that must be recorded. It is an advisory usability policy;
it cannot block, reject, or change an edit.

## Frozen identity and inputs

- Policy id: `edit-blast-radius-v1`.
- Baseline source commit: `4da4699a6fb087d9113cc8e683fa0b3135757a0a`.
- Observation corpus: `docs/incremental-mutation-corpus.json` at SHA-256
  `82f8b06ea365b79dd5c2d17960e5b46785200ef3eae27efc51ea2dbb15217fbe`.
  Every declared mutation is included; failed or skipped events remain in the
  observation.
- The hook may use only the project's existing saved graph snapshot. A fresh
  hook process may read and deserialize that snapshot inside the latency
  ceiling, but it must not scan source files, reconcile the snapshot against
  the working tree, build or update a graph, invoke Cargo, or make a network
  call on the edit path. No valid saved snapshot means no report.

## Reach contract

An edit event is the successful write of one supported source file. Its origin
set is every saved non-test function node whose source file is
that project-relative path. If the file has no such node, the event has an
empty origin set and produces no impact list.

For each origin, `reached` is the origin plus the union of nodes found by a
shortest-path breadth-first traversal of incoming graph edges whose kinds are
`Calls`, `DataFlow`, or `Impacts`. These are exactly the graph edge kinds whose
`propagates_impact` contract is true. Other edge kinds do not establish reach.
Route-aware traversal uses the graph's existing route metadata; unresolved,
ambiguous, or absent metadata remains fail-open according to the graph's
current impact implementation. A node is listed once, with its minimum hop
distance across all origins. Paths are sorted by `(distance, node path)`.

The report evaluates coverage only for reached non-test function nodes. A
reached node is covered when the existing graph test relation returns at least
one test node for that node (`tests_for` semantics). Test nodes themselves are
not reported as uncovered targets. The policy describes the saved graph's
file-level blast radius; it does not claim to identify the exact changed
function or improve semantic resolution.

The saved graph predates the completed edit event. Results therefore describe
the last analyzed version of functions from that path and their callers. New
functions that exist only after the edit are absent until the user next
analyzes the project. The advisory must label this as saved-graph impact rather
than imply that it parsed or understood the new file contents.

## Report bound

The advisory may emit at most:

- 8 origin paths;
- 32 reached paths; and
- 16 uncovered reached paths.

The complete advisory, including its summary and truncation counts, is capped
at 8 KiB on stderr. Origins, reached paths, and uncovered paths use the stated
deterministic ordering. When a cap removes entries, the report says how many
were omitted. The report contains no source text, secrets, environment data,
or arbitrary user-controlled error text. It is valid for the hook to emit
nothing when a cap-safe report cannot be produced.

## Latency ceiling and failure behavior

The impact computation timer starts immediately before reading the saved graph
snapshot and stops after the bounded report data has been assembled. The
ceiling is **20 ms wall-clock per edit event**, including snapshot I/O and
deserialization. Source scanning, graph reconciliation, graph building, and
file parsing are forbidden. Process startup is outside this computation
metric. If computation reaches 20 ms, or any lookup, serialization, or output
operation fails, the hook emits nothing and returns success to the agent. No
timeout diagnostic is sent to stdout or to the agent's tool result.

## Observation

After implementation, run every mutation in the frozen corpus from a clean
fixture, in declared order, with one event for each changed source path. A
multi-file mutation therefore contributes one event per path; events are not
collapsed. Record one JSON observation at
`docs/edit-blast-radius-observation.json` containing:

- policy id, corpus SHA-256, implementation commit, binary SHA-256, platform,
  and clean-input status;
- every event's case and mutation id, path, graph-ready status, origin count,
  reached count, uncovered count, truncation counts, emitted byte count,
  computation duration, and whether it emitted or timed out; and
- aggregate eligible, skipped, timed-out, and failed event counts, p50/p95/max
  computation duration for eligible events, maximum emitted bytes, and the
  complete list of failed events.

The observation is descriptive. It must retain red results and does not tune
the corpus, caps, or 20 ms ceiling after measurement. It must explicitly state
that the latency measurement excludes process startup while the hook's
fail-open behavior covers startup and all other errors.
