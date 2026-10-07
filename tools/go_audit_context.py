#!/usr/bin/env python3
"""Precompute per-site evidence for Go audit labeling (Stage 3, Go).

Plain Python, regex only; never runs `girder`. For every selected site it records
the facts a labeler needs to read the language truth from source: the site line
and surrounding lines, the enclosing function header, local bindings of the
callee's first identifier before the site, the package clause, imports, and the
package-level declarations of the callee name across the directory.

  python3 -m tools.go_audit_context --root TREE --sites sites.json --output context.json
"""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

from tools.go_audit_sites import blank, import_names

FUNC_START = re.compile(r"(?m)^func\b[^\n]*")


def decl_lines(text: str, name: str):
    """Top-level declarations of `name` in one file: (line, kind, has_body, header)."""
    out = []
    lines = text.split("\n")
    for i, line in enumerate(lines, 1):
        m = re.match(rf"^func\s+{re.escape(name)}\s*[\[(]", line)
        if m:
            out.append((i, "func", line.rstrip().endswith("{") or "{" in line, line.strip()))
            continue
        m = re.match(rf"^func\s*\([^)]*\)\s*{re.escape(name)}\s*[\[(]", line)
        if m:
            out.append((i, "method", "{" in line, line.strip()))
            continue
        m = re.match(rf"^(type|var|const)\s+{re.escape(name)}\b", line)
        if m:
            out.append((i, m.group(1), True, line.strip()))
            continue
        m = re.match(rf"^\s+{re.escape(name)}\s*(=|\s[A-Za-z\[*])", line)
        if m and i > 1:  # grouped var/const/type member; context lets the labeler decide
            prev = "\n".join(lines[max(0, i - 40):i - 1])
            if re.search(r"(?m)^(var|const|type)\s*\($", prev):
                out.append((i, "grouped-decl", True, line.strip()))
    return out


def build(root: Path, sites: list[dict]) -> list[dict]:
    originals = {p.relative_to(root).as_posix(): p.read_text("utf-8", "replace")
                 for p in sorted(root.rglob("*.go"))}
    blanked = {f: blank(t.encode()).decode("latin-1") for f, t in originals.items()}
    out = []
    for idx, site in enumerate(sites):
        f, off = site["file"], site["byte_offset"]
        text, lines = originals[f], originals[f].split("\n")
        directory = f.rsplit("/", 1)[0] if "/" in f else "."
        line_no = site["line"]
        first = re.match(r"\.?([A-Za-z_]\w*)", site["callee_text"])
        name = first.group(1) if first else site["callee_text"]
        enclosing = None
        for m in FUNC_START.finditer(blanked[f]):
            if m.start() <= off:
                enclosing = (blanked[f][:m.start()].count("\n") + 1, m.group(0).strip())
        local = []
        if enclosing:
            start = enclosing[0]
            for i in range(start, line_no + 1):
                raw = lines[i - 1]
                if re.search(rf"\b{re.escape(name)}\b\s*(:=|=[^=]|,\s*\w+\s*:=)", raw) or \
                        (i == start and re.search(rf"\b{re.escape(name)}\b", raw)) or \
                        re.search(rf"\b(var|range)\b[^\n]*\b{re.escape(name)}\b", raw):
                    local.append({"line": i, "text": raw.strip()[:200]})
        decls = []
        for other, other_text in originals.items():
            if (other.rsplit("/", 1)[0] if "/" in other else ".") != directory:
                continue
            for line, kind, body, header in decl_lines(other_text, name):
                decls.append({"file": other, "line": line, "kind": kind,
                              "has_body": body, "header": header[:200]})
        pkg = re.search(r"(?m)^package\s+(\w+)", text)
        out.append({
            "id": idx, "file": f, "line": line_no, "col": site["col"],
            "stratum": site["stratum"], "callee_text": site["callee_text"],
            "call_end_byte": site["call_end_byte"], "package": pkg.group(1) if pkg else None,
            "build_constraint": bool(re.search(r"(?m)^//go:build|^// \+build", text)),
            "filename_suffix_constraint": bool(re.search(
                r"_(linux|windows|darwin|plan9|js|wasm|amd64|arm64|386|arm|s390x|ppc64le)\.go$", f)),
            "site_line": lines[line_no - 1].rstrip(),
            "context": [{"line": i, "text": lines[i - 1].rstrip()[:240]}
                        for i in range(max(1, line_no - 3), min(len(lines), line_no + 2) + 1)],
            "enclosing_function": {"line": enclosing[0], "header": enclosing[1][:240]} if enclosing else None,
            "imports": sorted(import_names(text)),
            "callee_first_identifier": name,
            "local_bindings_before_site": local[:12],
            "package_level_declarations_of_name": decls[:12],
        })
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", required=True)
    ap.add_argument("--sites", required=True)
    ap.add_argument("--output", required=True)
    a = ap.parse_args()
    sites = json.loads(Path(a.sites).read_text())
    Path(a.output).write_text(json.dumps(build(Path(a.root), sites), indent=2) + "\n")
    print(f"{len(sites)} sites")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
