#!/usr/bin/env python3
"""Fail-open launcher for the native Girder advisory hook."""

import json
import subprocess
import sys


def main() -> None:
    binary = sys.argv[1] if len(sys.argv) > 1 else "girder"
    payload = sys.stdin.buffer.read()
    try:
        event = json.loads(payload).get("hook_event_name")
    except (AttributeError, TypeError, ValueError, UnicodeDecodeError):
        event = None
    try:
        result = subprocess.run(
            [binary, "hook"],
            input=payload,
            capture_output=True,
            check=False,
        )
    except (OSError, ValueError):
        return
    if result.returncode != 0:
        return
    try:
        if event == "PreToolUse":
            sys.stdout.buffer.write(result.stdout)
            sys.stdout.buffer.flush()
        else:
            sys.stderr.buffer.write(result.stderr)
            sys.stderr.buffer.flush()
    except OSError:
        pass


if __name__ == "__main__":
    main()
