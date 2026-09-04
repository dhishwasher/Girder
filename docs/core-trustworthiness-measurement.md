# Core Trustworthiness Measurement

This milestone establishes a checked, reproducible function-execution oracle
for Girder's affected-test selection. It does not claim representative-repo
coverage or general superiority.

## What is measured

The committed Rust and Python templates are materialized into disposable Git
repositories. The runner commits each baseline, applies one source mutation,
and asks the exact supplied Girder binary for `test-impact`.

Every test is then executed alone in a fresh materialized checkout with
`GIRDER_ORACLE_PROBE` pointing at a checkout-local fresh file. Only the changed
function/method writes that probe, so the resulting set is direct runtime
evidence that a test executed changed code. This is a function-level dynamic
coverage oracle, not line or branch coverage.

The runner preserves Girder's full graph test paths and maps them explicitly
to framework test ids, so equal leaf names in different modules/classes cannot
collapse. Cargo and `unittest` independently enumerate the runnable tests; that
inventory must exactly equal the declared universe and Girder's impacted plus
skipped counts. Each isolated command must report that exactly one requested
test ran. Every child command has a timeout and a combined stdout/stderr limit;
on POSIX, members of its newly spawned process group are killed, and tests prove
that inherited-group descendants do not survive either failure path. The
supplied Girder executable is copied privately, made read/execute-only, and
SHA-256 checked before and after the measurement so a build cannot replace the
program under test mid-run.

Artifacts:

- `tools/core_trustworthiness_oracle.py` — end-to-end runner and baseline check.
- `tools/test_core_trustworthiness_oracle.py` — parser/metric unit tests.
- `fixtures/core-trustworthiness/` — inert `.txt` source templates, excluded
  from Girder's own source graph until materialized in a temporary project.
- `docs/core-trustworthiness-baseline.json` — machine-checked expected sets and
  metrics.

## Reproduce

From the repository root, first build the exact binary:

```sh
env CARGO_HOME=/mnt/chromeos/removable/MOVESPEED/.cargo \
RUSTUP_HOME=/mnt/chromeos/removable/MOVESPEED/.rustup \
CARGO_TARGET_DIR=/mnt/chromeos/removable/MOVESPEED/aetherforge-target \
CARGO_INCREMENTAL=0 RUSTFLAGS=-Dwarnings \
PATH=/mnt/chromeos/removable/MOVESPEED/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:/usr/local/bin:/usr/bin:/bin \
cargo build -p aether-app --all-features -j1
```

Run the lightweight oracle tests:

```sh
python3 -m unittest -v tools.test_core_trustworthiness_oracle
```

Run the checked end-to-end measurement:

```sh
env CARGO_HOME=/mnt/chromeos/removable/MOVESPEED/.cargo \
RUSTUP_HOME=/mnt/chromeos/removable/MOVESPEED/.rustup \
CARGO_TARGET_DIR=/mnt/chromeos/removable/MOVESPEED/aetherforge-target \
CARGO_INCREMENTAL=0 RUSTFLAGS=-Dwarnings \
PATH=/mnt/chromeos/removable/MOVESPEED/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:/usr/local/bin:/usr/bin:/bin \
python3 tools/core_trustworthiness_oracle.py \
  --girder /mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder
```

Add `--verbose` to include Girder's raw `test-impact` output. A result that
differs from the checked JSON baseline exits unsuccessfully and prints both
expected and actual data. Per-command limits default to 120 seconds and 1 MiB;
use `--command-timeout-seconds` and `--max-command-output-bytes` to lower them
for failure probes or raise them for a documented representative corpus.

## Baseline results

| Fixture | TP | FP | FN | TN | Precision | Recall |
|---|---:|---:|---:|---:|---:|---:|
| Rust | 4 | 0 | 0 | 2 | 1.000 | 1.000 |
| Python | 3 | 0 | 0 | 2 | 1.000 | 1.000 |
| Combined | 7 | 0 | 0 | 4 | 1.000 | 1.000 |

Rust:

- Runtime-executed changed code: `rust_direct_selected`, `rust_cli_selected`,
  `rust_custom_bin_selected`, `rust_raii_drop_selected`.
- Girder selects exactly these four and correctly excludes
  `rust_cli_unrelated`/`rust_unrelated`. Argument-specific route modeling
  (failure-closed: any unprovable evidence leaves resolution unchanged)
  closed the prior `0.667` precision defect —
  `rust_cli_unrelated` launches the same binary as the true CLI test but its
  concrete argument cannot reach the changed branch, and this is now
  correctly excluded.
- `rust_custom_bin_selected` and `rust_raii_drop_selected` are new cases
  proving two previously measured false negatives are now modeled: an exact
  `[[bin]] path` manifest override resolves a non-conventional Cargo binary
  entrypoint, and a resolved `Self`-returning constructor for a
  `Drop`-implementing type gains a call edge to that type's `drop`.

