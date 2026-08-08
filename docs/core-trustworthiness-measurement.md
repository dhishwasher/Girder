# Core Trustworthiness Measurement

This milestone establishes a checked, reproducible function-execution oracle
for Bit Code's affected-test selection. It does not claim representative-repo
coverage or general superiority.

## What is measured

The committed Rust and Python templates are materialized into disposable Git
repositories. The runner commits each baseline, applies one source mutation,
and asks the exact supplied Bit Code binary for `test-impact`.

Every test is then executed alone with `BITCODE_ORACLE_PROBE` pointing at a
fresh file. Only the changed function/method writes that probe, so the resulting
set is direct runtime evidence that a test executed changed code. This is a
function-level dynamic coverage oracle, not line or branch coverage.

Artifacts:

- `tools/core_trustworthiness_oracle.py` — end-to-end runner and baseline check.
- `tools/test_core_trustworthiness_oracle.py` — parser/metric unit tests.
- `fixtures/core-trustworthiness/` — inert `.txt` source templates, excluded
  from Bit Code's own source graph until materialized in a temporary project.
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
  --bitcode /mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/bitcode
```

Add `--verbose` to include Bit Code's raw `test-impact` output. A result that
differs from the checked JSON baseline exits unsuccessfully and prints both
expected and actual data.

## Baseline results

| Fixture | TP | FP | FN | TN | Precision | Recall |
|---|---:|---:|---:|---:|---:|---:|
| Rust | 2 | 1 | 0 | 1 | 0.667 | 1.000 |
| Python | 2 | 0 | 0 | 1 | 1.000 | 1.000 |
| Combined | 4 | 1 | 0 | 2 | 0.800 | 1.000 |

Rust:

- Runtime-executed changed code:
  `rust_direct_selected`, `rust_cli_selected`.
- Bit Code selected both true tests and also `rust_cli_unrelated`.
- The false positive is the known argument-route limitation: both CLI tests
  launch the same Cargo binary, and the graph reaches every branch called by
  `main` without modeling the concrete argument.

Python:

- Runtime-executed changed code:
  `test_python_direct_selected`, `test_python_optional_selected`.
- Bit Code selected both runtime-proven tests and excluded the same-method
  `DecoyIdentity` test.
- Nullable annotations now provide a receiver owner only when exactly one
  non-null type remains. PEP 604, parenthesized, and forward-string forms are
  supported directly. `Optional[T]` and `Union[T, None]` are supported only
  when the wrapper resolves through an import from `typing` or
  `typing_extensions`, including aliases and imports nested under
  `TYPE_CHECKING` or inside the function. Multi-owner and untrusted custom
  unions deliberately remain unresolved rather than creating false edges.

## Bit Code dogfood assessment

Useful output:

- `test-impact` identified the exact changed function in each disposable Git
  repository.
- It preserved the direct Rust/Python receiver path and the conventional Cargo
  binary subprocess path.
- It restored the dynamically proven nullable Python receiver path without
  selecting the same-method decoy owner.
- Its listed tests were machine-parseable and its impacted/skipped header counts
  were internally consistent on these fixtures.
- On the milestone tree, query correctly reports `main` as the sole caller of
  `measure_fixture`, test-impact selects all four oracle unit tests, and review
  reconstructs the runner's internal helper chain.

Incorrect or incomplete output:

- Rust subprocess reachability over-selected an argument-incompatible CLI test.
- The suggested commands are selection output, not dynamic proof. The Python
  fixture is deliberately executed with standard-library `unittest`; the oracle
  does not assume the emitted `pytest -k` command establishes coverage.
- Self-review labels the end-to-end runner functions uncovered even though the
  checked oracle command executes `main`, fixture initialization, Bit Code
  invocation, dynamic test execution, metric calculation, rendering, and
  baseline comparison.
- Self-impact reports 285 tests (`80` impacted plus `205` skipped). The
  authoritative suites contain 269 Cargo cases, including the intentional
  debugpy ignore, plus four oracle unit tests: 273 total. Bit Code therefore
  overcounts this tree's declared test inventory by 12 and substantially
  over-selects through broad builder/application reachability.

## Limits and next defects

The oracle is deliberately small and deterministic. Its probes are fixture
instrumentation, not a general coverage backend; it measures one mutation per
language and does not establish behavior on real repositories. The measured
defect order is:

1. Improve CLI argument-route precision (observed Rust precision `0.667`)
   without losing subprocess-entrypoint recall.
2. Extend the oracle to representative repositories and broader mutations,
   then measure implicit RAII/`Drop`, custom Cargo target paths, and shared
   infrastructure over-selection.

Those semantic fixes are explicitly outside this measurement milestone.
