# TypeScript lexical-binding proof policy v1

Preimplementation candidate: `105f87d`; binary from `67d65f6`, SHA-256
`13976d976157318878512ae6eb5458f0cf5ca4f19b5c642a9bf16bbde54381f7`.
The existing language policy, 49-case dispatch corpus, and frozen 100-call
real-repository audit remain unchanged. This policy extends lexical proofs and
strengthens refusals; it does not certify structural or imported calls.

## Proof obligations

1. Retain parse-error, duplicate-node-identity, and transformed-scope refusals.
   TypeScript must have an import/export directly at the source root. An export
   inside a namespace is insufficient to establish module scope.
2. A candidate is a function declaration with a body and exactly one matching
   extracted Function node at the declaration's byte span. Its binding scope is
   either the module root (including a root export wrapper) or the direct body
   of a function declaration, function expression, arrow function, or method.
   Other block declarations, generator contexts, constructors, overloads,
   variable-bound callables, and imports remain outside this first proof subset.
3. Index every identifier-shaped syntax kind, including shorthand properties
   and shorthand binding patterns. For a candidate spelling, every occurrence
   in the file must be either its own declaration-name node or the bare
   identifier callee of a call expression. Assignments, other declarations,
   parameters, imports, type references, property accesses, and escaping values
   therefore refuse the candidate, even where a finer proof might be possible.
4. Every admitted call must be lexically inside that candidate's binding scope.
   A sole same-spelled declaration somewhere in the file is not enough.
   Recursive calls and calls before a function declaration are eligible under
   the same obligations. Nested closures may refer to an enclosing binding.
5. Refuse all such TypeScript proofs in a file containing an identifier-shaped
   `eval`, a `with` statement, or an escaped identifier. Unicode spelling must
   not hide a rebind or eval use. This intentionally includes conservative
   refusals for harmless/indirect eval and unrelated escaped identifiers until
   those forms receive a separate normalization/scope proof.
6. Emit Must only for the certified declaration's NodeId, at the actual call
   expression span. Report a TypeScript lexical-binding reason rather than
   labeling a nested proof top-level. Every unproved call remains Unknown with
   no invented target. No May enumeration is added by this change.

These obligations apply to existing top-level TypeScript proofs as well as new
nested proofs. Rust, Python, and Go proof paths are unchanged. Candidate and
identifier indexing must avoid rescanning an entire large compiler file for
each nested declaration. No new runtime dependency or network behavior is needed.

The semantic basis is lexical binding and function-declaration instantiation;
direct eval can use its caller's environment. See the published ECMAScript
[function instantiation rules](https://tc39.es/ecma262/multipage/ordinary-and-exotic-objects-behaviours.html#sec-functiondeclarationinstantiation)
and [eval environment rules](https://tc39.es/ecma262/multipage/global-object.html#sec-performeval),
checked on 2026-10-02. The refusal rules above are deliberately stricter than
the language semantics; they are not a claim to implement the whole specification.

## Frozen checks and acceptance

The checked-in [v1 binding corpus](../../../../fixtures/typescript-binding-corpus/v1/manifest.json)
contains 22 cases: six required Must answers and sixteen required Unknown
answers. These are implementation proof-contract cases, **not independent
ground-truth samples or additions to the published dispatch benchmark count**.
The manifest pins every source hash and marks one call per file. Required Must
answers must name the exact marked declaration. Unknown answers may not contain
a guessed target. No missing call evidence may count as a passing refusal.

Nineteen files use directly executable JavaScript syntax and have separate Node
runtime probes. The named-expression-shadow probe does not execute its marked
conditional call; it cannot dynamically disprove that call's target. Three
TypeScript-only forms are explicitly skipped at runtime, not counted as passed.
Probes cannot turn a proof-contract mismatch into an unsoundness claim unless
the marked call actually executes and contradicts the claimed declaration.

Before product changes, run and publish the rebuilt current CLI against all 22
cases and preserve each mismatch. Afterwards, run the same sources and scoring
rules. Acceptance for this proof slice: 22/22 exact contract answers, nonempty
correct Must targets, zero missing evidence, zero unexpected targets, and all
19 runtime probes matching their frozen outcomes. Retain every failed run.

Run focused builder regression checks for this corpus and existing TypeScript
identity/extraction checks. Before language completion, rerun the full dispatch
corpus, unchanged 100-call audit, and all four common gates serially. Publish
precision numerators/denominators, recall availability, and remaining Unknown
holes. Passing these 22 fixtures alone does not satisfy the language criterion
or authorize starting Go. Structural target extraction and the existing failed
structural-object-literal origin remain outstanding work.
