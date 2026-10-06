#!/usr/bin/env python3
"""Enumerate candidate Go call sites by regex only (Stage 3, Go audit).

Independent of Girder and of any Go parser: a call site Girder's grammar misses
must still be able to enter the sample. Plain Python, stdlib only. Never run
`girder` on the tree this reads (see docs/observations/stage3-go-audit/).

  python3 -m tools.go_audit_sites --root DIR --counts
  python3 -m tools.go_audit_sites --root DIR --emit --seed 20261006
  python3 -m tools.go_audit_sites --self-test

Known approximations: a call whose callee is parenthesized or a type literal
(`(f)(x)`, `(*T)(x)`, `map[K]V(x)`) is not seen; an immediately-invoked function
literal is detected by brace matching after the parameter list and may miss
result types containing braces; a method header is recognized by the `func (`
that opens its receiver.
"""
from __future__ import annotations

import argparse
import collections
import json
import random
import re
import sys
from pathlib import Path

BUILTINS = {"len", "cap", "append", "make", "new", "copy", "delete", "close", "panic",
            "recover", "print", "println", "min", "max", "clear", "complex", "real", "imag"}
KEYWORDS = {"if", "for", "switch", "select", "return", "case", "range", "else", "go", "defer",
            "chan", "map", "struct", "interface", "var", "const", "type", "package", "import"}
CAND = re.compile(r"(?<![\w.])([A-Za-z_]\w*(?:\s*\.\s*[A-Za-z_]\w*)*)(\s*\[[^\[\]()]*\])?\s*\(")
AFTER_CALL = re.compile(r"(?<=[)\]])\s*\.\s*([A-Za-z_]\w*)\s*\(")
TOP_FUNC = re.compile(r"(?m)^func\b\s*(?:\([^)]*\)\s*)?([A-Za-z_]\w*)")
DECL = re.compile(r"(?m)^func\s+([A-Za-z_]\w*)\s*[\[(]")


def blank(src: bytes) -> bytes:
    """Replace comments and string/rune/raw-string contents with spaces (offsets kept)."""
    out, i, n = bytearray(src), 0, len(src)
    while i < n:
        c = src[i:i + 1]
        two = src[i:i + 2]
        if two == b"//":
            j = src.find(b"\n", i)
            j = n if j < 0 else j
        elif two == b"/*":
            j = src.find(b"*/", i + 2)
            j = n if j < 0 else j + 2
        elif c in (b'"', b"'"):
            j = i + 1
            while j < n and src[j:j + 1] != c and src[j:j + 1] != b"\n":
                j += 2 if src[j:j + 1] == b"\\" else 1
            j = min(j + 1, n)
        elif c == b"`":
            j = src.find(b"`", i + 1)
            j = n if j < 0 else j + 1
        else:
            i += 1
            continue
        for k in range(i, j):
            if out[k] != 10:
                out[k] = 32
        i = j
    return bytes(out)


def match_close(text: str, i: int, open_c: str, close_c: str) -> int:
    depth = 0
    for j in range(i, len(text)):
        depth += text[j] == open_c
        depth -= text[j] == close_c
        if depth == 0:
            return j
    return -1


def import_names(original: str) -> set[str]:
    names = set()
    spans = [m.group(1) for m in re.finditer(r"(?s)\bimport\s*\((.*?)\)", original)]
    spans += re.findall(r"(?m)^import\s+([^\n(]+)$", original)
    for span in spans:
        for m in re.finditer(r'(?:([A-Za-z_]\w*|\.)\s+)?"([^"]+)"', span):
            alias, path = m.group(1), m.group(2)
            name = alias or path.rsplit("/", 1)[-1]
            if name not in ("_", "."):
                names.add(name)
    return names


def scan_sources(files: dict[str, bytes]):
    texts = {f: blank(b).decode("latin-1") for f, b in files.items()}
    decls = collections.defaultdict(lambda: collections.defaultdict(set))
    for f, t in texts.items():
        d = f.rsplit("/", 1)[0] if "/" in f else "."
        for m in DECL.finditer(t):
            decls[d][m.group(1)].add(f)
    cands, excluded = [], collections.Counter()
    for f in sorted(files):
        t, d = texts[f], (f.rsplit("/", 1)[0] if "/" in f else ".")
        imports = import_names(files[f].decode("latin-1"))
        tops = [(m.start(), m.group(1)) for m in TOP_FUNC.finditer(t)]
        iface = []
        for m in re.finditer(r"\binterface\s*\{", t):
            end = match_close(t, m.end() - 1, "{", "}")
            iface.append((m.end(), end if end > 0 else len(t)))
        line_starts = [0] + [m.end() for m in re.finditer(r"\n", t)]

        def locate(off):
            import bisect
            ln = bisect.bisect_right(line_starts, off) - 1
            enc = None
            for pos, name in tops:
                if pos <= off:
                    enc = name
            return ln + 1, off - line_starts[ln] + 1, enc

        def add(off, text, stratum):
            line, col, enc = locate(off)
            cands.append(dict(file=f, byte_offset=off, line=line, col=col,
                              callee_text=re.sub(r"\s+", "", text), stratum=stratum,
                              enclosing_func=enc))

        for m in CAND.finditer(t):
            chain, inst, off = m.group(1), m.group(2), m.start(1)
            before = t[:off].rstrip()
            if any(a <= off < b for a, b in iface):
                excluded["interface_method_spec"] += 1
                continue
            if chain == "func":
                end = match_close(t, m.end() - 1, "(", ")")
                brace = t.find("{", end) if end > 0 else -1
                nl = t.find("\n", end) if end > 0 else -1
                if brace > 0 and (nl < 0 or brace < nl or t[end + 1:brace].strip() != ""):
                    close = match_close(t, brace, "{", "}")
                    if close > 0 and t[close + 1:].lstrip().startswith("("):
                        add(off, "func", "iife")
                        continue
                excluded["func_literal_or_type_or_receiver"] += 1
                continue
            if chain in KEYWORDS:
                excluded["keyword"] += 1
                continue
            if before.endswith("func"):
                excluded["func_declaration_header"] += 1
                continue
            if before.endswith(")"):
                start = before.rfind("(")
                depth, k = 0, len(before) - 1
                while k >= 0:
                    depth += before[k] == ")"
                    depth -= before[k] == "("
                    if depth == 0:
                        break
                    k -= 1
                if k >= 0 and before[:k].rstrip().endswith("func"):
                    excluded["method_declaration_header"] += 1
                    continue
            prefix = re.search(r"\b(go|defer)\s*$", before)
            bare = "." not in chain
            if prefix:
                stratum = "go_defer"
            elif inst:
                stratum = "generic"
            elif bare and chain in BUILTINS:
                stratum = "builtin"
            elif bare and (decls[d].get(chain, set()) - {f}):
                stratum = "bare_cross_file"
            elif bare:
                stratum = "bare_other"
            elif re.split(r"\s*\.\s*", chain)[0] in imports:
                stratum = "pkg_qualified"
            else:
                stratum = "selector"
            add(off, chain + (inst or ""), stratum)
        for m in AFTER_CALL.finditer(t):
            if not any(a <= m.start(1) < b for a, b in iface):
                add(m.start(1), "." + m.group(1), "selector")
    cands.sort(key=lambda c: (c["file"], c["byte_offset"]))
    return cands, dict(excluded)


