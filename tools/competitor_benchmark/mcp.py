"""Bounded JSON-lines MCP client used by persistent competitor adapters."""

from __future__ import annotations

import json
import os
import selectors
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence

from tools.harness_support import terminate_process_tree

from .protocol import Status
from .process import process_group_rss_bytes
from .resources import read_meminfo, resource_block_reason


@dataclass(frozen=True)
class McpCall:
    method: str
    params: Mapping[str, Any]
    result: Mapping[str, Any] | None
    status: Status
    detail: str
    elapsed_seconds: float
    stdout_bytes: int
    stderr_bytes: int
    stdout_artifact: str
    stderr_artifact: str
    peak_rss_bytes: int | None


class McpSession:
    def __init__(
        self,
        command: Sequence[str],
        *,
        cwd: Path,
        env: Mapping[str, str],
        artifact_root: Path,
        minimum_available_bytes: int,
        emergency_available_bytes: int,
        maximum_tree_rss_bytes: int,
        max_output_bytes: int,
        sample_interval_seconds: float = 0.1,
    ) -> None:
        reason = resource_block_reason(read_meminfo(), minimum_available_bytes)
        if reason is not None:
            raise RuntimeError(f"RESOURCE_BLOCKED: {reason}")
        self.command = tuple(str(x) for x in command)
        self.artifact_root = artifact_root
        artifact_root.mkdir(parents=True, exist_ok=True)
        self.emergency_available_bytes = emergency_available_bytes
        self.maximum_tree_rss_bytes = maximum_tree_rss_bytes
        self.max_output_bytes = max_output_bytes
        self.sample_interval_seconds = sample_interval_seconds
        self.next_id = 0
        self.call_index = 0
        self.stdout_pending = bytearray()
        self.process = subprocess.Popen(
            self.command, cwd=cwd, env=dict(env), stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            start_new_session=(os.name == "posix"),
        )
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        assert self.process.stderr is not None
        os.set_blocking(self.process.stdout.fileno(), False)
        os.set_blocking(self.process.stderr.fileno(), False)

    def request(self, method: str, params: Mapping[str, Any], timeout_seconds: float) -> McpCall:
        self.next_id += 1
        self.call_index += 1
        request_id = self.next_id
        message = {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
        started = time.monotonic()
        prefix = f"{self.call_index:04d}-{_safe_name(method)}"
        stdout_path = self.artifact_root / f"{prefix}.stdout.jsonl"
        stderr_path = self.artifact_root / f"{prefix}.stderr.txt"
        response_raw = bytearray()
        stderr_raw = bytearray()
        peak_rss: int | None = 0 if os.name == "posix" else None
        status = Status.PASS
        detail = ""
        response: Mapping[str, Any] | None = None
        try:
            encoded = (json.dumps(message, separators=(",", ":"), ensure_ascii=False) + "\n").encode()
            self.process.stdin.write(encoded)
            self.process.stdin.flush()
            deadline = started + timeout_seconds
            selector = selectors.DefaultSelector()
            selector.register(self.process.stdout, selectors.EVENT_READ, "stdout")
            selector.register(self.process.stderr, selectors.EVENT_READ, "stderr")
            while response is None:
                now = time.monotonic()
                if now >= deadline:
                    status, detail = Status.TIMEOUT, f"MCP request exceeded {timeout_seconds:g}s"
                    break
                available = read_meminfo().available_bytes
                rss = process_group_rss_bytes(self.process.pid)
                if peak_rss is not None:
                    peak_rss = max(peak_rss, rss)
                if available < self.emergency_available_bytes:
                    status = Status.RESOURCE_BLOCKED
                    detail = f"MemAvailable={available} below emergency floor {self.emergency_available_bytes}"
                    break
                if rss > self.maximum_tree_rss_bytes:
                    status = Status.RESOURCE_BLOCKED
                    detail = f"process-tree RSS={rss} exceeded cap {self.maximum_tree_rss_bytes}"
                    break
                if self.process.poll() is not None:
                    status, detail = Status.ERROR, f"MCP server exited {self.process.returncode}"
                    break
                for key, _ in selector.select(timeout=min(self.sample_interval_seconds, 0.05)):
                    try:
                        chunk = os.read(key.fileobj.fileno(), 64 * 1024)
                    except BlockingIOError:
                        continue
                    if not chunk:
                        continue
                    if key.data == "stderr":
                        stderr_raw.extend(chunk)
                    else:
                        self.stdout_pending.extend(chunk)
                    if len(self.stdout_pending) + len(stderr_raw) > self.max_output_bytes:
                        status, detail = Status.ERROR, f"MCP output exceeded {self.max_output_bytes} bytes"
                        break
                if status is not Status.PASS:
                    break
                newline = self.stdout_pending.find(b"\n")
                if newline >= 0:
                    response_raw.extend(self.stdout_pending[:newline + 1])
                    del self.stdout_pending[:newline + 1]
                    try:
                        parsed = json.loads(response_raw)
                        if parsed.get("id") != request_id:
                            raise ValueError(f"response id {parsed.get('id')!r} != {request_id}")
                        if "error" in parsed:
                            status = Status.ERROR
                            detail = str(parsed["error"].get("message", parsed["error"]))
                        else:
                            response = parsed.get("result", {})
                    except (json.JSONDecodeError, ValueError) as error:
                        status, detail = Status.ERROR, f"invalid MCP response: {error}"
                        break
            selector.close()
        except (BrokenPipeError, OSError) as error:
            status, detail = Status.ERROR, f"MCP transport failed: {error}"
        if status is not Status.PASS:
            terminate_process_tree(self.process)
        stdout_path.write_bytes(bytes(response_raw or self.stdout_pending))
        stderr_path.write_bytes(bytes(stderr_raw))
        return McpCall(
            method, params, response, status, detail, time.monotonic() - started,
            stdout_path.stat().st_size, stderr_path.stat().st_size,
            str(stdout_path), str(stderr_path), peak_rss,
        )

    def initialize(self, timeout_seconds: float) -> McpCall:
        return self.request("initialize", {"protocolVersion": "2025-06-18"}, timeout_seconds)

    def call_tool(self, name: str, arguments: Mapping[str, Any], timeout_seconds: float) -> McpCall:
        return self.request("tools/call", {"name": name, "arguments": arguments}, timeout_seconds)

    def close(self) -> None:
        if self.process.poll() is None:
            try:
                assert self.process.stdin is not None
                self.process.stdin.close()
                self.process.wait(timeout=2)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                terminate_process_tree(self.process)
        for stream, name in ((self.process.stdout, "stdout"), (self.process.stderr, "stderr")):
            if stream is not None:
                try:
                    remainder = stream.read() or b""
                except BlockingIOError:
                    remainder = b""
                (self.artifact_root / f"{self.call_index + 1:04d}-cleanup.{name}.txt").write_bytes(remainder)
                stream.close()

    def __enter__(self) -> "McpSession":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


def tool_text(call: McpCall) -> str:
    if call.status is not Status.PASS or call.result is None:
        raise ValueError(call.detail or call.status.value)
    if call.result.get("isError"):
        content = call.result.get("content", [])
        message = content[0].get("text", "tool error") if content else "tool error"
        raise ValueError(message)
    content = call.result.get("content", [])
    if not content or not isinstance(content[0].get("text"), str):
        raise ValueError("MCP tool result lacks text content")
    return content[0]["text"]


def _safe_name(value: str) -> str:
    return "".join(char if char.isalnum() else "-" for char in value).strip("-") or "request"
