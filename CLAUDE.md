# CLAUDE.md

Guidance for working in this repository.

## Active soundness program

Read `docs/roadmap.md` at the beginning of every session and update its checkpoint
before ending. It is the committed resume contract: preserve stage order, frozen
policies, failed observations, and resource constraints. A stage is
not DONE without committed passing evidence. Stage 4 is client-agnostic (Claude
Code, Cursor, and Codex at minimum), with raw MCP JSON as the universal fallback.
The program's full stage gates override advisory test selection below.

## What this is

**Girder** is a semantic-graph MCP server and CLI for coding agents. It provides
exact function source, symbol lookup, graph queries, and advisory test selection.
The native GUI, parallel agent swarm, and time-travel debugger are supporting
components built around the same graph.

It is a Cargo workspace, not a fork of any editor. See `README.md` for usage
and the measured engineering documentation under `docs/` for implementation
status and limitations.

## Workspace layout

- `crates/aether-graph` — semantic graph: nodes/edges, impact analysis,
  similarity/search, `.aether` serialization. **The source of truth; everything
  depends on it.**
- `crates/aether-builder` — tree-sitter → graph (Rust, Python, TypeScript/TSX,
  and Go), file edit sync, project-wide call resolution, highlight spans.
- `crates/aether-ai` — `AiProvider` trait, offline `MockProvider` (default),
  `Router`, and implemented OpenAI, Anthropic, and local Ollama providers behind
  `--features live-providers`; Gemini/Grok remain compile-clean extension points.
- `crates/aether-agents` — the swarm: broadcast bus, orchestrator, 8 agents.
- `crates/aether-debugger` — toy-language recording interpreter + branching
  timeline + what-if.
- `crates/aether-dap` — Debug Adapter Protocol client/session layer and
  graph-aware breakpoint support.
- `crates/aether-app` — the `girder` binary: MCP server, graph discovery and
  editing CLI, headless smoke, and egui GUI (`--features gui`).

## Common commands

```bash
cargo test --workspace                 # all tests (must stay green)
cargo clippy --workspace --all-targets # must be warning-free (CI uses -D warnings)
cargo fmt --all                        # format before committing
cargo run -p aether-app                # headless end-to-end demo
cargo run -p aether-app -- analyze sample-project
cargo check -p aether-ai --features live-providers
cargo test -p aether-dap --test debugpy -- --ignored
```

## Agent tooling: use Girder's own CLI

*(moved here from `AGENTS.md` so it carries project-instruction weight)*

### Token discipline

- Run `cargo test --workspace -j1 --quiet`. Never print full test output.
- Pipe clippy and fmt through `tail -5`.
- Long-running jobs: redirect to a log file and wait once. Do not poll
  repeatedly — each poll re-sends the entire context for zero new information.
- Select tests with `girder test-impact . --quiet` before invoking cargo.
- Use `girder query` for call relationships instead of grep or reading files.
- Always pass `--quiet` to `girder review` and `girder test-impact`.

### Authority

Girder's coverage output is advisory only. It routinely reports tested paths
as uncovered and over-selects unrelated tests. Cargo and the mutation oracle
are authoritative.

Selecting extra tests costs CPU time; missing a relevant test can conceal a
regression. The current bounded Rust and Python trustworthiness fixtures both
measure precision/recall `1.000/1.000`; the historical Rust precision defect at
`0.667` is closed ([measurement](docs/core-trustworthiness-measurement.md)).
The documented dynamic-dispatch resolution gap remains open (Girder still
cannot resolve a `self.invoke` dispatch through an untyped fixture
parameter), but the representative mutation that exposed it now measures
recall `1.000`/precision `0.667` instead of recall `0.000` — a 2026-10
`test-impact` fix (see `docs/observations/stage3-typescript-audit/
before-observation-addendum-5.md`) stopped the narrow, resolved-reachability
selection from silently returning empty whenever the dispatch can't be
resolved, falling back to the conservative union instead. The underlying
resolution gap is unchanged; only the empty-selection symptom is fixed
([evidence](docs/core-representative-mutations.md)).

### Use Girder instead of reading files

Before reading a file to understand a function, run:
    girder context . --nodes <node::path> --json --source-only
That returns the selected node's `{path, language, source}` and nothing
else. Measured 97.85% fewer bytes than reading the whole file, across ten
nodes sampled by source-size decile, and cheaper on all ten
(`docs/context-vs-read-cost.md`).

Pass `--source-only` whenever you are *reading*. Without it, `context` also
emits a Plan Format v2 schema and plan skeleton for authoring a plan — a
fixed ~6 KB that made it *more expensive* than reading the file on 2 of
those 10 nodes, and 15.6x the file for a small one. Bytes, not tokens; no
tokenizer was run.

To find the node path: girder search . "<description>"

To see what changed: girder review . --quiet
Not git diff. --quiet prints only changed node paths, one per line, where
the full report adds impact radius and coverage for each. How much that
saves depends entirely on the size of the diff, so there is no fixed ratio.

To run tests:
    T=$(girder test-impact . --quiet); if [ -n "$T" ]; then cargo test -- $T; else echo "no impacted tests"; fi
