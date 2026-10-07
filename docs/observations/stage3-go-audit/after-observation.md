# Go after-observation (final candidate `08fcbdc0`)

Measured 2026-10-06 against the frozen [policy](policy.md), [methodology](methodology.md),
fixtures, and labels, after the baseline in [baseline-observation.md](baseline-observation.md).
Nothing in the frozen inputs changed. All four attempts are published; earlier ones are kept.

## Identity

| Item | Value |
| --- | --- |
| Final candidate | `08fcbdc0891d7700b52b1820b6a5bcb10de5a646` (tracked tree clean) |
| Binary | debug `girder`, sha256 `c8fb49e3cf83105198871704c9ac591b52e943d8b7494fc766685f108db3b76b` |
| Fixture manifest | `fc88e266701a2e671837b19131e0647e447cfb87989d5f6cf91808f57bf164bd` |
| Dispatch corpus | `9e3208a8f3fdc8faaee55cece63c1b5e25526322ebebbba915652f4f526ccd0a` (unchanged) |
| Labels | `labels-final.json` sha256 `1562c9700c01a4eb12a5e3dea9e67df1f7dbcf50a25a56a6548f604fe62619e9` |
| Implementation | `crates/aether-builder/src/sync/go_package.rs`; tests `crates/aether-builder/tests/go_package.rs`; verifier `tools/verify_go_must_claims.py` |
| Environment | Linux x86_64, 2 CPUs, 2.7 GB RAM, rustc/cargo 1.97.1, node v22.23.1, serial, offline; see [gates-08fcbdc/environment.txt](gates-08fcbdc/environment.txt) |

## Attempts (all published)

| Attempt | Candidate | Contract | Audit (118 calls) | Notes |
| --- | --- | --- | --- | --- |
| 1: [contract](after-contract-1/), [audit](after-audit-1/), [corpus](after-corpus/) | `4e20195` | 21/21 exact | 28 / 87 / 0 / **3 unsafe exclusion** | generic `reflect.F[T](x)` calls (ids 72, 73, 76) parse as type conversions, so no claim existed |
| 2: [contract](after-contract-2/), [audit](after-audit-2/), [corpus](after-corpus-2/) | `c5d101e` | 21/21 exact | 28 / 90 / 0 / 0 | **clippy gate FAILED** (`type_complexity`); [gates-c5d101e](gates-c5d101e/) |
| 3: [contract](after-contract-3/), [audit](after-audit-3/), [corpus](after-corpus-3/) | `cc01618` | 21/21 exact | 28 / 90 / 0 / 0 | all gates passed ([gates-cc01618](gates-cc01618/)) but **superseded**: it certified `go`/`defer` calls, which frozen G1-a forbids (see below) |
| **4 (final)**: [contract](after-contract-4/), [audit](after-audit-4/), [corpus](after-corpus-4/) | `08fcbdc` | 21/21 exact | **27 / 91 / 0 / 0** | all four gates pass, [gates-08fcbdc](gates-08fcbdc/) |

## Results (baseline to final; format exact / conservative / overclaim / unsafe_exclusion)

| Measurement | Baseline | Final |
| --- | --- | --- |
| Fixture contract (21) | 10 / 7 / 3 / 1 (failed); Must claims 3, correct 0 | **21 / 0 / 0 / 0**; Must claims 7, correct 7 |
| Dispatch corpus, pooled (unchanged, 49 cases) | 23 / 33 / 0 / 1; Must 6/6 | **25 / 31 / 0 / 1**; Must 8/8 |
| Dispatch corpus, Go | 5 exact / 9 conservative | **7 / 7** |
| Real audit (118 calls) | 15 / 96 / 0 / **7** | **27 / 91 / 0 / 0**; Girder Must claims 11 (11 correct) to **23 (23 correct)** |
| Whole-tree Must verification | n/a | **401 Must claims across 70 files, 0 violations** |

The pre-written predictions in [policy.md](policy.md) held exactly for the corpus: Go 7 / 7, pooled
25 / 31, Must 8/8, with `go-direct-cross-file` and `go-closure-captures-direct-call` both flipping
to exact. The one corpus failure, `typescript-structural-object-literal`, is retained.

Audit by stratum (baseline / final, same cell order):

| Stratum | Baseline | Final |
| --- | --- | --- |
| `bare_cross_file` | 2 / 36 / 0 / 2 | 15 / 25 / 0 / 0 |
| `bare_other` | 10 / 2 / 0 / 0 | 10 / 2 / 0 / 0 |
| `selector` | 2 / 28 / 0 / 0 | 2 / 28 / 0 / 0 |
| `pkg_qualified` | 0 / 18 / 0 / 0 | 0 / 18 / 0 / 0 |
| `go_defer` | 1 / 6 / 0 / 1 | 0 / 8 / 0 / 0 |
| `iife` | 0 / 5 / 0 / 0 | 0 / 5 / 0 / 0 |
| `generic` | 0 / 1 / 0 / 4 | 0 / 5 / 0 / 0 |

Rerunning the baseline with the final runners against the independent pre-Go release binary
(`db8d59ac…`) reproduced the baseline cells exactly: contract 10 / 7 / 3 / 1 and audit 15 / 96 / 0 / 7
([baseline-contract-2](baseline-contract-2/), [baseline-audit-2](baseline-audit-2/)).

## A departure from the frozen policy, found and fixed (attempt 3 to 4)

