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

| Tool | What it answers |
|---|---|
| `get_source` | The source of specific functions, without the file around them. |
| `find_definition` | Where an exact identifier is declared. Not a substring search. |
| `search_code` | Which functions match a description, when you don't know the name. |
| `ask_codebase` | Callers, callees, and blast radius, by graph traversal. |
| `impacted_tests` | Only the tests that can reach what changed. |
| `review_changes` | What changed in the working tree, as semantics rather than text. |

Every tool is read-only. None of them run a model, and none write to your
repository.

## Measured cost

Two comparisons, both against precommitted policies, both measured in **bytes
of output rather than tokens** (no tokenizer was run):

- `get_source` vs reading the whole file: **97.85% fewer bytes** across ten
  functions sampled by source-size decile, and cheaper on all ten.
  ([method and honest limits](https://github.com/dhishwasher/Girder/blob/main/docs/context-vs-read-cost.md))
- `find_definition` vs `grep`: **97.98% fewer bytes** across ten identifiers.
  ([method](https://github.com/dhishwasher/Girder/blob/main/docs/names-cost.md))

Both are single-repository measurements. The direction should hold anywhere,
since it is driven by file size and by grep returning every mention rather than
only declarations, but the exact percentages are not portable.

`impacted_tests` is **advisory**: it over-selects unrelated tests and misses
tests reached only through dynamic dispatch. A full test run is still the
authority before you call a change safe.

## Languages

Rust and Python.

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
