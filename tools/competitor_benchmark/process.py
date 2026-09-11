"""Serial subprocess supervision with byte, time, and memory bounds."""

from __future__ import annotations

import hashlib
import os
import selectors
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Sequence

from tools.harness_support import terminate_process_tree

from .protocol import Status
from .resources import read_meminfo, resource_block_reason


@dataclass(frozen=True)
class SupervisedResult:
    command: tuple[str, ...]
    returncode: int | None
    status: Status
    detail: str
    stdout_bytes: int
    stderr_bytes: int
    stdout_sha256: str
    stderr_sha256: str
    wall_seconds: float
    peak_rss_bytes: int | None


def run_supervised(
    command: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str],
    stdout_path: Path,
    stderr_path: Path,
    timeout_seconds: float,
    max_output_bytes: int,
    minimum_available_bytes: int,
    emergency_available_bytes: int,
    maximum_tree_rss_bytes: int,
    sample_interval_seconds: float = 0.1,
) -> SupervisedResult:
    """Run one process group and always preserve the delivered raw streams."""

    argv = tuple(str(part) for part in command)
    if not argv or not argv[0]:
        raise ValueError("command must not be empty")
    if min(timeout_seconds, max_output_bytes, minimum_available_bytes,
           emergency_available_bytes, maximum_tree_rss_bytes, sample_interval_seconds) <= 0:
        raise ValueError("all supervision limits must be positive")
    preflight = resource_block_reason(read_meminfo(), minimum_available_bytes)
    stdout_path.parent.mkdir(parents=True, exist_ok=True)
    stderr_path.parent.mkdir(parents=True, exist_ok=True)
    if preflight is not None:
        stdout_path.write_bytes(b"")
        stderr_path.write_bytes(b"")
        return _result(argv, None, Status.RESOURCE_BLOCKED, preflight, b"", b"", 0.0, None)

    started = time.monotonic()
    try:
        process = subprocess.Popen(
            argv, cwd=cwd, env=dict(env), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            start_new_session=(os.name == "posix"),
        )
    except OSError as error:
        message = str(error).encode("utf-8", errors="replace")
        stdout_path.write_bytes(b"")
        stderr_path.write_bytes(message)
        return _result(argv, None, Status.ERROR, f"launch failed: {error}", b"", message,
                       time.monotonic() - started, None)

    assert process.stdout is not None and process.stderr is not None
    selector = selectors.DefaultSelector()
    buffers = {"stdout": bytearray(), "stderr": bytearray()}
    for stream, name in ((process.stdout, "stdout"), (process.stderr, "stderr")):
        os.set_blocking(stream.fileno(), False)
        selector.register(stream, selectors.EVENT_READ, name)
    deadline = started + timeout_seconds
    next_sample = started
    peak_rss: int | None = 0 if os.name == "posix" else None
    forced_status: Status | None = None
    detail = ""
    try:
        while selector.get_map():
            now = time.monotonic()
            if forced_status is None and now >= deadline:
                forced_status, detail = Status.TIMEOUT, f"exceeded {timeout_seconds:g}s timeout"
                terminate_process_tree(process)
            if forced_status is None and now >= next_sample:
                available = read_meminfo().available_bytes
                rss = process_group_rss_bytes(process.pid)
                if peak_rss is not None:
                    peak_rss = max(peak_rss, rss)
                if available < emergency_available_bytes:
                    forced_status = Status.RESOURCE_BLOCKED
                    detail = f"MemAvailable={available} below emergency floor {emergency_available_bytes}"
                    terminate_process_tree(process)
                elif rss > maximum_tree_rss_bytes:
                    forced_status = Status.RESOURCE_BLOCKED
                    detail = f"process-tree RSS={rss} exceeded cap {maximum_tree_rss_bytes}"
                    terminate_process_tree(process)
                next_sample = now + sample_interval_seconds
            for key, _ in selector.select(timeout=min(sample_interval_seconds, 0.05)):
                try:
                    chunk = os.read(key.fileobj.fileno(), 64 * 1024)
                except BlockingIOError:
                    continue
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                remaining = max_output_bytes - sum(len(value) for value in buffers.values())
                buffers[key.data].extend(chunk[:max(0, remaining)])
                if len(chunk) > remaining and forced_status is None:
                    forced_status = Status.ERROR
                    detail = f"combined output exceeded {max_output_bytes} bytes"
                    terminate_process_tree(process)
            if process.poll() is not None and not selector.get_map():
                break
        if process.poll() is None:
            process.wait(timeout=1)
    finally:
        selector.close()
        process.stdout.close()
        process.stderr.close()
        if process.poll() is None:
            terminate_process_tree(process)

    stdout = bytes(buffers["stdout"])
    stderr = bytes(buffers["stderr"])
    stdout_path.write_bytes(stdout)
    stderr_path.write_bytes(stderr)
    if forced_status is not None:
        status = forced_status
    elif process.returncode == 0:
        status = Status.PASS
    else:
        status = Status.ERROR
        detail = f"process exited {process.returncode}"
    return _result(argv, process.returncode, status, detail, stdout, stderr,
                   time.monotonic() - started, peak_rss)


def process_group_rss_bytes(process_group: int, proc_root: Path = Path("/proc")) -> int:
    """Sum resident pages for members of a POSIX process group."""

    if os.name != "posix":
        return 0
    page_size = os.sysconf("SC_PAGE_SIZE")
    total_pages = 0
    for entry in proc_root.iterdir():
        if not entry.name.isdigit():
            continue
        try:
            fields = (entry / "stat").read_text().split()
            if int(fields[4]) == process_group:
                total_pages += int(fields[23])
        except (OSError, IndexError, ValueError):
            continue
    return total_pages * page_size


def _result(
    command: tuple[str, ...], returncode: int | None, status: Status, detail: str,
    stdout: bytes, stderr: bytes, wall_seconds: float, peak_rss_bytes: int | None,
) -> SupervisedResult:
    return SupervisedResult(
        command, returncode, status, detail, len(stdout), len(stderr),
        hashlib.sha256(stdout).hexdigest(), hashlib.sha256(stderr).hexdigest(),
        wall_seconds, peak_rss_bytes,
    )
