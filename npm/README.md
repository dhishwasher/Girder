# girder-mcp

Girder saves AI coding-agent context by giving the agent exactly the code it
needs instead of whole files. It builds a local semantic graph of your
repository so Claude Code, Codex, Cursor, and other agents can retrieve exact
functions, definitions, callers, callees, and task context without filling
their context window with irrelevant code — read one function instead of a
whole file, find a declaration without grep.

As an additional capability on the same graph, Girder also answers a
conservative (over-selecting, never knowingly under-selecting) list of the
tests a change might reach.

Requires no Rust toolchain: `postinstall` downloads a prebuilt binary.

## Setup

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
Shell commands are intentionally not parsed. Cursor is MCP-only: its documented
post-edit hook input and output semantics do not establish the standalone
additional-context protocol used by this launcher, so setup does not register a
hook there. A generic MCP client has no universal config path and is reported as
not detected. Setup never writes outside your home directory. Existing foreign
`girder` entries remain untouched, including with `--force`; `girder setup
--uninstall` removes only setup-owned changes.

The packaged launcher forwards each event to the pinned native `girder hook`
executable and fails open on errors. Only standalone `PreToolUse` JSON is
forwarded on stdout; post-edit native diagnostics pass through stderr, and MCP
remains JSON-RPC on its own stdout. The read advisory only gives guidance when
`project.aether` already exists. The post-edit advisory uses that saved graph
snapshot, does not scan or rebuild the project, and describes the last analyzed
version of changed files. See the [setup and client path guide](https://github.com/dhishwasher/Girder/blob/main/docs/setup.md).

For clients setup cannot detect, add this entry to their documented MCP config
manually:

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

The path argument is the project to serve. It is fixed when the server starts,
so no tool call can reach another directory.

### Opt-in watch mode

Girder 0.2.6 adds an opt-in MCP watcher:

```bash
npx -y girder-mcp . --watch
```

The server keeps a validated graph generation in memory, coalesces source
changes, reparses changed files, and runs full project-wide resolution before
publishing the next generation. The normal invocation without `--watch`
retains its existing behavior. The recorded 45-mutation campaign matched fresh
cold analysis while reusing 98.70% of file extractions. The claim is limited to
preserving cold-analysis resolution while reusing parsing; see the
[watcher result and limitations](https://github.com/dhishwasher/Girder/blob/main/docs/mcp-watching.md).

## Tools

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

None of them run a model, and none write to your repository.

## Measured cost

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
runner-up is close. See [`docs/orient-tool.md`](https://github.com/dhishwasher/Girder/blob/main/docs/orient-tool.md) and
the committed [policy](https://github.com/dhishwasher/Girder/blob/main/docs/orient-tool-policy.json) /
[original observation](https://github.com/dhishwasher/Girder/blob/main/docs/orient-tool-observation.json) /
[post-fix observation](https://github.com/dhishwasher/Girder/blob/main/docs/orient-tool-observation-post-fix.json).

Two additional comparisons, both against precommitted policies, both measured in **bytes
of output rather than tokens** (no tokenizer was run):

- `get_source` vs a naive whole-file-read baseline: **97.85% fewer bytes** across ten
  functions sampled by source-size decile, and cheaper on all ten.
  ([method and honest limits](https://github.com/dhishwasher/Girder/blob/main/docs/context-vs-read-cost.md))
- `find_definition` vs a plain-grep baseline: **97.98% fewer bytes** across ten identifiers.
  ([method](https://github.com/dhishwasher/Girder/blob/main/docs/names-cost.md))

Both are single-repository measurements. The direction should hold anywhere,
since it is driven by file size and by grep returning every mention rather than
only declarations, but the exact percentages are not portable.

These comparisons do not measure a competent agent choosing grep searches and
bounded file reads adaptively. No agentic-grep cost claim is established here.

The subsequent [agentic-grep campaign](https://github.com/dhishwasher/Girder/blob/2690fd9024a6a219827ea7c5f84f0de27df66c30/docs/agentic-grep.md) with a local
1.5B model stopped incomplete and produced no pairs with two correct answers.
It establishes no comparative cost advantage or frontier-model behavior.

`impacted_tests` is **advisory**: it over-selects unrelated tests, and some
dynamic-dispatch shapes still aren't resolved — when Girder can't prove a
call site, the frozen policy requires including the related tests rather
than guessing they're safe to skip, so the measured consequence is
over-inclusion, not a silent miss (recall `1.000`, precision `0.667` on the
representative case; see
[`docs/core-representative-mutations.md`](https://github.com/dhishwasher/Girder/blob/main/docs/core-representative-mutations.md)).
A full test run is still the authority before you call a change safe.

Selecting extra tests costs CPU time; missing a relevant test can conceal a
regression.

## Languages

Rust and Python have completed, hand-audited measurements against real
open-source repositories and are the mature path. TypeScript is measured and
gated but explicitly **in progress** — it still has a separate, disclosed
node-identity collision mechanism (duplicate `it()`/`describe()` description
strings can silently overwrite each other's graph node). Go's Stage 3 audit
is **not complete**. See
[TypeScript](https://github.com/dhishwasher/Girder/blob/main/docs/typescript-support.md)
and
[Go](https://github.com/dhishwasher/Girder/blob/main/docs/go-support.md)
for the exact, measured limits of each.

## Environment

| Variable | Effect |
|---|---|
| `GIRDER_MCP_TIMEOUT_SECONDS` | Per-tool-call budget (default 120). Raise for very large repositories. |
| `GIRDER_BASE_URL` | Download host for the postinstall binary, for an internal mirror or air-gapped network. |
| `GIRDER_SKIP_DOWNLOAD` | Set to `1` to skip the postinstall download and use a `girder` already on PATH. |
| `GIRDER_SKIP_CHECKSUM` | Set to `1` to install without verifying the download. Only for a mirror that does not carry the `.sha256` files. |

A `girder` found on PATH takes precedence over the downloaded copy, so a
build from source or a newer release is never shadowed by an older vendored
binary.

## Validating a release

On a machine that already has a `girder` on PATH — for example a developer's
own machine, with a build installed via `cargo install`, or `install.sh` —
`npx -y girder-mcp` does **not** test the published package. It downloads
and checksum-verifies the correct binary, then runs the one already on PATH
instead, because that ordering is deliberate (see above). The version an MCP
client sees in `serverInfo.version` can then be the PATH binary's, not the
package's.

To actually exercise the vendored download, force it:

```bash
GIRDER_FORCE_VENDORED=1 npx -y girder-mcp .
```

This skips the PATH search entirely. If the postinstall download did not
land a binary, it fails loudly naming the path it expected, rather than
silently falling back to PATH the way a normal run does.

## License

Girder is **source-available** under the
[Business Source License 1.1](https://github.com/dhishwasher/Girder/blob/main/LICENSE).
The source is public and free to read, use, modify, and run, including inside
a company, subject to its license terms. Every tool above is free,
permanently — no expiry, no account, no license key, and no current paid
tier. You may not circumvent license-key functionality or remove or obscure
protected functionality if a future release adds one, and you may not offer
Girder itself to third parties as a competing hosted or managed service
whose primary value is Girder's functionality.

On September 4, 2030, the license converts to the Apache License, Version 2.0.
