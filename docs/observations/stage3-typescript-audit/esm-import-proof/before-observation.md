# ESM import proof: cold CLI baseline, 2026-10-05

**The frozen contract fails: 66/73 exact answers.** All 73 marked calls have
one explicit Unknown claim with no target. The 66 refusal cases match; all
seven required conditional Must cases remain conservative misses. There are
no missing marked claims, command errors, or malformed-output errors.

Candidate and rebuilt binary source revision:
`c209a2248a5a33c45d2428193dbd4ad3ea5b2aa3`. This revision adds the measurement
runner, not an ESM resolver. The serial, locked, offline CLI build passed in
598.150 seconds; [build log and provenance](baseline-build-1/run.json) record
the command and environment. Binary SHA-256:
`469f25194a826ff7aacb1b4240c8307c26f35cf0ff60c422bdf7cd6b8e26a1be`.

The [raw observation](before-1/run.json) records all 146 command invocations,
exit statuses, durations, output hashes, marked claims, target identities, and
assumptions. Compressed stdout and stderr logs are adjacent. Commands took
38.966 seconds in total; this is not a performance comparison.

```sh
python3 -m tools.measure_typescript_esm_imports --name before-1 \
  --binary /mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder \
  --binary-revision c209a2248a5a33c45d2428193dbd4ad3ea5b2aa3
```

Exit status **1** means the contract was measured and failed. Every Girder
command exited 0. The pinned manifest hash remains
`daa7310248f8e2adc3bdbbae340a3301dcca6b18983ca29ef3c6738bb97b94cd`;
all source hashes, symlink texts, and frozen benchmark inputs matched before
measurement. Analysis used isolated copies with symlinks preserved. The
temporary copies were removed after their outputs were preserved.

| Expected conditional Must case | Observed |
| --- | --- |
| `aliased-import` | Unknown, no target |
| `async-target` | Unknown, no target |
| `explicit-ts` | Unknown, no target |
| `mock-other-module` | Unknown, no target |
| `mts-extension` | Unknown, no target |
| `parent-directory` | Unknown, no target |
| `unrelated-imports-allowed` | Unknown, no target |

All seven expected declarations were found at the pinned file, semantic path,
and declaration position. The failures are absent import proofs, not failed
target lookup. There are no observed Musts here: Must precision is undefined,
not 1.000. Future Must answers are conditional on the frozen ESM execution and
no-unmodeled-hook assumptions; the runner requires those assumptions and the
exact single target in addition to the class and proof reason.

The runner's nine unit tests passed (`python3 -m unittest
tools.test_measure_typescript_esm_imports -q`, exit 0). They include controls
for missing assumptions, wrong/duplicate targets, wrong target file/span/kind,
missing/duplicate/unparseable evidence, Unknown with a target, same-offset
claims in another file, project-relative inspect paths, and symlink pinning.
The original runner's absolute-path assumption was corrected in `c209a22`
before any corpus measurement; no baseline was discarded or repeated.

## Scope and next action

This observation covers the cold CLI proof contract only. It does not rerun
Node runtime checks, the 19-step incremental sequence, watched MCP, ingestion
route checks, the 49-case dispatch corpus, or the 100-call real audit. No
language checkpoint or common stage gate was completed by this observation.

Implement the frozen E1–E8, cycle, mock, hook, and identity gates together with
snapshot invalidation. Then score a new after-observation, exercise every
ingestion route and the incremental sequence, and run the unchanged corpus,
real audit, and common gates. Keep this failed baseline. TypeScript remains
IN PROGRESS; Go remains NOT STARTED.
