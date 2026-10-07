# Go after-observation (final candidate `cc016189`)

Measured 2026-10-06 against the frozen [policy](policy.md), [methodology](methodology.md),
fixtures, and labels, after the baseline in [baseline-observation.md](baseline-observation.md).
Nothing in the frozen inputs changed. All three attempts are published; the earlier ones are kept.

## Identity

| Item | Value |
| --- | --- |
| Final candidate | `cc016189eddff1a0cea984d149be0658b66dab8d` (tracked tree clean) |
| Binary | debug `girder`, sha256 `a277658e5a3d6a12a27a2a1643b88e0a5b0a558d298d40f5c1b02c88c4a09d66` |
| Fixture manifest | `fc88e266701a2e671837b19131e0647e447cfb87989d5f6cf91808f57bf164bd` |
| Dispatch corpus | `9e3208a8f3fdc8faaee55cece63c1b5e25526322ebebbba915652f4f526ccd0a` (unchanged) |
| Labels | `labels-final.json` sha256 `1562c9700c01a4eb12a5e3dea9e67df1f7dbcf50a25a56a6548f604fe62619e9` |
| Implementation | `crates/aether-builder/src/sync/go_package.rs`, tests `crates/aether-builder/tests/go_package.rs` |
| Environment | Linux x86_64, 2 CPUs, 2.7 GB RAM, rustc/cargo 1.97.1, node v22.23.1, serial, offline; see [gates-cc01618/environment.txt](gates-cc01618/environment.txt) |

## Attempts (all published)

| Attempt | Candidate | Contract | Audit (118 calls) | Gates |
| --- | --- | --- | --- | --- |
| 1 — [after-contract-1](after-contract-1/), [after-audit-1](after-audit-1/), [after-corpus](after-corpus/) | `4e20195` | 21/21 exact | 28 / 87 / 0 / **3 unsafe exclusion** (ids 72, 73, 76: generic `reflect.F[T](x)` calls the grammar parses as type conversions, so no claim existed) | not run |
| 2 — [after-contract-2](after-contract-2/), [after-audit-2](after-audit-2/), [after-corpus-2](after-corpus-2/) | `c5d101e` | 21/21 exact | 28 / 90 / 0 / 0 | **clippy FAILED** (`type_complexity`), published in [gates-c5d101e](gates-c5d101e/); tests, fmt, npm passed |
| 3 (final) — [after-contract-3](after-contract-3/), [after-audit-3](after-audit-3/), [after-corpus-3](after-corpus-3/) | `cc01618` | 21/21 exact | 28 / 90 / 0 / 0 | all four pass, [gates-cc01618](gates-cc01618/) |

Attempt 3 differs from attempt 2 only by a named type alias; the measurements are identical.

## Results (baseline to final; format exact / conservative / overclaim / unsafe_exclusion)

| Measurement | Baseline | Final |
| --- | --- | --- |
| Fixture contract (21) | 10 / 7 / 3 / 1 (failed); Must claims 3, correct 0 | **21 / 0 / 0 / 0**; Must claims 7, correct 7 |
| Dispatch corpus, pooled (unchanged, 49 cases) | 23 / 33 / 0 / 1; Must 6/6 | **25 / 31 / 0 / 1**; Must 8/8 |
| Dispatch corpus, Go | 5 exact / 9 conservative | **7 / 7** |
| Real audit (118 calls) | 15 / 96 / 0 / **7** | **28 / 90 / 0 / 0**; Girder Must claims 11 (11 correct) to **24 (24 correct)** |

The pre-written predictions in [policy.md](policy.md) held exactly: Go 7 exact / 7 conservative,
pooled 25 / 31, Must 8/8, with `go-direct-cross-file` and `go-closure-captures-direct-call` both
flipping to exact. The one corpus failure, `typescript-structural-object-literal`, is retained.

Audit by stratum (baseline / final, same cell order):

