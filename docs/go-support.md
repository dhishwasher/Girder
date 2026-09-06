# Go support

Go is Girder's fourth accepted source language. The initial extractor is usable,
but it did **not** pass the frozen `go-support-v1` policy and must not be
described as having measured parity with Rust, Python, or TypeScript.

The policy was committed before implementation in
[`go-support-policy.json`](go-support-policy.json). The raw, machine-readable
one-shot result is
[`go-support-observation.json`](go-support-observation.json). Neither the policy
nor the recorded observation was changed or rerun after measurement.

## Measured result

The observation measured five fresh graph builds per pinned repository with two
Rayon threads. All performance and determinism checks passed. Semantic precision
was 1.0, but one frozen positive probe was absent:

| Repository | Sources | Graph | Semantic probes | Median / max build | Peak RSS | Inspect max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Afero 1.11.0 | 55 files, 12,544 lines | 856 nodes, 1,603 edges | 14 TP, 2 TN, 0 FP, 0 FN | 1.320 s / 1.428 s | 18,176 KiB | 345 ms |
| Gorilla WebSocket 1.5.3 | 35 files, 7,337 lines | 505 nodes, 758 edges | 7 TP, 3 TN, 0 FP, 1 FN | 0.904 s / 1.070 s | 11,136 KiB | 236 ms |

Across all ten runs, each repository produced one artifact digest, one canonical
semantic digest, and one node/edge-count pair. Aggregate results were 21 TP,
5 TN, 0 FP, and 1 FN: micro precision 1.0, micro recall 0.954545, macro
precision 1.0, and macro recall 0.9375. The policy requires perfect precision
and recall, so its final assessment is `FAIL`.

The failed `same-directory-package-call` probe expected a `Calls` edge from
`crate::examples::chat::main` to `crate::examples::chat::serveHome`. In the
pinned source, `serveHome` is passed to `http.HandleFunc` as a function value
rather than invoked directly. Girder currently models direct call expressions,
not function-value references, so both endpoints exist but that edge does not.
This is retained as the measured failure rather than repaired or reclassified
after seeing the result.

These numbers describe only the pinned corpus, host, and implementation commit
recorded in the observation. They are not a general throughput guarantee.

## Semantic paths and supported surface

Go packages use directory-addressed graph paths: the repository root is
`crate`, and subdirectories append `::` components. The declared package name
and root `go.mod` module path are retained for import resolution, but package
names do not replace directory identity. This keeps repeated `package main`
directories distinct and permits exact resolution of imports within the current
module. External imports remain unresolved rather than falling back by name.

The extractor accepts `.go` files and produces the existing Girder vocabulary
for:

- packages, functions, value and pointer receiver methods;
- structs, interfaces, named types, aliases, fields, and constants;
- aliased, dot, and blank imports, retained with their exact import paths;
- package containment and struct/interface embedding relationships;
- direct same-package, imported-package, and receiver-method calls; and
- `_test.go` functions following `Test`, `Benchmark`, `Fuzz`, and `Example`
  naming conventions.

Focused fixtures cover grammar routing, package vocabulary, value and pointer
receivers (including unnamed receivers), embedding, exact internal imports,
alias/dot/blank import retention, test discovery, package isolation, and
fail-closed shadowing.

## Extension points

The extractor marks each of these in code with `// EXTENSION POINT` and states
the retrieval consequence beside it:

- **Generics:** generic declarations remain retrievable, but type-parameter
  constraints and instantiations do not create semantic edges.
- **Goroutines and channels:** `go` statements and channel sends/receives do not
  create concurrency-flow edges.
- **Inferred interface satisfaction:** explicit interface embedding is modeled,
  but structural satisfaction does not create `Implements` edges.
- **cgo:** `C` pseudo-imports and selector dispatch are unresolved.
- **Build tags and platforms:** conditional file selection is not evaluated, so
  mutually exclusive declarations can coexist in one graph.
- **Method values, method expressions, and function-typed fields:** these do not
  create call edges. The measured function-value miss is the same broader class
  of reference-versus-invocation limitation.
- **Reflection:** reflection-driven dispatch is not modeled.
- **Vendoring and replacements:** vendor precedence and `replace` directives do
  not rewrite import paths; only exact imports inside the root `go.mod` module
  resolve.

## Reproduction

Build the dedicated corpus driver and Girder binary, then run:

```sh
python3 tools/go_support_benchmark.py \
  --driver target/debug/examples/go_corpus \
  --inspector target/debug/girder
```

Artifacts are verified against the exact byte counts and SHA-256 values in the
policy before extraction. Use `--offline` once the verified cache is populated.
