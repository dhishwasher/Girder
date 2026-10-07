# Stage 4 observation: client-agnostic packaging and orient-first guidance

Measured 2026-10-07 on candidate `0fc977417f07c0d867d05b03140961b76821a185` (tracked tree clean), debug
`girder` binary sha256 `37739238f4477e534445cecbc9a1078b31f9849696b3a6cc77061fc788b0552d`. Documentation facts were frozen first, in
[doc-verification.md](doc-verification.md), before implementation; hook output schemas were added after
review and fetched directly. This replaces the first candidate's observation, kept under
[superseded-aa153de/](superseded-aa153de/) with its smoke reports and gates (reason below).

## Why the first candidate was superseded

Review of candidate `aa153de` found two blocking defects, both fixed here:
1. **A data-loss bug.** When setup rewrote an owned instruction block, the ownership record kept the
   first install's `original_text`, so uninstall could restore stale bytes over edits made since
   (install on "A", user rewrites the file to "B", setup re-appends, uninstall restored "A"). The record
   is now recomputed from the file's current content minus the block. Two tests reproduce it; reverting
   the fix makes both fail (`"A\n"` instead of `"B\n"`, and `"A\n"` instead of `"A\nC\n"`). The MCP record
   has the same pre-existing pattern; it predates this stage and is not changed here.
2. **The MCP entry launched `npx -y girder-mcp .` at every client start**, contradicting the stage text
   "Runtime uses installed local Girder; setup acquisition is separate", and the first smoke had to launch
   a different command to avoid the network, so nothing proved the configured entry worked. Setup now writes
   `<path-to-girder> mcp .` for the binary that ran setup; only when setup runs from npx's transient cache
   (path contains `_npx`) does it fall back to the npx launcher form, and the README says so.

## What was built

- One canonical instruction, [`npm/instructions/orient-first.md`](../../../npm/instructions/orient-first.md),
  embedded in the binary: with a Girder graph present, call Girder's `orient` before broad reads or grep;
  disclose low confidence; fall back to reading source rather than guessing.
- `girder setup` installs it through each client's own documented mechanism (formats verified on
  2026-10-06, quotes in doc-verification.md), with ownership records, exact diffs, atomic writes, and exact
  uninstall:

  | Client | Mechanism | Behavior |
  | --- | --- | --- |
  | Claude Code | delimited block in `~/.claude/CLAUDE.md` | appended; exact restore on uninstall |
  | Codex | delimited block in `$CODEX_HOME/AGENTS.md` | skipped and reported when a non-empty `AGENTS.override.md` exists (Codex then ignores `AGENTS.md`) |
  | Cursor | project rule `.cursor/rules/girder-orient.mdc` (`alwaysApply: true`) | only with `--project`; owned whole; a non-owned file is left alone |

  New flags: `--no-instructions`, `--project`. MCP entries launch the installed local binary. The Claude
  Code and Codex advisory hooks are unchanged; Cursor registers MCP and the rule, no hook.
- README and `docs/setup.md` cover all three clients, the instruction, which form installs what, the
  installed-binary entry and its npx-cache fallback, and the raw MCP JSON/TOML fallback with per-client
  placement; raw `npx` setup is preserved.

## Evidence

**Unit tests** (`crates/aether-app/src/project/commands/setup.rs`): 14 new tests; the 22 existing setup
tests pass unchanged except two assertions updated for the new entry form. New coverage: dry-run diff;
idempotent re-install; exact byte restore; deletion of a file setup created; a user-edited block survives
uninstall; only the block is removed when the file changed elsewhere; the `AGENTS.override.md` guard;
Cursor `--project` ownership (needs the flag, owned whole, a non-owned rule untouched, nothing written
without `~/.cursor`); `--no-instructions`; canonical text content; the two stale-record regression tests;
and the entry function (installed path, npx-cache fallback). **Mutation-checked:** disabling the override
guard, the edited-block guard, the foreign-rule guard, or the stale-record fix each fails its own test(s).

