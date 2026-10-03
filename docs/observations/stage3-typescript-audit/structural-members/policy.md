# TypeScript structural callable member identity policy v1

**Status: preimplementation draft for review, with freeze corrections 1 and 2 applied**
(see [Correction history](#correction-history)). Base `e3812c5`; first draft
`6ac3124`. No product implementation exists for any version of this policy. Frozen inputs,
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
- Function declarations and `const`-bound arrows nested inside a `pair` value are
  lowered with the *enclosing* scope, so a nested `function helper` inside an
  arrow member already gets the same path as a module-level `function helper`.
- Claims for a call site are owned by the smallest enclosing Function node, or by
  the Module when no Function encloses it (`owner()` in `mapper/claims.rs`).
- `method_signature`, `abstract_method_signature`, `function_signature`, call,
  construct, and index signatures produce no node. `property_signature` inside
  an interface or type produces a **Field** node `<Type>::<key>`, including for a
  function-typed property such as `name: () => string`.

## Identity rules

**R1 — Supported owner.** The owner is a `variable_declarator` in a `const`
declaration whose name is a plain identifier. Its value must be an object literal,
either directly or through any nesting of these erased wrappers only:
parentheses, `satisfies T`, and `as T`.

**R2 — Supported member.** The literal is not refused (K1, U6) and does not lie
inside a refused subtree (T1). The member's key is not duplicated (R4). The key
is a plain identifier (see K1). The member is one of:
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
for the owner object in v1. Declarations nested inside a *supported* member's body
are scoped under the member path (`supported-subtree.ts` is the positive control).

**K1 — Plain keys only, decided per literal (freeze correction 1).** A key is
*plain* only when it is a `property_identifier` whose source text contains no
escape (`\`). String keys, numeric keys, computed keys (including `[Symbol.…]`
and computed aliases of a plain spelling), and escaped identifier spellings are
non-plain. If **any** property of a literal has a non-plain key, whether it is a
callable member, a data property, or an unrelated value, **every** callable member
of that literal is refused, plain-identifier siblings included. The refusal
applies to that literal only. Other literals, including a sibling branch of the
same owner, keep their identities. Basis: a non-plain key can alias a plain one
(`'name'`, `[k]`, `n\u0061me` all define `name`, and the later definition wins,
which the fixtures check at runtime). Proving non-aliasing needs string
normalization and constant evaluation that v1 does not do. Refusing the whole
literal never guesses which definition survives.

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

**T1 — Refused subtrees emit no callable descendants (freeze correction 1).** A
*refused subtree* is either:
- a refused callable member (R3, R4, K1, or U1–U7) together with everything
  lexically inside its value or body; or
- a refused object literal (K1, U6, or a literal under a U1/U2/U7 owner)
  together with everything inside it.

No Function identity may be emitted for any callable descendant in a refused
subtree. That covers nested function declarations, `const`-bound arrows and
function expressions, members of nested object literals (even if those are
clean on their own), class methods, and test-registration callbacks. Type and
Field declarations inside the subtree are omitted too, so nothing nested can
flatten onto an outer path. Refusal never moves a descendant to a fallback path.
In particular, a nested `function helper` must not become `<scope>::helper` and
collide with, or alias, a same-named outer declaration. That outer declaration
keeps exactly one node. Sibling members and branches outside the refused subtree
stay indexed.

**B1 — No silent exclusion (freeze correction 1).** For every *maximal* refused
subtree that contains at least one call or `new` expression, the nearest existing
enclosing Function node (or the Module, if none) must carry:
- one call-evidence claim with `coverage_gap: true`, class Unknown, no targets,
  reason `typescript-refused-structural-member-subtree`, and a site covering the
  subtree's span; and
- for every call inside the subtree, an Unknown claim owned by that same owner
  with no invented target.

So a test that reaches the owner is never excluded because of calls hidden in the
refused subtree. No call inside a refused subtree may be certified Must against
any node, including a same-named outer declaration. The fixtures demonstrate at
runtime that such a call reaches the *inner* declaration. **B1 can only be
observed in extractor output. It is pinned in the manifest as a postimplementation
acceptance requirement (`checked_now: false`), not as something checked now.**

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
- **U3** String, numeric, computed, and escaped keys are governed by K1: they refuse
  the whole literal, not just their own member. A computed key is runtime-valued.
  A non-computed string `'__proto__'` key is a prototype setter, not a property.
  v1 does not normalize spellings.
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

Every U-rule refusal also refuses its subtree under T1 and gets a boundary under B1.
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
   (manifest SHA-256 `fa86e16327d9d727aa570e52ad1372d32462325b794efe0eb309acc232d143dc`)
   has 31 files, 106 marked members (44 required identities, 62 required
   no-identity), 10 declaration-only signatures, and 6 pinned refused-subtree
   boundaries (B1, postimplementation only). Every file's hash is pinned.
   `expected_identity` is the exact path, or `null` for "no Function node at this
   span". `stability/before.ts` → `after.ts` covers sibling insertion, literal
   insertion above, reordering, body edits, and form changes, with identical
   suffixes required.
   Freeze correction 1 adds the following:
   - K1 mixed-key literals: a plain key plus a string duplicate; a computed alias;
     escaped aliases, and an escaped spelling with no plain twin; and an unrelated
     string, numeric, or `Symbol` key. Each comes with a separate plain-keyed
     literal that must keep its identity.
   - T1/B1 refused-subtree cases (collision, duplicate key, unsupported key,
     `let` owner inside a function). Each has a nested function, a nested object,
     a call inside, and a same-named outer declaration that must stay a single
     node.
   - A nested-branch case, where a refused branch sits beside an indexed sibling
     branch.
   - A supported-subtree positive control.

   These are proof-contract fixtures, not ground-truth samples.
2. [Qualified validation cases](../../../../fixtures/typescript-structural-validation/v1/manifest.json)
   (manifest SHA-256 `aa114b8e049b2c19e952af00cbb0107010c00b413621a73f3db0b36c6b916692`)
   has 17 cases over 11 fixtures. Correction 1 adds `refused-subtree-reachability`:
   a test reaches `target()` only through a refused literal's member. Its ground
   truth is `must`, so Unknown is conservative and `excluded` is the B1 violation.
   The same fixture includes a true negative (`excluded`). `object-literal-arrow/app.test.ts` is a
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
v22.22.0 type stripping (Node only). Results after correction 1: 30/31 identity files executed with
their inline semantic assertions passing; `angle-assertion.ts` is skipped because
the syntax is not erasable, and it is not counted as passed. All 11 validation
fixtures pass `node --test`. A byte-level check confirms that both escaped-key
sites in `mixed-key-computed-alias.ts` (lines 15 and 23) contain backslash
(0x5c) + `u` + four hex digits, and that they decode to `name` and `list`
(correction 2). Every marker is unique. All 6 boundary pins are
flagged postimplementation-only. The unchanged original fixture passes 2/2. All pinned
hashes match.

## Acceptance (after implementation, not now)

1. The identity corpus has exact contracts. Every non-null identity exists exactly
   once at the marked span with the stated `member_form`. Every null has no
   Function node at its span. No declaration-only span is a Function node or a
   call-evidence target. Stability suffixes are equal. Under T1, no refused
   descendant has a Function node, and each outer same-named declaration
   appears exactly once.
1a. B1 (postimplementation): each pinned boundary exists on the stated owner
   with `coverage_gap: true`, the stated reason, and a site covering the refused
   subtree. Every marked inner call is an Unknown, target-free claim owned by
   that owner. No inner call is Must.
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
| Withdraw flattened shorthand paths (R6) | Keep `<scope>::<key>` | Those paths alias distinct members, and alias nested functions. Cost (larger after T1): callable descendants of refused subtrees and shorthand methods in owner-less, `let`/`var`, destructuring, class-field, string-keyed, spread, or `__proto__` literals lose the nodes they have today, a retrieval regression on real code, disclosed here. |
| Identity only, no proofs | Add structural May now | Keeps this freeze reviewable. Proof rules need their own precommitted adversarial cases. |
| K1: refuse the whole literal on any non-plain key | Refuse only the non-plain member; normalize strings | Plain siblings can be silently overwritten by an aliasing non-plain key. A per-member refusal would keep a wrong identity. |
| T1: omit callable descendants of refused subtrees | Keep descendants under a fallback or flattened path | A fallback path collides with or aliases outer declarations, which is the defect this policy removes. |
| B1: explicit coverage boundary on the nearest owner | Rely on implicit per-call ownership only | Omitting nodes must never look like "no calls here". The boundary makes the gap visible in impact. |
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

## Correction history

- `6ac3124`: first draft.
- **Freeze correction 1** (`d8d1645`), a policy/fixture correction before any
  product implementation, made after the high review of `6ac3124`:
  - **Decision 1 → K1.** Any non-plain key refuses every callable member of its
    literal.
  - **Decision 2 → T1 and B1.** Refused subtrees emit no callable descendants
    and get an explicit Unknown coverage boundary on the nearest existing owner.
  - New fixtures were added and the manifests extended. Earlier manifest
    entries, and the files they pin, are unchanged.
  - B1 is recorded as a postimplementation acceptance requirement because Node
    cannot observe extractor output.
  - `docs/dispatch-corpus.json` and the original fixture are byte-unchanged
    (hashes above, re-verified).
- **Freeze correction 2** (this revision), before any product implementation.
  - **Failure (retained).** Review found that at `d8d1645`,
    `mixed-key-computed-alias.ts` lines 15 and 23 held the plain identifier bytes
    `name` and `list`, not escaped identifiers. That contradicted K1 and the `null`
    expectations those members were meant to exercise. The pinned SHA-256 was
    `6c13638cdc0c526cd5cc25b5dae007017a6bce7910c6273126c69b95443ed9d1`.
    Correction 1's Node checks still reported success, because the runtime
    assertions hold with either spelling and no byte check existed. Its record is
    kept unchanged as
    [preimplementation-checks-correction-1.json](preimplementation-checks-correction-1.json).
  - **Fix.** Both sites now contain the literal source bytes `n\u0061me` and
    `l\u0069st`. The runtime keys stay `name` and `list`, which the inline
    assertions confirm. The file was re-pinned to SHA-256
    `f4ce72ba47dbf1725a1ee1df8c7e53b27a148b3603f1bf1e61ab24b4001fb140`.
  - **New check.** The manifest now records `escaped_key_sites`.
    `run_preimplementation_checks.py` checks them byte-for-byte (results in
    [preimplementation-checks.json](preimplementation-checks.json)).
  - **Negative control.** The same check fails on the `d8d1645` bytes:
    [correction-2-negative-control.json](correction-2-negative-control.json).
  - No rule text changed. The dispatch corpus and the original fixture are
    byte-unchanged.
