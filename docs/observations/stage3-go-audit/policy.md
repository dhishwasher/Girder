# Go same-package direct-call proof policy v1 (frozen before implementation)

Status: **drafted for review; becomes frozen only in the commit that marks it so.**
Scope: the first Stage 3 Go resolver step. No product code has changed and no
Girder measurement of the Go audit exists. Classification vocabulary and the
`must`/`may`/`unknown` meaning come from
[call-classification-policy.md](../../call-classification-policy.md).

## Rule

A Go call is **Must** only when every condition holds:

- **G1-a (bare identifier).** The callee is a bare identifier `F`, called with
  ordinary call syntax. Not `pkg.F`, `x.F`, `F[T]`, a literal call, or `go`/`defer`
  (those stay Unknown in v1).
- **G1-b (same package).** `F` resolves to exactly one top-level `func F`
  declaration in the caller's own package. Package identity is the directory plus
  the package clause: every `.go` file in the directory that shares the caller's
  package clause. A `_test.go` file whose clause equals the directory's package
  (e.g. `package app`) belongs to that package; one with the `_test` clause
  (`package app_test`) is a different package.
- **G1-c (unique name).** No other top-level declaration of any kind named `F`
  exists in that package (var, const, type, or a second func), and the
  directory holds exactly one package clause among non-`_test` files and no
  `_test`-clause file declares a top-level name that collides with `F`.
- **G1-d (no shadowing).** `F` is not rebound by a local, parameter, receiver,
  named result, or labeled scope between the call and the top-level scope.
- **G1-e (not generic, not cgo).** The target has no type parameters; neither
  the caller file nor the target file imports `"C"`.
- **G1-f (no build constraints).** Neither file carries a `//go:build` or
  `// +build` line, a GOOS/GOARCH filename suffix, and no second file defines
  `F` under any constraint.
- **G1-g (no dot imports).** The caller file has no `import . "..."`.
- **G1-h (caller scope).** The call is directly in the body of a top-level
  `func` or method, not inside a function literal. (A call inside a function
  literal stays Unknown in v1; a later policy may relax this.)
- **G1-i (clean parse).** Neither file contains a parse error.

Everything else keeps its current answer. In particular package-qualified calls
(`pkg.F`) stay Unknown: the extractor normalizes `go-import:` targets only by
the `go.mod` module path, and the pinned stdlib root declares `module std`
while its imports are `"io"`, not `"std/io"`, so that leg is unreachable there.
It is a **deferred hole**, not a closed one.

Must claims are conditional on the Go toolchain compiling the package for the
analyzed platform (an uncompilable program has no dispatch).

## Predicted dispatch-corpus result (written before implementation)

Only the first row is expected to change; every other cell must be
byte-identical in class. Current Go cells (after-1): 5 exact / 9 conservative.

| Case | Current | Predicted after |
| --- | --- | --- |
| `go-direct-cross-file` | conservative (must to unknown) | **exact (must)** |
| `go-direct-same-file` | exact (must) | exact (must) |
| `go-closure-captures-direct-call` | conservative | **conservative** (G1-h refuses the call inside the literal; this is a prediction, not a goal) |
| `go-interface-2-impls`, `go-embedding-promotion`, `go-interface-implicit-satisfaction`, `go-generic-function-type-param` (may cells) | conservative | conservative (May is not implemented in this step) |
| `go-negative-unreachable` | conservative | conservative (whole-graph flood unchanged) |
| `go-method-value`, `go-function-variable`, `go-reflect-based-call`, `go-name-shadowing-parameter` | exact (unknown) | exact (unknown) |

Predicted Go totals **6 exact / 8 conservative**; pooled corpus **24 exact /
32 conservative / 0 unsound / 1 failed** (from 23 / 33 / 0 / 1); Must
precision stays 1.0 (7/7).
A result that differs from this table (better or worse) is reported as is.

## Adversarial fixtures

[`fixtures/go-direct-call-proof/v1/manifest.json`](../../../fixtures/go-direct-call-proof/v1/manifest.json)
(sha256 `633c8b6472215f22eca40d6d208eab3984af1c95618ec8ea85fd0cf503c6a168`):
17 cases, 5 expected Must and 12 expected Unknown, each a compiling Go module
with a test asserting a value unique to the intended target.
[Runtime validation](fixture-runtime-validation.json) (go1.19.8): 17/17 compile,
vet, and pass. The cases cover: cross-file direct calls (test file, non-test,
three files, same-named method, same name in another directory), shadowing
(local, parameter), a package-level function value, duplicate targets under
build constraints, a GOOS-suffixed target, a constrained caller, a dot import
with a decoy in a third package, a call inside a function literal, a generic
target, an external `_test` package with a same-named function, a package-qualified
call, and a cgo caller.

Acceptance for the first implementation: all 5 Must cases answer Must with the
correct target, and all 12 Unknown cases answer Unknown with no target.

## Per-language criterion (unchanged from the roadmap)

Measured dispatch-corpus improvement, nonempty Must precision 1.000, and zero
classification errors on the frozen real-source audit (an error is an
`overclaim` or `unsafe_exclusion`; an honest Unknown is conservative, not an
error). Fixture perfection alone is insufficient.

## Known limits (disclosed in advance)

- The runtime fixtures use go1.19.8; the audited source is go1.27.1 and is only
  analyzed, never built.
- Package-qualified calls, method calls, interface dispatch, embedding,
  function values, generics, `reflect`, and cgo are out of scope for the first
  step and remain Unknown.
- The audit corpus is the standard library only (see methodology); it exercises
  same-package direct calls heavily but cannot exercise the deferred
  package-qualified leg.
