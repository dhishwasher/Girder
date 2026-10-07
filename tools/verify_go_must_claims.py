#!/usr/bin/env python3
"""Independently verify every Go Must claim in a graph against the source (Stage 3, Go).

Reads an existing `girder inspect --json` output (no Girder run) and a fresh
extraction of the pinned audit tree. For every claim with reason
`proven-go-same-package-call` it checks, from source text alone: the site starts
with the target's name and `(`; the call is not the direct operand of `go` or
`defer`; caller and target share a directory and package clause; neither file has
a build constraint or imports "C"; the target is the only top-level declaration
of that name in its package; and the caller is a function node.

  python3 -m tools.verify_go_must_claims --inspect after-audit-3/inspect.json.gz --output verification.json
"""
from __future__ import annotations

import argparse
import gzip
import json
import re
import tempfile
from collections import defaultdict
from pathlib import Path

from tools.dispatch_audit_scorer_typescript import extract_call_claims, resolve_node_id
from tools.go_audit_inventory import ARCHIVE, selected
from tools.go_audit_sites import blank
import tarfile

REASON = "proven-go-same-package-call"
GOOS = {"aix", "android", "darwin", "dragonfly", "freebsd", "hurd", "illumos", "ios", "js", "linux",
        "nacl", "netbsd", "openbsd", "plan9", "solaris", "wasip1", "windows", "zos"}
GOARCH = {"386", "amd64", "amd64p32", "arm", "armbe", "arm64", "arm64be", "loong64", "mips", "mipsle",
          "mips64", "mips64le", "mips64p32", "mips64p32le", "ppc", "ppc64", "ppc64le", "riscv", "riscv64",
          "s390", "s390x", "sparc", "sparc64", "wasm"}


def suffix_constrained(file: str) -> bool:
    stem = file.rsplit("/", 1)[-1].removesuffix(".go").removesuffix("_test")
    parts = stem.split("_")
    if len(parts) < 2:
        return False
    last = parts[-1]
    return last in GOOS or last in GOARCH or (len(parts) >= 3 and parts[-2] in GOOS and last in GOARCH)


def constrained(file: str, text: str) -> bool:
    return suffix_constrained(file) or bool(re.search(r"(?m)^\s*(//go:build|// \+build)", text))


def imports_c(text: str) -> bool:
    return bool(re.search(r'(?m)^\s*import\s+"C"', text) or re.search(r'(?s)import\s*\((?:[^)]*?)\n\s*"C"\s*\n', text))


def declarations(text: str, name: str) -> int:
    """Top-level declarations of `name` (func, type, var, const, including grouped blocks)."""
    clean = blank(text.encode()).decode("latin-1")
    n = len(re.findall(rf"(?m)^func\s+{re.escape(name)}\s*[\[(]", clean))
    n += len(re.findall(rf"(?m)^(?:type|var|const)\s+{re.escape(name)}\b", clean))
    for block in re.finditer(r"(?ms)^(?:type|var|const)\s*\((.*?)^\)", clean):
        n += len(re.findall(rf"(?m)^\s+{re.escape(name)}\b", block.group(1)))
    return n


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--inspect", required=True)
    ap.add_argument("--output", required=True)
    a = ap.parse_args()
    raw = gzip.open(a.inspect).read() if a.inspect.endswith(".gz") else Path(a.inspect).read_bytes()
    doc = json.loads(raw)
    with tempfile.TemporaryDirectory(prefix="girder-go-verify-") as scratch:
        root = Path(scratch) / "tree"
        with tarfile.open(ARCHIVE) as archive:
            for rel, data in selected(archive):
                (root / rel).parent.mkdir(parents=True, exist_ok=True)
                (root / rel).write_bytes(data)
        files = {p.relative_to(root).as_posix(): p.read_text("utf-8", "replace") for p in root.rglob("*.go")}

        def rel(path: str) -> str:
            parts = Path(path).parts
            for i in range(len(parts)):
                candidate = "/".join(parts[i:])
                if candidate in files:
                    return candidate
            raise KeyError(path)

        by_dir = defaultdict(list)
        for f in files:
            by_dir[f.rsplit("/", 1)[0] if "/" in f else "."].append(f)
        clause = {f: (re.search(r"(?m)^package\s+(\w+)", t) or [None, None])[1] for f, t in files.items()}
        nodes = {n["id"]: n for n in doc["nodes"]}
        tmp = Path(scratch) / "inspect.json"
        tmp.write_bytes(raw)
        total, violations = 0, []
        for claim in extract_call_claims(tmp):
            if claim["reason"] != REASON or claim["class"] != "must":
                continue
            total += 1
            problems = []
            try:
                caller_file = rel(claim["file"])
                target = nodes[resolve_node_id(claim["targets"][0])]
                target_file = rel(target["file"])
            except Exception as error:  # a claim that cannot be resolved is itself a finding
                violations.append({"site": claim["file"], "problems": [f"unresolvable: {error}"]})
                continue
            src = files[caller_file]
            site = src.encode()[claim["start_byte"]:claim["end_byte"]].decode("utf-8", "replace")
            name = target["path"].rsplit("::", 1)[-1]
            if not re.match(rf"{re.escape(name)}\s*\(", site):
                problems.append(f"site text does not start with {name}(")
            before = src.encode()[:claim["start_byte"]].decode("utf-8", "replace")
            if re.search(r"\b(go|defer)\s*$", before):
                problems.append("direct operand of go/defer")
            d1 = caller_file.rsplit("/", 1)[0] if "/" in caller_file else "."
            d2 = target_file.rsplit("/", 1)[0] if "/" in target_file else "."
            if d1 != d2 or clause[caller_file] != clause[target_file]:
                problems.append("different directory or package clause")
            for f in {caller_file, target_file}:
                if constrained(f, files[f]):
                    problems.append(f"build constraint in {f}")
                if imports_c(files[f]):
                    problems.append(f"cgo in {f}")
            same_package = [f for f in by_dir[d2] if clause[f] == clause[target_file]]
            if sum(declarations(files[f], name) for f in same_package) != 1:
                problems.append("target name is not declared exactly once in its package")
            caller_nodes = [n for n in doc["nodes"] if n["path"] == claim["caller"] and n.get("file") and rel(n["file"]) == caller_file]
            if not caller_nodes or caller_nodes[0]["kind"] != "Function":
                problems.append("caller is not a function node")
            if problems:
                violations.append({"file": caller_file, "start_byte": claim["start_byte"], "site": site[:60], "problems": problems})
    report = {"must_claims_verified": total, "files_in_tree": len(files), "violations": violations}
    Path(a.output).write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k != "violations"}, indent=2), "violations:", len(violations))
    for v in violations[:10]:
        print(" ", v)
    return 1 if violations else 0


if __name__ == "__main__":
    raise SystemExit(main())
