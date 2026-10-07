#!/usr/bin/env python3
"""Isolated per-client install, MCP, instruction and hook smoke checks (Stage 4).

For each client (Claude Code, Codex, Cursor) this creates a fresh temporary HOME and
project, runs the candidate `girder setup` against it, then checks from the written
files alone: the MCP config parses in the client's documented format, the shared
orient-first instruction is present in the client's native location, the MCP server
answers `initialize` and `tools/list`, and the installed hook command (Claude Code and
Codex) behaves correctly for normal, missing-graph, malformed-input and hook-failure
cases. Offline; nothing outside the temporary directories is touched.

Setup writes an MCP entry that launches the installed local binary (`<path> mcp .`);
the server leg launches exactly the command read from each written config, and the real
`claude` and `codex` CLIs read the configs in the isolated HOME (no model calls). The
published-package path (`npx -y girder-mcp ...`) is not exercised.

  python3 -m tools.stage4_client_smoke --binary BIN --output report.json
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
BODY = (BASE / "npm/instructions/orient-first.md").read_text().strip()
BEGIN = "<!-- girder:orient-first begin"
EXPECTED_TOOLS = {"get_source", "find_definition", "search_code", "ask_codebase",
                  "impacted_tests", "review_changes", "orient"}
EXPECTED_ENTRY = {"command": "npx", "args": ["-y", "girder-mcp", "."]}


def run(cmd, cwd, env, stdin=None, timeout=60):
    return subprocess.run(cmd, cwd=cwd, env=env, input=stdin, capture_output=True,
                          text=True, timeout=timeout)


def mcp_handshake(entry: dict, project: Path, env) -> dict:
    """Speak newline-delimited JSON-RPC to the exact command from the written config."""
    proc = subprocess.Popen([entry["command"], *entry["args"]], cwd=project, env=env,
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True)
    try:
        init = {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2024-11-05", "capabilities": {},
            "clientInfo": {"name": "stage4-smoke", "version": "0"}}}
        lines = [json.dumps(init), json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                 json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})]
        out, err = proc.communicate("\n".join(lines) + "\n", timeout=60)
    finally:
        if proc.poll() is None:
            proc.kill()
    replies = {}
    for line in out.splitlines():
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if "id" in msg:
            replies[msg["id"]] = msg
    tools = {t["name"] for t in replies.get(2, {}).get("result", {}).get("tools", [])}
    return {"initialize_ok": "result" in replies.get(1, {}), "tools": sorted(tools),
            "all_expected_tools": EXPECTED_TOOLS <= tools, "stderr_bytes": len(err)}


def real_client_reader(client: str, project: Path, env) -> dict:
    """Ask the real client CLI to read the written config (no model calls, no auth)."""
    tool = {"claude": ["claude", "mcp", "list"], "codex": ["codex", "mcp", "list"]}.get(client)
    if tool is None or shutil.which(tool[0]) is None:
        return {"status": "not available", "reason": "client CLI not installed here" if tool else "no CLI reader for this client"}
    try:
        p = run(tool, project, env, timeout=120)
    except subprocess.TimeoutExpired:
        return {"status": "timeout"}
    out = (p.stdout + p.stderr).strip()
    ok = p.returncode == 0 and "girder" in out and (client != "claude" or "Connected" in out)
    return {"status": "ok" if ok else "FAILED", "exit": p.returncode, "output": out[:400]}


def hook_command(config: Path, event: str) -> str:
    doc = json.loads(config.read_text())
    return doc["hooks"][event][0]["hooks"][0]["command"]


def documented_shape(stdout: str) -> bool:
    """The context-injection shape both clients document for a PreToolUse hook."""
    try:
        out = json.loads(stdout)
    except ValueError:
        return False
    specific = out.get("hookSpecificOutput") if isinstance(out, dict) else None
    return (isinstance(specific, dict) and specific.get("hookEventName") == "PreToolUse"
            and isinstance(specific.get("additionalContext"), str) and specific["additionalContext"] != ""
            and set(out) == {"hookSpecificOutput"} and set(specific) == {"hookEventName", "additionalContext"})


def hook_cases(command: str, project: Path, empty_project: Path, env, home: Path) -> dict:
    payload = {"hook_event_name": "PreToolUse", "tool_name": "Read",
               "tool_input": {"file_path": str(project / "src/lib.rs")}}
    results = {}

    def case(name, cmd, cwd, stdin):
        p = run(["sh", "-c", cmd], cwd, env, stdin)
        results[name] = {"exit": p.returncode, "stdout": p.stdout.strip()[:300], "stderr_bytes": len(p.stderr)}

    # The native hook has a 20 ms work deadline and answers silently when it expires, so a
    # cold first call can legitimately be silent. Warm it up, then require advice within a
    # bounded number of attempts and record how many were silent.
    attempts, advised = 6, 0
    for _ in range(attempts):
        p = run(["sh", "-c", command], project, env, json.dumps(payload))
        if p.returncode == 0 and documented_shape(p.stdout):
            advised += 1
            results["normal"] = {"exit": 0, "stdout": p.stdout.strip(), "stderr_bytes": len(p.stderr)}
    if "normal" not in results:
        results["normal"] = {"exit": p.returncode, "stdout": p.stdout.strip()[:300], "stderr_bytes": len(p.stderr)}
    results["normal_attempts"] = {"attempts": attempts, "advised": advised, "silent": attempts - advised}
    case("missing_graph", command, empty_project, json.dumps(payload))
    case("malformed_input", command, project, "this is not json {")
    broken = command.rsplit("'", 2)[0] + "'/nonexistent/girder-binary'"
    case("hook_failure_missing_binary", broken, project, json.dumps(payload))
    # The binary exists but fails after printing junk: the launcher must stay fail-open and silent.
    failing = home / "failing-girder.sh"
    failing.write_text("#!/bin/sh\nprintf 'junk output'\nexit 1\n")
    failing.chmod(0o755)
    case("hook_failure_native_exit", command.rsplit("'", 2)[0] + f"'{failing}'", project, json.dumps(payload))
    results["ok"] = (
        results["normal"]["exit"] == 0 and documented_shape(results["normal"]["stdout"])
        and all(results[k]["exit"] == 0 and results[k]["stdout"] == ""
                for k in ("missing_graph", "malformed_input", "hook_failure_missing_binary", "hook_failure_native_exit")))
    return results


def check_client(client: str, binary: Path) -> dict:
    report = {"client": client, "checks": {}}
    with tempfile.TemporaryDirectory(prefix=f"girder-stage4-{client}-") as scratch:
        home = Path(scratch).resolve()
        project, empty = home / "proj", home / "empty"
        (project / "src").mkdir(parents=True)
        empty.mkdir()
        (project / "src/lib.rs").write_text("pub fn answer() -> i32 { 42 }\npub fn other() -> i32 { answer() + 1 }\n")
        (home / {"claude": ".claude", "codex": ".codex", "cursor": ".cursor"}[client]).mkdir()
        env = dict(os.environ, HOME=str(home))
        env.pop("CODEX_HOME", None)
        analyzed = run([str(binary), "analyze", str(project)], project, env)
        report["checks"]["analyze_exit"] = analyzed.returncode
        args = [str(binary), "setup", "--agents", client] + (["--project"] if client == "cursor" else [])
        setup = run(args, project, env)
        report["checks"]["setup_exit"] = setup.returncode
        # A. MCP config in the client's documented format.
        if client == "codex":
            doc = tomllib.loads((home / ".codex/config.toml").read_text())
            entry = doc.get("mcp_servers", {}).get("girder")
        else:
            path = home / (".claude.json" if client == "claude" else ".cursor/mcp.json")
            entry = json.loads(path.read_text()).get("mcpServers", {}).get("girder")
        report["checks"]["mcp_entry"] = entry
        report["checks"]["mcp_entry_ok"] = (
            isinstance(entry, dict) and Path(entry.get("command", "")).resolve() == binary
            and entry.get("args") == ["mcp", "."])
        # B. Shared instruction in the client's native location.
        native = {"claude": home / ".claude/CLAUDE.md", "codex": home / ".codex/AGENTS.md",
                  "cursor": project / ".cursor/rules/girder-orient.mdc"}[client]
        text = native.read_text() if native.exists() else ""
        report["checks"]["instruction_path"] = str(native.relative_to(home))
        report["checks"]["instruction_ok"] = BEGIN in text and BODY in text
        if client == "cursor":
            report["checks"]["cursor_frontmatter_ok"] = text.startswith("---\n") and "alwaysApply: true" in text.split("---")[1]
        # C. The MCP server answers (documented `girder mcp <project>` form).
        report["checks"]["server"] = mcp_handshake(entry, project, env)
        report["checks"]["real_client_reader"] = real_client_reader(client, project, env)
        # D. Installed hook command (Claude Code and Codex); Cursor registers none.
        if client == "cursor":
            report["checks"]["hook_registered"] = (home / ".cursor/hooks.json").exists()
            report["checks"]["hook_note"] = "not built: no documented pre-read hook can add advisory context"
        else:
            config = home / (".claude/settings.json" if client == "claude" else ".codex/hooks.json")
            report["checks"]["hooks"] = hook_cases(hook_command(config, "PreToolUse"), project, empty, env, home)
        # E. Uninstall restores a clean home.
        un = run(args + ["--uninstall"], project, env)
        report["checks"]["uninstall_exit"] = un.returncode
        report["checks"]["instruction_removed"] = (not native.exists()) or BEGIN not in native.read_text()
    c = report["checks"]
    report["passed"] = (
        c["real_client_reader"]["status"] in ("ok", "not available")
        and c["analyze_exit"] == 0 and c["setup_exit"] == 0 and c["mcp_entry_ok"] and c["instruction_ok"]
        and c["server"]["initialize_ok"] and c["server"]["all_expected_tools"]
        and (c["hooks"]["ok"] if client != "cursor" else not c["hook_registered"])
        and c["uninstall_exit"] == 0 and c["instruction_removed"]
        and c.get("cursor_frontmatter_ok", True))
    return report


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", required=True)
    ap.add_argument("--output", required=True)
    a = ap.parse_args()
    binary = Path(a.binary).resolve(strict=True)
    reports = [check_client(c, binary) for c in ("claude", "codex", "cursor")]
    out = {"binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
           "git_head": subprocess.run(["git", "rev-parse", "HEAD"], cwd=BASE, capture_output=True, text=True).stdout.strip(),
           "npx_published_package_path_exercised": False,
           "clients": reports, "all_passed": all(r["passed"] for r in reports)}
    Path(a.output).write_text(json.dumps(out, indent=2) + "\n")
    for r in reports:
        print(r["client"], "PASS" if r["passed"] else "FAIL")
    return 0 if out["all_passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
