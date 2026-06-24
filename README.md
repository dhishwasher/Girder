# AetherForge IDE ⚡

**The Semantic Agentic Forge** — a native, GPU-accelerated IDE whose source of
truth is a **living semantic graph** of your codebase, edited in real time by a
**parallel AI agent swarm**, and debugged with a first-class **time-travel &
branching debugger**.

This repo is a runnable Rust prototype of that architecture. It is *not* a fork
of any existing editor. See [`BLUEPRINT.md`](./BLUEPRINT.md) for the full design.

## Quickstart

```bash
# Full end-to-end demo — no GPU, display, or API key required:
cargo run -p aether-app

# Run the test suite (graph, builder, AI router, agent swarm, debugger):
cargo test --workspace
```

### Use it on a real project

AetherForge is also a CLI that operates on actual directories:

```bash
# Build the semantic graph from a project and save it as <dir>/project.aether:
cargo run -p aether-app -- analyze sample-project

# Inspect a saved graph and a node's impact set:
cargo run -p aether-app -- inspect sample-project/project.aether crate::lib::add

# Concept search: rank functions by relevance to a natural-language query:
cargo run -p aether-app -- search sample-project "sum numbers in a list"

# Dispatch the agent swarm on a project with a natural-language intent:
cargo run -p aether-app -- forge sample-project "add a subtract function"

# Full help:
cargo run -p aether-app -- --help
```

`analyze`/`forge` walk every `.rs`/`.py` file (skipping `target`, `.git`, …),
build the graph with directory-aware module paths, resolve calls across files,
and persist the `.aether` graph.

### The pipeline demo

`cargo run -p aether-app` (no args) walks the entire pipeline over stdout:

1. **Builds a semantic graph** from source with tree-sitter (the graph is the
   source of truth; text is a projection).
2. **Dispatches the agent swarm** on a natural-language intent
   (*"Add a multiply function to the math module"*) and shows the
   Planner→Coder→Tester collaboration mutating the graph live, including the new
   function's agent-authored source, summary, risk tag, and generated test.
3. **Runs predictive impact analysis** over the graph.
4. **Time-travels a deliberate bug**: records an execution trace, branches a
   what-if fix that re-propagates downstream, and asks the AI layer for a root
   cause.

## Workspace

| Crate | Role |
|---|---|
| `aether-graph` | The living semantic graph: nodes/edges, impact analysis, `.aether` serialization. **Source of truth.** |
| `aether-builder` | tree-sitter → graph mapping, incremental edit sync, syntax-highlight spans. |
| `aether-ai` | `AiProvider` trait, offline `MockProvider`, multi-provider `Router`, provider EXTENSION POINTs. |
| `aether-agents` | The parallel swarm: message bus, orchestrator, 7 specialized agents. |
| `aether-debugger` | Recording interpreter, execution trace, branching timeline, what-if + AI root-cause. |
| `aether-app` | egui/wgpu GUI (feature `gui`) + the headless `smoke` demo binary. |

## The GUI

```bash
cargo run -p aether-app --features gui
```

Four resizable panels: a graph visualizer, a code-projection editor with
tree-sitter highlighting that folds edits back into the graph, an agent console,
and a time-travel debugger timeline with branch controls.

> **Known limitation:** the `gui` feature currently fails to **compile on rustc
> 1.94** because the transitive `winit` crate hits an upstream closure
> type-inference regression (`E0282`) on that toolchain — before any AetherForge
> UI code is reached. The UI is written against the egui 0.31 API and is expected
> to build on a toolchain where `winit` compiles. **The default headless build,
> all tests, and the `cargo run` demo are unaffected.**

## Local-first AI

The default provider is a deterministic, offline `MockProvider`, so everything
runs with no network and no API key. Real providers (Anthropic Claude, OpenAI,
Gemini, xAI Grok, local Ollama) are compile-clean extension points behind the
`live-providers` feature; the `Router` prefers them when keys are present and
falls back to the mock otherwise.

## Status

A focused, honest prototype: the three pillars (graph-as-truth, agent swarm,
branching time-travel debug) are real, tested, and runnable at small scale. The
production roadmap (CRDT collaboration, generative extensions, self-optimization,
web/mobile projections, real execution tracing) is in `BLUEPRINT.md §9`.
