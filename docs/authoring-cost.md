# Graph-addressed plan authoring cost

This measurement compares the model input required to author text-addressed
Plan Format v1 edits with graph-addressed Plan Format v2 edits. It records an
observed outcome, not an architectural claim.

## Precommitted method

- Policy: `docs/authoring-cost-policy.json` (SHA-256
  `e0494609a4ec5177df30148e784f32c1e6559f02118329bcc33f665f5054fc8e`).
- Model: local Ollama `tinyllama:1.1b`, manifest
  `2644915ede352ea7bdfaff0bfac0be74c719d5d5202acb63a6fb095b52f394a4`,
  temperature 0, seed 42, and at most three repair attempts.
- Corpus: replace, rename, delete, and module insertion in both Rust and
  Python, run on fresh clones of source commit
  `8e0045b3a64399c70b521be857e13d84f961bfbc`.
- The text arm received the complete target-file projection. The graph arm
  received only node path, language, intent, and the permitted v2
  edit shape. The runner mechanically rejected graph prompts containing the
  pinned target projection.
- A generated plan counted as successful only if its envelope and edit shape
  matched the assigned arm, `plan validate` passed, a real `plan run` passed,
  and its project tree matched an independently text-derived expected tree.
  `.bitcode/` runtime reports were excluded from the project-tree digest.
- Ollama `prompt_eval_count` was accumulated for every attempt. When a full
  generation exceeded the fixed 300-second limit, a one-token replay after
  unloading the local worker recovered the same prompt's input-token count;
  the generation itself remained a failure.

## Observation

| Task | Text tokens | Text result | Graph tokens | Graph result |
|---|---:|---|---:|---|
| Rust replace | 2,018 | failed | 402 | failed |
| Rust rename | 643 | failed | 572 | failed |
| Rust delete | 637 | failed | 545 | failed |
| Rust insert | 653 | failed | 596 | failed |
| Python replace | 1,031 | failed | 551 | failed |
| Python rename | 327 | failed | 548 | failed |
| Python delete | 1,004 | failed | 518 | failed |
| Python insert | 1,046 | failed | 572 | failed |
| **Total** | **7,359** | **0/8** | **4,304** | **0/8** |

Graph addressing reduced measured input tokens by 3,055, or **41.5%**
(`4,304 / 7,359 = 0.5849`). It did not make this model capable of authoring a
working plan: neither arm solved any task, so common successes were 0 against
the precommitted minimum of 4. The policy result is therefore **FAIL**.

The predominant completed-response failure was an invalid plan envelope after
all permitted repairs. Five arms also encountered a bounded provider timeout:
three Rust text arms, Python rename text, and Rust replace graph. Their prompt
tokens were recovered and recorded, but the generations remained failures.
An independent canonical-plan probe—not the model observation, whose generated
rename plan failed shape validation first—also found that Python graph rename
cannot lower safely in the current repository because unrelated same-named
identifiers are not call-site-proven. That executor limitation is recorded
separately in `core-gap-analysis.md`.

The complete per-attempt record, hashes, host metadata, and failure details are
in `docs/authoring-cost-observation.json`. The finding is narrower than “graph
addressing works for local authoring”: graph addressing lowers context cost,
but this model and prompt protocol did not complete work with no remote calls.