def scan_tree(root: Path):
    files = {p.relative_to(root).as_posix(): p.read_bytes() for p in sorted(root.rglob("*.go"))}
    return scan_sources(files), len(files)


def counts(root: Path) -> dict:
    (cands, excluded), nfiles = scan_tree(root)
    per_pkg: dict = collections.defaultdict(collections.Counter)
    for c in cands:
        d = c["file"].rsplit("/", 1)[0] if "/" in c["file"] else "."
        per_pkg[d][c["stratum"]] += 1
    return {"files": nfiles, "candidates": len(cands),
            "per_stratum": dict(collections.Counter(c["stratum"] for c in cands)),
            "excluded_by_reason": excluded,
            "per_package": {k: dict(v) for k, v in sorted(per_pkg.items())}}


def emit(root: Path, seed: int) -> list:
    (cands, _), _ = scan_tree(root)
    rng, by = random.Random(seed), collections.defaultdict(list)
    for c in cands:
        by[c["stratum"]].append(c)
    out = []
    for stratum in sorted(by):
        group = list(by[stratum])
        rng.shuffle(group)
        out.extend(group)
    return out


def self_test() -> int:
    def strata(src, name="p/a.go", extra=None):
        files = {name: src.encode()}
        files.update({k: v.encode() for k, v in (extra or {}).items()})
        c, ex = scan_sources(files)
        return [(x["callee_text"], x["stratum"]) for x in c if x["file"] == name], ex

    checks = [
        ("comment/string ignored", 'package p\nfunc F() {\n// foo(1)\ns := "bar(2)"\n_ = s\n}\n', [], None),
        ("builtin", "package p\nfunc F(x []int) int { return len(x) }\n", [("len", "builtin")], None),
        ("method header excluded", "package p\ntype T struct{}\nfunc (r *T) M() {}\n", [], "method_declaration_header"),
        ("interface spec excluded", "package p\ntype I interface {\n\tDo(x int) int\n}\n", [], "interface_method_spec"),
        ("defer", "package p\nfunc F() { defer g(1) }\nfunc g(int) {}\n", [("g", "go_defer")], None),
        ("selector chain", "package p\nfunc F(a A) { a.b.c(1) }\n", [("a.b.c", "selector")], None),
        ("generic", "package p\nfunc F() { G[int](1) }\nfunc G[T any](x T) {}\n", [("G[int]", "generic")], None),
        ("iife", "package p\nfunc F() int { return func() int { return 1 }() }\n", [("func", "iife")], None),
        ("func type excluded", "package p\ntype H func(int) int\n", [], "func_literal_or_type_or_receiver"),
    ]
    bad = 0
    for name, src, want, excl in checks:
        got, ex = strata(src)
        if got != want or (excl and excl not in ex):
            print("FAIL", name, got, ex)
            bad += 1
    cross, _ = strata('package p\nfunc F() { H() }\n', extra={"p/b.go": "package p\nfunc H() {}\n"})
    if cross != [("H", "bare_cross_file")]:
        print("FAIL cross-file", cross)
        bad += 1
    pq, _ = strata('package p\nimport "math/rand"\nfunc F() { rand.Int() }\n')
    if pq != [("rand.Int", "pkg_qualified")]:
        print("FAIL pkg-qualified", pq)
        bad += 1
    print("OK" if not bad else f"{bad} FAILED")
    return 1 if bad else 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--root")
    ap.add_argument("--counts", action="store_true")
    ap.add_argument("--emit", action="store_true")
    ap.add_argument("--seed", type=int, default=20261006)
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.root or not (a.counts or a.emit):
        ap.error("--root and one of --counts/--emit are required")
    json.dump(counts(Path(a.root)) if a.counts else emit(Path(a.root), a.seed), sys.stdout, indent=2)
    print()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
