# bitcode-mcp

A semantic-graph MCP server for AI coding agents. It answers questions about a
repository from a parsed graph of it, so an agent can read one function instead
of a whole file, find a declaration without grep, and run only the tests a
change can reach.

Requires no Rust toolchain: `postinstall` downloads a prebuilt binary.

## Setup

Claude Code:

```bash
claude mcp add bitcode -- npx -y bitcode-mcp .
```

Any MCP client config:

```json
{
  "mcpServers": {
    "bitcode": {
      "command": "npx",
      "args": ["-y", "bitcode-mcp", "."]
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
  ([method and honest limits](https://github.com/dhishwasher/Bit-code/blob/main/docs/context-vs-read-cost.md))
- `find_definition` vs `grep`: **97.98% fewer bytes** across ten identifiers.
  ([method](https://github.com/dhishwasher/Bit-code/blob/main/docs/names-cost.md))

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
| `BITCODE_MCP_TIMEOUT_SECONDS` | Per-tool-call budget (default 120). Raise for very large repositories. |
| `BITCODE_BASE_URL` | Download host for the postinstall binary, for an internal mirror or air-gapped network. |
| `BITCODE_SKIP_DOWNLOAD` | Set to `1` to skip the postinstall download and use a `bitcode` already on PATH. |

A `bitcode` found on PATH takes precedence over the downloaded copy, so a
build from source or a newer release is never shadowed by an older vendored
binary.

## License

MIT OR Apache-2.0
