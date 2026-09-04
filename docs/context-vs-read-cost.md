# `girder context` vs reading the file

This measurement compares retrieving one function's source three ways:
reading the whole file it lives in, running `girder context`, and running
`girder context --source-only`. It records an observed outcome, not an
architectural claim.

## Why this measurement exists

`CLAUDE.md` instructed agents:

> Before reading a file to understand a function, run:
> `girder context . --nodes <node::path> "<what you need>" --json`
> That returns the function's source alone. Measured 41% fewer tokens than
> reading the file.

Both sentences were wrong.

`girder context` does not return the function's source alone — it returns
the source *plus* a Plan Format v2 JSON Schema and a plan skeleton, because
its purpose is to let an external model author a plan. And the 41% figure
is from `docs/authoring-cost.md`, which measured something else entirely:
plan-authoring prompt tokens, text-addressed vs graph-addressed, on 5 of 8
paired tasks, under a policy whose overall result was **FAIL**. No
measurement of `girder context` against a file read existed.

That claim was also injected into every agent session by
`.claude/hooks/girder_context_advisory.py`, so an unmeasured number was
being asserted to a model thousands of times.

## Precommitted method

- Policy: `docs/context-vs-read-cost-policy.json`.
- Harness: `tools/context_vs_read_cost.py` (reproducible — see below).
- Source commit: `073c4eceb1a2b8ecf3f1e3b49a4d4bb1625cb890`.
- Ten Function nodes, pinned in the policy before measurement. Selected by
  a stated rule rather than by hand: every Function node in this
  repository's graph sorted by `(source span bytes, node path)`, sampled at
  each 10th percentile. Deciles by source size matter here because the
  envelope under test is a *fixed* cost, so it weighs most on the small
  functions that make up most of a codebase — the median Function node in
  this repo is 549 bytes. Hand-picking large functions would have flattered
  the tool.
- Three arms, all measured in bytes:
  - **read** — byte count of the file containing the node, which is what an
    agent's Read tool returns when it opens a file to see one function.
  - **context** — `girder context . --nodes <path> --json` stdout.
  - **source_only** — `girder context . --nodes <path> --json --source-only`
    stdout. This mode was added by this measurement: it emits the selected
    nodes' `{path, language, source}` and drops the authoring envelope.
- Threshold: aggregate reduction of the **source_only** arm against **read**
  ≥ 40%, matching the bar in `docs/authoring-cost.md` and reused by
  `docs/names-cost-policy.json`. The **context** arm is reported but not
  gated.

## Observation

| Node | read | `context` | `--source-only` | `context` | `--source-only` |
|---|---:|---:|---:|---:|---:|
| `greet` | 392 | 6,120 | 188 | **−1461.22%** | 52.04% |
| `collaboration_result` | 26,690 | 6,673 | 325 | 75.00% | 98.78% |
| `read_project_bytes` | 66,860 | 6,593 | 397 | 90.14% | 99.41% |
| `cargo_bin_targets_is_empty_without_a_readable_manifest` | 66,860 | 7,024 | 540 | 89.49% | 99.19% |
| `test_archive_path_validation_rejects_escape_and_platform_ambiguity` | 12,328 | 7,560 | 700 | 38.68% | 94.32% |
| `require_state` | 18,884 | 6,949 | 745 | 63.20% | 96.05% |
| `multiple_frames_in_sequence` | 4,136 | 7,171 | 951 | **−73.38%** | 77.01% |
| `ans_callees` | 24,749 | 7,389 | 1,161 | 70.14% | 95.31% |
| `atomic_write` | 66,860 | 7,639 | 1,491 | 88.57% | 97.77% |
| `_require_isolated_occurrence` | 120,378 | 8,439 | 2,267 | 92.99% | 98.12% |
| **Total** | **408,137** | **71,557** | **8,765** | **82.47%** | **97.85%** |

The policy result is **PASS** (97.85% ≥ 40%).

## What the numbers say

**`--source-only` is cheaper than reading the file on 10 of 10 nodes**, by
97.85% in aggregate and by at least 52% on every individual node. That is
the honest version of the claim `CLAUDE.md` was making.

**`girder context` is not.** It costs *more* than reading the file on 2 of
the 10 nodes, and the failure mode is severe: retrieving a 39-byte function
from a 392-byte file costs 6,120 bytes, **15.6× the entire file**. The
`context` arm's output never drops below roughly 6.1 KB no matter how small
the function is, because the Plan Format v2 schema and plan skeleton are a
fixed cost paid on every call. Across these ten nodes the envelope accounts
for 62,792 of the 71,557 bytes the `context` arm spent — 87.8% of its output
is not the source the caller asked for.

So the aggregate 82.47% reduction for `context` is real but misleading. It
comes almost entirely from four nodes that happen to live in files of 66 KB
and 120 KB. Advising an agent to reach for `context` instead of Read is a
loss whenever the target file is smaller than about 7 KB, which is a large
fraction of any real codebase.

## Honest limits

- **Bytes, not tokens.** Every arm is a raw stdout/file byte count, the same
  metric `docs/names-cost.md` and `docs/context-with-tests-cost.md` use.
  Bytes are a proxy for tokens; JSON's punctuation and the schema's repeated
  key names likely tokenize *worse* per byte than prose source, so the
  `context` arm's real token cost is plausibly worse than shown here, not
  better. No tokenizer was run. This measurement does not license a token
  claim.
- **One repository.** All ten nodes come from this repo, so file-size
  distribution is this repo's. The direction of the result is
  size-driven and should hold anywhere, but the exact percentage is not
  portable.
- **Retrieval only.** This says nothing about whether `context`'s envelope
  is worth its cost for its actual purpose, plan authoring, where the schema
  is the point. `docs/authoring-cost.md` is the measurement for that, and it
  failed its own policy.
- **`--source-only` is not free.** It still builds the whole graph, which is
  seconds of CPU, where Read is a single syscall. This measurement is about
  context-window bytes, not wall time.

## Reproducing

Unlike `docs/names-cost.md` and `docs/context-with-tests-cost.md`, whose
numbers were hand-recorded with no script to regenerate them, this one has a
harness:

```bash
cargo build -p aether-app
python3 tools/context_vs_read_cost.py --girder target/debug/girder \
    --output docs/context-vs-read-cost-observation.json
```

It exits non-zero if the gated arm misses the threshold. The complete
per-node record, including the observed commit and whether it matched the
policy's pinned commit, is in `docs/context-vs-read-cost-observation.json`.
