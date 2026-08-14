# Graph-addressed plan authoring cost

This measurement compares the model input required to author text-addressed
Plan Format v1 edits with graph-addressed Plan Format v2 edits. It records an
observed outcome, not an architectural claim.

## Precommitted corrected method

- Policy: `docs/authoring-cost-policy.json` (SHA-256
  `4efed6d5759c9f4ef836d2d46ebe0648c8da2b10c46e712ec4022ec464080d01`).
- Model: local Ollama `qwen2.5-coder:1.5b`, manifest
  `d7372fd828518a4d38b1eb196c673c31a85f2ed302b3d1e406c4c2d1b64a0668`,
  temperature 0, seed 42, and at most three repair attempts.
- Corpus: replace, rename, delete, and module insertion in both Rust and
  Python, run on fresh clones of clean source commit
  `fd74d3468b65fddce6e853103aa9368767a0c90d`. The model, all eight tasks,
  task-source hashes, intents, sampling options, timeout, repair bound, and
  success thresholds are unchanged from the preceding capable-model control.
- Every authoring request used Ollama `/api/chat` with a task-specific plan
  JSON schema in `format`. Required fields, exact singleton arrays, JSON types,
  and the absence of extra fields were grammar-constrained. Envelope shape was
  therefore enforced by construction rather than counted as model success or
  failure. The repair loop remained responsible for content errors.
- The text arm received the complete target-file projection. The graph arm
  received node path, language, intent, and bounded current node source. For
  module insertion, it received the current source of the bounded semantic
  insertion anchor because the module node's stored projection is the whole
  file. The runner mechanically rejected any graph prompt containing the
  complete pinned target-file projection.
- Every generated plan had to retain a declared task-specific semantic check.
  Success required `plan validate`, a real `plan run` (including that declared
  check), and `bitcode test-impact . --run --quiet` to pass. The semantic checks
  execute Python behavior and verify the requested Rust API/source semantics.
  Regression tests prove that a differently spelled but behaviorally
  equivalent Python edit passes and a behavior-changing edit fails. No
  reference tree or tree-equality score is used.
- Ollama `prompt_eval_count` is reported separately for the first attempt and
  for all attempts summed. A generation exceeding the fixed 300-second bound
  remained a failure. A one-token schema-constrained recovery request supplied
  the prompt count when it completed; unavailable counts were not estimated.

## Observation

| Task | Text first | Text all attempts | Text result | Graph first | Graph all attempts | Graph result |
|---|---:|---:|---|---:|---:|---|
| Rust replace | — | — | failed | 263 | 263 | failed |
| Rust rename | — | — | failed | 235 | 235 | failed |
| Rust delete | 579 | 579 | failed | 225 | 225 | failed |
| Rust insert | — | — | failed | 235 | 235 | failed |
| Python replace | 300 | 300 | failed | 208 | 433 | passed |
| Python rename | 296 | 296 | failed | 200 | 746 | failed |
| Python delete | 290 | 290 | failed | 188 | 393 | passed |
| Python insert | 300 | 300 | failed | 221 | 221 | failed |
| **Known/exact total** | **1,765 known** | **1,765 known** | **0/8** | **1,775** | **2,751** | **2/8** |

`—` means both the original request and its one-token recovery exceeded the
bounded provider timeout. This occurred for the Rust replace, rename, and
insert text arms. Consequently, no honest full-corpus text-versus-graph token
percentage can be calculated.

For the five tasks with complete first-attempt counts in both arms, the text
prompts used 1,765 tokens and the corresponding graph prompts used 1,042
tokens, a **41.0% first-attempt reduction**. This paired subtotal measures
context size. Across all attempts for the same five tasks, text used 1,765
tokens and graph used 2,018; the graph total includes repairs and is not a
context-size comparison. The graph arm's exact eight-task totals were 1,775
first-attempt tokens and 2,751 tokens across all attempts.

The graph arm authored two working plans: Python replace and Python delete.
Both passed plan validation, their declared semantic checks, real execution,
and the impacted-test command. The selector named no additional impacted tests
for those sample-project edits. No text arm completed within the generation
bound. Common successes were therefore 0 against the precommitted minimum of
4, and incomplete text token counts also prevented evaluation of the
full-corpus token threshold. The policy result is **FAIL**.

There were 20 attempts: 13 bounded provider timeouts and seven completed
schema-constrained responses. None had a missing or extra envelope field. Of
the completed responses, two passed after one content repair, four used the
wrong `on_failure` value, and one failed plan validation. This distinguishes
the remaining model/content and provider-limit failures from the earlier
envelope-harness failure.

## Relationship to the earlier controls

The earlier 0/8 text and 0/8 graph observations remain in Git history as
controls, but they were harness-limited and are not evidence that the model
could not author a plan. In the immediately preceding qwen control, 28 of 35
attempts were rejected for missing or extra envelope fields because
`/api/generate` requested only plain JSON rather than enforcing the plan
schema. Its graph prompt also omitted the target node's source, which the
signature-preservation probe showed was necessary, and its reference-tree
criterion could reject correct work solely because it was spelled differently.
The corrected protocol removes all three limitations. Its two semantically
verified graph successes are the first evidence in this corpus of working
fully local plan authoring, while the failed policy and provider timeouts show
that the evidence is not yet broad or reliable enough to close the gap.

The complete per-attempt record, policy and source hashes, host metadata,
clean-worktree evidence, semantic-check hash, bounded command results, and tool
provenance are in `docs/authoring-cost-observation.json`.
