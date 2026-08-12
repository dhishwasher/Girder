# CLAUDE.md

Guidance for working in this repository.

## What this is

**Bit Code** — a native Rust IDE prototype built on three pillars:
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
  `Router`, and implemented OpenAI, Anthropic, and local Ollama providers behind
  `--features live-providers`; Gemini/Grok remain compile-clean extension points.
- `crates/aether-agents` — the swarm: broadcast bus, orchestrator, 8 agents.
- `crates/aether-debugger` — toy-language recording interpreter + branching
  timeline + what-if.
- `crates/aether-dap` — Debug Adapter Protocol client/session layer and
  graph-aware breakpoint support.
- `crates/aether-app` — the `bitcode` binary: CLI (`analyze`/`search`/
  `forge`/`inspect`/`demo`) + headless smoke + egui GUI (`--features gui`).

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

## Conventions & invariants

- **The default build must always compile and all tests must pass.** This is the
  binding constraint.
- The **graph is the source of truth**; text/files are projections. New code
  should mutate the graph, not treat files as authoritative.
- **Node ids are path-derived** (`NodeId::from_path`, FNV-1a of the semantic
  path). They survive body edits and serialization, but a semantic rename creates
  a new id and remaps graph edges.
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
