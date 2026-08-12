#!/usr/bin/env python3
"""Shared fail-closed subprocess support for Bit Code measurement harnesses."""

from __future__ import annotations

import hashlib
import os
import selectors
import shlex
import signal
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Sequence


@dataclass(frozen=True)
class BoundedProcessResult:
    args: tuple[str, ...]
    returncode: int
    stdout: str
    stderr: str
    stdout_sha256: str
    stderr_sha256: str
    wall_seconds: float


def run_bounded(
    command: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str] | None = None,
    timeout_seconds: float,
    max_output_bytes: int,
    check: bool = True,
) -> BoundedProcessResult:
    """Run argv without a shell, enforcing one hard combined output budget."""

    if not command or not command[0]:
        raise ValueError("command must name a program")
    if timeout_seconds <= 0:
        raise ValueError("timeout_seconds must be positive")
    if max_output_bytes <= 0:
        raise ValueError("max_output_bytes must be positive")

    argv = tuple(str(part) for part in command)
    rendered = shlex.join(argv)
    started = time.monotonic()
    process = subprocess.Popen(
        argv,
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=(os.name == "posix"),
    )
    assert process.stdout is not None
    assert process.stderr is not None

    selector = selectors.DefaultSelector()
    streams = {
        process.stdout.fileno(): ("stdout", process.stdout),
        process.stderr.fileno(): ("stderr", process.stderr),
    }
    for descriptor, (name, stream) in streams.items():
        os.set_blocking(descriptor, False)
        selector.register(stream, selectors.EVENT_READ, name)

    output = {"stdout": bytearray(), "stderr": bytearray()}
    digests = {"stdout": hashlib.sha256(), "stderr": hashlib.sha256()}
    total_bytes = 0
    deadline = started + timeout_seconds
    failure: str | None = None
    descendants_terminated = False

    try:
        while selector.get_map():
            now = time.monotonic()
            if failure is None and now >= deadline:
                failure = f"command timed out after {timeout_seconds:g}s: {rendered}"
                terminate_process_tree(process)

            events = selector.select(timeout=max(0.0, min(0.05, deadline - now)))
            for key, _ in events:
                name = key.data
                remaining = max_output_bytes - total_bytes
                chunk = _read_available(key.fileobj.fileno(), min(64 * 1024, remaining + 1))
                if chunk is None:
                    selector.unregister(key.fileobj)
                    continue
                if not chunk:
                    continue
                accepted = chunk[: max(0, remaining)]
                output[name].extend(accepted)
                digests[name].update(accepted)
                total_bytes += len(chunk)
                if total_bytes > max_output_bytes and failure is None:
                    failure = (
                        f"command output exceeded {max_output_bytes} bytes: {rendered}"
                    )
                    terminate_process_tree(process)

            if process.poll() is not None and not descendants_terminated:
                terminate_descendants(process.pid)
                descendants_terminated = True

        if process.poll() is None:
            process.wait(timeout=1)
    finally:
        selector.close()
        process.stdout.close()
        process.stderr.close()
        if process.poll() is None:
            terminate_process_tree(process)

    stdout = output["stdout"].decode("utf-8", errors="replace")
    stderr = output["stderr"].decode("utf-8", errors="replace")
    if failure is not None:
        raise RuntimeError(f"{failure}\nstdout:\n{stdout}\nstderr:\n{stderr}")

    returncode = process.returncode
    if check and returncode != 0:
        raise RuntimeError(
            f"command failed ({returncode}): {rendered}\n"
            f"stdout:\n{stdout}\nstderr:\n{stderr}"
        )
    return BoundedProcessResult(
        args=argv,
        returncode=returncode,
        stdout=stdout,
        stderr=stderr,
        stdout_sha256=digests["stdout"].hexdigest(),
        stderr_sha256=digests["stderr"].hexdigest(),
        wall_seconds=time.monotonic() - started,
    )


def _read_available(descriptor: int, limit: int) -> bytes | None:
    try:
        chunk = os.read(descriptor, max(1, limit))
    except BlockingIOError:
        return b""
    return chunk if chunk else None


def terminate_descendants(process_group: int) -> None:
    if os.name != "posix":
        return
    try:
        os.killpg(process_group, signal.SIGKILL)
    except ProcessLookupError:
        pass


def terminate_process_tree(process: subprocess.Popen[bytes]) -> None:
    if os.name == "posix":
        terminate_descendants(process.pid)
    elif process.poll() is None:
        process.kill()
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
