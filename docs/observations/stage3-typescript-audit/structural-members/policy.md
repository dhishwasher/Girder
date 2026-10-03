# TypeScript structural callable member identity policy v1

**Status: preimplementation draft for review.** Base `e3812c5`. Frozen inputs,
unchanged by this commit: [dispatch corpus](../../../dispatch-corpus.json)
SHA-256 `9e3208a8f3fdc8faaee55cece63c1b5e25526322ebebbba915652f4f526ccd0a`;
original fixture `fixtures/dispatch-corpus/typescript/structural-object-literal/app.test.ts`
SHA-256 `5b85efacdb41b4a02b9b306a58339e57f4dc3950d48210d7bed1dc01af346e1d`.
No Rust product code changed, no Girder binary was run, and no Cargo job ran.
Nothing here is a measurement. The [lexical-binding policy](../lexical-bindings/policy.md),
the classification policy, the 49-case corpus, and the 100-call audit are unchanged.

This policy only assigns **identities** to callable members of object literals
and fixes which declarations can never be executable targets. **It authorizes no
Must or May proof.** Structural May enumeration and any member-call Must rule need
their own precommitted rules. Until those exist, every call through a member
stays Unknown. That outcome is conservative, not an error.

## Current extractor state (read-only inspection of `crates/aether-builder/src/mapper/typescript.rs` at `e3812c5`, not run)

- `pair` values (`name: () => …`, `name: function () {}`) get no node. This is
  why the original case's unqualified origin `name` resolves to zero candidates.
- Shorthand `name() {}` inside an object literal reaches the `method_definition`
  arm with the *enclosing* scope, so it becomes `<scope>::name`. Two literals in
  one scope therefore already produce a **duplicate path**, and that also aliases
  a nested `function name` in that scope. The current state is a collision, not
  just an absence.
- `method_signature`, `abstract_method_signature`, `function_signature`, call,
  construct, and index signatures produce no node. `property_signature` inside
  an interface or type produces a **Field** node `<Type>::<key>`, including for a
  function-typed property such as `name: () => string`.

## Identity rules

**R1 — Supported owner.** The owner is a `variable_declarator` in a `const`
declaration whose name is a plain identifier. Its value must be an object literal,
either directly or through any nesting of these erased wrappers only:
parentheses, `satisfies T`, and `as T`.

**R2 — Supported member.** The literal is not refused (see U6 and R4). The key is
a plain identifier (`property_identifier`). The member is one of:
- `key: <arrow function>` (including `async`), form `arrow`;
- `key: function [name] (…) {…}` (including `async`), form `function_expression`;
- `key(…) {…}` or `async key(…) {…}`, form `method_shorthand`.

A value that is itself an identifier-keyed object literal in an unrefused literal
extends the owner path, recursively.

**Path grammar.** `<scope>::<owner>::@object::<key>`. For nesting, append
`::<key>::@object::<key>…`. `<scope>` is the existing extractor scope path at the
declaration (module, function, or registration scope; blocks add nothing, as
today). `@` cannot occur in an identifier. So the `@object` segment makes member
paths disjoint from every declaration path, including a nested function with the
same owner and key, and from registration paths (`@test[…]#n`, `@describe[…]#n`).
The display name is the key. The form is recorded as a node attribute
(`member_form`) and is **not** in the path. No occurrence counter is used. The
member is `Contains`-edged from the nearest actual graph owner. No node is created
for the owner object in v1. Declarations nested inside a member body are scoped
under the member path.

**R3 — Collisions fail closed.** If two members compute the same path (for example,
same-named `const` owners in sibling blocks), *every* colliding member is refused:
it gets no Function node and is never a candidate. They are never told apart by
order. The only accepted contract is omission: no Function node at any colliding
span (`colliding-owners.ts` pins `null`). Keeping duplicate nodes and relying on
the file-level `duplicate-semantic-path` refusal does not satisfy this policy.

**R4 — Duplicate keys.** If one literal defines the same key more than once, every
member with that key is refused. The later definition wins at runtime, and the
fixture checks this.

**R5 — Identity is not proof.** An identity names a definition site. Later writes
(`obj.key = …`), `delete`, `Object.defineProperty`/`assign`, escapes, and
`this`-dependence do not remove the identity. They are proof obligations for a
future rule. Any call whose slot may hold a value without identity remains
Unknown (see the `mutated-member` validation case, where the runtime value
differs from the literal member).

**R6 — Existing flattened shorthand paths are withdrawn.** An object-literal
`method_definition` must no longer emit `<scope>::<key>`. It gets an R1/R2 identity
or none (U1–U7). This intentionally changes NodeIds. Graphs must be rebuilt.