**Isolated per-client smoke** ([smoke-report.json](smoke-report.json), `tools/stage4_client_smoke.py`): a
fresh temporary HOME and project per client; the candidate `girder setup` is run; then, from the written
files alone:
- the MCP entry parses in the client's documented format (JSON `mcpServers.girder` for Claude Code and
  Cursor, TOML `[mcp_servers.girder]` for Codex) and its command resolves to the candidate binary with
  args `["mcp", "."]`;
- the instruction is in its native location (Cursor rule with the documented frontmatter);
- **the exact command from the written config is launched** and answers `initialize` and `tools/list` with
  all 7 tools;
- **the real client CLIs read the written configs** (no model calls, no auth):
  `claude mcp list` reported `Checking MCP server health… /  / girder: /mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder mcp . - ! Connected · tools fetch failed — Invalid result for tools/list: missing required resultType — servers implementing protocol`; `codex mcp list` reported `Name    Command                                                            Args   Env  Cwd  Status   Auth        / girder  /mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder  mcp .  -    -    enabled  Unsupported / WARNING: p`; no Cursor CLI is
  installed here, so Cursor's reader is "not available";
- uninstall removes the instruction. **Claude Code: PASS, Codex: PASS, Cursor: PASS.**

**Hook cases through the installed commands** (Claude Code and Codex; commands read from each written
hook config, run via `sh -c`):

| Case | Result (both clients) |
| --- | --- |
| normal (graph present) | exact documented shape `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": "..."}}` on 3/6 warm attempts (Claude Code) and 6/6 (Codex) |
| missing graph | exit 0, no output |
| malformed input | exit 0, no output |
| hook failure, binary missing | exit 0, no output (fail-open) |
| hook failure, binary exists but exits 1 after printing junk | exit 0, no output (fail-open) |

The shape was fetched from each vendor's own docs (doc-verification.md) and matches what `girder hook`
emits. The native hook is covered at the unit level by `crates/aether-app/tests/hook.rs` and
`npm/test/hook.test.js`. Cursor registers no hook, by design (below).

**Common gates** on `0fc9774` ([gates-0fc9774/](gates-0fc9774/)): `cargo test --workspace` exit 0, 28 suites,
**802 passed**, 0 failed, 2 ignored; clippy `-D warnings` exit 0; `cargo fmt --check` exit 0;
`node --test npm/test/*.test.js` exit 0 (29 passed, 2 skipped).

## Attempts

- Candidate `aa153de`: first smoke failed one check (a cold hook call is silent within the hook's 20 ms
  deadline), redesigned to warm up; gates passed; then superseded for the two defects above. All of it is
  in [superseded-aa153de/](superseded-aa153de/).
- Candidate `0fc9774` (this one): smoke and all four gates pass.

## Disclosed limits

- **No live model session was driven.** The real CLIs' own config readers confirm each configured server
  is read (and, for Claude Code, launched and connected), but "loads the instruction" rests on the vendors'
  documentation plus file placement, not on watching a model read the file.
- **The published-package path was not exercised:** `npx -y girder-mcp setup` runs the published 0.3.3 setup
  (no instruction until a release containing it; none is permitted here), and the npx-cache fallback entry
  was verified only by a unit test, not launched.
- **Cursor:** the instruction is a project rule written only with `--project` (user rules are UI-only, no
  file path is documented, so none is invented); no Cursor CLI is available to read the config; the hook is
  documented and deliberately not built because `preToolUse` and `beforeReadFile` can only allow, deny, or
  rewrite input, so no documented pre-read hook can add advisory context without blocking.
- **Codex:** only the global `AGENTS.md` is managed; an `AGENTS.override.md` disables it by Codex's rules;
  the 32 KiB combined cap applies and the instruction is under 1.5 KB. **Claude Code:** only the user-level
  `CLAUDE.md` is managed.
- The hook's "normal" case depends on a 20 ms work deadline and answers silently when it expires; the smoke
  requires advice within bounded warm attempts, not on every call.
- Linux only. No telemetry or runtime network call was added; the smoke is offline except that the real
  `claude` CLI may contact its own service on startup, which is outside Girder.
