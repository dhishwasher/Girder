# Agent instructions

## Token discipline

- Run `cargo test --workspace -j1 --quiet`. Never print full test output.
- Pipe clippy and fmt through `tail -5`.
- Long-running jobs: redirect to a log file and wait once. Do not poll
  repeatedly — each poll re-sends the entire context for zero new information.
- Select tests with `bitcode test-impact . --quiet` before invoking cargo.
- Use `bitcode query` for call relationships instead of grep or reading files.
- Always pass `--quiet` to `bitcode review` and `bitcode test-impact`.

## Authority

Bit Code's coverage output is advisory only. It routinely reports tested paths
as uncovered and over-selects unrelated tests. Cargo and the mutation oracle
are authoritative.