## Declaration-only rule

**D1.** These are declaration-only: `method_signature` (interface, type literal,
class overload), `abstract_method_signature`, `function_signature` (function
overload heads), call, construct, and index signatures, function-typed
`property_signature`, and ambient declarations. They never become Function
nodes. They are never origins, May candidates, Must targets, or call evidence
targets. The existing Field node for a `property_signature` may stay for
retrieval, but it must never be accepted as an executable origin or proof target.
A validation scorer must check node kind, not just a name match.

## Unknown / no-identity forms (each refused, with a reason)

- **U1** Owner-less literals: call arguments, `return` values, array elements,
  default parameters, `export default {…}`. There is no binding name to make a
  stable path from, and an occurrence counter would rekey when siblings are
  inserted (the defect `registration_scope` already discloses).
- **U2** `let`/`var`, destructuring, class-field, and assignment owners. **This is a
  v1 scope choice, not an identity requirement.** Under R5 a `let` owner's member
  path would be just as stable and distinct, and `var` redeclaration in one scope
  is already an R3 collision. They are excluded only because `const` (ECMAScript
  `CreateImmutableBinding`) is the one owner shape a future proof rule can rely
  on without rebinding analysis. Destructuring, class-field, and assignment
  owners have no `variable_declarator` identifier holding the literal. Widening
  the owner set is an open review question.
- **U3** String, numeric, and computed keys, including string-keyed methods. A
  computed key is runtime-valued. A non-computed string `'__proto__'` key is a
  prototype setter, not a property. v1 does not normalize string spellings.
- **U4** `get`/`set` accessors. `obj.key()` calls the getter's *result*, not the
  accessor.
- **U5** Generators and async generators (`*m(){}`, `function*`), which match the
  generator exclusion in the lexical-binding proof subset.
- **U6** A literal with any spread element or any `__proto__` entry (identifier or
  string key) refuses **all** of its members. Spread copies properties in source
  order and can overwrite an own member (fixture-checked). `__proto__` changes
  lookup for absent keys. v1 does not reason about order. Shorthand properties
  `{ name }` define no function, so they get no member identity.
- **U7** Angle-bracket assertions `<T>{…}` and any wrapper not listed in R1.

When resolution is ambiguous, the result is a refusal (R3, and multiple matches
during origin resolution). No form here may be resolved by choosing a
same-spelled candidate.

## Precommitted prediction for the original corpus case

`typescript-structural-object-literal` is unchanged: origin `{file: app.test.ts,
symbol: name}` with no qualifier, expected may/may, status failed. Under this
policy `girder names name` should return exactly two Function candidates,
`crate::app.test::alice::@object::name` and `crate::app.test::bob::@object::name`.
`Named.name()` is a method signature, so no node exists for it. The scorer's
existing rule turns two candidates into an **ambiguity failure**. **The case must
stay failed as ambiguous.** Changing the case, adding a qualifier to it, or making
any component pick one candidate to "pass" it violates this policy and the
corpus's never-tune rule. The failure changes from "zero matches" to "ambiguous",
and that change must be published as such. It is not a dispatch-corpus improvement.

## Frozen fixtures

1. [Identity contract corpus](../../../../fixtures/typescript-structural-member-corpus/v1/manifest.json)
   (manifest SHA-256 `0d6e4838168c21cec9d0973b718d25cd9d2260f6c4fffd8ec150c641b4309af4`)
   has 22 files, 55 marked members (27 required identities, 28 required
   no-identity), and 10 declaration-only signatures. Every file's hash is pinned.
   `expected_identity` is the exact path, or `null` for "no Function node at this
   span". `stability/before.ts` → `after.ts` covers sibling insertion, literal
   insertion above, reordering, body edits, and form changes, with identical
   suffixes required. These are proof-contract fixtures, not ground-truth samples.
2. [Qualified validation cases](../../../../fixtures/typescript-structural-validation/v1/manifest.json)
   (manifest SHA-256 `32a96e7fea8483577a720d96b5849c7de51f7485b4c4adf0c5d8b902c93c467d`)
   has 15 cases over 10 fixtures. `object-literal-arrow/app.test.ts` is a
   byte-identical copy of the original fixture, now with qualifiers `alice::@object` /
   `bob::@object` and labels cited from the frozen
   `typescript-structural-class-no-implements` case (same shape, may/may). The
   cases cover function-expression, shorthand, and mixed forms; an owner-scope
   **ambiguity** that must fail closed and its fully qualified twin; a direct-call
   Must ground truth; a runtime-disproved mutated member (Unknown); a true negative
   (`excluded`); spread refusal; and two declaration-only origins
   (`no-executable-target`). Each qualifier was checked mechanically against the
   scorer's existing `::{qualifier}::{bare}` substring rule and the predicted path.
   **These cases are never counted in the 49-case denominator, its fixed-cohort
   scores, or a dispatch-corpus-improvement claim.**

