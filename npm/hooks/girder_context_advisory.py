#!/usr/bin/env python3
"""Fail-open launcher for the native Girder structured-read hook."""

import subprocess
import sys


def main() -> None:
    binary = sys.argv[1] if len(sys.argv) > 1 else "girder"
    try:
        subprocess.run(
            [binary, "hook"],
            stdin=sys.stdin.buffer,
            stdout=sys.stdout.buffer,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except Exception:
        pass


if __name__ == "__main__":
    main()
