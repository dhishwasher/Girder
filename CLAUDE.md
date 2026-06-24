# CLAUDE.md

Guidance for working in this repository.

## What this is

**AetherForge IDE** — a native Rust IDE prototype built on three pillars:
1. A **living semantic graph** as the source of truth (not files).
2. A **parallel AI agent swarm** that mutates the graph.
3. A **time-travel & branching debugger**.

It is a Cargo workspace, not a fork of any editor. See `BLUEPRINT.md` for the
full design and `README.md` for usage.

## Workspace layout

- `crates/aether-graph` — semantic graph: nodes/edges, impact analysis,
  similarity/search, `.aether` serialization. **The source of truth; everything
  depends on it.**
- `crates/aether-builder` — tree-sitter → graph (Rust + Python), incremental
  edit sync, project-wide call resolution, highlight spans.
- `crates/aether-ai` — `AiProvider` trait, offline `MockProvider` (default),
  multi-provider `Router`; remote providers behind `--features live-providers`.
- `crates/aether-agents` — the swarm: broadcast bus, orchestrator, 7 agents.
- `crates/aether-debugger` — toy-language recording interpreter + branching
  timeline + what-if.
- `crates/aether-app` — the `aetherforge` binary: CLI (`analyze`/`search`/
  `forge`/`inspect`/`demo`) + headless smoke + egui GUI (`--features gui`).

## Common commands

```bash
cargo test --workspace                 # all tests (must stay green)
cargo clippy --workspace --all-targets # must be warning-free (CI uses -D warnings)
cargo fmt --all                        # format before committing
cargo run -p aether-app                # headless end-to-end demo
cargo run -p aether-app -- analyze sample-project
```

## Conventions & invariants

- **The default build must always compile and all tests must pass.** This is the
  binding constraint.
- The **graph is the source of truth**; text/files are projections. New code
  should mutate the graph, not treat files as authoritative.
- **Node ids are stable** (`NodeId::from_path`, FNV-1a of the semantic path) so
  edges/agent refs/debugger steps survive edits and serialization.
- **Calls are resolved project-wide** in `aether-builder::sync::resolve_calls`,
  not per file. Don't add `Calls` edges during extraction.
- Agents must **not hold the graph mutex across an `.await`** (lock, mutate,
  drop, then await/emit).
- Keep the offline `MockProvider` the default so the demo/tests run with no
  network or API key. Remote providers are EXTENSION POINTs.
- Mark unfinished depth with `// EXTENSION POINT` rather than leaving it implicit.

## Known limitation

The `gui` feature does **not compile on rustc 1.94** due to an upstream `winit`
type-inference regression (`E0282`), independent of this code. CI and the
default build deliberately do not build the GUI. The headless paths are the
verification surface in restricted environments.
