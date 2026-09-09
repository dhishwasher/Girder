# girder-mcp

A semantic-graph MCP server for AI coding agents. It answers questions about a
repository from a parsed graph of it, so an agent can read one function instead
of a whole file, find a declaration without grep, and run only the tests a
change can reach.

Requires no Rust toolchain: `postinstall` downloads a prebuilt binary.

## Setup

Claude Code:

```bash
claude mcp add girder -- npx -y girder-mcp .
```

Any MCP client config:

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

## Tools

| Tool | What it answers | Tier |
|---|---|---|
| `get_source` | The source of specific functions, without the file around them. | Free |
| `find_definition` | Where an exact identifier is declared. Not a substring search. | Free |
| `search_code` | Which functions match a description, when you don't know the name. | Free |
| `ask_codebase` | Callers, callees, and blast radius, by graph traversal. | Free |
| `impacted_tests` | Only the tests that can reach what changed. | Paid |
| `review_changes` | What changed in the working tree, as semantics rather than text. | Free |
| `orient` | Source, callers, callees, tests, and impact for one node, in one call. | Paid |

Every tool is read-only. None of them run a model, and none write to your
repository.

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

`impacted_tests` is **advisory**: it over-selects unrelated tests and misses
tests reached only through dynamic dispatch. A full test run is still the
authority before you call a change safe.

## Languages

Rust, Python, TypeScript/TSX, and Go. Rust and Python are the most mature;
TypeScript and Go are measured and gated, with their limits written down
([TypeScript](https://github.com/dhishwasher/Girder/blob/main/docs/typescript-support.md),
[Go](https://github.com/dhishwasher/Girder/blob/main/docs/go-support.md)).

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

## Buy a license

Paid access to `orient` and `impacted_tests` costs **$39, one-time and
perpetual, with no subscription**. [Buy a Girder license on
Gumroad](https://maynard42.gumroad.com/l/zwpsjl).

Set the purchased key as the complete value of the `GIRDER_LICENSE_KEY`
environment variable. Alternatively, save it as the only contents of the key
file that Girder reads for your platform:

- Linux and other non-macOS Unix: `$XDG_CONFIG_HOME/girder/license.key`, or
  `$HOME/.config/girder/license.key` when `XDG_CONFIG_HOME` is unset
- macOS: `$HOME/Library/Application Support/girder/license.key`
- Windows: `%APPDATA%\girder\license.key`

Verification is offline, and the key never expires.

## License

Girder is **source-available** under the
[Business Source License 1.1](https://github.com/dhishwasher/Girder/blob/main/LICENSE).
The source is public and free to read, use, modify, and run, including inside
a company, subject to its license terms. You may not circumvent its license-key
functionality or remove or obscure protected functionality, and you may not
offer Girder itself to third parties as a competing hosted or managed service
whose primary value is Girder's functionality.

On September 4, 2030, the license converts to the Apache License, Version 2.0.