[Preimplementation checks](preimplementation-checks.json) were produced by
[`run_preimplementation_checks.py`](run_preimplementation_checks.py) with Node
v22.22.0 type stripping (Node only). Results: 21/22 identity files executed with
their inline semantic assertions passing; `angle-assertion.ts` is skipped because
the syntax is not erasable, and it is not counted as passed. All 10 validation
fixtures pass `node --test`. The unchanged original fixture passes 2/2. All pinned
hashes match.

## Acceptance (after implementation, not now)

1. The identity corpus has exact contracts. Every non-null identity exists exactly
   once at the marked span with the stated `member_form`. Every null has no
   Function node at its span. No declaration-only span is a Function node or a
   call-evidence target. Stability suffixes are equal.
2. A **separate** validation scorer (a new tool, not an edit to
   `tools/dispatch_corpus_scorer.py`) uses the corpus's exact / conservative /
   unsound rules. It requires the stated origin resolution and accepts only
   Function nodes as origins. Since v1 adds no proofs, conservative Unknown answers
   are expected. Any overclaim (a Must or May beyond the label) or unsafe
   exclusion is a failure.
3. Re-run the unchanged 49-case corpus. Confirm the prediction above and publish
   every case, including the retained failure.
4. Re-run the unchanged 100-call audit. R6 changes paths and duplicate-path status
   in files with object-literal shorthand methods, and that can change
   lexical-proof eligibility. Measure this; don't assume it. Retain every prior
   observation.
5. Run the four common gates serially before any language-completion claim.

## Decision log

| Choice | Rejected alternatives | Reason |
| --- | --- | --- |
| Key-derived member identity | Occurrence counter or byte offset; inner function name | Stable under sibling insertion and reorder. Callers reach the member by its key. ECMAScript `NamedEvaluation` names anonymous member functions after the key. A named function expression's own name is bound only inside its body. |
| `@object` marker segment | `owner::key`; `owner.key` | `owner::key` equals a nested function's path. A marker that no identifier can contain makes the grammar injective, so redeclaration rules aren't needed to prevent collisions. |
| Form as an attribute | Form in the path | Changing arrow to shorthand is a body edit, not a rename. |
| `const` owners only (scope choice) | Include `let`/`var` | Not required for identity (see U2). It keeps v1 aligned with the only owner a later proof rule can use without rebinding analysis. It is an open question for review. |
| Refuse colliding members | Pick first or last; add a counter | A refusal is never wrong. Choosing one guesses. |
| Refuse whole literal on spread/`__proto__` | Order-aware partial identities | Spread overwrites by order, and `__proto__` (including the string-key form) is a setter. v1 avoids reasoning about either. |
| Withdraw flattened shorthand paths (R6) | Keep `<scope>::<key>` | Those paths alias distinct members, and alias nested functions. Cost: shorthand methods in owner-less, `let`/`var`, destructuring, class-field, string-keyed, spread, or `__proto__` literals lose the nodes they have today, a retrieval regression on real code, disclosed here. |
| Identity only, no proofs | Add structural May now | Keeps this freeze reviewable. Proof rules need their own precommitted adversarial cases. |
| Separate qualified fixtures | Qualify the original case | The user's instruction and the corpus's never-tune rule both forbid editing the original. |

Basis: ECMAScript `PropertyDefinitionEvaluation` (spread uses `CopyDataProperties`;
`PropertyName : AssignmentExpression` uses `NamedEvaluation` for anonymous
functions; a non-computed `"__proto__"` key calls `[[SetPrototypeOf]]`;
`CreateDataPropertyOrThrow` makes the later duplicate win), the
object-initializer early error for duplicate `__proto__`, and
`BlockDeclarationInstantiation` (`const` → `CreateImmutableBinding`). These were
checked on 2026-10-03 against `tc39/ecma262` `main` `spec.html` (raw source
SHA-256 `0ceb261e3e42764d70a9199d29f28c04e20df0f8227f44b0a4e44c0daabeb2bd`,
sections `sec-runtime-semantics-propertydefinitionevaluation`,
`sec-object-initializer-static-semantics-early-errors`,
`sec-blockdeclarationinstantiation`). `tc39.es` itself was blocked by this
session's egress policy, so the source file was read instead. Node type
stripping (erasable syntax only, no type checking): https://nodejs.org/api/typescript.html,
checked 2026-10-03. The current tree-sitter node kinds come from reading the
extractor source, not from running it.
