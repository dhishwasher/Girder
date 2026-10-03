# TypeScript relative ESM named-import proof policy v1

**Status: preimplementation draft for review.** Base `de2b364`, the local audit
checkpoint. This is policy and fixtures only: no resolver or product code, no
Girder run, no Cargo job, and no new measurement. Every frozen input is
unchanged: the [49-case corpus](../../../dispatch-corpus.json) (SHA-256
`9e3208a8f3fdc8faaee55cece63c1b5e25526322ebebbba915652f4f526ccd0a`), its labels,
the 100-call audit and its baselines, and the structural member policy. The
`typescript-structural-object-literal` failure (ambiguous `alice`/`bob` origin)
remains visible and is out of scope here.

## Scope

This policy covers one bounded dispatch case from the unchanged corpus:
`typescript-direct-cross-file`. In that case, `app.test.ts` contains
`import { target } from './app.ts'`, `app.ts` contains `export function target()`,
and the frozen label for `test_cross_file` is `must`. The local checkpoint
currently answers Unknown, which is conservative. This policy decides when such a
call may be certified Must, and it requires every other form to stay Unknown.

**It authorizes Must for one narrow form only. It never authorizes May.** It
neither widens nor weakens the lexical-binding or structural member policies. A
call that is not certified stays Unknown with no invented target, exactly as
today.

## Current state (read-only inspection, `de2b364`)

- `typescript.rs` records each named import as an exact `RustImportRef`
  (`local` → `<target module>::<name>`). It computes target modules with
  `import_module_path`, which accepts only specifiers starting with `.` and
  maps them through `module_path`.
- `module_path` strips `.ts/.tsx/.mts/.cts/.d.ts` and a leading `src/`.
  Distinct files can therefore share one module path (`app.ts`/`app.tsx`,
  `app.ts`/`src/app.ts`, `app.ts`/`app.d.ts`).
- `.js` and extensionless specifiers map to different or ambiguous paths.
- `claims::annotate` runs per file. A cross-file Must therefore needs a
  project-wide pass after every file is extracted. This policy does not
  prescribe how that pass is built, only what it may certify.

## Eligibility (all must hold, or the call is Unknown)

- **E1 — Import form.**
  - The importer contains exactly one static `import { name } from '<spec>'` or
    `import { name as local } from '<spec>'` binding for that local name.
  - The import is not `import type`, has no inline `type` modifier, and is not a
    default, namespace, side-effect-only, or dynamic `import()`.
  - Specifier attributes (`with { … }`) are refused.
- **E2 — Call form.** The marked call is a `call_expression` whose callee is the
  bare identifier `local`. These forms are refused: optional calls (`local?.()`),
  `new`, tagged templates, member callees (`ns.name()`, `local.call()`), and
  calls inside a refused structural subtree (B1).
- **E3 — Exact module resolution.**
  - `<spec>` starts with `./` or `../` and contains no `?` or `#`.
  - It ends in exactly `.ts`, `.tsx`, `.mts`, or `.cts`, but not `.d.ts`,
    `.d.mts`, or `.d.cts`.
  - Lexical normalization against the importer's directory must stay inside
    the analyzed root and name exactly one indexed source file, matched by
    byte-exact relative path.
  - The resolution is refused if another indexed file differs only by ASCII case.
- **E4 — Export form.**
  - The target file has exactly one top-level `export function name(…) { … }`
    (including `async`), extracted as exactly one Function node at that span.
  - Its module has no other declaration of `name` in any meaning space (value,
    type, namespace, or merged declaration).
  - It has no overload signatures for `name`, no `export * from`, no
    `export … from`, and no export clause that names `name`.
- **E5 — Importer binding is clean.**
  - Every identifier-shaped occurrence of `local` in the importer is either the
    import specifier's local name or the bare callee of a call.
  - Any other occurrence refuses the spelling for the whole file. That includes
    parameters, block or function declarations, writes, value uses such as
    `[local]`, shorthand properties, and type references.
  - This rule matches the lexical-binding rule (policy v1, obligation 3).
- **E6 — Target binding is clean.** In the target module, every occurrence of
  `name` is its declaration name or the bare callee of a call. This refuses any
  module-internal reassignment of the live binding.
- **E7 — File and graph gates.**
  - Neither file has a parse error.
  - Neither file trips the existing gates: duplicate semantic path, transformed
    scope (decorators), escaped identifier, `eval`, or `with`.
  - Both files are modules.
  - No other indexed file shares the target file's module path.
  - The importer's recorded import target equals the target Function node's
    semantic path exactly.

