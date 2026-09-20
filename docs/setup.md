# Agent setup

Start with a preview, then apply the detected client changes:

```bash
npx -y girder-mcp setup --dry-run
npx -y girder-mcp setup
```

Setup writes only under the home directory and records its exact changes in
`girder-setup-state.json` beside each detected client config. Uninstall restores
only those owned changes. A foreign server already occupying the `girder` name
is preserved even with `--force`; force is limited to setup-owned Girder
artifacts. Cursor receives MCP configuration only because malformed or empty
Cursor PreToolUse output can block a tool, and `agent_message` is only exposed
on DENY.

| Client | MCP config | Hook config | Hook coverage |
| --- | --- | --- | --- |
| Claude Code | `~/.claude.json` or in-home project `.mcp.json` | `~/.claude/settings.json` | `PreToolUse` matcher `Read` |
| Codex | `$CODEX_HOME/config.toml` or `~/.codex/config.toml` | `$CODEX_HOME/hooks.json` or `~/.codex/hooks.json` | `PreToolUse` matcher `Read\|read_file\|mcp__.*__read_file` |
| Cursor | `~/.cursor/mcp.json` | not registered | MCP only |
| Other clients | documented by that client | no universal path | add MCP manually |

Codex also supports inline `[hooks]` in `config.toml`. When that
representation is present, setup leaves it alone rather than creating a
duplicate `hooks.json`; MCP setup still proceeds. Shell commands are not parsed
as structured reads. The native hook accepts only whole-file structured reads,
checks for an existing `project.aether`, and exits silently when the graph is
missing, the input is malformed, or the read is bounded. It never builds or
loads a graph.

Input parsing and metadata checks have a 20 ms deadline; expiry produces no
advice. Process startup and operating-system scheduling are outside that
deadline, so this is not a promise of zero elapsed overhead.

Local diagnostic observation (2026-09-20, Linux, debug binary, ten sequential
warm requests per run; includes startup and scheduling):

| Run | Advice / silent responses | Elapsed min / median / max |
| --- | --- | --- |
| Before the startup fast path | 8 / 2 | 106 / 136 / 504 ms |
| With the startup fast path | 5 / 5 | 87 / 141 / 950 ms |

These observations do not establish a startup improvement or satisfy a literal
zero-overhead requirement. The 20 ms work deadline was not increased. Silent
timeout responses remain intentional; this debug-host result is not a release
performance measurement.

Standalone hook stdout is the client hook protocol: valid responses contain
`additionalContext` JSON. The shell/Python package launchers pass stdin and
stdout to the pinned native `girder hook` binary, suppress errors, and always
exit successfully. MCP startup and tool calls remain JSON-RPC on MCP stdout;
the two protocols must not be mixed.

For a client without setup detection, add the MCP entry to its documented
config:

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

With a direct install, use `"command": "girder", "args": ["mcp", "."]`.
The project path is fixed when MCP starts. Official client references:
[Claude settings](https://code.claude.com/docs/en/settings), [Claude MCP](https://code.claude.com/docs/en/mcp),
[Codex advanced config](https://learn.chatgpt.com/docs/config-file/config-advanced),
[Codex hooks](https://learn.chatgpt.com/docs/hooks), [Cursor MCP](https://docs.cursor.com/context/mcp), and
[Cursor hooks](https://prod.cursor.com/docs/hooks#pretooluse).
