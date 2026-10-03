# TypeScript relative ESM named-import proof policy v1

**Status: preimplementation draft for review, with correction 1 applied**
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

- **E8 — Mocking (MOCK-1..3).**
  - The proof is refused if the snapshot contains a mock whose resolved module
    identity equals the target file.
  - It is refused for every importer if the mocking environment is uncertain.
  - See [Mocking model](#mocking-model).

**Claim.** Certify Must with exactly one target, the E4 Function node. The reason
is `proven-typescript-relative-esm-named-import`, and the claim is placed at the
call expression's span. There is no May.

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
| (unrelated imports) | No refusal: builtin, type-only, resolvable sibling, and side-effect imports of other modules leave an eligible call Must | `unrelated-imports-allowed` (positive control), `mock-other-module` (positive control) |

## Execution assumptions (outside the proof model, stated)

- **ESM-preserving execution.**
  - Indexed `.ts` and `.mts` files are executed as ECMAScript modules under
    standard linking semantics, as with Node native type stripping. Imports are
    live, immutable, indirect bindings.
  - A pipeline that rewrites modules is outside the model. That includes
    transpiling to CommonJS, bundling with scope hoisting, and test runners that
    re-evaluate or wrap modules other than through the mocking APIs handled
    below.
  - A `.ts` file executed as CommonJS (for example, `"type": "commonjs"` with a
    loader that accepts ESM syntax) is outside the model.
- **No hooks outside the snapshot.** There are no loader hooks, `--import`
  preloads, `--experimental-*` loaders, or import maps outside the indexed
  snapshot.
  - Preloads inside the snapshot are only covered as far as MOCK-1..3 inspect
    them.
  - A preload's command-line flag is itself outside the snapshot. MOCK-3 is the
    conservative backstop.
- **`indexed-source-snapshot`**, as in every policy.

These are honest limits. A claim is sound only under them, and the reason string
must carry them as assumptions.

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
- **MOCK-2 — Uncertain mocks refuse the whole snapshot.** Every relative-import
  proof in the snapshot is refused if any of the following occurs:
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
  outside the model (see the execution assumptions).

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
SHA-256 `05eeb33be49b972538f936d68aaf8ee9207f706e647e3834cfa2d65e4cce31cd`:
- 64 case directories: 7 `must` and 57 `unknown`. Correction 1 adds 24 cases;
  the entries for the original 40 are unchanged.
- An incremental sequence of 7 steps (`v1-incremental/`, pinned in the same
  manifest) covers exporter mutation, an import edit, a path collision added and
  removed, and target deletion.
- Symlinks are pinned by their link text, never followed.
- Each case marks exactly one call with `/* claim */`.
- Every file in every case is pinned by SHA-256, and the frozen corpus inputs are
  pinned too.
- Must cases name the exact predicted target path. These are `explicit-ts`,
  `aliased-import`, `mts-extension`, `parent-directory`, `async-target`,
  `unrelated-imports-allowed`, and `mock-other-module`.
- These are proof-contract fixtures. They are **not** ground-truth samples and
  are **not** added to the 49-case denominator.

[Preimplementation checks](preimplementation-checks.json), produced by
[`run_preimplementation_checks.py`](run_preimplementation_checks.py) with Node
v22.22.0 only:
- All pins match, every case has a single marker, and the raw specifier bytes
  of all 10 spelling-hazard cases are verified.
- 58 of 64 cases pass `node --test`, asserting the runtime truth recorded in the
  manifest. The two mock cases run with `--experimental-test-module-mocks`, and
  one of them with `--import`.
- Six are skipped and not counted as passed: one needs runtime namespaces and
  five need vitest.
- All 7 incremental steps produce their expected Node outcome on a temporary
  copy.

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
2. **Incremental invalidation.** Replay the pinned sequence through Girder's
   incremental update path (file-change sync and a watched MCP graph). After
   every step:
   - the importer's claim equals the claim from a cold rebuild of the same
     snapshot;
   - no claim names a deleted or replaced NodeId.

   The importer's evidence depends on other files: the exporter, colliding
   paths, mocks, and configuration. A change to any of them must re-derive it.
3. **Ingestion routes (C-3).** For each route, a symlinked file or directory is
   either skipped or marks the snapshot uncertain, and never yields a Must. The
   routes are CLI analyze, watch/incremental sync, MCP, `load_file`, and plan
   projection.
4. There is no new Must anywhere outside E1–E8. That includes every existing
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

The correction resolves the review's four required items by refusing,
conservatively. These remain open; any of them could block a freeze:

1. **Specifier scope (R-RES-1, E3-X1).** Only explicit `.ts`/`.mts` with an
   ASCII allow-list is accepted. Real repositories mostly use extensionless or
   `.js` specifiers, so the real audit is likely to stay unchanged.
2. **Cycles (R-CYCLE-1).** Refused, although the spec and the fixture suggest a
   direct function declaration is cycle-safe given E6.
3. **Local export clauses (R-EXP-1).** Refused even for one hop.
4. **Mocking coverage (MOCK-1..3).** The API list, config file names, and
   `__mocks__` rule cover the named tools (vitest, jest, `node:test`). Other
   runners, such as bun, `ava` with `esmock`, `testdouble`, or `proxyquire`-style
   loaders, are not enumerated and fall only under the execution assumption
   "no hooks outside the snapshot". Choose between:
   - **(a)** listing more APIs; or
   - **(b)** a broader rule: refuse whenever any test-runner dependency other
     than `node:test` appears.
5. **Preload flags.** `--import`/`--require` preload files are named on the
   command line, outside the snapshot. MOCK-1 inspects mocking calls in every
   indexed file, which covers preloads that are indexed. A preload outside the
   root, or one that mocks through an unlisted API, is only covered by the
   assumption.
6. **ESM-preserving execution** is an assumption, not a check. v1 does not read
   `package.json` `"type"` or tsconfig `module` settings.
7. **Whole-file gates (E7).** Whether an escaped identifier or `eval` anywhere
   in the importer should refuse every call in it.

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
