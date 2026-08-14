# Graph-addressed plan authoring cost

This measurement compares the model input required to author text-addressed
Plan Format v1 edits with graph-addressed Plan Format v2 edits. It records an
observed outcome, not an architectural claim.

## Precommitted method

- Policy: `docs/authoring-cost-policy.json` (SHA-256
  `6e160be6a7893eff565109414e33c4b53ca42d5b4bb670f9151c4d4aed9f7790`).
- Model: local Ollama `qwen2.5-coder:1.5b`, manifest
  `d7372fd828518a4d38b1eb196c673c31a85f2ed302b3d1e406c4c2d1b64a0668`,
  temperature 0, seed 42, and at most three repair attempts.
- Corpus: replace, rename, delete, and module insertion in both Rust and
  Python, run on fresh clones of clean source commit
  `423e6528e05dcb4e1f4048f6e815fd79d66b4fb1`. The two task-source hashes,
  intents, prompts, repair protocol, options, and success thresholds are
  unchanged from the TinyLlama control.
- The text arm received the complete target-file projection. The graph arm
  received only node path, language, intent, and the permitted v2 edit shape.
  The runner mechanically rejected graph prompts containing the pinned target
  projection.
- A generated plan counted as successful only if its envelope and edit shape
  matched the assigned arm, `plan validate` passed, a real `plan run` passed,
  and its project tree matched an independently text-derived expected tree.
  `.bitcode/` runtime reports were excluded from the project-tree digest.
- Before tree comparison, both Python trees passed through the same
  deterministic token formatter. It canonicalizes string and f-string tokens
  through Python's parser/unparser while retaining all other tokens. Non-Python
  files remain byte-exact. The regression test proves double- versus
  single-quoted equivalent f-strings compare equal while a changed string value
  still fails.
- Ollama `prompt_eval_count` is reported separately for the first attempt and
  for all attempts summed. A full generation that exceeded the fixed
  300-second limit remained a failure. The one-token recovery request supplied
  the prompt count when it completed; unavailable counts were not estimated.

## Observation

| Task | Text first | Text all attempts | Text result | Graph first | Graph all attempts | Graph result |
|---|---:|---:|---|---:|---:|---|
| Rust replace | — | — | failed | 171 | 551 | failed |
| Rust rename | — | — | failed | 157 | 509 | failed |
| Rust delete | 551 | 551 | failed | 151 | 491 | failed |
| Rust insert | — | — | failed | 161 | 521 | failed |
| Python replace | 271 | 561 | failed | 150 | 488 | failed |
| Python rename | 267 | 267 | failed | 146 | 476 | failed |
| Python delete | 262 | 824 | failed | 139 | 455 | failed |
| Python insert | 272 | 272 | failed | 150 | 488 | failed |
| **Known/exact total** | **1,623 known** | **2,475 known** | **0/8** | **1,225** | **3,979** | **0/8** |

`—` means both the original request and its one-token recovery timed out, so
the input-token count is unavailable. This occurred for the Rust replace,
rename, and insert text arms. Consequently, no honest full-corpus text-versus-
graph token percentage can be calculated.

For the five tasks with complete first-attempt counts in both arms, the text
prompts used 1,623 tokens and the corresponding graph prompts used 736 tokens,
a **54.7% first-attempt reduction**. This paired subtotal measures context
size. Across all repairs for the same five tasks, the counts were 2,475 text
and 2,398 graph, only a 3.1% difference; that second figure primarily mixes
context with different retry counts and is not the authoring-context result.
The graph arm's exact eight-task totals were 1,225 first-attempt tokens and
3,979 tokens across repairs.

Neither arm produced a working plan. Common successes were 0 against the
precommitted minimum of 4, and the missing text counts also prevent evaluation
of the full-corpus total-token threshold. The policy result is therefore
**FAIL**.

Of 35 attempts, 28 completed responses failed because the plan envelope had
missing or extra fields; the other seven attempts hit the bounded provider
timeout. Three of those seven also lacked a recovered token count. The earlier
3/3 minimal envelope probe used a schema-constrained `/api/chat` request with
the edit body supplied verbatim. This run used the precommitted real-task
`/api/generate` protocol with JSON-object output, larger prompts, task-specific
edit shapes, and repairs. The probe established model capability under its
narrow protocol, but it did not predict success under this one.

The complete per-attempt record, hashes, host metadata, clean-source evidence,
and tool provenance are in `docs/authoring-cost-observation.json`. This rerun
does not establish successful local plan authoring or a full-corpus context
reduction. It establishes a 54.7% first-attempt reduction on the five complete
pairs and shows that real-task envelope emission and provider reliability
remain blockers under the precommitted protocol.
