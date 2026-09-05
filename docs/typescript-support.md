# TypeScript and TSX support

TypeScript is Girder's third supported source language. The initial support is
usable and passed the frozen `typescript-support-v1` policy, but it is narrower
than the mature Rust and Python support described below.

The policy was committed before implementation in
[`typescript-support-policy.json`](typescript-support-policy.json). The raw,
machine-readable result is
[`typescript-support-observation.json`](typescript-support-observation.json).
The policy was not changed after measurement.

## Measured result

The observation measured five fresh graph builds per pinned repository with two
Rayon threads. All policy checks passed:

| Repository | Sources | Graph | Semantic probes | Median / max build | Peak RSS | Inspect max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Zod 3.23.8 | 165 files, 30,448 lines | 2,914 nodes, 3,348 edges | 12 TP, 2 TN, 0 FP, 0 FN | 2.821 s / 3.099 s | 50,396 KiB | 176 ms |
| type-fest 4.26.1 | 310 files, 18,952 lines | 1,182 nodes, 880 edges | 4 TP, 2 TN, 0 FP, 0 FN | 1.883 s / 1.973 s | 23,916 KiB | 299 ms |

Across all ten runs, each repository produced one artifact digest, one canonical
semantic digest, and one node/edge-count pair. Aggregate precision and recall
were both 1.0 for the declared probes. These numbers describe this pinned
corpus and host; they are not a general throughput guarantee.

## Supported surface

The extractor accepts `.ts`, `.tsx`, `.mts`, and `.cts`. `.tsx` uses the TSX
grammar; JavaScript is intentionally not included. It produces the existing
Girder graph vocabulary for:

- named functions, arrow/function expressions with bindings, and methods;
- classes, interfaces, type aliases, enums, and fields;
- ES module default, named, namespace, wildcard, and re-export forms;
- call, containment, inheritance, and interface-implementation relationships;
- Jest/Vitest `describe`, `it`, and `test` callbacks; and
- functions that return JSX, while leaving JSX elements themselves unmodeled.

The corpus observation directly confirms representative functions, arrow
functions, methods, classes, interfaces, aliases, same-file and named-import
calls, inheritance, implementation, Jest discovery, negative resolution cases,
and declaration-file aliases. The remaining listed forms are covered by the
focused integration fixtures in `crates/aether-builder/tests/typescript.rs`, not
by independent corpus probes in this policy version.

## Declaration-file boundary

`.d.ts` is not wholly deferred. Ordinary declarations at a declaration file's
top level are in scope:

- top-level type aliases, whether exported or private; and
- top-level interfaces.

Those constructs become normal `Type` nodes. The type-fest policy gates on
their containment edges resolving, including a private helper alias.

Ambient namespace mechanics remain an **EXTENSION POINT** and are out of scope:

- `declare global` blocks;
- module augmentation such as `declare module "package-name"`;
- triple-slash reference directives; and
- declarations nested inside ambient/internal modules.

The consequence is that definitions introduced only through those mechanisms
are not available for retrieval or cross-file resolution. This boundary
reconciles the policy's `.d.ts` aliases with the extractor's ambient-declaration
extension point: file format alone is supported, while ambient scope mutation
is not.

## Other extension points

The extractor marks each of these in code with `// EXTENSION POINT`:

- **Decorators:** decorator syntax does not create nodes or edges, so
  decorator-driven framework relationships are not retrievable.
- **Generics and type parameters:** generic declarations still produce their
  enclosing nodes, but constraints and type-parameter relationships do not
  produce semantic edges.
- **Dynamic `import()`:** no import or call edge is emitted, so targets loaded
  only dynamically are not connected.
- **CommonJS `require`:** no import edge is emitted; TypeScript CommonJS
  bindings are not resolved.
- **JSX component usage:** JSX elements are not treated as call sites, so a
  component used only as `<Component />` has no `Calls` edge from that usage.
- **Ambient declarations:** only the ordinary top-level `.d.ts` constructs
  listed above are represented; ambient scope mutation remains unmodeled.

Compared with Rust and Python, TypeScript therefore has a smaller observed
corpus and explicit gaps around framework/type-system-specific semantics. Its
core source graph is measured and gated, while those extension points prevent a
claim of feature parity.

## Reproduction

Build the dedicated corpus driver and Girder binary, then run:

```sh
python3 tools/typescript_support_benchmark.py \
  --driver target/debug/examples/typescript_corpus \
  --inspector target/debug/girder
```

Artifacts are verified against the exact byte counts and SHA-256 values in the
policy before extraction. Use `--offline` once the verified cache is populated.
