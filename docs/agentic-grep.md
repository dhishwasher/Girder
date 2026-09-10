# Agentic discovery cost: frozen protocol

This measures discovery costs with the local **1.5B**
`qwen2.5-coder:1.5b` model. It does **not** establish frontier-model behavior,
or correctness of complete caller, test, or impact answers.

The single frozen campaign is **incomplete; the overall gate failed**.
There are no pairs with two correct answers, so it supports no comparative
cost claim. The existing 97.85% naive whole-file and 97.98% plain-grep
comparisons do not establish an advantage over adaptive agentic grep.

## Recorded outcome

The [observation](agentic-grep-observation.json) retains all 15 tasks,
including the 21 arms that never started. The policy, prompts, evaluator
targets, tool schemas, harness, tests, and settings were committed before
execution; their hashes and the verified published 0.2.4 binary hash are
recorded in the observation. No targets were excluded or changed.

| Prompt class | Arm | Correct | Wrong | No answer | Completed arms | Comparable pairs |
|---|---|---:|---:|---:|---:|---:|
| Identifier (12 tasks) | Grep | 0 | 1 | 11 | 5 | 0 |
| Identifier (12 tasks) | Graph | 0 | 2 | 10 | 4 | 0 |
| Description (3 tasks) | Grep | 0 | 0 | 3 | 0 | 0 |
| Description (3 tasks) | Graph | 0 | 0 | 3 | 0 | 0 |
| All (15 tasks) | Grep | 0 | 1 | 14 | 5 | 0 |
| All (15 tasks) | Graph | 0 | 2 | 13 | 4 | 0 |

Six completed arms hit the configured 300-second chat-request timeout.
Three returned incorrect semantic paths. The next per-arm model metadata
check (`/api/show`, with its frozen 30-second timeout) timed out, stopping
the campaign before task 5's graph arm. No-answer totals include both
timed-out and unstarted arms; the observation distinguishes their states.
Observed arm wall time exceeded the request timeout on this heavily loaded
2.7 GiB VM (up to 372.21 seconds); the timeout setting is not a hard process
deadline. No timeout or model setting was changed after execution began.

Raw operational totals are six model requests, one tool call, and 348
confirmed tool-response bytes for grep; four model requests, zero tool
calls, and zero tool-response bytes for graph. These are failed-task
counters, **not evidence of graph savings**. There are zero comparable
pairs in either class, so byte ratios and comparative losses are
unavailable. Four completed cold graph constructions took 41.71 seconds
in total; their largest recorded peak RSS was 43,916 KiB. Construction is
separate from retrieval accounting.

The [per-arm transcripts](agentic-grep-observation-transcripts/) preserve
the requests, responses, errors, and tool outputs. A prior
[preflight failure](agentic-grep-preflight-observation.json) found GNU
`time` missing before any campaign claim or model request. Installing that
required profiler did not alter frozen inputs. The recorded campaign was
not restarted after its timeout failure.

## Inputs and independence

The [policy](agentic-grep-policy.json) reuses the 15 targets and ten pinned
repository archives from the [orient corpus](orient-tool-policy.json).
There are [12 identifier prompts and three description prompts](agentic-grep-prompts.json).
Prompts omit full semantic paths; description prompts omit their target
identifier. Their wording is frozen before either arm runs.

[Evaluator targets](evaluator/agentic-grep-targets.json) retain each original
orient record and its independently pinned source target. They are never
copied into either accessible repository, initial prompt, or tool response.
The two arms receive independently extracted snapshots, the same task, and
the same semantic-path naming conventions. Their conversations are isolated.
Neither arm receives evaluator feedback, authoring history, or the other
arm's transcript. Paths obtained by searching the actual source or graph
remain legitimate discovery results.

The grep arm chooses among file listing, ripgrep, and bounded line-offset
reads. It chooses searches, filters, page sizes, and offsets adaptively.
There is no whole-file requirement or target-specific scripted sequence.
The graph arm chooses among the [seven existing MCP tools](agentic-grep-tools.json),
invoked through the actual stdio server and its existing command response
adapters. Both arms receive documented schemas and the same result envelope.

## Execution and accounting

The harness requires the exact installed model digest
`d7372fd828518a4d38b1eb196c673c31a85f2ed302b3d1e406c4c2d1b64a0668`.
It stops on absence or mismatch; there is no substitute model. Requests use
Ollama's `/api/chat`, temperature 0, seed 42, an 8,192-token context, and
at most 1,024 generated tokens per turn. Responses use schema-constrained
tool-action or final-answer envelopes. Each arm has 30 tool actions, a
300-second request timeout, and a 1,800-second task timeout. Odd tasks run
grep first; even tasks run graph first. Execution is serial, without Cargo.

Before each request, the harness counts the entire rendered conversation
with the tokenizer vocabulary, merges, special tokens, and chat template
from the pinned local model. It reserves the full generation allowance.
If the history no longer fits, the result is **no answer**. It never drops
earlier messages. A disagreement with Ollama's prompt-token count stops the
campaign and is retained as an incomplete result.

Tool-output bytes count the UTF-8 encoding of each complete new result
envelope, including errors. Each response is counted once when an Ollama
reply confirms delivery. Model prose, initial prompts, and replay of old
conversation messages are excluded. Produced but undelivered results and
unconfirmed delivery after transport failures have separate raw counters.
Every attempted tool action counts, including rejected arguments and errors.
Model requests and their token counters are reported separately.

Cold graph construction has a separate elapsed-time and peak-RSS record.
The current default MCP tools reconstruct the graph during retrieval;
their retrieval elapsed time still includes that work. This benchmark
does not pretend the future watcher or parsing cache already exists.

## Correctness and gates

Each arm's answer must equal the independently pinned target. Agreement
between two wrong answers fails. Reports distinguish **correct**, **wrong**,
and **no answer** for each arm and for identifier and description prompts.

Cost comparisons include only pairs where both arms are correct. Failed
tasks retain raw operational counters without comparative cost claims.
Each prompt class reports comparable-pair counts and byte/call losses.

The original overall pass requires all 15 correctness pairs, aggregate graph
output bytes at most 80% of grep bytes, and no more graph tool calls than
grep calls. The assessment computes these fixed gates afresh; a stored
`pass` field cannot override them. An explicitly documented ambiguous target
may be excluded with its original record retained, but it cannot be repinned
or replaced, and the original all-15 pass cannot then be claimed.

## Running and preserving the record

Run the harness unit tests first:

```bash
python3 -m unittest -q tools.test_agentic_grep_benchmark
```

Commit every file named by `freeze_inputs` in the policy before measurement.
With no Cargo process running, execute once:

```bash
python3 -m tools.agentic_grep_benchmark \
  --girder /path/to/verified/girder \
  --output docs/agentic-grep-observation.json
```

The harness checks committed input hashes and records its binary hash.
An exclusive campaign claim prevents rerunning the same policy; an existing
observation cannot be overwritten. It writes pending records before model
execution, appends per-arm transcripts outside accessible roots, and saves
observations after each request and tool result. Failures, interruptions,
and incomplete arms remain visible. A corrective measurement requires a
distinct policy/record, not altered thresholds or removal of difficult tasks.

Routine CI runs harness tests only. Model execution remains outside CI.