This is the conservative must∪may∪unknown union (docs/call-classification-policy.md),
not a targeted answer: it includes any test that cannot be proven unreachable,
which as of Stage 1 is frequently close to the full suite (see
docs/observations/stage1/measurement-summary-final.json). Pass --classified to
see why each test was included. Per the frozen policy's own exact wording:
"Empty output with Unknown boundaries never means no tests need running" —
empty is only trustworthy when stderr shows zero boundaries too. A change
with no Function-node origin at all (a const-only or type-only edit, a
`describe`/`beforeEach`-level statement in TypeScript, or a node that lost
a semantic-path collision — see docs/observations/stage3-typescript-audit/
before-observation-addendum-4.md) used to return empty here with NO
boundary notice regardless of whether boundaries existed elsewhere in the
graph (`classified_impact(&[])` short-circuiting on an empty origin list,
before ever computing them) — fixed for every invocation form, in both
`test-impact`'s own CLI path AND the Plan Format v2 mandatory
`tests.impacted` check, which had the identical gap independently
(`crates/aether-app/src/project/git.rs::semantic_changed_impact_with_config`,
`crates/aether-app/src/project/commands/test_impact.rs`,
`crates/aether-app/src/project/planfile/checks/test_checks.rs`; see
before-observation-addendum-5.md, -6.md, -8.md, and -9.md). A Module-only
origin's fail-closed behavior was checked directly against a real,
genuinely zero-boundary Go fixture (confirmed via `inspect`: a same-file
call proven Must with zero Unknown claims anywhere) and found NOT
fail-closed on its own for Go specifically — fixed in `classified_impact`
itself (`crates/aether-graph/src/claims.rs`) to push a boundary for a
Module origin with no same-file Function origin. The guard still matters
for the common case: a bare `cargo test $(...)` would run everything
instead of respecting an empty selection. Note the real cost of the
fallback: it is graph-wide once triggered, not scoped to the edited
function — see before-observation-addendum-9.md's "scale" disclosure for
a real measurement (472 unknown-classified nodes graph-wide from one
unresolved-dispatch edit in a representative repository).

To answer a question about the codebase: girder query . "<question>"
No file reading required.

## Conventions & invariants

- **The default build must always compile and all tests must pass.** This is the
  binding constraint.
- The **graph is the source of truth**; text/files are projections. Plan Format
  v2 graph edits address exact semantic paths and lower verified Rust/Python
  projections to journaled file writes; v1 text-addressed plans remain supported.
  New code should mutate the graph, not treat files as authoritative.
- **Node ids are path-derived** (`NodeId::from_path`, FNV-1a of the semantic
  path). They survive body edits and serialization, but a semantic rename creates
  a new id and remaps graph edges.
- Graph-addressed plan edits re-resolve after every edit. A later step must use
  a rename's new semantic path; old paths fail closed instead of aliasing.
  Format v2 supports `replace_node`, `rename_node`, `delete_node`, and
  `insert_into_module` on Rust/Python projections. `replace_node` replaces the
  complete `Node.source` declaration, including its signature. Function renames
  rewrite the definition and syntax-verified graph-proven call sites; ambiguous
  call sites, imports/re-exports, recursive calls, or any other unproven
  identifier occurrence reject the rename instead of risking a partial projection.
- **Calls are resolved project-wide** in `aether-builder::sync::resolve_calls`,
  not per file. Don't add `Calls` edges during extraction.
- Agents must **not hold the graph mutex across an `.await`** (lock, mutate,
  drop, then await/emit).
- Keep the offline `MockProvider` as the terminal fallback so the demo/tests run
  with no network or API key. OpenAI, Anthropic, and Ollama are implemented live
  providers; Ollama is enabled only by a live-provider build plus `OLLAMA_HOST`.
  Gemini and Grok must remain explicit EXTENSION POINTs until their real
  request/response code exists.
- Mark unfinished depth with `// EXTENSION POINT` rather than leaving it implicit.

## GUI/DAP notes

CI type-checks the `gui` feature on stable Rust. Rendering needs a GPU or a
software Vulkan adapter (Mesa **lavapipe**) plus X11 libs (e.g.
`libxkbcommon-x11`); with no surface it logs the wgpu error and falls back to the
headless demo. CI builds the default headless profile and keeps GUI compilation
guarded. To run the GUI headlessly: `Xvfb` + lavapipe (`WGPU_BACKEND=vulkan`,
`VK_ICD_FILENAMES=.../lvp_icd.json`).

The DAP adapter test is ignored in the default suite because it requires
`python3 -m debugpy.adapter`. Run it explicitly with
`cargo test -p aether-dap --test debugpy -- --ignored --nocapture` after
installing `debugpy`.

## Environment

- This VM (ChromeOS Crostini, ~2.7 GB RAM) cannot sustain a parallel build.
  Always build with `-j1` and never run a second `cargo` job concurrently —
  a concurrent build has starved this VM before.
- The toolchain and target dir live on
  `/mnt/chromeos/removable/MOVESPEED`. `cargo install` **ignores
  `CARGO_TARGET_DIR`**, so `--target-dir` must be passed explicitly on every
  invocation, e.g. `cargo install --path crates/aether-app --target-dir
  /mnt/chromeos/removable/MOVESPEED/aetherforge-install`.
- Two `girder` binaries exist: the one `cargo install` places on
  `~/.cargo/bin`, and the debug build under `target/debug/`. A stale
  `~/.cargo/bin/girder` left on `PATH` from before a feature change has
  silently invalidated verification before — rerun the `cargo install`
  above after any feature work, and check `which girder` if a just-added
  flag or command appears not to exist.
- `sample-project/` is a pinned measurement fixture (enforced by
  `reject_measurement_fixture_root`) and is refused for authored plan runs.
  Use `demo-project/` for any live plan execution.
- The unguarded `cargo test $(girder test-impact . --quiet)` form (broken
  by cargo's single-positional-filter limit) was checked across this repo:
  it appears only in `docs/core-gap-analysis.md`'s past-tense narrative
  about the incident that found it, not as live guidance anywhere. `main.rs`
  USAGE, this file, and `README.md` already use the guarded form.
