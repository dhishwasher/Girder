# Stage 3 TypeScript before-observation: eleventh addendum -- the actual node-collision fix (design + precommitted prediction)

Follow-up to `before-observation-addendum-10.md` (`a4e5748`). Everything
in addenda 5-10 fixed the SYMPTOM (silent `test-impact`/`review`
results on a Module-only origin). This addendum designs and begins the
fix for the actual root cause named throughout: `SemanticGraph::
upsert_node` silently destroying a colliding node's entire evidence.

## After-fix repro outputs captured before this round's rebuild

Per review, captured on the binary as of `5bc7b9c`/`a4e5748`
(sha256 `beadb6c91907bf0e20830319e4cbbeff19eab4172cacc17caea6980cb336113f`,
recorded in `collision-repro/after-fix/binary-sha256.txt`), under
`--quiet`, bare, and `--quiet --out x`, committed under
`docs/observations/stage3-typescript-audit/collision-repro/after-fix/`:
`ts-describe-beforeeach/` (the `repro-nocoll` scenario, not previously
captured after any fix), `ts-lost-body/`, `rust-lost-impl/`, and
`go-module-var/`. All four correctly select their real tests in every
form, confirmed in the captured output files.

## A/B-verified the two previously-untested-against-pre-fix items

- **`aether-graph`'s two new unit tests**: temporarily wrapped the
  Module-origin boundary-push block in `if false { ... }` (keeping both
  tests' code intact), rebuilt, ran `cargo test -p aether-graph claims`.
  `a_module_origin_with_no_same_file_function_origin_still_escalates`
  FAILED exactly as predicted (`left: [], right: [NodeId(...)]`).
  `a_module_origin_with_a_same_file_function_origin_is_not_separately_
  escalated` still passed (expected -- disabling the fix trivially
  satisfies "no extra boundary was added"; this test was never at risk
  of a false pass either way, since its assertion is about the
  boundary's ABSENCE). Restored the real code afterward (`git checkout
  -- crates/aether-graph/src/claims.rs`, confirmed zero diff against
  HEAD).
- **`executor.rs`'s new plan-execution test**: same approach, `if false
  && ...` guard on `test_checks.rs`'s fallback condition.

## Design decision for the actual fix (recorded, with evidence, before writing code)

See `docs/roadmap.md`'s entry dated 2026-10-01 for the full reasoning.
Summary: qualify only same-file COLLIDING trait-impl methods (not every
trait-impl method unconditionally), because a direct grep
(`format!("{}::`) found `sync.rs:967`'s RAII-drop heuristic hardcodes
an unqualified `{owner}::drop` path lookup -- qualifying every
trait-impl method would have broken it for every `Drop` impl in every
corpus. Also confirmed, directly: Rust's own same-file Must-proof
mechanism (`sync/rust_methods.rs`) is already scoped to inherent
methods only (it explicitly checks for and refuses competing trait
declarations), so this fix cannot retroactively corrupt any currently-
published Rust Must claim -- the entire DONE audit has exactly one
`must`-observed site, and it targets a plain inherent method with no
competing definition.

## Precommitted prediction, for the Rust `repro-rust2` fixture specifically

Before writing or running the fix. My own prediction, not assumed:

1. **Two `go` nodes will exist** after the fix: a qualified path for
   `impl A for S`'s `go` and a separately-qualified path for `impl B
   for S`'s `go`, each carrying its own, correct call evidence
   (`only_via_a`/`only_via_b` respectively).
2. **`review` will list the specific qualified node**, not just the
   module, when only `impl A`'s body is edited -- the whole point of
   the fix.
3. **`calls_a`'s `A::go(&S)` call resolution is genuinely uncertain to
   me, and I'm recording that uncertainty rather than guessing
   confidently.** Reading `sync.rs::select_candidate` directly: its
   `qualifier` handling treats a non-`self`/`Self`/`cls` qualifier as a
   *type* hint, not a *trait* hint -- for `A::go(&S)`, `receiver_type`
   (inferred from the `&S` argument) would most likely dominate over
   the literal qualifier text `"A"`, and the deciding field,
   `candidate.owner`, is the TYPE's path (`S`), which my planned fix
   does NOT change (only the function's own final path segment changes,
   per the collision-group qualification approach, specifically to
   avoid corrupting `Contains` edges and field-flow owner lookups, per
   review). **My prediction**: both candidates still have `owner ==
   "S"`, `select_candidate` cannot distinguish them by trait identity
   (no trait field exists in `FunctionCandidate`), and the call remains
   ambiguous -- no `Calls` edge gained from this fix alone for this
   specific explicitly-qualified call form. This does not reduce
   safety: the call already classifies `unknown` under `claims.rs`
   today (trait methods were never Must-eligible), and the already-
   fixed escalation means `calls_a` is still conservatively selected by
   `test-impact` regardless of whether this specific edge resolves.
   **To be checked, not assumed, once the fix is built.**

## Status

Design and prediction recorded; implementation not yet started in this
document's own commit. Continuing immediately.
