# Structural member identity policy v1: implementation observation

Frozen policy: [policy.md](policy.md), frozen at `de4a4b0`. Fixtures and
manifests are unchanged since `2e5c8bc`. The dispatch corpus
(`9e3208a8…`) and the original structural fixture (`5b85efac…`) are
byte-unchanged. No Must or May proof rule was added.

## Candidates

| Commit | Content | Status |
| --- | --- | --- |
| `0c9db9b` | First implementation: K1/R3/R4/U1–U7/R6/T1 extractor, B1 claims, contract and focused tests, validation scorer | All four gates passed ([gates-0c9db9b](implementation-1/gates-0c9db9b/)). **The high review returned NO-GO**; the result is retained, not current. |
| `230418e` | Correction (see below) | **Current candidate.** All four gates passed ([gates-230418e](implementation-2/gates-230418e/)). |

Review findings on `0c9db9b`, all fixed in `230418e`, each with a regression
test that fails when the defect is restored ([mutation checks](implementation-2/mutation-checks.json)):

1. **Class-field arrows were not walked.** A class-field arrow's body was never
   lowered, so const-owned literals inside it had no identity and let-owned ones
   had no T1/B1. It is now walked under the field's Function scope.
2. **R3 pruned by path prefix.** This removed an unrelated literal's nested member
   that extended the refused path. Pruning is now by the colliding members'
   source spans.
3. **K1 accepted shorthand properties as plain keys.** Under the frozen K1 rule,
   only an unescaped `property_identifier` is plain. A shorthand property now
   refuses the whole literal.
4. **The validation scorer could abort before writing output.** It now records
   command failures, timeouts, invalid JSON, and input-hash mismatches in its
   output and exits nonzero. Six negative-control tests cover this.

## Results on `230418e` (binary SHA-256 `eb77061d…`, [environment](implementation-2/environment.json))

- **Identity contract.** All 31 manifest files match exactly: 44 identities with
  their `member_form`, display name, and uniqueness; 62 refusals; 10
  declaration-only spans that are never Function nodes or targets; and 6 B1
  boundaries on the stated owner. Each boundary's site exactly covers the refused
  subtree (start and end bytes), and every marked inner call is Unknown with no
  target. There are no T1 nodes inside refused subtrees, no duplicate paths, and
  no structural member is ever a call target.
- **Focused tests.** There are 11, covering collision propagation and lexical
  pruning, branch refusal, B1 for both call and `new` subtrees (exact spans,
  maximal only), identity/span/form/containment, declaration-only exclusions,
  no proof escalation, R6, the class-field scope, K1 shorthand, and the
  original-fixture ambiguity. Together with the contract test there are 12
  tests, all passing. All 155 + 3 builder tests pass ([log](implementation-2/builder-tests.log)).
- **Mutation checks.** Nine source mutations were made in total: five on
  `0c9db9b` ([implementation-1](implementation-1/mutation-checks.json)) and four
  on the correction. Every one was caught.
- **[Validation scorer](implementation-2/validation-scoring.json).** It only
  accepts Function nodes with the exact predicted identity.
  - All 17 of 17 resolution contracts are met. The refusal, ambiguity, and
    declaration-only cases resolve to zero (or exactly the predicted) Function
    candidates, and the interface `Field` is rejected.
  - The 21 test cells come out as 1 exact, 20 conservative, **0 unsound**.
  - Conservative answers are expected because the policy authorizes no proofs.
  - `test_host`, which reaches its target only through a refused subtree, is
    Unknown, not excluded.
- **[49-case corpus](implementation-2/dispatch-corpus-scoring.json).** The
  matrix is identical to the previous published run: 22 exact / 34
  conservative / 0 unsound / 1 failed, with Must precision 1.0.
  - The only change is the frozen prediction:
    `typescript-structural-object-literal` now fails as **ambiguous** between
    exactly `crate::app.test::alice::@object::name` and
    `crate::app.test::bob::@object::name`, instead of failing with zero matches.
  - This is **not a dispatch-corpus improvement**. The case is retained as failed.
- **Common gates (serial, `-j1`, `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`).**
  - `cargo test --workspace`: 26 suites, 769 passed, 0 failed, 2 ignored.
  - `clippy -D warnings`: clean.
  - `fmt --check`: clean.
  - npm: 29 passed, 2 skipped, 0 failed.

## Blockers and limitations

- **The 100-call real-repository audit re-run is BLOCKED** (policy acceptance
  item 4). The pinned archives are not cached in this cloud session, and the
  hash-verified downloader got HTTP 403 from the session's egress proxy for all
  four pinned URLs ([record](implementation-2/audit-rerun-blocked.json)). No
  substitute source was used and no audit result is claimed. Because R6 and
  R3 change paths and duplicate-path status, lexical-proof eligibility on real
  files may change. That effect must be measured on the pinned cache.
- This session is a cloud container, not the MOVESPEED workstation: the default
  `./target` was used. Gate logs were produced here; the user may re-run them
  locally on the same commit.
- Legacy `Calls` edges (navigation only, not classification evidence) still
  name-resolve calls inside refused subtrees to the nearest owner. Classified
  `impacted_tests`/`--quiet` use the Unknown evidence above.
- The retrieval cost disclosed in the policy (R6/T1) is real: callable members
  in refused or owner-less literals no longer have nodes.

TypeScript stays **IN PROGRESS**. This change adds identities only and
publishes no dispatch-corpus improvement. The audit re-run is blocked here.
