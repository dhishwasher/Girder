#!/usr/bin/env python3
"""Advisory-by-default PreToolUse hook for Read on this repo's Rust/Python
source. Optionally enforcing when GIRDER_HOOK_ENFORCE=1 is set.

Default (GIRDER_HOOK_ENFORCE unset or not "1"): never blocks, always exits
0, byte-identical to this hook's original advisory-only behavior. When the
Read targets a .rs or .py file, it prints a JSON object with
`systemMessage` (shown to the user) and `hookSpecificOutput.additionalContext`
(injected into the model's context) pointing at `girder context` instead,
per CLAUDE.md's "Agent tooling: use Girder's own CLI" section. Any other
Read (or malformed stdin) produces no output and exits 0 -- silently
advisory, not silently broken.

Opt-in enforcing (GIRDER_HOOK_ENFORCE=1): a Read of a non-allowlisted .rs
or .py file gets `hookSpecificOutput.permissionDecision: "deny"` instead,
naming the exact girder command to run. Opt-in matters: a hook that denies
Read the moment it ships can wedge the very session that ships it, so
enforcement never activates unless a human explicitly turns it on.
"""
import json
import os
import sys
from pathlib import Path

try:
    payload = json.load(sys.stdin)
except (json.JSONDecodeError, ValueError):
    sys.exit(0)

file_path = payload.get("tool_input", {}).get("file_path", "")
if not (file_path.endswith(".rs") or file_path.endswith(".py")):
    sys.exit(0)

message = (
    "Girder repo: prefer `girder context . --nodes <node::path> --json "
    "--source-only` over Read for a single function's source (CLAUDE.md's "
    "\"Agent tooling: use Girder's own CLI\" section) -- measured 97.85% "
    "fewer bytes than reading the whole file across ten nodes, and cheaper "
    "on all ten (docs/context-vs-read-cost.md). `--source-only` matters: "
    "without it the same command also emits a plan-authoring schema, which "
    "measured more expensive than reading the file on 2 of those 10. "
    "`girder query . \"<question>\"` covers callers/callees instead of "
    "grepping. This is advisory only; the Read will still proceed."
)

enforce = os.environ.get("GIRDER_HOOK_ENFORCE") == "1"

# Exact filename allowlist. `Cargo.toml` and anything under `docs/` are
# already excluded by the .rs/.py suffix check above -- listed here anyway,
# belt-and-suspenders, so a future reader isn't confused by their absence.
# `main.rs` is the one entry doing real work: it IS a .rs file, so without
# it enforcing mode would deny reads of it like any other Rust file.
ALLOWLISTED_NAMES = {"main.rs", "Cargo.toml"}


def is_allowlisted(path: str) -> bool:
    normalized = path.replace(os.sep, "/")
    if Path(path).name in ALLOWLISTED_NAMES:
        return True
    return "/docs/" in normalized or normalized.startswith("docs/")


if enforce and not is_allowlisted(file_path):
    deny_message = (
        f"{message} Enforced: this Read was denied -- run the girder "
        "command above instead."
    )
    print(json.dumps({
        "systemMessage": deny_message,
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": deny_message,
        },
    }))
    sys.exit(0)

print(json.dumps({
    "systemMessage": message,
    "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "additionalContext": message,
    },
}))
sys.exit(0)
