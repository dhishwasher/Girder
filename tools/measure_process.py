#!/usr/bin/env python3
"""Execute one argv and atomically record its POSIX wait4 resource usage."""

from __future__ import annotations

import json
import os
import platform
import sys
import tempfile
from pathlib import Path
from typing import Sequence


def atomic_write(path: Path, value: dict[str, int | float]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        os.fchmod(descriptor, 0o600)
        payload = (json.dumps(value, sort_keys=True) + "\n").encode("utf-8")
        with os.fdopen(descriptor, "wb") as output:
            output.write(payload)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        temporary.unlink(missing_ok=True)


def main(argv: Sequence[str]) -> int:
    if os.name != "posix" or not hasattr(os, "wait4"):
        raise RuntimeError("resource measurement requires POSIX wait4")
    if len(argv) < 3 or argv[1] != "--":
        raise ValueError("usage: measure_process.py <output.json> -- <program> [args...]")
    output = Path(argv[0])
    command = tuple(argv[2:])
    child = os.fork()
    if child == 0:
        try:
            os.execvpe(command[0], command, os.environ)
        except BaseException as error:
            print(f"could not execute {command[0]}: {error}", file=sys.stderr)
            os._exit(127)

    _, status, usage = os.wait4(child, 0)
    peak_rss_kib = int(usage.ru_maxrss)
    if platform.system() == "Darwin":
        peak_rss_kib //= 1024
    atomic_write(
        output,
        {
            "schema_version": 1,
            "peak_rss_kib": peak_rss_kib,
            "user_seconds": usage.ru_utime,
            "system_seconds": usage.ru_stime,
        },
    )
    return os.waitstatus_to_exitcode(status)


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except (OSError, RuntimeError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
