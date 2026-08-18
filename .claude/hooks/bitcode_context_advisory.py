#!/usr/bin/env python3
"""Advisory PreToolUse hook for Read on this repo's Rust/Python source.

Never blocks: always exits 0. When the Read targets a .rs or .py file, it
prints a JSON object with `systemMessage` (shown to the user) and
`hookSpecificOutput.additionalContext` (injected into the model's context)
pointing at `bitcode context` instead, per CLAUDE.md's "Agent tooling: use
Bit Code's own CLI" section. Any other Read (or malformed stdin) produces no
output and exits 0 -- silently advisory, not silently broken.
"""
import json
import sys

try:
    payload = json.load(sys.stdin)
except (json.JSONDecodeError, ValueError):
    sys.exit(0)

file_path = payload.get("tool_input", {}).get("file_path", "")
if not (file_path.endswith(".rs") or file_path.endswith(".py")):
    sys.exit(0)

message = (
    "Bit Code repo: prefer `bitcode context . --nodes <node::path> --json` "
    "over Read for a single function's source (CLAUDE.md's \"Agent tooling: "
    "use Bit Code's own CLI\" section) -- measured 41% fewer tokens than "
    "reading the file. `bitcode query . \"<question>\"` covers callers/"
    "callees instead of grepping. This is advisory only; the Read will "
    "still proceed."
)
print(json.dumps({
    "systemMessage": message,
    "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "additionalContext": message,
    },
}))
sys.exit(0)