Frozen G1-a says `go` and `defer` calls stay Unknown. Attempts 1 to 3 certified them: the pass checked
only that the callee is a bare identifier, and no fixture used `go` or `defer`, so the fixture contract
and the sampled audit were both blind to the rule (the audit scored `defer errorHandler(&err)` exact
because its language truth is `must`). The advisor flagged it from the `go_defer` stratum's one exact
cell. A whole-tree check of the attempt-3 graph found **404 Must claims, of which
3 violated G1-a** (`errorHandler(&err)` twice in `fmt/scan.go` and
`errRecover(&err)` in `text/template/exec.go`, all direct operands of `defer`), and no other kind of
violation ([attempt-3 verification](after-audit-3/must-claim-verification.json)). The fix refuses only
when the call is the *direct* operand of a `go` or `defer` statement; nested calls (an argument, or
a call inside a deferred literal) stay eligible. It has a builder test (same-file `defer`,
cross-file `go`, argument, literal body) and was mutation-checked: removing the guard fails it with
"same-file defer: left Must, right Unknown". Attempt 4 verifies **401 Must claims,
0 violations** ([verification](after-audit-4/must-claim-verification.json)).

The verifier (`tools/verify_go_must_claims.py`) reads the graph only and checks each Must claim from
source: the site starts with the target's name and `(`; it is not the direct operand of `go`/`defer`;
caller and target share a directory and package clause; neither file has a build constraint or imports
`"C"`; the target is the only top-level declaration of its name in the package; and the caller is a
function node.

## What changed

- **Same-package cross-file direct-call proof** (policy G1) replaces the same-file-only proof; the
  refusals apply to every Go Must, same-file included. This closed the 3 contract overclaims (an
  `app`/`app_test` identity collision, a build-constrained file, a cgo file).
- **Lost-evidence safety net:** every call expression with no claim on a node of its own file gets an
  explicit Unknown claim, anchored on that file's first surviving function node. This closed the
  package-level initializer hole and the calls lost in `encoding/json` v1/v2 duplicate definitions
  (ids 15, 32, 74, 78).
- **Type-conversion-shaped generic calls** (`reflect.TypeAssert[encoding.TextMarshaler](v)` parses as a
  `type_conversion_expression`) are treated as call-shaped and get an explicit Unknown claim (ids 72,
  73, 76). A first guess that these sat in unparseable regions was tested with a tree dump, found
  wrong, and the speculative scanner was discarded.
- **`go`/`defer` direct-operand refusal** (above).

## Verification of the guards

Six refusals have a failing-then-passing or mutation-checked test in
`crates/aether-builder/tests/go_package.rs` (21 frozen fixtures plus extra tests): caller build
constraint, identity collision, the lost-evidence net, the generic-target rule, the
enclosing-function rule (needed an extra test because the frozen fixture's claim sits on a different
file's module node), and the `go`/`defer` rule. The type-conversion fix is backed by a test that failed
before the fix ("no claim for the call ending at 82"). A cold-versus-incremental equality test runs
through six package edits. **Process slip, disclosed:** during mutation testing a backup copy raced the
background job and captured an already-mutated file; the missing guard was caught by a guard-count
check and restored before any commit or measurement.

## Common gates (final candidate `08fcbdc`, serial, logs in [gates-08fcbdc/](gates-08fcbdc/))

| Gate | Exit | Result |
| --- | --- | --- |
| `cargo test --workspace -j1 --quiet` | 0 | 28 suites, 788 passed, 0 failed, 2 ignored |
| `cargo clippy --workspace --all-targets -j1 -- -D warnings` | 0 | clean |
| `cargo fmt --all --check` | 0 | clean |
| `node --test npm/test/*.test.js` | 0 | 29 passed, 0 failed, 2 skipped |

## Limits and disclosed holes

- **Zero audit errors is reached partly with explicit Unknown claims.** The 91 conservative cells are
  honest Unknowns for calls this first step does not attempt: selector (method) calls, package-qualified
  calls, `go`/`defer` statement calls, immediately-invoked literals, generics, and build-constrained files
  (including the `encoding/json` v1/v2 pairs, refused by design). Must recall on labeled `must` calls is
  low (23 of 114); precision is what the criterion requires and it is 23/23.
- **Safety-net claims are coverage boundaries, not caller attribution.** They attach to the file's
  first surviving function, not to the call's real caller. That adds conservatism but records no caller;
  it makes no difference to soundness under today's whole-graph flood, and will matter once the flood is
  narrowed. Carried forward.
- Must claims are conditional on the package compiling for the analyzed platform.
- The audited corpus is a 15-package standard-library subset, non-test code only; it cannot exercise
  package-qualified resolution (no `go.mod`) and is not a substitute for application code.
- The safety net needs the file to have a surviving function node to anchor on, and does not cover a call
  inside a region the grammar reports as `ERROR` (no instance occurs in the audit; a speculative scanner
  for it was removed as untested).
- Labels were drafted by a second agent (7 of 7 batches completed; 3 needed a second attempt) and audited
  by the lead, with 2 disagreements out of 140 labels. The Go tooling and resolver were written by the lead.
- **Stage 2 obligation outstanding:** `go-direct-cross-file` and `go-closure-captures-direct-call` started
  passing; a harder successor for each (and TypeScript's case) must be appended as a new versioned corpus
  file (the frozen file is pinned by hash). Not done here.
