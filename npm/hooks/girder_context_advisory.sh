#!/bin/sh
# Forward a structured hook payload to the pinned native Girder executable.
# PreToolUse is the only standalone stdout protocol. PostToolUse reports use
# native stderr, so discard native stdout for every other event. Capture both
# streams and forward them only after a successful native exit; a failed native
# process is fully fail-open and silent.

binary=${1:-girder}
payload=$(cat 2>/dev/null) || payload=
if ! command -v "$binary" >/dev/null 2>&1 && [ ! -x "$binary" ]; then
  exit 0
fi

stderr_file=$(mktemp "${TMPDIR:-/tmp}/girder-hook.XXXXXX") || exit 0
trap 'rm -f "$stderr_file"' EXIT HUP INT TERM
native_stdout=$(printf '%s' "$payload" | "$binary" hook 2>"$stderr_file")
native_status=$?
[ "$native_status" -eq 0 ] || exit 0

if printf '%s' "$payload" | grep -Eq '"hook_event_name"[[:space:]]*:[[:space:]]*"PreToolUse"'; then
  printf '%s' "$native_stdout"
else
  cat "$stderr_file" >&2
fi
exit 0
