# Frozen 100-call audit after lexical-binding and identity repairs

Product implementation: `5733be6`. Measurement revision: `3bb01f4`.
Binary SHA-256: `fe6901793bb8055a66359ad235e631219ba87b3f6ab96a396e4741925a7deaef`.
The [run record](run.json) contains commands, identities, source inventories,
environment, timings, and exit statuses; raw analyze/inspect outputs are retained.

Fresh offline extractions matched all four pinned archive hashes and complete
source inventories. Every frozen policy, label, and scorer input hash matched.
All 110 entries were accounted for: **100 actual calls and 10 retained noncalls**.
No case was relabeled or discarded.

| Measure | Published before | This candidate |
| --- | ---: | ---: |
| Exact answers | 53 | 55 |
| Conservative answers | 47 | 45 |
| Scored unsound answers | 0 | 0 |
| Observed Must | 0 | 2 |
| Correct Must targets | 0 | 2 |
| Must precision | Undefined | 2/2 = 1.000 |
| Observed Unknown | 100 | 98 |
| Observed May | 0 | 0 |

**This is only two measured Must predictions.** It is not broad accuracy evidence
for every claim emitted across these repositories. No compiler test was executed
as a dynamic oracle for these two calls; precision is against the independently
frozen source-reviewed labels. The sample contains no ground-truth May calls,
so May recall remains undefined rather than 1.000 or a measured zero.

## Exact target checks

Only two classifications changed, both Unknown to Must:

- Index 37: `emitter.ts:2282` calls `popNameGenerationScope`, declared at
  `emitter.ts:5313`, in the shared `createPrinter` lexical scope.
- Index 91: `projectReferences.ts:1189` calls `verifySolutionScenario`, declared
  in the enclosing suite callback at `projectReferences.ts:1082`. Its test body
  was previously affected by the published registration-identity collision.

The unchanged scorer checks the claimed NodeId against the frozen target file
and declaration line. Both caller and declaration excerpts were also inspected
directly in the fresh pinned source. [Changed answers](changed-answers.json)
retains all 38 changed result records: two classification changes and 36 changes
to ownership/reason details without a classification change. No other cell
became more or less accurate.

## Remaining holes and limits

Forty-five ground-truth Must calls still receive Unknown: 37 cross-file targets
and eight same-file targets. Their shapes include 15 decorator calls, five
constructors, three optional calls, nine plain calls, seven method calls, and
six qualified calls. Imports, decorators, constructors, escaping/ambiguous uses,
and structural dispatch still need proofs beyond this restricted lexical pass.
The structural-object-literal dispatch origin remains an outstanding separate
failure; this real-repository audit does not score or repair that case.

The compiler analyze command took 355.820 seconds versus the published
580.707-second before-run; other analyze times were 4.164 seconds (Zod),
14.779 (date-fns), and 2.662 (class-validator). All eight analyze/inspect commands
exited zero. These are single-run timings on the shared machine, with both
identity and proof changes since the before-run; they do not isolate a speedup.

The real-audit acceptance helper passes its existing 100-site/nonempty-Must/
zero-unsound conditions. **TypeScript is not DONE:** the dispatch corpus,
remaining extraction work, and four common language gates still have to be
completed and assessed under the original language criterion. Go remains
NOT STARTED.
