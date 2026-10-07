# Stage 4 client documentation verification (2026-10-06)

Each format Girder relies on was checked against the client's own published
documentation on **2026-10-06**. Quotes are verbatim extractions (Claude Code and the three
facts marked "direct" were fetched by the lead; the remaining Cursor and Codex rows were
fetched by the second agent, whose raw output is kept unedited in
[provenance/](provenance/second-agent-cursor-codex-lookup-raw.txt)). A fact the page did
not state is recorded as NOT STATED, and no adapter step depends on it.

## Claude Code (docs: code.claude.com)

| Fact | Source | Verbatim evidence |
| --- | --- | --- |
| MCP config locations | <https://code.claude.com/docs/en/mcp> | Table: `Local ... ~/.claude.json`, `Project ... .mcp.json in project root`, `User ... ~/.claude.json` |
| MCP config top-level key | same | `"mcpServers": { "shared-server": { ... } }` in `.mcp.json` ("follows a standardized format") |
| stdio launch with npx | same | `claude mcp add --transport stdio db -- npx -y @bytebase/dbhub` |
| Persistent instructions | <https://code.claude.com/docs/en/memory> | `User instructions ~/.claude/CLAUDE.md`; `Project instructions ./CLAUDE.md or ./.claude/CLAUDE.md`; "All discovered files are concatenated into context rather than overriding each other." |
| Hook configuration | <https://code.claude.com/docs/en/hooks> | `"hooks": { "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "...", "args": [] } ] } ] }` |
| Hook stdin | same | JSON with `session_id`, `cwd`, `permission_mode`, `hook_event_name`, `tool_name`, `tool_input`, `tool_use_id` |
| Hook context output | same | "When several hooks return additionalContext for the same event, Claude receives all of the values."; for `PreToolUse` the reminder appears "next to the tool result" |

### Hook output schemas, fetched directly (added after review)

| Client | Source | Verbatim evidence | Matches `girder hook`? |
| --- | --- | --- | --- |
| Claude Code | <https://code.claude.com/docs/en/hooks> | "Return additionalContext inside hookSpecificOutput alongside the event name:" followed by `{"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": "..."}}`; for `PreToolUse` the reminder appears "next to the tool result" | Yes: `girder hook` emits `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": "..."}}` (`crates/aether-app/tests/hook.rs::expected_hook_output`) |
| Codex | <https://developers.openai.com/codex/hooks> | "To add model-visible context without blocking, return hookSpecificOutput.additionalContext:" followed by `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": "..."}}` | Yes, identical |

The smoke check now asserts this exact shape (an object with only `hookSpecificOutput`, which holds
only `hookEventName: "PreToolUse"` and a non-empty `additionalContext` string), not a substring.

## OpenAI Codex (docs: developers.openai.com/codex, now served from learn.chatgpt.com)

| Fact | Source | Verbatim evidence |
| --- | --- | --- |
| MCP config (direct) | <https://developers.openai.com/codex/mcp> | "By default this is ~/.codex/config.toml, but you can also scope MCP servers to a project with .codex/config.toml (trusted projects only)."; "Configure each MCP server with a [mcp_servers.<server-name>] table"; "command (required)", "args (optional)", "env (optional)" |
| Global instructions (direct) | <https://developers.openai.com/codex/guides/agents-md> | "In your Codex home directory (defaults to ~/.codex, unless you set CODEX_HOME), Codex reads AGENTS.override.md if it exists. Otherwise, Codex reads AGENTS.md. Codex uses only the first non-empty file at this level." |
| Merge order and size limit (direct) | same | "Codex concatenates files from the root down, joining them with blank lines."; "stops adding files once the combined size reaches the limit defined by project_doc_max_bytes (32 KiB by default)." |
| Hooks | <https://developers.openai.com/codex/hooks> (second agent) | `hooks.json` and inline `[hooks]` in `config.toml`; paths `~/.codex/hooks.json`, `<repo>/.codex/hooks.json`; events include `PreToolUse`, `PostToolUse`; "Every command hook receives one JSON object on stdin."; PreToolUse can "deny" or return `additionalContext` without blocking. |

Consequence for the adapter: an existing `AGENTS.override.md` makes Codex ignore `AGENTS.md`,
so setup must not install an instruction into `AGENTS.md` there; it reports that and skips.

## Cursor (docs: cursor.com/docs)

| Fact | Source | Verbatim evidence |
| --- | --- | --- |
| MCP config (second agent) | <https://cursor.com/docs/mcp> | "Create `.cursor/mcp.json` in your project for project-specific tools." / "Create `~/.cursor/mcp.json` in your home directory for tools available everywhere."; top-level `mcpServers`, entry `command`, `args`, `env` |
| Project rules (direct) | <https://cursor.com/docs/rules> | "Project rules live in .cursor/rules as .mdc files and are version-controlled."; "A plain .md file in .cursor/rules is ignored by the rules system because it has no frontmatter to specify description, globs, and alwaysApply."; `alwaysApply: true`: "Always included. Globs and description are ignored." |
| User-level rules | same page | "User Rules are global preferences defined in **Customize → Rules**". **NOT STATED:** any file path for user rules, so setup writes none. |
| AGENTS.md | same page (second agent) | "Cursor supports AGENTS.md in the project root and subdirectories." |
| Hooks | <https://cursor.com/docs/hooks> (second agent) | project `<project>/.cursor/hooks.json` or `~/.cursor/hooks.json`; `{ "version": 1, "hooks": { "afterFileEdit": [{ "command": "./hooks/format.sh" }] } }`; `beforeReadFile` stdout is allow/deny only; `postToolUse` can return `additional_context`. |

| `preToolUse` output (direct, free-form extraction of the page) | <https://cursor.com/docs/hooks> | stdout fields: `"permission": "allow" \| "deny"`, `"user_message"` ("message shown in client when denied"), `"agent_message"` ("message sent to agent when denied"), `"updated_input"`. The page lists **no** `additional_context` for `preToolUse`. |

**Cursor hook decision (frozen before implementation):** Cursor documents hooks, but no documented
pre-read hook can add model-visible context without blocking: `preToolUse` and `beforeReadFile` can
only allow, deny, or rewrite input, and only `postToolUse` / `sessionStart` return
`additional_context` (after the fact). Girder's hook contract is an advisory that never denies a
read, so the Cursor hook adapter is **documented and deliberately not built** (a hook that can only
allow/deny cannot deliver the advisory). Cursor receives MCP plus the project instruction rule. The
repo's earlier wording that Cursor hooks are undocumented is stale and is corrected in the README and
`docs/setup.md`. Two retries of the second agent on this lookup returned nothing; this row was
fetched directly.