| Stratum | Baseline | Final |
| --- | --- | --- |
| `bare_cross_file` | 2 / 36 / 0 / 2 | 15 / 25 / 0 / 0 |
| `bare_other` | 10 / 2 / 0 / 0 | 10 / 2 / 0 / 0 |
| `selector` | 2 / 28 / 0 / 0 | 2 / 28 / 0 / 0 |
| `pkg_qualified` | 0 / 18 / 0 / 0 | 0 / 18 / 0 / 0 |
| `go_defer` | 1 / 6 / 0 / 1 | 1 / 7 / 0 / 0 |
| `iife` | 0 / 5 / 0 / 0 | 0 / 5 / 0 / 0 |
| `generic` | 0 / 1 / 0 / 4 | 0 / 5 / 0 / 0 |

The runners were corrected after the baseline (a same-named method made the expected-target lookup
ambiguous). Rerunning the baseline with the final runners against the independent pre-Go release
binary (`db8d59ac…`) reproduced the baseline cells exactly: contract 10 / 7 / 3 / 1 and audit
15 / 96 / 0 / 7, Must 11/11 ([baseline-contract-2](baseline-contract-2/), [baseline-audit-2](baseline-audit-2/)).

## What changed

- **Same-package cross-file direct-call proof** (policy G1) replaces the same-file-only proof; the
  refusals apply to every Go Must, same-file included. This closed the 3 contract overclaims
  (an `app`/`app_test` identity collision, a build-constrained file, a cgo file).
- **Lost-evidence safety net:** every call expression with no claim on a node of its own file gets
  an explicit Unknown claim, anchored on that file's first surviving function node. This closed
  the package-level initializer hole and the calls lost in `encoding/json` v1/v2 duplicate
  definitions (ids 15, 32, 74, 78).
- **Type-conversion-shaped generic calls** (`reflect.TypeAssert[encoding.TextMarshaler](v)` parses
  as a `type_conversion_expression`) are treated as call-shaped and get an explicit Unknown claim
  (ids 72, 73, 76). A first guess that these sat in unparseable regions was tested with a tree
  dump, found wrong, and the speculative scanner was discarded.

## Verification of the guards

Five refusals were mutation-checked in `crates/aether-builder/tests/go_package.rs` (21 frozen
fixtures plus extra tests): caller build constraint, identity collision, the lost-evidence net,
the generic-target rule, and the enclosing-function rule (the last needed a new test, because the
frozen fixture's claim sits on a different file's module node). The type-conversion fix is backed
by a test that failed before the fix ("no claim for the call ending at 82"). A cold-versus-incremental
equality test runs through six package edits (constraint added and removed, duplicate definition
added and removed). **Process slip, disclosed:** while mutation-testing, a backup copy raced the
background job and captured an already-mutated file; the missing guard was caught by a guard-count
check and restored before any commit or measurement. The committed code passed all tests and gates.

## Common gates (final candidate `cc01618`, serial, logs in [gates-cc01618/](gates-cc01618/))

| Gate | Exit | Result |
| --- | --- | --- |
| `cargo test --workspace -j1 --quiet` | 0 | 28 suites, 787 passed, 0 failed, 2 ignored |
| `cargo clippy --workspace --all-targets -j1 -- -D warnings` | 0 | clean |
| `cargo fmt --all --check` | 0 | clean |
| `node --test npm/test/*.test.js` | 0 | 29 passed, 0 failed, 2 skipped |

## Limits and disclosed holes

- **Zero audit errors is reached partly with explicit Unknown claims.** The 90 conservative cells are
  honest Unknowns for calls this first step does not attempt: selector (method) calls, package-qualified
  calls, `go`/`defer`, immediately-invoked literals, generics, and build-constrained files (including
  the `encoding/json` v1/v2 pairs, refused by design). Must recall on labeled `must` calls is low
  (24 of 114); precision is what the criterion requires and it is 24/24.
- Must claims are conditional on the package compiling for the analyzed platform.
- The audited corpus is a 15-package standard-library subset, non-test code only; it cannot exercise
  package-qualified resolution (no `go.mod`) and is not a substitute for application code.
- The safety net needs the file to have a surviving function node to anchor on, and does not cover a
  call inside a region the grammar reports as `ERROR` (no instance occurs in the audit; a speculative
  scanner for it was removed as untested).
- Labels were drafted by a second agent and audited by the lead (2 disagreements of 140).
- **Stage 2 obligation outstanding:** `go-direct-cross-file` and `go-closure-captures-direct-call`
  started passing; a harder successor for each must be appended as a new versioned corpus file
  (the frozen file is pinned by hash). Not done here.
