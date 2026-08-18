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

## Use Bit Code instead of reading files

Before reading a file to understand a function, run:
    bitcode context . --nodes <node::path> "<what you need>" --json
That returns the function's source alone. Measured 41% fewer tokens than
reading the file.

To find the node path: bitcode search . "<description>"

To see what changed: bitcode review . --quiet
Not git diff. --quiet is 9 lines where the full output is 801.

To run tests:
    T=$(bitcode test-impact . --quiet); if [ -n "$T" ]; then cargo test $T; else echo "no impacted tests"; fi
Not the full suite. Runs only tests reachable from what changed.
The guard matters: an empty selection means nothing needs testing, and a bare
`cargo test $(...)` would run everything instead.

To answer a question about the codebase: bitcode query . "<question>"
No file reading required.
