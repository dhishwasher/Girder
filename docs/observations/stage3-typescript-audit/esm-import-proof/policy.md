# TypeScript relative ESM named-import proof policy v1

**Status: preimplementation draft for review, with corrections 1 and 2 applied**
(see [Correction history](#correction-history)). It is not frozen. First draft
`79b7e70`; correction base `dee89a5`. This is policy and fixtures only: no resolver or product code, no
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
- **E3 — Exact module resolution.** Node resolves a specifier as a URL, and
  URL parsing can disagree with any lexical path (runtime-checked; see E3-S1 to
  E3-S4). So the specifier must be spelled such that URL resolution and lexical
  path resolution provably name the same file:
  - **E3-S — Spelling.** Both conditions must hold:
    - the raw text between the quotes is byte-identical to its cooked string
      value; and
    - the cooked value matches
      `^(\./|(\.\./)+)([A-Za-z0-9_-]+(\.[A-Za-z0-9_-]+)*/)*[A-Za-z0-9_-]+(\.[A-Za-z0-9_-]+)*\.(ts|mts)$`,
      and its last segment does not end in `.d.ts` or `.d.mts`. The regex alone
      would accept a declaration file, so this condition is separate.

    A refusal is attributed to one hazard class, as follows. These labels are
    the ones used in the refusal table and the manifest.
    - **E3-S1 — percent-encoding.** Any `%`, including `%61`, `%2e`, `%2F`, and
      `%5C`. URL decoding changes the destination.
    - **E3-S2 — escape syntax or backslash.** The raw text differs from the
      cooked value (`\x61`, `\u…`, `\t`, `\\`, line continuations), or the
      cooked value contains a backslash, which URL parsing treats as a
      separator.
    - **E3-S3 — controls and whitespace.** ASCII controls, TAB, CR, LF, or
      spaces anywhere, raw or cooked, including leading and trailing ones,
      which URL parsing strips.
    - **E3-S4 — anything else outside the allow-list.** That covers non-ASCII
      characters, `?`, `#`, empty segments, and `.` or `..` segments after the
      leading run.
  - **E3-R — Root-relative resolution.** Segments are resolved lexically against the importer's
    root-relative directory. The result must stay inside the analyzed root and
    name exactly one indexed regular file (C-1, C-2), matched by byte-exact
    relative path. It is refused if another indexed file differs only by ASCII
    case.
  - **E3-X1 — Extensions: v1 accepts only `.ts` and `.mts`.**
    - `.tsx` needs JSX semantics, and Node type stripping cannot load it.
    - `.cts` is CommonJS, so the named import goes through CJS interop rather
      than an ESM live binding.
    - Declaration files (`.d.ts`, `.d.mts`) have no body.
    - `.js`, `.mjs`, `.cjs`, and extensionless specifiers are tooling-dependent.
    - Each of these needs its own semantics and proof.
    - The importer itself must also be a `.ts` or `.mts` file.
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

- **E8 — Mocking and hooks (MOCK-1..3, HOOK-1..3).**
  - The proof is refused if the snapshot contains a mock whose resolved module
    identity equals the target file.
  - Every importer is refused if any mock identity proof fails, or if the
    snapshot contains mocking configuration, hook registration, a pinned hook,
    mock, or runner library, or preload configuration.
  - See [Execution model](#execution-model-correction-2-closed) and
    [Mocking model](#mocking-model).

**Claim.** Certify Must with exactly one target, the E4 Function node. The reason
is `proven-typescript-relative-esm-named-import`, and the claim is placed at the
call expression's span. The node's call evidence must list the assumptions
`esm-native-execution` and
`no-unmodeled-module-hooks-loaders-or-mocks-outside-or-inside-snapshot`, in
addition to `indexed-source-snapshot`. The claim is conditional on them. There
is no May.

## Refusals (each Unknown with no target, plus the fixture that pins it)

| Rule | Refused form | Fixture(s) |
| --- | --- | --- |
| R-RES-1 | Extensionless or `.js`/`.mjs`/`.cjs` relative specifiers. Resolution depends on tsconfig or bundler settings outside the snapshot; Node ESM rejects both forms here. | `extensionless-specifier`, `js-extension-specifier` |
| E3-S1 | Percent-encoding. URL decoding loads a different file than the lexical path names (`%61`), encoded dots escape a directory, and `%2F`/`%5C` are rejected. | `percent-encoded-specifier`, `encoded-dot-segment`, `encoded-slash-specifier` |
| E3-S2 | Escape syntax and backslashes (a cooked backslash is a URL separator) | `backslash-specifier`, `hex-escape-specifier`, `line-continuation-specifier`, `tab-escape-specifier` |
| E3-S3 | Controls and whitespace, raw or cooked, which URL parsing strips | `tab-escape-specifier`, `raw-tab-byte-specifier`, `trailing-space-specifier` |
| E3-S4 | Non-ASCII characters (normalization differs across filesystems) | `non-ascii-specifier` |
| E3-X1 | `.tsx` and `.cts` specifiers | `tsx-extension`, `cts-extension` |
| MOCK-1 | A mock whose resolved identity is the target, in any spelling or any file, including `vi.mock(import(…))` | `mock-setup-different-spelling`, `vi-mock-import-expression`, `test-module-mocking` |
| MOCK-2 | A mock with an unresolvable specifier, or an aliased mocking method | `mock-alias-specifier`, `mock-non-literal-argument`, `mock-method-alias` |
| MOCK-3 | Mocking or aliasing configuration, or a `__mocks__` directory | `mock-config-present`, `manual-mocks-directory` |
| C-2 | A symlinked file or path component | `symlinked-target-file`, `symlinked-directory` |
| R-RES-2 | Bare, package, `node:` builtin, path-alias, or absolute specifiers | `bare-specifier` |
| R-RES-3 | Specifiers with a query or hash, which create a distinct module instance | `query-specifier` |
| R-RES-4 | Declaration files (`.d.ts`): no runtime body (D1) | `declaration-file-target` |
| R-GRAPH-1 | A parse error in either file | `target-parse-error` |
| R-GRAPH-2 | A module-path collision with any indexed file in any language (`.ts`/`.tsx`, `src/` stripping, `app.py`) | `module-path-collision-tsx`, `module-path-collision-src`, `mixed-language-module-collision` |
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
| HOOK-1 | Loader or hook registration in any indexed file, including preloads | `hook-register-node-module`, `hook-register-hooks-sync` |
| HOOK-2 | Imports of a pinned hook, mock, or runner library | `hook-unlisted-library` (`esmock`) |
| HOOK-3 | Preload flags in package scripts, or runner rc files | `hook-package-script-flags` |
| MOCK-2 (identity) | A mock whose E3-valid specifier is missing, unindexed, out of root, symlinked, or colliding | `mock-missing-module`, `mock-unindexed-file`, `mock-out-of-root`, `mock-symlink-identity`, `mock-colliding-identity` |
| (unrelated imports) | No refusal: builtin, type-only, resolvable sibling, and side-effect imports of other modules leave an eligible call Must | `unrelated-imports-allowed` (positive control), `mock-other-module` (positive control) |

## Execution model (correction 2: closed)

**Every Must is conditional.** It is sound only under the assumptions below.
- Each Must claim's call evidence must list them verbatim as assumptions:
  `indexed-source-snapshot`, `esm-native-execution`, and
  `no-unmodeled-module-hooks-loaders-or-mocks-outside-or-inside-snapshot`.
- No observation, audit, or report may describe these claims as unconditional
  runtime correctness. An audit counts them as "Must (conditional on the
  stated execution assumptions)".

- **EXEC-1 — ESM-native execution (assumed, not checked).**
  - Indexed `.ts`/`.mts` files are executed as ECMAScript modules with standard
    linking semantics, as with Node native type stripping. Imports are live,
    immutable, indirect bindings.
  - Transpiling to CommonJS, bundling, and re-evaluating or wrapping test
    runners are excluded from the model.
- **EXEC-2 — Unmodeled hooks, loaders, and mocks are excluded wherever they
  are.**
  - The model knows only the mocking APIs in MOCK-1 and the hook APIs and
    sources in HOOK-1..3.
  - Any other mechanism that can change what an import binds to, whether inside
    or outside the snapshot, is outside the model. That includes other loaders,
    preloads, import maps, runner plugins, and mocking libraries.
  - Claims carry the assumption above instead of being described as
    unconditional.
- **HOOK-1 — Hook registration refuses the whole snapshot.** Every
  relative-import proof in the snapshot is refused if any indexed file does any
  of the following:
  - imports `register` or `registerHooks` from `node:module`/`module`, by
    static import, `import()`, or `require`;
  - calls `.register(` or `.registerHooks(` on a namespace, default import, or
    `require` of `node:module`/`module`;
  - references those names in any other way, such as by destructuring.

  This holds whatever the file is, including indexed preloads. The
  runtime-checked fixtures `hook-register-node-module` (async `register`) and
  `hook-register-hooks-sync` (`registerHooks`) show both mechanisms redirecting
  `./app.ts` to another file.
- **HOOK-2 — Hook and mock libraries refuse the whole snapshot.** Every
  relative-import proof in the snapshot is refused if any indexed file imports
  or requires a package from the pinned list:
  - mocking libraries: `esmock`, `testdouble`, `quibble`, `proxyquire`,
    `mock-require`, `rewire`, `rewiremock`, `mockery`, `jest-mock`;
  - loaders: `@babel/register`, `ts-node`, `tsx`, `jiti`, `@swc-node/register`,
    `esbuild-register`;
  - runners: `vitest`, `@jest/globals`, `bun:test`.

  The runners are on this list because their own module systems are not
  EXEC-1. `node:test` is modeled through MOCK-1 and is not refused. The list is
  not exhaustive; EXEC-2 covers everything else.
- **HOOK-3 — Preload configuration refuses the whole snapshot.** Every
  relative-import proof in the snapshot is refused if any of the following is
  true:
  - any indexed `package.json` script or `NODE_OPTIONS` value contains
    `--import`, `--require`, `-r `, `--loader`, `--experimental-loader`,
    `--experimental-test-module-mocks`, or `--experimental-default-type`;
  - any `.npmrc`, `.mocharc.*`, `.taprc`, or `.c8rc*` file is present anywhere
    in the root.

  Hidden files are not indexed by default, so this check runs over the root's
  file listing, not just the index.

## Mocking model

- **MOCK-1 — Identity, not spelling.** Each mocking call's specifier is
  resolved by the E3 rules against the file that contains the call. If the
  resolved canonical identity equals the target file, every proof to that
  target is refused.
  - The spelling does not matter. The runtime-checked fixture
    `mock-setup-different-spelling` shows a setup file's
    `mock.module('../app.ts')` replacing the importer's static `./app.ts`
    binding.
  - `vi.mock(import('<spec>'), …)` is resolved through its `import()` argument.
  - The mocking calls covered are `vi.mock`, `vi.doMock`, `vi.unmock`,
    `vi.doUnmock`, `jest.mock`, `jest.doMock`, `jest.unmock`, `jest.setMock`,
    `jest.unstable_mockModule`, and `<x>.mock.module` (`node:test`).
  - These are matched by member property name on any object, so aliasing the
    object (`import { vi as v }`) is still caught.
  - A mock whose resolved identity is a different file does not refuse
    anything. The `mock-other-module` positive control stays Must.
- **MOCK-2 — Any failed mock identity proof refuses the whole snapshot.** If
  any mocking call's identity proof fails, every relative-import proof in the
  snapshot is refused. That includes a specifier that is E3-valid but:
  - names a missing file (`mock-missing-module`);
  - names an unindexed file, for example one under an excluded directory
    (`mock-unindexed-file`);
  - escapes the root (`mock-out-of-root`);
  - passes through a symlink (`mock-symlink-identity`). That fixture shows
    Node keying mocks by real path: `../lib/app.ts` through `lib -> real`
    replaced `./real/app.ts` at runtime;
  - is ambiguous by case;
  - collides on module path (`mock-colliding-identity`).

  An identity proof succeeds only when the specifier resolves, by E3-S and
  E3-R against the calling file, to exactly one indexed, regular,
  non-symlinked, in-root, non-colliding file. MOCK-2 also refuses the whole
  snapshot if any of the following occurs:
  - A mocking call has a specifier that is not a single E3-valid string literal
    or `import()` of one. That covers aliases such as `'@/app'`, bare
    specifiers, variables, template literals, and computed values.
  - A mocking method is referenced other than as a direct call. That covers
    destructuring (`const { mock } = vi`), assignment, and passing as a value.
- **MOCK-3 — Config-driven mocking refuses the whole snapshot.** Every
  relative-import proof in the snapshot is refused if it contains any of:
  - a test-runner or bundler configuration that can alias, auto-mock, or add
    setup files (`vitest.config.*`, `vite.config.*`, `vitest.workspace.*`,
    `jest.config.*`, or a `package.json` with a `jest` key);
  - any `__mocks__` directory.

  Configuration outside the snapshot (CLI flags, home-directory config) is
  outside the model (see EXEC-2).

## Canonical identity

- **C-1 — Root and file identity.**
  - The analyzed root is canonicalized once (real path) when analysis starts.
  - A file's identity is its root-relative path as discovered, using `/`
    separators and byte-exact.
  - The importer and target identities used by E3 and MOCK-1 are those root-
    relative paths. Two identities that differ only by ASCII case refuse.
- **C-2 — Symlinks.**
  - If a symlink appears in the resolved target path or the importer path
    (checked with `lstat` on every component under the root), the proof is
    refused.
  - Node follows symlinks to a real path, which then differs from the lexical
    identity (fixtures `symlinked-target-file`, `symlinked-directory`).
  - The current CLI walk skips symlinks, so such targets are simply not indexed.
- **C-3 — Every ingestion route must agree.**
  - Every route that can add or change indexed files must either attest to C-2
    or mark the snapshot uncertain, which refuses these proofs. The routes are
    CLI analyze, watch/incremental sync, MCP watched graphs, `load_file` and
    other library entry points, and plan projections.
  - No route may silently ingest a symlinked or out-of-root path as an ordinary
    file.

## Frozen fixtures

[Validation manifest](../../../../fixtures/typescript-esm-import-proof/v1/manifest.json),
SHA-256 `daa7310248f8e2adc3bdbbae340a3301dcca6b18983ca29ef3c6738bb97b94cd`:
- **Cases.** 73 case directories: 7 `must` and 66 `unknown`.
  - Correction 1 added 24 cases and correction 2 adds 9.
  - Correction 2 also tightened two assertions (`tsx-extension`,
    `declaration-file-target`) that had accepted any rejection. Their previous
    pins are kept under `superseded_pins`. Every other earlier entry is
    unchanged.
- **Must assumptions.** `must_claim_assumptions` pins the assumption strings
  every Must must carry.
- **Incremental sequence.** It now has 19 steps (`v1-incremental/`). For each
  step it pins the expected answer, the requirement that cold and incremental
  answers match, and a categorized Node runtime expectation. The steps cover:
  - an exporter mutation;
  - an import edit;
  - a path collision added and removed;
  - a mock of the target added and removed;
  - an unresolvable mock added and removed;
  - a mocking config added and removed;
  - a `__mocks__` tree added and removed;
  - an inert loader file;
  - a hook registration added and removed;
  - target deletion.

  The 7-step sequence from correction 1 is kept under
  `superseded_incremental_sequences`.
- **Symlinks** are pinned by their link text, never followed.
- These are proof-contract fixtures. They are not ground-truth samples and are
  not added to the 49-case denominator.

[Preimplementation checks](preimplementation-checks.json), from
[`run_preimplementation_checks.py`](run_preimplementation_checks.py) with Node
v22.22.0 only:
- **Pins and markers.** All pins match, every case has a single marker, and
  the raw bytes of all 10 spelling-hazard specifiers are verified.
- **Runnable cases.** Every runnable case passes `node --test`, asserting its
  recorded runtime truth.
  - Cases that test expected errors assert the specific error code or class.
  - Node arguments are passed per case: `--experimental-test-module-mocks` and
    `--import <preload>`.
- **Expected failures.** An expected failure in the incremental sequence counts
  only if Node exits nonzero *and* reports the stated category. Any other
  nonzero exit is a mismatch. [Negative controls](harness-negative-controls.json)
  show that the matcher rejects:
  - a wrong category;
  - an unexpected pass;
  - an unexpected failure.
- **Skipped cases** are listed with their reasons and not counted as passed.

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

1. The contract has exact answers on all 64 cases: each Must names exactly the
   pinned target, and each Unknown has no target and no missing evidence. That
   includes the cycle gate (`import-cycle`), all mock gates (MOCK-1..3), the
   mixed-language collision, and both symlink cases.
2. **Incremental invalidation.** Replay the pinned 19-step sequence through
   Girder's incremental update path (file-change sync and a watched MCP graph).
   After every step:
   - the importer's claim equals the step's `expected_class`;
   - it equals the claim from a cold rebuild of the same snapshot;
   - no claim names a deleted or replaced NodeId.

   The importer's evidence depends on files the importer does not import: the
   exporter, colliding paths, mocks, configuration, rc files, and hook
   registrations. Creating, editing, or deleting any of them must re-derive it.
   That includes removing an entire `__mocks__` tree.
3. **Ingestion routes (C-3).** For each route, a symlinked file or directory is
   either skipped or marks the snapshot uncertain, and never yields a Must. The
   routes are CLI analyze, watch/incremental sync, MCP, `load_file`, and plan
   projection.
3a. **Hooks and assumptions.** HOOK-1..3 refuse their fixtures. Every Must
   claim carries the three pinned assumption strings. The observation and
   audit report Must as conditional on them.
4. There is no new Must anywhere outside E1–E8 and HOOK-1..3. That includes every existing
   TypeScript fixture corpus and the structural-member contract.
5. Re-run the unchanged 49-case corpus and the unchanged 100-call audit. Publish
   every changed answer and keep the existing failure.
6. Run the four common gates serially, with `-j1`.

## Decision log

| Choice | Rejected alternative | Reason |
| --- | --- | --- |
| Raw = cooked plus an ASCII allow-list (E3-S1..S4) | Lexical normalization of whatever the string cooks to | URL parsing decodes `%xx`, treats `\` as a separator, and strips TAB, LF, and trailing spaces. Lexical and runtime destinations diverged in every hazard fixture. |
| `.ts`/`.mts` only (E3-X1) | Also `.tsx`/`.cts` | `.tsx` adds JSX semantics that Node cannot run. `.cts` is CJS interop, not an ESM live binding. |
| Mock refusal by resolved identity, with uncertainty refusing the whole snapshot (MOCK-1..3) | Literal specifier matching; ignoring configuration | A setup file's `'../app.ts'` mocked `./app.ts` at runtime. Aliases and configuration cannot be resolved from the snapshot. |
| Conditional Must under EXEC-1/2, with hook and preload sources refusing the whole snapshot (HOOK-1..3) | Allowing indexed preloads or unlisted APIs | A hook or preload can redirect any import (runtime-checked twice). Unmodeled mechanisms cannot be enumerated soundly, so they are excluded by assumption, and every claim says so. |
| Any failed mock identity proof refuses the whole snapshot (MOCK-2) | Refusing only literal or unresolvable specifiers | A symlinked mock path replaced a real-path import at runtime. Missing, unindexed, out-of-root, and colliding mocks cannot be proven to differ from the target. |
| Refuse symlinks; all ingestion routes agree (C-2/C-3) | Following symlinks | Node's real-path identity differs from Girder's lexical identity. A route that ingests links silently would bypass E3. |
| Explicit extensions only (R-RES-1) | Extensionless or `.js` → `.ts` mapping | That mapping depends on `moduleResolution`, `allowImportingTsExtensions`, bundlers, and files outside the snapshot. Node ESM rejects both forms (runtime-checked). |
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

Correction 2 closes the review's four points by refusing, conservatively, and
by stating conditional assumptions. These remain open; any of them may still
block a freeze:

1. **Specifier scope (R-RES-1, E3-X1).** Only explicit `.ts`/`.mts` with an
   ASCII allow-list is accepted, so the real audit is likely to stay unchanged.
2. **Cycles (R-CYCLE-1).** Refused, although they appear cycle-safe.
3. **Local export clauses (R-EXP-1).** Refused even for one hop.
4. **HOOK-2's package list is not exhaustive.** The residual risk is carried by
   the EXEC-2 assumption, not removed. Alternative: refuse any snapshot whose
   `package.json` declares any dependency outside an allow-list.
5. **HOOK-3's file-name and flag lists.** Detection is textual. A script that
   builds its flags dynamically, or a shell wrapper, is only covered by EXEC-2.
6. **EXEC-1** is assumed, not checked. `"type"` and tsconfig `module` are not
   read.
7. **Whole-file gates (E7).** Whether an escaped identifier or `eval` anywhere
   in the importer should refuse every call in it.
8. **Snapshot-wide refusals.** MOCK-2/3 and HOOK-1..3 refuse the whole snapshot.
   That is very conservative on real repositories: almost any repository using
   vitest or jest gets no Must at all from this rule.

## Correction history

- `79b7e70`: first draft. The high review returned NO-GO on four points:
  1. E3 lexical resolution can disagree with URL resolution (percent-encoding,
     backslash, controls and whitespace, escapes).
  2. `.tsx`/`.cts` need separate semantics.
  3. Mock refusal matched spellings rather than resolved identities, and
     ignored uncertain or config-driven mocking.
  4. Acceptance omitted the cycle and mock gates, mixed-language roots,
     incremental invalidation, and canonical root/symlink identity.
- **Correction 1** (this revision) addresses all four with:
  - E3-S1..S4 and E3-X1;
  - the execution assumptions;
  - MOCK-1..3;
  - C-1..3;
  - acceptance items 1–4;
  - 24 new runtime-checked or explicitly skipped fixtures and a 7-step
    incremental sequence.

  No product code was written and nothing was measured. The corpus and labels
  are unchanged.
- **Correction 2** (this revision), after the second high review NO-GO on
  `7fdfdd2`:
  - **Execution model.**
    - EXEC-1/2 replace the earlier "hooks outside the snapshot" wording.
    - HOOK-1..3 refuse the whole snapshot for hook registration (including
      indexed preloads), pinned hook, mock, or runner libraries, and preload
      flags or rc files.
    - Every Must is conditional on three pinned assumption strings, and audits
      must say so.
  - **MOCK-2** now refuses the whole snapshot for every failed mock identity
    proof: missing, unindexed, out of root, symlinked, case-ambiguous, or
    colliding. The `mock-symlink-identity` fixture shows a real-path mock
    replacing the import.
  - **The incremental sequence** grew from 7 to 19 steps, adding mock, config,
    `__mocks__`, and hook creation and removal. Each step has an expected answer
    and a cold-versus-incremental equality requirement.
  - **The harness** checks the error category of every expected failure. The
    matcher has negative controls, and two fixture assertions that accepted any
    rejection were tightened.
  - 9 new cases.
  - No product code, measurement, or corpus/label change. Still a draft.
