# Girder

<!-- mcp-name: io.github.dhishwasher/girder -->

**Girder saves AI coding-agent context by giving the agent exactly the code
it needs instead of whole files.** It builds a local semantic graph of your
repository so Claude Code, Codex, Cursor, and other agents can retrieve
exact functions, definitions, callers, callees, and task context without
filling their context window with irrelevant code. It runs as one static
binary, with no account and no cloud dependency.

```bash
npx -y girder-mcp setup
```

That one command detects and configures Claude Code, Codex, and Cursor.

[![CI](https://github.com/dhishwasher/Girder/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/dhishwasher/Girder/actions/workflows/ci.yml)
Latest stable release: [v0.4.0](https://github.com/dhishwasher/Girder/releases/tag/v0.4.0) (Apache-2.0).
Contributions: see [`CONTRIBUTING.md`](./CONTRIBUTING.md).

**Measured output bytes, not tokens (no tokenizer was run):**

- CLI stdout of `girder context ... --json --source-only` (MCP `get_source`)
  vs reading the whole file: **97.85% fewer bytes**, across ten functions
  sampled by source-size decile
  ([method and honest limits](./docs/context-vs-read-cost.md)).
- CLI stdout of `girder names ... --json` (MCP `find_definition`) vs plain
  grep: **97.98% fewer bytes**, across ten identifiers
  ([method](./docs/names-cost.md)).
- `orient` (one bundled call: source, callers, callees, tests, and impact
  for one node) vs the equivalent chain of separate calls: **48,814 bytes
  vs 101,302**, using **15 calls vs 78**, on a 15-task corpus spanning ten
  pinned repositories ([method](./docs/orient-tool.md)).

The two percentages measure CLI output; MCP transport overhead was not measured.

## Support Girder

Girder is independently developed to help coding agents consume less code:
precise semantic context instead of whole-file reads and repeated repository
greps. Sponsorship helps fund context efficiency, semantic graph correctness,
agent integrations, reproducible benchmarks, and maintenance. Girder publishes
its measurements, including failures and limitations. If Girder saves you
context, compute, or development time, [sponsor its continued development](https://github.com/sponsors/dhishwasher).

**Funding goal:** $1,000/month to fund ongoing maintenance, releases,
benchmarks, and continued context-efficiency work on Girder. See
[what sponsorship supports](./docs/sponsorship.md).

Real, captured output (see
[`docs/observations/release-prep/`](./docs/observations/release-prep/) for
exact provenance):

```
$ girder context demo-project --nodes crate::greeter::shout_greeting --json --source-only
{
  "nodes": [{
    "language": "python",
    "path": "crate::greeter::shout_greeting",
    "source": "def shout_greeting(name):\n    return format_greeting(name).upper()"
  }]
}
```

One function's exact source returned, not the whole file around it — that
is the core of what Girder gives an agent's context window.

**Languages, stated plainly, not as parity:** Rust and Python each have a
completed 100-call-site audit across three pinned repositories, with no unsound
answers in that sample. Each produced **1 Must and 99 Unknown** answers; the
precision evidence is limited ([measurement](./docs/observations/stage3-audit-reconciliation/after-observation.md)).
TypeScript is measured and gated but explicitly **in progress** — it still has a separate, disclosed node-identity collision
mechanism (duplicate `it()`/`describe()` description strings can silently
overwrite each other's graph node; see
[`docs/observations/stage3-typescript-audit/`](./docs/observations/stage3-typescript-audit/)).
Go's Stage 3 audit is **not complete**. See
[`docs/typescript-support.md`](./docs/typescript-support.md) and
[`docs/go-support.md`](./docs/go-support.md) for the exact, measured limits
of each.

**Also included, as an additional capability built on the same graph —
conservative change-impact and test selection:** `test-impact` names the
tests a change may affect, from the same semantic graph, erring toward
including a test it can't rule out rather than silently dropping it. Real
captured output:

```
$ girder test-impact . --quiet
test-impact: 82 unresolved call-evidence boundaries found; this selection is the conservative must∪may∪unknown union — pass --classified to see why each test is included
test_farewell
test_format_greeting
test_shout_greeting_delegates_to_format_greeting
```

See [Status](#status) for exactly what that selection does and doesn't
prove.

**Pricing and license:** Girder is free and open source under the
[Apache License 2.0](./LICENSE). There is no current paid tier, and no license
key or account is required — see [License](#license).

**Evidence, for a skeptical reader:**
[`docs/evidence-index.md`](./docs/evidence-index.md) points at every
committed audit, measurement, and adversarial finding behind the claims
above — including the bugs found and fixed along the way, not just a final
number.

## Install

A prebuilt binary, no Rust toolchain needed:

```bash
curl -fsSL https://raw.githubusercontent.com/dhishwasher/Girder/main/install.sh | sh
girder --version
```

Or from source:

```bash
cargo install --path crates/aether-app
girder --help
```

This builds the default headless profile and installs the `girder` binary to
`~/.cargo/bin` (make sure it's on your `PATH`). No GPU, display, network, or API
key is required.

### Windows installer

Each Windows release also includes `Girder-<version>-setup.exe`. It installs
for the current user under `%LOCALAPPDATA%\Programs\Girder`, adds Girder to the
user `PATH`, creates a Start Menu shortcut, and does not request administrator
access. Open a new terminal after installation so it sees the updated `PATH`.
The installer includes the desktop GUI, and its Start Menu shortcut opens it.
The archives and npm installation continue to provide the headless CLI.

The installer is **not code-signed yet**, so Windows SmartScreen will warn on
first run. After downloading the installer from the GitHub release, double-click
it, choose **More info** on the “Windows protected your PC” dialog, verify that
the app is Girder and the publisher is shown as unknown, then choose **Run
anyway**. If those details do not match, cancel instead.

## Competitor benchmark

On the 20-task modest cross-language base set, Girder passed **5/20** versus
Ripwire's **7/20** — a loss. But across the 60 attempts where both products
passed, Girder returned **35,764 bytes** versus Ripwire's **158,514** — 77.4%
fewer — at identical call counts (120 each). GitNexus could not be scored: its
install exceeded the frozen 1 GiB ceiling on this measurement machine, which is
a fact about install footprint on a constrained host, not a claim about its
quality.

No product in this benchmark reached full correctness, and the benchmark does
not establish an overall best product. See the full
[report](./docs/competitor-benchmark/results/modest-final/report.md) for
methodology, per-language results, and everything the numbers above don't
establish.

## Use it from an AI coding agent

`girder mcp` serves the read-only graph commands over the
[Model Context Protocol](https://modelcontextprotocol.io), so an agent can ask
about your codebase instead of reading files into its context window.

Preview the changes, then configure detected agents:

```bash
npx -y girder-mcp setup --dry-run
npx -y girder-mcp setup
```

`setup` detects Claude Code (`~/.claude` or an existing in-home project
`.mcp.json`), Codex (`~/.codex` or an in-home `CODEX_HOME`), and Cursor
(`~/.cursor`). It merges the `girder` MCP entry into each detected agent's
documented config and installs nested `PreToolUse` and `PostToolUse` hooks for
Claude Code and Codex. Pre hooks cover structured `Read`, `read_file`, and
`mcp__.*__read_file` events; post hooks cover Claude's
`Edit|Write|NotebookEdit` and Codex's `apply_patch|Edit|Write` edit names.
Shell commands are intentionally not parsed. Cursor documents hooks, but no
pre-read hook can add advisory context without blocking (`preToolUse` and
`beforeReadFile` only allow, deny, or rewrite input; only `postToolUse` and
`sessionStart` return `additional_context`, after the fact), so setup registers
MCP and the project rule for Cursor but no hook. A generic MCP client has no
universal config path and is reported as not detected. Setup never writes outside your home directory. Existing foreign
`girder` entries remain untouched, including with `--force`; `girder setup
--uninstall` removes only setup-owned changes.

**The orient-first instruction.** Besides MCP, setup installs one short shared
instruction through each client's own mechanism, so the agent calls Girder's
`orient` before broad reads or grep, says so when Girder is unsure, and falls back
to reading source rather than guessing:

| Client | Instruction location | Notes |
| --- | --- | --- |
| Claude Code | a delimited block in `~/.claude/CLAUDE.md` | Claude Code concatenates user and project `CLAUDE.md` files into context |
| Codex | a delimited block in `$CODEX_HOME/AGENTS.md` (default `~/.codex/AGENTS.md`) | skipped and reported if a non-empty `AGENTS.override.md` exists, because Codex then ignores `AGENTS.md` |
| Cursor | `.cursor/rules/girder-orient.mdc` in the current project, only with `girder setup --project` | project rules are `.mdc` files with `alwaysApply: true`; Cursor documents no file path for user rules, so none is written |

The block is marked and recorded in `girder-setup-state.json`: re-running setup
changes nothing, `--uninstall` restores the file exactly, an edited block is left
alone, and `--no-instructions` skips this step. The formats above were checked
against each client's own documentation on 2026-10-06; sources and quotes are in
[`docs/observations/stage4-clients/`](docs/observations/stage4-clients/doc-verification.md).

**Which form installs what.** `girder setup` from a built or installed binary
installs MCP, hooks, and the instruction, and writes an MCP entry that launches
that installed binary (`<path-to-girder> mcp .`), so clients run local Girder with
no package download at launch. `npx -y girder-mcp setup` runs the *published*
package's setup, which installs the instruction only once a release that includes
it ships; when setup runs from npx's transient cache there is no stable binary path,
so the entry falls back to the `npx -y girder-mcp .` launcher form and setup says so
(install Girder, then run `girder setup`, to switch to the local binary).

The packaged hook launcher forwards each event to the pinned native `girder hook`
executable and fails open on errors. Only standalone `PreToolUse` JSON is
forwarded on stdout; post-edit native diagnostics pass through stderr, and MCP
continues to use JSON-RPC on its own stdout. The read advisory only gives
guidance when `project.aether` already exists. The post-edit advisory uses that
saved graph snapshot, does not scan or rebuild the project, and describes the
last analyzed version of changed files. See the [setup and client path
guide](./docs/setup.md) for config paths, ownership, and the manual JSON
fallback.

For clients setup cannot detect, or to do it by hand, add the entry to the
client's documented MCP config. Placement per client:

| Client | Where | Format |
| --- | --- | --- |
| Claude Code | project `.mcp.json` or user `~/.claude.json` | JSON `mcpServers.girder` (below) |
| Cursor | project `.cursor/mcp.json` or user `~/.cursor/mcp.json` | JSON `mcpServers.girder` (below) |
| Codex | `~/.codex/config.toml` (or a trusted project's `.codex/config.toml`) | TOML table, shown after the JSON |
| Any other MCP client | that client's documented MCP config | the same `command` and `args` |

JSON form:

```json
{
  "mcpServers": {
    "girder": {
      "command": "npx",
      "args": ["-y", "girder-mcp", "."]
    }
  }
}
```

Codex uses a TOML table instead:

```toml
[mcp_servers.girder]
command = "npx"
args = ["-y", "girder-mcp", "."]
```

With a binary already installed, `"command": "girder", "args": ["mcp", "."]`
skips npm entirely. To give any client the instruction by hand, paste the text of
[`npm/instructions/orient-first.md`](npm/instructions/orient-first.md) into its
documented instruction file (`CLAUDE.md`, `AGENTS.md`, or a Cursor `.mdc` rule).

Girder 0.2.6 and later can opt into a cached graph generation with
`girder mcp . --watch` or `npx -y girder-mcp . --watch`. The server keeps
one validated graph generation in memory and incrementally reparses changed
files. The committed watcher measurement matched fresh cold analysis after all
45 mutations while reusing 98.70% of file extractions; the claim is limited to
preserving cold-analysis resolution while reusing parsing. See the
[watcher result and limitations](./docs/mcp-watching.md).

Seven tools, all read-only and all free:

| Tool | What it answers |
|---|---|
| `get_source` | The source of specific functions, without the file around them. |
| `find_definition` | Where an exact identifier is declared. Not a substring search. |
| `search_code` | Which functions match a description, when you don't know the name. |
| `ask_codebase` | Callers, callees, and blast radius, by graph traversal. |
| `impacted_tests` | The conservative set of tests a change might reach — advisory, see below. |
| `review_changes` | What changed in the working tree, as semantics rather than text. |
| `orient` | Source, callers, callees, tests, and impact for one node, in one call. |

### What that saves, and what it doesn't

`orient` bundles what `get_source` + `ask_codebase` (callers, callees, and
impact) + `impacted_tests` otherwise answer across 5-6 separate calls into
one. On a 15-task corpus spanning ten pinned repositories, that one call used
**fewer aggregate bytes than the chain it replaces** (48,814 vs 101,302,
a 0.48 ratio) while cutting 78 round trips to 15 — one per task — and, after
two disclosed defects were fixed, **37 of 37 gated checks pass**. The first
run found `impacted_tests --quiet` silently dropping non-Rust/Python test
names (`orient`'s own test-coverage section did not share the bug, which is
how it was found); that filter is now removed. Its natural-language `intent`
input still inherits `search_code`'s accuracy — all three intent tasks in
this corpus resolved to the wrong node, unchanged and out of scope for this
fix — but `orient`'s confidence heuristic, which originally caught none of
the three, now flags all three `"confidence": "low"` with candidate scores
attached, at the cost of also flagging some correct resolutions when a
runner-up is close. See [`docs/orient-tool.md`](./docs/orient-tool.md) and
the committed [policy](./docs/orient-tool-policy.json) /
[original observation](./docs/orient-tool-observation.json) /
[post-fix observation](./docs/orient-tool-observation-post-fix.json).

Two additional precommitted measurements, both counting **bytes of command output rather
than tokens** (no tokenizer was run):

- `girder context ... --json --source-only` (what MCP `get_source` routes to)
  against a naive whole-file-read baseline: **97.85% fewer CLI stdout bytes**
  across ten functions sampled by source-size decile, cheaper on all ten
  ([`docs/context-vs-read-cost.md`](./docs/context-vs-read-cost.md)).
- `girder names ... --json` (what MCP `find_definition` routes to) against a
  plain-grep baseline: **97.98% fewer CLI stdout bytes** across ten
  identifiers ([`docs/names-cost.md`](./docs/names-cost.md)).

MCP transport overhead and tokens were not measured.

Both are single-repository measurements. The direction is structural — files
are much larger than the functions in them, and grep returns every mention
where `find_definition` returns only declarations — but the exact percentages
are not portable.

These comparisons do not measure a competent agent choosing grep searches and
bounded file reads adaptively. No agentic-grep cost claim is established here.

The subsequent [agentic-grep campaign](./docs/agentic-grep.md) with a local
1.5B model stopped incomplete and produced no pairs with two correct answers.
It establishes no comparative cost advantage or frontier-model behavior.

`impacted_tests` is **advisory**. It over-selects unrelated tests, and some
dynamic-dispatch shapes still aren't resolved — when Girder can't prove a call
site, the frozen policy requires including the related tests rather than
guessing they're safe to skip, so the current, measured consequence is
over-inclusion (recall `1.000`, precision `0.667` on the representative
polymorphic-dispatch case) rather than a silent miss. The underlying
dispatch-resolution gap is still open; only the selection no longer drops the
test silently. See
[`docs/core-representative-mutations.md`](./docs/core-representative-mutations.md)
for the measurement and exactly what changed. A full test run remains the
authority before calling a change safe.

Selecting extra tests costs CPU time; missing a relevant test can conceal a
regression.

The project root is fixed when the server starts, so no tool call can reach
another directory. `GIRDER_MCP_TIMEOUT_SECONDS` (default 120) bounds each
call; raise it for a very large repository.

Everything below assumes `girder` is on your `PATH`. Building from a source
checkout without installing works the same way with `cargo run -p aether-app --`
in place of `girder`.

## Quickstart

```bash
# Full end-to-end demo — no GPU, display, or API key required:
girder

# Run the test suite:
cargo test --workspace
```

### Analyze a repository

Use the CLI to build and query a repository's semantic graph:

```bash
# Build the semantic graph from a project and save it as <dir>/project.aether:
girder analyze sample-project

# Inspect a saved graph and a node's impact set:
girder inspect sample-project/project.aether crate::lib::add

# Graph-semantic rename — follows Calls edges (not text search) and rewrites
# only the real callers, then saves the updated graph:
girder refactor sample-project rename crate::lib::add plus

# Emit graph context, a real Plan Format v2 authoring schema, and a plan
# skeleton as one JSON object — for pasting into any external chat model.
# See "External authoring" below for the full loop to a verified, applied edit:
girder context demo-project --nodes crate::greeter::farewell "add an exclamation mark to the farewell" --json

# Validate/inspect/execute a plan file directly:
girder plan validate my-plan.json
girder plan explain my-plan.json
girder plan run my-plan.json --dry

# Semantic code review vs HEAD (typed mutations, not text diffs):
girder review sample-project --since HEAD~1

# Conservative test selection: find every test that might be reachable
# from changed functions:
girder test-impact sample-project --run

# Knowledge-graph query — answer a question by traversing the semantic graph:
girder query sample-project "what would break if I change add?"
girder query sample-project "what calls sum_list?"
girder query sample-project  # interactive REPL (reads stdin)

# Full help:
girder --help
```

## External authoring: `girder context` + `plan run --authored`

`girder do` drives a wired-in provider (OpenAI, Anthropic, Ollama, or the
offline `MockProvider`) automatically. `girder context` and `plan run
--authored` split that same workflow at the model boundary, so *any* chat
model — one with no API integration in this codebase at all — can author a
verified graph edit.

> **Reading, not authoring?** Add `--source-only`. The default output below
> carries a Plan Format v2 schema and plan skeleton, which is a fixed ~6 KB
> that a model authoring an edit needs and a model merely *reading* code does
> not — it measured *more expensive* than reading the whole file on 2 of 10
> nodes, and 15.6× the file for a small one. `--source-only` drops the
> envelope and measured 97.85% cheaper than a file read across the same ten
> nodes ([`docs/context-vs-read-cost.md`](./docs/context-vs-read-cost.md)).
> It also needs no git repository, having no `base_commit` to pin.

The authoring loop:

```bash
# 1. Emit graph context, a real Plan Format v2 authoring schema, and a plan
#    skeleton (base_commit, on_failure, the mandatory tests.impacted check)
#    as one JSON object:
girder context demo-project --nodes crate::greeter::farewell \
  "add an exclamation mark to the farewell" --json > context.json

# 2. Paste context.json into any chat model. Ask it to fill in the plan
#    skeleton's step (id, description, edits) so the step satisfies the
#    schema exactly, and save its response as plan.json — the schema is the
#    literal Plan Format v2 the executor loads, not an approximation of it,
#    so a model that follows it produces a file that loads without
#    translation.

# 3. Run and verify it. `plan run` has no project-directory argument — the
#    root is always the current directory, so run this from inside
#    demo-project/. --authored applies the same harness guarantees `do`
#    applies internally to a model-authored plan: on_failure is forced to
#    rollback_plan, a tests.impacted check is injected if the plan doesn't
#    already carry one, and a zero-step plan is refused outright rather than
#    passing vacuously. --authored-by <name> records who authored it in the
#    written report:
(cd demo-project && girder plan run ../plan.json --authored --authored-by claude-opus-5)
```

A passing run applies the edit to the real tree and writes a report under
`.girder/reports/`; a failing check rolls the tree back to `base_commit`
automatically, so a bad edit from an untrusted external model never lands
half-applied. `plan_schema()`'s exact shape (a `oneOf` discriminated union
per edit/check kind, matching `planfile::schema`'s deserializer field for
field) is what makes step 2 reliable — see gap 17 in
[`docs/core-gap-analysis.md`](docs/core-gap-analysis.md) for the defect this
closed and the round-trip test that proves it.

### Demo target: `demo-project/`, never `sample-project/`

`girder do` and `plan run --authored` both refuse `sample-project/` as a
target outright, with an error naming `demo-project/` as the place to run
instead:

```
$ girder do sample-project "uppercase greet's return value"
error: sample-project/ is a pinned measurement fixture (see gap 18/21 in
docs/core-gap-analysis.md) and refuses authored writes; run demos against
demo-project/ instead
```

`sample-project/` is a pinned baseline `tools/authoring_task_check.py` and
`tools/plan_executor_oracle.py` read against a specific clean source commit
for the graph-addressed-authoring-cost measurement corpus, not a scratch
target. A real (non-dry) authored write there mutates the same file the
measurement harness depends on — this happened twice, three days apart, and
cost a full day of debugging a broken referee before the fixture drift was
found; see gap 18 in `docs/core-gap-analysis.md`. `demo-project/` exists so
that never has to happen again: a small, disposable Python project nothing
under `tools/` or `docs/` reads, safe for real (non-dry) authored writes.
Read-only commands (`context`, `search`, `analyze`, `test-impact`) still work
against `sample-project/` — only the two commands that write are refused.

## Workspace

| Crate | Role |
|---|---|
| `aether-graph` | The living semantic graph: nodes/edges, impact analysis, and `.aether` serialization. **Source of truth.** |
| `aether-builder` | tree-sitter → graph mapping, incremental edit sync, syntax-highlight spans. |
| `aether-ai` | `AiProvider` trait, offline `MockProvider`, OpenAI/Anthropic live providers, multi-provider `Router`, provider extension points. |
| `aether-app` | The `girder` binary: the MCP server and the graph CLI. |

The workspace also contains older subsystems (agent swarm, collaboration, tracer and
DAP adapter, extensions, a desktop GUI). They are not part of the public tool, are off
by default, and build only with `--features legacy`.

## Status

Girder is a semantic-graph MCP server and CLI for coding agents. The
measurements below describe the graph's current behavior and limits.

The checked
[Core Trustworthiness Measurement](docs/core-trustworthiness-measurement.md)
compares affected-test selection with isolated runtime execution. Its bounded
baseline measures both Rust and Python precision/recall at `1.000/1.000`
(the earlier `0.667` Rust precision defect is closed). That is a result on
small fixtures, not a representative-repository superiority claim — and the
counter-evidence is checked in alongside it: on a real dependency, a
polymorphic-dispatch mutation currently measures recall `1.000`, precision
`0.667` ([`docs/core-representative-mutations.md`](docs/core-representative-mutations.md))
— a 2026-10 fix stopped the selection from silently dropping the test when
the dispatch can't be resolved, falling back to the conservative union
instead; the underlying dispatch-resolution gap is unchanged, only the
silent-miss symptom is fixed. This is why test selection is documented as
advisory rather than authoritative.

Implemented features:

| Feature | What it does |
|---|---|
| Semantic graph | Nodes/edges, Rust alias/return/scoped-pattern/Cargo-entrypoint-aware and Python alias/nullable-annotation/constructor-aware cross-file call resolution, literal-aware macro calls, impact BFS, similarity edges, strict versioned `.aether` persistence and source reconciliation |
| Semantic review | Typed diff (added/modified/removed nodes + edges), impact radius, test gap report |
| Conservative test selection | Call-graph reachability from changed functions, optional `--run` |
| Project contract | Validated `girder.toml` for source scope, graph path, test runners, and agent output |
| Source projection | Authored plan edits and graph rename commit validated source plus graph through recoverable journaled transactions |
| Candidate validation | Disposable project copy, optional bubblewrap isolation, Cargo build/tests, configured checks, cancellation/timeouts, bounded diagnostics, snapshot-bound commit gate |
| External authoring | `girder context` emits graph context plus a real Plan Format v2 schema for any external chat model; `plan run --authored` re-enforces rollback/impacted-test guarantees on the result |

The measured table-stakes comparison, current correctness evidence, and
prioritized open risks are maintained in
[`docs/core-gap-analysis.md`](docs/core-gap-analysis.md).

## License

Girder's source in this repository is licensed under the [Apache License,
Version 2.0](./LICENSE). That permits commercial use, modification,
redistribution, forks, and hosted use, subject to the license terms; `LICENSE`
is the authoritative text.

The previously published v0.3.2 npm package and release binaries remain under
the BUSL license they shipped with; published releases are immutable. The
current v0.4.0 npm package and release binaries ship with Apache-2.0.

There is no current paid tier, and every shipped tool is ungated: no account
or license key is required. Anyone who already holds a signed license key from
an earlier release keeps it working for whatever it unlocked then; the key
system itself (`GIRDER_LICENSE_KEY` or a per-OS key file, verified offline, no
phone-home) remains only for that historical compatibility.

Earlier releases remain under the terms they shipped with; this is not a
retroactive relicensing. See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for
contribution terms.
