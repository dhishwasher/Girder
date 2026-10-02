# TypeScript registration identity repair — preimplementation policy

Candidate before implementation: `f741f53`. The frozen 100-call baseline and
all dispatch expectations remain unchanged. This is an intermediate Stage 3
repair, not the TypeScript language completion checkpoint.

## Proof restrictions observed before implementation

[Label-derived profile](before-profile.json): 47 ground-truth Must calls are
currently Unknown. Of these, 37 target another file; the remaining ten target
eight nested functions and two constructors. None satisfies the current
same-file, top-level function-declaration candidate rule. The shape counts and
individual same-file sites are retained in the profile. These are counts from
frozen labels and source inspection, not instrumented causal gate counts.

`mapper/claims.rs::annotate` additionally requires an error-free parse, unique
node IDs, no transformed scope, an import/export in TypeScript, and only
declaration/direct-call occurrences of the candidate name. Decorators and
duplicate IDs block the whole file. Removing either gate alone cannot make
these 47 sites proven calls. This repair does not relax any proof gate.

## Identity policy

The existing [collision fixture](../collision-repro/typescript-repro/sample.ts)
loses one test body because `describe` ancestry is discarded and repeated test
titles become identical node IDs. Retain that fixture unchanged.

For every recognized literal-title registration, encode a synthetic path
segment with its kind (`@describe` or `@test`), the literal title's UTF-8 bytes
in hexadecimal, and a one-based occurrence number among identical registrations
in that scope. `it` and `test` share the test kind. Suite ancestry contributes
to descendant paths. Suite scopes need not introduce graph nodes: containment
continues to refer to the nearest actual graph owner. Test display names remain
the original title. Definitions inside each callback inherit its new path.

This deliberately changes existing registration paths, including unique tests;
graphs must be rebuilt and stored test paths refreshed. Ordinary definitions
outside suites keep their existing paths. Hex encoding prevents title delimiter
collisions. Occurrences distinguish identical sibling suite or test titles.
IDs must survive body edits, unrelated byte shifts, and inserting differently
named siblings. Inserting or reordering identically named siblings may rekey
them: no stable identity or verified-edit guarantee is claimed for that case.
Only already-recognized registrations are covered; aliases, computed titles,
parameterized registrations, and runtime registration counts remain unmodeled.

## Precommitted acceptance and checks

1. Both identically named tests in the original collision fixture survive with
   distinct IDs, original source bodies, and correctly attributed call evidence.
   No registration body may replace another one.
2. Add a harder fixture before implementation: identical sibling suites, repeated
   `it`/`test` titles within one suite, delimiter-like titles, and nested helpers.
   Check caller attribution and absence of cross-body call edges, not just counts.
3. Check identity stability under body edits, leading whitespace, and insertion
   of differently named registrations. Retain the explicit same-name limitation.
4. Build and run focused TypeScript builder checks serially with logs. Publish
   failures as well as the eventual result. Rebuild the CLI before claiming
   end-to-end evidence; measure the unchanged collision fixture and both real
   compiler files containing audit sites 23 and 91.
5. Keep the structural-object-literal corpus failure and the empty-Must baseline
   public. Extracting a missing member alone does not establish a bounded target
   set. A separate proof policy is required before implementing call promotion.

The four common gates remain required at the next language checkpoint. This
intermediate identity repair cannot mark TypeScript DONE or unblock Go.
