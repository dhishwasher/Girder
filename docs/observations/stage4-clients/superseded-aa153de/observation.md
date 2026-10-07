# Stage 4 observation: client-agnostic packaging and orient-first guidance

Measured 2026-10-06 on candidate `aa153de67afa1397631c287a9ece79b13ab844e1` (tracked tree clean),
debug `girder` binary sha256 `625025fa7c0f58da4400b4d6df96f489c7f0b254f7abb9dbf0058c72219a47ba`. Documentation facts were frozen first, in
[doc-verification.md](doc-verification.md) (commits `4aa3f6d`, `119ae88`), before any implementation.

## What was built

- One canonical instruction, [`npm/instructions/orient-first.md`](../../../npm/instructions/orient-first.md),
  embedded in the binary and shared by every client: with a Girder graph present, call Girder's `orient`
  before broad reads or grep; disclose low confidence; fall back to reading source rather than guessing.
- `girder setup` installs it through each client's own documented mechanism, with the same ownership
  record, exact diff, atomic write, and uninstall discipline as the existing MCP and hook setup:

  | Client | Mechanism (verified 2026-10-06) | Behavior |
  | --- | --- | --- |
  | Claude Code | delimited block in `~/.claude/CLAUDE.md` | appended; exact restore on uninstall |
  | Codex | delimited block in `$CODEX_HOME/AGENTS.md` | skipped and reported when a non-empty `AGENTS.override.md` exists (Codex then ignores `AGENTS.md`) |
  | Cursor | project rule `.cursor/rules/girder-orient.mdc` (`alwaysApply: true`) | written only with `--project`; owned whole; a non-owned file is left alone |

  New flags: `--no-instructions` (skip the step) and `--project` (Cursor project rule). MCP entries and
  the Claude Code / Codex advisory hooks are unchanged; Cursor registers MCP and the rule, no hook.
- README and `docs/setup.md` now cover all three clients, the instruction, which form installs what, and
  the raw MCP JSON/TOML fallback with per-client placement; the raw `npx` setup is preserved.

## Evidence

**Unit tests** (`crates/aether-app/src/project/commands/setup.rs`): 11 new tests, plus the 22 existing
setup tests unchanged and passing. They cover: dry-run diff shows the block and writes nothing;
idempotent re-install; exact byte restore of a pre-existing file; deletion of a file setup created; a
user-edited block survives uninstall; uninstall removes only the block when the file changed elsewhere;
the `AGENTS.override.md` guard (non-empty skips, empty installs); Cursor needs `--project` and is owned
whole; a non-owned Cursor rule is untouched; `--project` without `~/.cursor` writes nothing;
`--no-instructions`; and the canonical text content. **Mutation-checked:** disabling the override guard,
the edited-block guard, or the foreign-rule guard each fails exactly its own test.

**Isolated per-client smoke** ([smoke-report.json](smoke-report.json), `tools/stage4_client_smoke.py`):
a fresh temporary HOME and project per client; the candidate `girder setup` is run; then, from the
written files alone: the MCP entry parses in the client's documented format (JSON `mcpServers.girder` for
Claude Code and Cursor, TOML `[mcp_servers.girder]` for Codex); the instruction is in its native location
(and the Cursor rule has the documented frontmatter); the MCP server answers `initialize` and
`tools/list` with all 7 tools; and uninstall removes the instruction. **Claude Code: PASS, Codex: PASS,
Cursor: PASS.**

**Hook cases through the installed commands** (Claude Code and Codex; commands read from each client's
written hook config and run via `sh -c`):

| Case | Result (both clients) |
| --- | --- |
| normal (graph present) | advisory `additionalContext` on 6/6 warm attempts (Claude Code) and 6/6 (Codex) |
| missing graph | exit 0, no output |
| malformed input | exit 0, no output |
| hook failure (nonexistent binary) | exit 0, no output (fail-open) |

The native hook itself is covered at the unit level by `crates/aether-app/tests/hook.rs` (malformed,
oversized, missing graph, cold, bounded, outside-project, stalled stdin, log) and `npm/test/hook.test.js`.
Cursor: no hook is registered, by design (below).

**Common gates** on `aa153de` ([gates-aa153de/](gates-aa153de/)): `cargo test --workspace` exit 0, 28
suites, **799 passed**, 0 failed, 2 ignored; clippy `-D warnings` exit 0; `cargo fmt --check` exit 0;
`node --test npm/test/*.test.js` exit 0 (29 passed, 2 skipped).

## Attempts

The first smoke run ([smoke-report-attempt-1-cold-hook-failed.json](smoke-report-attempt-1-cold-hook-failed.json))
failed one check: Claude Code's hook "normal" case returned no advice on a single cold call. The native hook
has a documented 20 ms work deadline and answers silently when it expires, so the check was redesigned
to warm up and require advice within a bounded number of attempts, recording how many were silent. This is
a property of the hook, not a defect in the adapter; it is also why the result is not a guarantee of advice
on every read.

## Disclosed limits

- **The `npx -y girder-mcp .` launch path was not exercised.** It would download the published 0.3.3
  package (no network in measured runs, and no release is permitted). The server was launched with the
  documented alternative `girder mcp <project>`; the entry setup writes is verified only as parsed config.
  Likewise `npx -y girder-mcp setup` runs the *published* package's setup, which installs the instruction
  only after a release containing it.
- **No live client session was driven.** "Loads the instruction through its native mechanism" rests on the
  vendors' documentation (quoted in doc-verification.md) plus file-level and protocol-level checks, not on
  watching Claude Code, Codex, or Cursor read the file.
- **Cursor:** the instruction is a project rule written only with `--project` (user rules are UI-only,
  no file path is documented, so none is invented). The Cursor hook is documented and deliberately not
  built: `preToolUse` and `beforeReadFile` can only allow, deny, or rewrite input, so no documented
  pre-read hook can add advisory context without blocking.
- **Codex:** only the global `AGENTS.md` is managed; an `AGENTS.override.md` disables it by Codex's rules;
  the 32 KiB combined cap applies and the instruction is under 1.5 KB. **Claude Code:** only the user-level
  `CLAUDE.md` is managed.
- Linux only; Windows and macOS were not exercised. No telemetry or runtime network call was added.
