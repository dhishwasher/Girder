#!/usr/bin/env python3
"""Fail-open, stderr-only advisory for whole-file source reads."""

import json
import os
import sys


def advise() -> None:
    try:
        payload = json.load(sys.stdin)
        if not isinstance(payload, dict):
            return

        tool_name = payload.get("tool_name")
        if isinstance(tool_name, str) and tool_name.lower() != "read":
            return
        tool_input = payload.get("tool_input")
        if not isinstance(tool_input, dict):
            return
        if any(key in tool_input for key in ("offset", "limit", "start_line", "end_line")):
            return

        file_path = tool_input.get("file_path", tool_input.get("path"))
        if not isinstance(file_path, str) or not file_path.endswith(
            (".rs", ".py", ".ts", ".tsx", ".go")
        ):
            return

        cwd = payload.get("cwd")
        if not isinstance(cwd, str):
            roots = payload.get("workspace_roots")
            cwd = roots[0] if isinstance(roots, list) and roots else None
        if not isinstance(cwd, str):
            cwd = os.environ.get("CLAUDE_PROJECT_DIR", os.getcwd())

        # A single metadata lookup is the entire graph readiness check. Never
        # invoke Girder here: building or loading a graph would delay the read.
        if not os.path.isfile(os.path.join(cwd, "project.aether")):
            return

        sys.stderr.write(
            "Girder: consider `girder context . --nodes <node::path> --json "
            "--source-only` before this whole-file read.\n"
        )
    except Exception:
        return


advise()
