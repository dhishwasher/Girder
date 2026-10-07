# Stage 3 TypeScript: fourteenth addendum, function and method path collisions

Follow-up to `before-observation-addendum-12.md` and `-13.md`, which listed "TypeScript's own
collision mechanism" as still open (sites 23 and 91). This addendum records what was actually
still open, the fix, and its verification. It corrects those two notes: the duplicate-title
`it()`/`describe()` shape they name was already fixed by the structural-member work (`230418e`,
suite ancestry plus an occurrence counter) and has regression tests
(`repeated_titles_retain_both_bodies_and_their_call_evidence`).

## What was still broken

Probed with the shipped 0.4.0 binary on a file of collision shapes. Ordinary functions and
methods that compute the same semantic path collapsed to one node, silently discarding the
others' bodies and calls. A file-level `duplicate-semantic-path` coverage gap kept the result
conservative (Unknown), so it was a lost-evidence defect, not an unsound claim.

| Shape | Before | After |
| --- | --- | --- |
| Getter and setter, one name | one node `Box::value` | `Box::value@get`, `Box::value@set` |
| Same function in two blocks | one node `branch` | `branch#1`, `branch#2` |
| Duplicate top-level declaration | one node `dup` | `dup#1`, `dup#2` |
| Nested collision under a colliding parent | one node | resolved by a further pass |
| Test callbacks with the same title | already distinct | unchanged |
| Object-literal members that collide | refused (frozen R3) | unchanged |

Separately found, not a collision: namespace bodies are not lowered at all
(`namespace NS { export function f() {} }` yields no `NS::f`). That is the documented
ambient/internal-module extension point and is untouched.

## The fix

`crates/aether-builder/src/mapper/typescript.rs`. Extraction now runs to a fixed point (at most
four passes). A `Names` table counts every function/method base path in a pass; a base path
seen more than once is qualified in the next pass for every member: accessors by `@get`/`@set`,
members still sharing a key by `#n` in source order. `add_function` now returns the function's
own scope so nested definitions derive from the qualified path. Only colliding paths change.

## Verification

- `crates/aether-builder/tests/typescript_function_collisions.rs`: 8 tests (getter/setter pair,
  lone accessor and method keep plain paths, same function in two blocks with per-body call
  edges, duplicate declarations, nested collisions, non-colliding paths unchanged, stable ids
  across rebuilds and unrelated edits, no `duplicate-semantic-path` gap and no `Must` for a call
  to an ambiguous name). **Against the previous mapper 6 of the 8 fail**; the 2 that pass are
  the "non-colliding paths do not change" guards, as intended. Against the fix: 8 of 8 pass.
- Full `aether-builder` suite: all pass (134 unit tests plus the integration tests, including
  the existing collision and identity suites).
- Measurements on the fixed tree, offline, new names (`collision-fix-1`), logs in
  [`collision-fix/`](collision-fix/):

| Measurement | Result |
| --- | --- |
| 73-case ESM contract | 73/73 exact, 0 errors, `cold_contract_met: true` |
| 49-case dispatch corpus | 25 exact / 31 conservative / 0 unsafe_exclusion / 0 overclaim / 1 failed; Must precision 8/8 = 1.0 |
| 100-call real audit | 56 exact / 44 conservative, 0 unsound, Must 3/3 = 1.0, `passed: true` |

  Binary sha256 `e8b1c6cad2380b0f9ef15b7625a68717a592d97f3f922ba7dee9cafc00fe16ee`, a debug
  build (`cargo build --locked --offline -p aether-app --bin girder -j1`).

## Honest limits

- **The fix changes none of the measured numbers.** The contract, corpus and audit are
  identical to the ESM acceptance results (73/73, 25/31/0/0/1 pooled, 56/44/0). The pinned
  real-repository corpora evidently contain no colliding function paths, so the audit cannot
  show the fix's value. The synthetic tests above do; the real-world frequency of these
  shapes was not measured.
- The `#n` form numbers duplicates in source order, so inserting an identical duplicate before
  an existing one renumbers it. Accessors are stable.
- Overload signatures (`function f(a: string): string;`) are not function bodies and still do
  not produce nodes; only the implementation does.
- The measurement runner records the `HEAD` it started from. The binary was built from the
  working tree one commit later with this change applied; the change is committed immediately
  after, so the recorded tree and the committed tree are the same.
- The 1 failed corpus case, `typescript-structural-object-literal`, is the frozen R3 refusal
  (`alice`/`bob` object-literal `name` members) and is unchanged.
