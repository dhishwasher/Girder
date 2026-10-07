# Go baseline observation (before any resolver change)

Measured 2026-10-06 on the unchanged resolver, against the frozen
[policy](policy.md), [methodology](methodology.md), fixtures, and labels. Nothing
here was tuned: every cell is reported as measured, including defects.

## Identity

| Item | Value |
| --- | --- |
| Git head | `bb2c06a5c6a16c103c04b984a8d2833b0c1e2614` (no Rust changes since the TypeScript candidate) |
| Binary | debug `girder`, sha256 `cbc20b346129f4256d29e79bd0c644e17c9ba2f979010eb2da1731bb82e1c2b1` (byte-identical to the TypeScript after-observation binary) |
| Fixture manifest | `fc88e266701a2e671837b19131e0647e447cfb87989d5f6cf91808f57bf164bd` |
| Dispatch corpus | `9e3208a8f3fdc8faaee55cece63c1b5e25526322ebebbba915652f4f526ccd0a` (unchanged) |
| Labels | `labels-final.json` sha256 `1562c9700c01a4eb12a5e3dea9e67df1f7dbcf50a25a56a6548f604fe62619e9`; all frozen-input hashes verified by the runner before measuring |
| Commands | `python3 -m tools.measure_go_direct_calls --name baseline-contract-1 ...`; `python3 -m tools.dispatch_corpus_scorer --bitcode <bin> --corpus docs/dispatch-corpus.json --output baseline-corpus/dispatch-corpus-scoring.json --json`; `python3 -m tools.measure_go_real_audit --name baseline-audit-1 ...` |

## 1. Adversarial fixture contract (21 cases) — [baseline-contract-1/](baseline-contract-1/)

**10 exact, 7 conservative, 3 overclaim, 1 failed.** Girder made 3 Must claims; none
met the contract.

- **7 conservative:** all seven expected-Must cases (the five cross-file direct calls,
  the function-literal caller, and the test-file helper) answer Unknown, exactly as
  [policy.md](policy.md) predicted: Go proves Must only for same-file calls today.
- **9 exact Unknown:** shadowing (local, parameter), package-level function value,
  complementary build tags, GOOS-suffixed target, constrained caller, dot import,
  generic target, package-qualified call, and cgo caller.
- **3 overclaims (contract violations of the frozen refusals):**
  `u-external-test-package-duplicate-name`, `u-same-file-build-constraint`, and
  `u-same-file-cgo`: the existing same-file path certifies Must where the policy
  requires Unknown. In the first case the graph contains a single `crate::Target`
  node (the `app_test.go` one; the `app.go` function lost the semantic-path
  collision and is absent from the graph), so the named target happens to be the
  right function for that call; the defect is the contract violation and the
  missing node.
- **1 failed:** `u-package-var-initializer-call` yields **no claim** at the call in a
  package-level `var` initializer. In a multi-file package, module-level evidence
  is lost (one `Module` node per path survives), so such calls are invisible.

## 2. Unchanged 49-case dispatch corpus — [baseline-corpus/](baseline-corpus/)

Pooled **23 exact / 33 conservative / 0 unsafe_exclusion / 0 overclaim / 1 failed**;
Must precision 1.0; Go **5 exact / 9 conservative**, identical to the TypeScript
after-observation. The one failure is the retained `typescript-structural-object-literal`.

## 3. Frozen 118-call real-source audit — [baseline-audit-1/](baseline-audit-1/)

140 labeled sites: 118 actual calls, 22 not-a-call sites (not scored).

| Cell | Count |
| --- | ---: |
| exact | 15 |
| conservative | 96 |
| overclaim | 0 |
| **unsafe_exclusion** | **7** |

Girder's Must claims on labeled calls: **11, all correct** (precision 11/11 = 1.0).
These are same-file calls the existing proof already certifies.

**The 7 unsafe exclusions** are labeled actual calls with no matching claim (an
error under the frozen rule): ids 15, 32, 72, 73, 74, 76, 78.
- Verified: ids 15 (`MarshalIndent`) and 78 (`(*Encoder).Encode`) sit in functions
  defined in both the `!goexperiment.jsonv2` and `goexperiment.jsonv2` files of
  `encoding/json`; `MarshalIndent` has no function node in `encode.go`'s graph, so
  the colliding definition's call evidence is lost. Id 74 is a call in a
  package-level initializer, the same hole as the failed fixture.
- Not yet explained (stated as open, not asserted): ids 72, 73, 76 are generic
  `reflect.F[T](x)` calls with no claim at the site, while a similar generic call
  (id 75) has one; id 32 is in `HTMLEscape`, whose `v2` counterpart exists. The
  cause of each is to be established during implementation, not assumed here.

## What this means

- The frozen predictions held where they could be checked: the cross-file Must
  cases and the closure-case baseline are conservative today; corpus and Go cells
  are exactly as predicted.
- **The per-language criterion cannot be met by the first rule alone.** The audit
  shows 7 errors at baseline, all of them lost-evidence exclusions, none of them
  false Musts. The implementation must (a) add the same-package cross-file proof,
  (b) tighten the same-file path for build constraints, cgo, and identity
  collisions, and (c) make calls that currently leave no claim (package-level
  initializers, calls inside functions that lose a semantic-path collision, and the
  unexplained generic sites) emit explicit Unknown claims, per the roadmap rule that
  a hole that cannot be closed is Unknown, never omitted.
- Existing Go Must claims on the audit are sound (11/11).
