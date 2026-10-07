# Go direct-call proof policy v1 (frozen before implementation)

Status: **FROZEN** (the commit that sets this line is the freeze; nothing below changes after it except by a new, separately named version).
Scope: the first Stage 3 Go resolver step. No product code has changed and no
Girder measurement of the Go audit or of these fixtures exists. Classification
vocabulary comes from [call-classification-policy.md](../../call-classification-policy.md).

## What exists today (read from `mapper/claims.rs`, not measured)

Go direct calls are certified Must only on the same-file path: the callee is a
bare identifier, exactly one top-level `function_declaration` of that name with
a body exists in the **same file**, the file has no parse error or duplicate
semantic path, and a `clean` scan finds every occurrence of the name in the file
to be the declaration or a direct call (which rejects local, parameter, and
variable shadowing and escaping function values). Methods are not eligible.
Go function literals are not Function nodes, so a call inside one is attributed to
the enclosing named function. The path does **not** inspect build constraints,
cgo, or identity collisions between packages sharing a directory; a file with a
`//go:` line only adds a file-level coverage boundary.

## Rule

A Go call is **Must** only when every condition holds. **The refusals apply to
every Go Must claim, including same-file calls the existing proof already
certifies;** where today's behavior is looser, the implementation tightens it,
and the baseline measurement will show where it was unsound.

- **G1-a (bare identifier).** The callee is a bare identifier `F` with ordinary
  call syntax: not `pkg.F`, `x.F`, `F[T]`, a literal call, or `go`/`defer`.
- **G1-b (same package).** `F` resolves to exactly one top-level `func F`
  declaration with a body, in the caller's own package. Package identity is the
  directory plus the package clause. A `_test.go` file whose clause equals the
  directory's package (`package app`) belongs to that package; one with the
  `_test` clause (`package app_test`) is a different package. A target declared
  only in an internal `_test.go` file is eligible only from `_test.go` callers.
- **G1-c (unique name, no identity collision).** No other top-level declaration of
  any kind named `F` exists in that package, and the directory contains exactly
  one package identity among its files for that name: if two packages in one
  directory (`app` and `app_test`) both declare `F`, every call to `F` in either
  is Unknown (they would share one semantic path).
- **G1-d (no shadowing).** `F` is not rebound by a local, parameter, receiver,
  named result, or label between the call and the top-level scope; the whole-file
  `clean` rule is kept.
- **G1-e (not generic, not cgo).** The target has no type parameters; neither the
  caller file nor the target file imports `"C"`.
- **G1-f (no build constraints).** Neither the caller file nor the target file
  carries a `//go:build` or `// +build` line or a GOOS/GOARCH filename suffix, and
  no other file in the package defines `F` at all.
- **G1-g (no dot imports).** The caller file has no `import . "..."`.
- **G1-h (enclosing function).** The call has an enclosing named function or
  method. A call inside a function literal is attributed to that enclosing
  function and is eligible (this is today's behavior). A call with **no**
  enclosing named function, such as one in a package-level `var` initializer
  (whose owner is the module), stays Unknown in v1.
- **G1-i (clean parse).** Neither file contains a parse error.

Package-qualified calls (`pkg.F`) stay Unknown. The pinned audit root contains no
`go.mod`, because the extraction filter copies only `.go` files, so
`read_go_module_path` returns nothing there and `go-import:` targets cannot be
normalized; in application repositories the extractor only maps imports under the
module path. This is a **deferred hole**, not a closed one.

Must claims are conditional on the Go toolchain compiling the package for the
analyzed platform.

## Predicted dispatch-corpus result (written before implementation)

Derived from the code reading above. Current Go cells (after-1): 5 exact /
9 conservative. Every cell not named as changing must keep its class.

| Case | Current | Predicted after | Why |
| --- | --- | --- | --- |
| `go-direct-cross-file` | conservative | **exact (must)** | same-package cross-file direct call (G1) |
| `go-closure-captures-direct-call` | conservative | **exact (must)** | `test` to `Run` becomes Must (G1); `Run` to `Target` inside the literal is already a same-file Must, attributed to `Run` |
| `go-direct-same-file` | exact (must) | exact (must) | unchanged |
| `go-interface-2-impls`, `go-embedding-promotion`, `go-interface-implicit-satisfaction`, `go-generic-function-type-param` | conservative | conservative | May is not implemented in this step |
| `go-negative-unreachable` | conservative | conservative | whole-graph flood unchanged |
| `go-method-value`, `go-function-variable`, `go-reflect-based-call`, `go-name-shadowing-parameter` | exact (unknown) | exact (unknown) | unchanged |

Predicted Go totals **7 exact / 7 conservative**; pooled corpus **25 exact /
31 conservative / 0 unsound / 1 failed** (from 23 / 33 / 0 / 1); Must precision
8/8. A result that differs from this table, better or worse, is reported as is.

## Adversarial fixtures

[`fixtures/go-direct-call-proof/v1/manifest.json`](../../../fixtures/go-direct-call-proof/v1/manifest.json)
(sha256 `fc88e266701a2e671837b19131e0647e447cfb87989d5f6cf91808f57bf164bd`):
21 cases, 7 expected Must (each with an `expected_target` file and symbol) and 14
expected Unknown, each a compiling Go module whose test asserts a value unique
to the intended target.
[Runtime validation](fixture-runtime-validation.json) (go1.19.8): 21/21 compile,
vet, and pass. Cases: cross-file direct calls (test file, non-test, three files,
same-named method, same name in another directory, function literal, helper in an
internal test file); shadowing (local, parameter); a package-level function
value; duplicate targets under build constraints; a GOOS-suffixed target; a
constrained caller; a dot import with a decoy in a third package; a generic
target; an external `_test` package with a same-named function; a
package-qualified call; a cgo caller; and same-file variants of the refusals
that can occur in one file (build constraint, cgo, package-level initializer).

Acceptance for the first implementation: all 7 Must cases answer Must naming the
expected target, and all 14 Unknown cases answer Unknown with no target.

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
- The audit corpus is the standard library subset only (see methodology); it
  exercises same-package direct calls but cannot exercise the deferred
  package-qualified leg.
