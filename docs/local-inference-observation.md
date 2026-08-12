# Local inference observation

Recorded on 2026-08-12 against fresh local clones of Bit Code commit
`303964abb4c97a8e9d9512834c22267cff0e5f35`. The plan executor binary had
SHA-256 `7b3c79b36123a1b2189847082d0996b850a54b0a7b2785ae5df6fb6750967b64`.

## Method

The only model available on the external drive was `tinyllama:1.1b` (Q4_0,
about 638 MB). It was served by Ollama 0.32.7 on `127.0.0.1:11435` with
`OLLAMA_NO_CLOUD=true`; `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` were empty.
Each run used a fresh local clone, an Ollama JSON-schema-constrained response as
the Plan Format v1 document, and a receipt populated from Ollama's returned
model and `eval_count`. Bit Code validated and ran the unchanged plan, then
wrote its ordinary report under `.bitcode/reports/`.

The schema fixed the target path, factual Markdown bytes, and declared command
check. The local model still authored the plan envelope and descriptive fields.
This bounded setup was necessary because this 1B model was not reliable enough
for unconstrained plan or factual-content generation; that ceiling is recorded
below rather than hidden.

## Passing runs

| Run | Task | Model | Authoring tokens | Wall time | Checks | Report result | `zero_remote` | Remote calls | Report SHA-256 |
| ---: | --- | --- | ---: | ---: | --- | --- | --- | ---: | --- |
| 1 | Create an Ollama host/runbook note | `tinyllama:1.1b` | 428 | 222.696 s | 1/1 passed | passed | true | 0 | `5cbd51b35aea018d9aec5b11be0721ac8bc19fb87d926e423cb24c1ea6d87dc1` |
| 2 | Create a local routing-policy note | `tinyllama:1.1b` | 407 | 208.893 s | 1/1 passed | passed | true | 0 | `f43d2343ef08aed0632d0ea4162688dbb1c2e71f31964703ffc1e63a76dfa701` |
| 3 | Create a plan token-ledger guide | `tinyllama:1.1b` | 407 | 204.376 s | 1/1 passed | passed | true | 0 | `37c0c554544624efba11805dd383fe25ee8fa5ee9fae09789683b00f00a3251b` |
| 4 | Create a plan safety note | `tinyllama:1.1b` | 385 | 225.700 s | 1/1 passed | passed | true | 0 | `15cc7748a46b1a81fb8d9f9de6001ed3a7bf0a38acea46b5feb085bdc947c00d` |
| 5 | Create an offline-work checklist | `tinyllama:1.1b` | 395 | 215.922 s | 1/1 passed | passed | true | 0 | `c8be444f607cef0a23c3909bb8745f1ba2b76d8ff54cf6ad2b7d110a2c350dce` |

All five reports contained one `ollama:local` call, named the model shown
above, marked that call `remote: false`, recorded `remote_call_count: 0`, and
derived `zero_remote: true`. Every plan committed its documentation file only
after its declared non-empty/content check passed.

## Observed quality ceiling

The local path is usable for tightly bounded work, but TinyLlama 1.1B is not a
general autonomous plan author on this repository:

- An unconstrained request for JSON returned a PHP explanation instead.
- An early schema-constrained plan passed validation but failed its exact-case
  content check; Bit Code rolled it back and recorded `zero_remote: true`.
- Three repeated attempts later exhausted the output cap with an unterminated
  JSON string.
- On a token-ledger task, three constrained attempts omitted the subject from
  their proposed content and were rejected before execution.
- Even successful free-text descriptions were sometimes truncated or awkward.

Accordingly, the measured success claim is narrow: Bit Code can complete real,
checked file-creation work with zero remote model calls when a small local model
is constrained by an exact output schema. A stronger local model is required
before relying on free-form plan contents or broader code changes.
