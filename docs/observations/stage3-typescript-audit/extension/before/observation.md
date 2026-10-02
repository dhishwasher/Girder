# Expanded TypeScript before-observation: criterion still unmet

Measured at `e1f911b`, using the unchanged product candidate from `c441899`:
binary SHA `da59183f4543a51a48e584e43ed06fcb76f5d6fc21ce8ba4b4763d000442f552`.
Policy, reserve, and labels were committed separately before measurement at
`cef8681`, `9aa8e3d`, and `e1f911b`. No TypeScript resolver changed in this work.

| Cohort | Actual calls | Noncalls retained | Exact | Conservative | Unsound | Must precision |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| Original | 97 | 8 | 51 | 46 | 0 | undefined (0 emitted) |
| Expanded | 100 | 10 | 53 | 47 | 0 | undefined (0 emitted) |

Every original answer is unchanged, including caller and reason. The new reserve
prefix contributed three calls and two declarations; no entry was skipped.
All selected entries relocated. Ground truth has 47 Must and 53 Unknown calls;
Girder answered Unknown for all 100. There are no labeled May sites in this
sample, so it cannot measure May recall. This is a static call-classification
audit, not a dynamic test-impact recall measurement.

**The language criterion still fails:** the sample-size deficiency is repaired,
but Must precision is undefined and the required nonempty Must set is absent.
Zero unsound answers under all-Unknown output is not evidence of useful resolved
dispatch. Go remains gated on completion of TypeScript; no language is added.

The three new calls were:

- `typescript/src/compiler/checker.ts:13509`: optional Array.find, correctly
  Unknown because the runtime implementation is external.
- `zod/deno/lib/__tests__/array.test.ts:25`: proven ZodType.parse binding under
  the frozen snapshot assumptions, still conservatively Unknown.
- `date-fns/src/isSameSecond/test.ts:8`: builtin Date construction, correctly
  Unknown. Ambient declarations do not supply its implementation.

All four snapshots were freshly analyzed and inspected serially, offline, after
separate acquisition verified their original archive sizes/hashes. Commands,
exits, elapsed time, and raw-output hashes are in `run.json`; compressed raw
outputs and stderr are retained. The compiler analysis took 580.707 seconds,
up from the older methodology's 276-second feasibility measurement. Hardware
load was not controlled, so this is an observed timing increase, not an isolated
implementation regression claim. All eight commands exited zero.

Reproduce in fresh output/scratch directories using the prepared pinned archives
and `python3 -m tools.score_typescript_audit_extension`. The original and combined
cohorts use the same extracts and unchanged scorer. Existing outputs deliberately
refuse overwrite. `changed-original-answers.json` preserves the full comparison.

Validation before measurement: 79 tests passed across the TypeScript selector,
TypeScript scorer, sample-size acceptance helper, and bounded acquisition tests.
Cargo/clippy/fmt/npm gates were not rerun: no product code changed and this is
an incomplete before-observation, not a language-completion checkpoint.

Next: profile the proof gates blocking the 47 ground-truth Must calls, freeze
an implementation policy, and address the already-published duplicate test-name
identities and unresolved structural-object-literal origin. Preserve this
baseline and the existing corpus failure; never change them to fit a resolver.
