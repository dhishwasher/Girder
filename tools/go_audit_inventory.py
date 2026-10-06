#!/usr/bin/env python3
"""Pin and inventory the Go standard-library audit snapshot (Stage 3, Go).

Plain Python only. This tool must never run `girder` on the audit tree: the audit
methodology requires ground truth to be read from source before Girder is run on
any site. It verifies the pinned archive, applies the frozen extraction filter,
and reports a content-pinned inventory (per-file sha256 and line counts).

  python3 -m tools.go_audit_inventory --verify docs/stage3-go-corpus.json
  python3 -m tools.go_audit_inventory --emit  > inventory.json
  python3 -m tools.go_audit_inventory --extract DEST   (extract the filtered tree)
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ARCHIVE_SHA256 = "4e408abae126d916b6164627193f2c54f0e3ca1312d693b86db45f862ab238b1"
ARCHIVE = ROOT / ".benchmark-cache" / "stage3-go-v1" / f"{ARCHIVE_SHA256}.tar.gz"
PACKAGES = [
    "io", "bufio", "sort", "container/heap", "container/list", "container/ring",
    "context", "errors", "encoding/json", "text/template", "text/template/parse",
    "strings", "bytes", "fmt", "sync",
]
IGNORE_RE = re.compile(r"^//go:build ignore", re.M)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def selected(archive: tarfile.TarFile):
    """Yield (relative path, bytes) for files passing the frozen filter, sorted."""
    found = []
    for member in archive:
        if not member.isfile() or not member.name.endswith(".go"):
            continue
        name = member.name.split("/", 1)[1] if "/" in member.name else member.name
        if not name.startswith("src/"):
            continue
        rel = name[len("src/"):]
        directory = rel.rsplit("/", 1)[0] if "/" in rel else "."
        base = rel.rsplit("/", 1)[-1]
        if directory not in PACKAGES:
            continue
        if base.endswith("_test.go"):
            continue
        data = archive.extractfile(member).read()
        if IGNORE_RE.search(data[:600].decode("utf8", "replace")):
            continue
        found.append((rel, data))
    return sorted(found)


def inventory() -> dict:
    if sha256_file(ARCHIVE) != ARCHIVE_SHA256:
        raise SystemExit("archive sha256 does not match the pin")
    with tarfile.open(ARCHIVE) as archive:
        files = [
            {
                "path": rel,
                "sha256": hashlib.sha256(data).hexdigest(),
                "lines": data.count(b"\n"),
            }
            for rel, data in selected(archive)
        ]
    return {
        "files": files,
        "file_count": len(files),
        "line_count": sum(f["lines"] for f in files),
        "per_package": {
            p: sum(1 for f in files if f["path"].rsplit("/", 1)[0] == p
                   or (p == "." and "/" not in f["path"]))
            for p in PACKAGES
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--emit", action="store_true")
    group.add_argument("--verify", metavar="MANIFEST")
    group.add_argument("--extract", metavar="DEST")
    args = parser.parse_args()
    if args.emit:
        json.dump(inventory(), sys.stdout, indent=2)
        print()
        return 0
    if args.verify:
        pinned = json.loads(Path(args.verify).read_text())["inventory"]
        actual = inventory()
        if pinned != actual:
            print("INVENTORY MISMATCH", file=sys.stderr)
            return 1
        print(f"inventory OK: {actual['file_count']} files, {actual['line_count']} lines")
        return 0
    dest = Path(args.extract)
    if dest.exists():
        raise SystemExit("destination must not exist")
    with tarfile.open(ARCHIVE) as archive:
        for rel, data in selected(archive):
            target = dest / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
    print(f"extracted to {dest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
