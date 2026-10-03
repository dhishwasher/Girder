# TypeScript structural member identity: local audit after implementation

Candidate revision `b3195561872d16ab440f71ef43434f643ce513b9` (Rust product
code last changed in `230418e`). The locally rebuilt Girder binary has SHA-256
`0e6e09c70355cb1e187a6b39b31a06d0ac4fcc62f4e81ce6af287d8537ead927`.
This is a fresh offline extraction from the four pinned archives, not a replay
of earlier inspect files. The [run record](run.json) preserves the full command
list, source inventories, input hashes, timings, exit codes, and binary identity;
the compressed raw analyze and inspect outputs are retained beside it.

The frozen structural [policy](../../structural-members/policy.md) (SHA-256
`09ed57ac9acd92543fba04325d34175de1e9ec7829f6d000092eddeac2a69181`),
identity manifest (`fa86e163…`), validation manifest (`aa114b8e…`), 49-case
dispatch corpus (`9e3208a8…`), and original structural fixture (`5b85efac…`)
were unchanged. All four cached archive hashes and extracted source inventories
matched their pins. No network acquisition or source substitution occurred.

The [frozen 100-call audit](combined-cohort.json) scored **56 exact, 44
conservative, and 0 unsound** cells. All 100 actual calls and 10 retained
noncalls were accounted for. Girder emitted 3 Must claims, all 3 pointing to the
frozen target; Must precision on this sample is 3/3 = 1.000. The other 97 calls
remain Unknown. There were no May claims or ground-truth May sites, so May
recall is undefined. The audit runner's precommitted acceptance check passed.

Against the immediately preceding [lexical-binding
checkpoint](../bindings-after-1/observation.md), the counts moved from 55
exact / 45 conservative / 0 unsound and 2/2 Must to 56 / 44 / 0 and 3/3 Must.
Only site 32 changed classification: `src/compiler/checker.ts:29877` calls
`narrowTypeByTypeFacts`, declared at line 29884 in the same lexical scope. The
answer moved from Unknown to Must with the correct frozen target. Eight other
records changed ownership or evidence details without changing class. The
[changed-answer file](changed-answers.json) compares against the older frozen
before-observation, so it reports 45 changes; this immediate-predecessor
comparison was made separately by exact site index. The structural extraction
removed a duplicate-path gate that had prevented the existing lexical proof at
site 32. That causal explanation is an inference from the implementation and
changed evidence, not a new proof rule. No compiler tests were executed as a
dynamic oracle for the three Must calls; correctness is against the previously
frozen source-reviewed labels.

On the same local binary, the separate [structural validation
scorer](structural-validation-scoring.json) met all 17 origin contracts: 1 exact,
20 conservative, 0 unsound test cells. The unchanged [49-case dispatch
corpus](dispatch-corpus-scoring.json) stayed at 22 exact / 34 conservative /
0 unsound / 1 failed. Its original `typescript-structural-object-literal`
case failed as the precommitted **ambiguity** between `alice` and `bob`, not as
a zero-match origin. This is **not a dispatch-corpus improvement**. The failure
remains in the result.

The four common gates passed locally on candidate `b319556`, serially with
`-j1`, `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, and the MOVESPEED target.
[Gate logs and exit statuses](gates-b319-local/summary.json) record 769 Rust
tests passed, 0 failed, 2 ignored; strict Clippy and format checks clean; and
29 npm tests passed, 0 failed, 2 pre-existing skips. The local validation and
dispatch scorer exit statuses and logs are stored there too. These local gates
supplement the cloud gates on `230418e`; the scorer-only correction in `b319556`
has its own 17-test unit suite and retained negative controls in the
[implementation observation](../../structural-members/implementation-observation.md).

**Disposition:** Structural identity policy v1 now has its required local
real-repository audit. TypeScript remains **IN PROGRESS** because the unchanged
dispatch corpus has no measured improvement. Go remains **NOT STARTED**. The
next TypeScript work is a separately frozen, adversarially tested proof rule
for a bounded dispatch case; identities alone do not authorize Must or May.
