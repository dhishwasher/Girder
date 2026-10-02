# TypeScript registration identities: measured repair

Policy: `69b4c7f`; [file-location erratum](policy-erratum.md): `f16f29b`.
Implementation: `67d65f6`. Offline measurement runner: `a36c60a`.
The original fixture, harder fixture, frozen labels, and repository pins were
not changed to obtain this result.

[Seven focused builder tests passed](focused-tests-1.json). The rebuilt CLI
also passed the [four-case observation](cli-observation-1/run.json):

| Source | Tests retained before | Tests retained after | Recovered bodies |
| --- | ---: | ---: | ---: |
| Original collision fixture | 3 | 4 | 1 |
| Harder repeated-suite/sibling-title fixture | Not measured | 5 | Not compared |
| date-fns `src/intlFormatDistance/test.ts` | 47 | 89 | 42 |
| TypeScript `src/testRunner/unittests/tsserver/projectReferences.ts` | 26 | 30 | 4 |

No previously retained registration offset disappeared. No duplicate-path
boundary remains in these four analyzed files. The two real audit calls now
have the correct test as their evidence owner:

- Index 23: built-in `Date`, still Unknown, exact against the frozen label.
- Index 91: nested `verifySolutionScenario`, still Unknown, conservative against
  the frozen Must label. This repair does not prove its target.

The unit checks additionally verify both original bodies' call edges, nested
helper containment and evidence ownership, no cross-body target attribution,
and stable IDs under body edits, leading whitespace, and insertion of unrelated
registrations. There is no claim that unresolved nested helper calls were
resolved merely by recovering their bodies.

## Evidence and limitations

[CLI build](cli-build-1.json) succeeded with one Cargo job and incremental
compilation disabled. Binary SHA-256:
`13976d976157318878512ae6eb5458f0cf5ca4f19b5c642a9bf16bbde54381f7`.
All eight analyze/inspect commands exited zero; total recorded command time was
5.044 seconds. Raw outputs and stderr are retained in `cli-observation-1/`.
The runner checked each original module source fingerprint before comparison;
baseline node records, source hashes, commands, and recovered byte offsets are
published. Workloads ran serially, offline, without new dependencies.

The real files were copied unchanged into isolated analysis roots. This is a
measurement of extraction and call ownership, **not a whole-repository
resolution rerun or the 100-site language acceptance measurement**. The public
empty-Must baseline and structural-object-literal corpus failure remain open.
The four common gates are still required at the next language checkpoint.

Registration paths deliberately changed. Rebuild graphs and refresh stored
paths; displayed test names remain unchanged. Identical sibling insertions or
reordering can rekey their ordinal identities. Dynamic/aliased/computed-title
registration discovery remains outside this repair. These limitations are
unchanged from the preimplementation policy.

Next: precommit TypeScript lexical-binding proofs and their hostile cases,
implement without relaxing existing proof gates, and measure the entire frozen
100-call audit and dispatch corpus before claiming language completion.