**Claim.** Certify Must with exactly one target, the E4 Function node. The reason
is `proven-typescript-relative-esm-named-import`, and the claim is placed at the
call expression's span. There is no May.

## Refusals (each Unknown with no target, plus the fixture that pins it)

| Rule | Refused form | Fixture(s) |
| --- | --- | --- |
| R-RES-1 | Extensionless or `.js`/`.mjs`/`.cjs` relative specifiers. Resolution depends on tsconfig or bundler settings outside the snapshot; Node ESM rejects both forms here. | `extensionless-specifier`, `js-extension-specifier` |
| R-RES-2 | Bare, package, `node:` builtin, path-alias, or absolute specifiers | `bare-specifier` |
| R-RES-3 | Specifiers with a query or hash, which create a distinct module instance | `query-specifier` |
| R-RES-4 | Declaration files (`.d.ts`): no runtime body (D1) | `declaration-file-target` |
| R-GRAPH-1 | A parse error in either file | `target-parse-error` |
| R-GRAPH-2 | A module-path collision (`.ts`/`.tsx`, `src/` stripping) | `module-path-collision-tsx`, `module-path-collision-src` |
| R-IMP-1 | `import type` and inline `type` specifiers (erased, so no runtime binding) | `type-only-import`, `inline-type-specifier` |
| R-IMP-2 | Default and namespace imports | `default-import`, `namespace-import` |
| R-IMP-3 | Dynamic `import()` bindings | `dynamic-import` |
| R-EXP-1 | Local or renamed export clauses (`export { target }`, `export { impl as target }`) | `local-export-clause`, `renamed-export` |
| R-EXP-2 | Re-exports and barrels (`export … from`, `export *`), including link-time ambiguous stars | `reexport-barrel`, `export-star-barrel`, `ambiguous-export-star` |
| R-EXP-3 | A target module containing any `export *` | `target-has-export-star` |
| R-EXP-4 | Duplicate export names (early SyntaxError) | `duplicate-export` |
| R-EXP-5 | Overload signatures for the name | `overload-signatures` |
| R-EXP-6 | Generator targets | `generator-target` |
| R-EXP-7 | Declaration merging with the name | `declaration-merging` |
| R-BIND-1 | Importer shadowing, writes, or non-call value uses | `shadowed-parameter`, `shadowed-block-declaration`, `importer-writes-binding`, `escaping-value-use` |
| R-BIND-2 | Reassignment of the live export binding in the target | `target-reassigns-binding` (the runtime returns `'swapped'`, so Must would be unsound) |
| R-BIND-3 | `eval` in either file | `target-eval` (the runtime returns `'evaled'`, so Must would be unsound), `importer-eval` |
| R-BIND-4 | Escaped identifier spellings | `escaped-callee` (contains the literal bytes `target`) |
| R-CALL-1 | Optional, `new`, and member callees | `optional-call`, `new-expression`, `namespace-import` |
| R-CYCLE-1 | Importer and target in the same static-import cycle (strongly connected component) of the indexed snapshot | `import-cycle` |
| R-MOCK-1 | Test-framework module mocking or hoisted mocks in any indexed file that name the specifier (`vi.mock`, `vi.doMock`, `jest.mock`, `jest.unstable_mockModule`, `mock.module`) | `test-module-mocking` |

Assumptions, in addition to `indexed-source-snapshot`: no loader hooks,
`--import` preloads, or import maps outside the snapshot.

## Frozen fixtures

[Validation manifest](../../../../fixtures/typescript-esm-import-proof/v1/manifest.json),
SHA-256 `dc34fe028fa9a7879a296981d6041a719b2e0d1fd56aa61a55e2b17c06824b27`:
- 40 case directories: 5 `must` and 35 `unknown`.
- Each case marks exactly one call with `/* claim */`.
- Every file in every case is pinned by SHA-256, and the frozen corpus inputs are
  pinned too.
- Must cases name the exact predicted target path. These are `explicit-ts`,
  `aliased-import`, `mts-extension`, `parent-directory`, and `async-target`.
- These are proof-contract fixtures. They are **not** ground-truth samples and
  are **not** added to the 49-case denominator.