Python:

- Runtime-executed changed code: `test_python_direct_selected`,
  `test_python_optional_selected`, `test_python_cross_module_selected`.
- Girder selects exactly these three and correctly excludes
  `test_python_decoy`/`test_python_third_party_decoy`.
- Nullable annotations provide a receiver owner only when exactly one
  non-null type remains. PEP 604, parenthesized, and forward-string forms are
  supported directly. `Optional[T]` and `Union[T, None]` are supported only
  when the wrapper resolves through an import from `typing` or
  `typing_extensions`, including aliases and imports nested under
  `TYPE_CHECKING` or inside the function. Multi-owner and untrusted custom
  unions deliberately remain unresolved rather than creating false edges.
- `test_python_cross_module_selected` is a new case proving a three-hop
  cross-module chain (`gateway.py` -> `service.py` -> `models.py`) resolves
  correctly. `test_python_third_party_decoy` extends the same-named-method
  disambiguation from two competing owners to three.

## Girder dogfood assessment

Useful output:

- `test-impact` identified the exact changed function in each disposable Git
  repository.
- It preserved the direct Rust/Python receiver path and the conventional Cargo
  binary subprocess path.
- It restored the dynamically proven nullable Python receiver path without
  selecting the same-method decoy owner.
- Its full-path listed tests were machine-parseable, and its impacted plus
  skipped counts exactly matched each independently declared fixture universe.
- On the milestone tree, query correctly reports `main` as the sole caller of
  `measure_fixture`; and review reconstructs the runner's internal helper
  chain.
- Argument-route modeling now resolves the exact CLI test that reaches each
  changed function instead of every subprocess launcher of the same binary.
- Python test identity now requires pytest/unittest-collectable context
  (`test_*.py`/`*_test.py`, module level or class method); a nested `def`
  never gains test identity or collides with a same-named sibling.

Incorrect or incomplete output:

- The suggested commands are selection output, not dynamic proof. The Python
  fixture is deliberately executed with standard-library `unittest`; the oracle
  does not assume the emitted `pytest -k` command establishes coverage.
- Self-review labels the end-to-end runner functions uncovered even though the
  checked oracle command executes `main`, fixture initialization, Girder
  invocation, dynamic test execution, metric calculation, rendering, and
  baseline comparison. This coverage-attribution gap is unchanged by this
  milestone and remains open.
- Python's `is_test` inventory now exactly matches `unittest` discovery over
  `tools/` (27 = 27).
- Rust's raw counts still differ (322 graph vs 312 `cargo test --workspace
  -- --list`), but every one of the 10 is now individually explained rather
  than an unexplained overcount: reconciling graph paths against Cargo test
  ids (accounting for the already-documented `mod`-flattening path-naming
  residual — an integration-test binary or a custom-named nested `mod`
  produces a Girder path that doesn't textually match Cargo's own
  module-qualified id, though both name the same real test) matches 312 of
  322 graph identities to a real, distinct Cargo test 1:1. The remaining 10
  are exactly the deliberately fail-open `#[cfg(feature = "...")]` tests (2
  under `live-providers` in `aether-ai::openai`, 8 under `gui` in
  `aether-app::graph_view`) — unknown feature predicates intentionally stay
  marked as tests so a differently-configured build cannot silently lose
  them. Zero graph-side Rust test identities are unexplained; zero are
  missing. Cargo/pytest identity *translation* (turning a graph path into
  Cargo's own id) remains a named residual, not implemented here — this
  reconciliation was done once, by hand, to verify the closure.

## Limits and next defects

The oracle is deliberately small and deterministic. Its probes are fixture
instrumentation, not a general coverage backend; it measures one mutation per
language and does not establish behavior on real repositories. Duplicate test
ids, hanging/noisy children, cross-test checkout contamination, incomplete test
inventories, and mid-run binary replacement now fail the measurement instead of
silently changing its truth set. The measured defect order is:

1. Reconcile the remaining graph/framework test-inventory mismatch beyond
   this tree: Cargo/pytest identity translation, custom `[[bin]]` targets in
   the discovery inventory itself, and macro-expanded conditional identities
   remain unmodeled.
2. Extend the oracle to more representative repositories and broader
   mutations. `docs/core-representative-mutations.md` is the first step —
   one cached real repository, three declared cases — and already found a
   new gap (fixture-mediated polymorphic dispatch; see
   `core-gap-analysis.md` gap 11).
3. Close the self-review coverage-attribution gap: the checked oracle
   command executes the end-to-end runner functions, but self-review still
   labels them uncovered.

Those semantic fixes are explicitly outside this measurement milestone.