[Preimplementation checks](preimplementation-checks.json), produced by
[`run_preimplementation_checks.py`](run_preimplementation_checks.py) with Node
v22.22.0 only:
- All hashes match, every case has a single marker, and the escaped-callee
  bytes are present.
- 38 of 40 cases pass `node --test`, asserting the runtime truth recorded in the
  manifest.
- Two are skipped and not counted as passed. `declaration-merging` uses a
  runtime namespace that type stripping can't execute, and
  `test-module-mocking` requires vitest.

## Predictions (stated now, not claimed)

- **Corpus.** `typescript-direct-cross-file` (`./app.ts` specifier) is predicted
  to move from conservative to exact. No other corpus case uses a relative
  import, so the other 48 cases are predicted unchanged. The structural
  object-literal case is predicted to remain failed as ambiguous. Any improvement
  is claimed only after implementation, a re-measurement, and the four gates.
- **Real-repository audit.** No prediction. Real repositories usually use
  extensionless or `.js` specifiers, which R-RES-1 refuses. The 100-call audit
  may therefore not move. It must be re-run, and E7 interactions must be
  measured, not assumed.

## Acceptance (after implementation, not now)

1. The contract has exact answers on all 40 cases: each Must names exactly the
   pinned target, and each Unknown has no target and no missing evidence.
2. There is no new Must anywhere outside E1–E7. That includes every existing
   TypeScript fixture corpus and the structural-member contract.
3. Re-run the unchanged 49-case corpus and the unchanged 100-call audit. Publish
   every changed answer and keep the existing failure.
4. Run the four common gates serially, with `-j1`.

## Decision log

| Choice | Rejected alternative | Reason |
| --- | --- | --- |
| Explicit `.ts`/`.tsx`/`.mts`/`.cts` only (R-RES-1) | Extensionless or `.js` → `.ts` mapping | That mapping depends on `moduleResolution`, `allowImportingTsExtensions`, bundlers, and files outside the snapshot. Node ESM rejects both forms (runtime-checked). |
| Direct `export function` only (E4/R-EXP-1) | Following export clauses and re-exports | Keeps the binding chain to one hop. ResolveExport over stars can be ambiguous. |
| Refuse cycles (R-CYCLE-1) | Allow them, since module function declarations are instantiated in InitializeEnvironment before evaluation | Conservative v1 choice. The fixture shows the cyclic call still reaches the target, so this refusal costs recall, not soundness. |
| The lexical clean rule on both sides (E5/E6) | Finer flow analysis | Imports are live bindings. A write or eval in the exporting module changes what the importer calls (runtime-checked). |
| Module-path collision refusal (R-GRAPH-2) | Disambiguating by file | Girder's semantic path is per module path; a collision makes the node identity uncertain. |
| Mock refusal (R-MOCK-1) | Ignoring test tooling | Hoisted mocks rebind static imports in common runners. |

Basis, checked on 2026-10-03 against `tc39/ecma262` `spec.html` (raw source
SHA-256 `0ceb261e…`; tc39.es itself is blocked in this session):
- `sec-source-text-module-record-initialize-environment`: module function
  declarations are instantiated before evaluation.
- `sec-resolveexport`: export resolution can return `~ambiguous~`.
- `sec-module-semantics-static-semantics-early-errors`: duplicate exported names
  are a SyntaxError.
- `sec-imports-static-semantics-early-errors`: duplicate import bound names are a
  SyntaxError.
- `sec-createimportbinding`: import bindings are indirect bindings to the
  exporting module's binding.
- `sec-performeval`: direct eval can assign existing bindings.

Node type stripping (https://nodejs.org/api/typescript.html) was used only for
runtime checks.

## Unresolved decisions for review

1. **Specifier scope (R-RES-1).** Explicit extensions only. This likely leaves
   real repositories unchanged. Widening to `.js`→`.ts` (NodeNext) or
   extensionless resolution needs a separately frozen resolution model.
2. **Cycles (R-CYCLE-1).** Refused, although the spec and the fixture suggest a
   direct function declaration is cycle-safe given E6.
3. **Local export clauses (R-EXP-1).** `function f(){}; export { f }` is refused
   even though it is a one-hop binding.
4. **Mock detection breadth (R-MOCK-1).** The listed APIs are matched across all
   indexed files. Should any call named `mock` anywhere refuse instead?
5. **Gate parity.** Should a Must here also require the importer to pass the
   existing whole-file lexical gates (no escaped identifiers or `eval` anywhere
   in the file), as E7 currently states?
