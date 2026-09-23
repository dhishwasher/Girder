#!/usr/bin/env python3
"""Stage 3 Python real-repository audit scorer.

Mirrors tools/dispatch_audit_scorer.py's Rust methodology exactly. A
straight import of the Rust scorer was checked and found NOT to work
unchanged for Python, contrary to methodology.md's original (explicitly
flagged as "unverified") expectation: `site_byte_offset` imports
`SHAPE_PATTERNS` from the Rust site selector specifically to relocate a
site's match span within its line, and Python's shape names
(`decorator`/`dynamic_dispatch`/`qualified_attribute_call`/
`operator_dunder`) aren't keys in that dict at all -- it would raise
`KeyError` on the first Python-shaped site. This file is a parallel
sibling, not a parameterized rewrite of the Rust one (same reasoning
`dispatch_audit_site_selector_python.py` used relative to its Rust
counterpart): lower risk to the frozen, well-tested Rust scorer than
genericizing it.

Scores each labeled site in a Python audit-sites-labeled file against
Girder's actual per-call-site answer, read from `girder analyze --json`
then `girder inspect <graph>.aether --json` (dumps every node's raw
call_evidence_v1 RON attribute) -- identical mechanism to Rust, confirmed
in methodology.md's prerequisite check that Python nodes carry the same
per-call-site evidence structure.

Sites whose true_class is "not_a_call_site" are not scored. For every
other site: if no CallClaim anywhere in the package contains the site's
byte offset, that is unsafe_exclusion, the most severe cell -- same
policy as Rust's scorer, not renegotiated for Python.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Sequence

CALL_RE = re.compile(
    r'\(site:\(start_byte:(\d+),end_byte:(\d+),start_row:(\d+),start_col:(\d+)\),'
    r'class:(\w+),targets:\[([^\]]*)\],reason:"((?:[^"\\]|\\.)*)",coverage_gap:(true|false)\)'
)

try:
    from tools.dispatch_audit_site_selector_python import SHAPE_PATTERNS
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from dispatch_audit_site_selector_python import SHAPE_PATTERNS


def run(binary: Path, args: Sequence[str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(binary), *args], capture_output=True, text=True, timeout=120
    )


def site_byte_offset(pkg_root: Path, site: dict) -> int | None:
    """Byte offset (into the whole file) of the start of the shape pattern's
    match on the site's sampled line. Matches against the RAW (unmasked)
    line -- the selector already used the masked text to decide whether a
    line counted as a call site at all, but the site's own recorded `line`
    number and `shape` were chosen from real code, and the actual call
    text at that position is what Girder's own claim span will cover."""
    path = pkg_root / site["file"]
    text = path.read_text(encoding="utf-8", errors="replace")
    lines = text.split("\n")
    line = lines[site["line"] - 1]
    pattern = dict(SHAPE_PATTERNS)[site["shape"]]
    match = pattern.search(line.strip())
    if match is None:
        return None
    stripped = line.strip()
    lead = len(line) - len(line.lstrip())
    col = lead + line.lstrip().index(match.group())
    prefix = "\n".join(lines[: site["line"] - 1])
    return len(prefix.encode("utf-8")) + (1 if site["line"] > 1 else 0) + len(
        line[:col].encode("utf-8")
    )


def extract_call_claims(inspect_path: Path) -> list[dict]:
    doc = json.loads(inspect_path.read_text())
    claims = []
    for node in doc["nodes"]:
        file = node.get("file")
        if not file:
            continue
        for key, value in node.get("attributes", []):
            if key != "call_evidence_v1":
                continue
            for m in CALL_RE.finditer(value):
                start_byte, end_byte, start_row, start_col, cls, targets, reason, cg = m.groups()
                claims.append(
                    {
                        "file": file,
                        "caller": node["path"],
                        "start_byte": int(start_byte),
                        "end_byte": int(end_byte),
                        "class": cls,
                        "reason": reason,
                        "coverage_gap": cg == "true",
                        "targets": [t.strip() for t in targets.split(",") if t.strip()],
                    }
                )
    return claims


# Reasons that do NOT count as covering a site, even when their claim's byte
# span contains it. `implicit-runtime-dispatch-not-certified` is attached
# with a WHOLE-MODULE span on every Python file unconditionally
# (crates/aether-builder/src/mapper/claims.rs) -- treating it as "covering"
# anything would make every offset in every Python file always covered,
# collapsing unsafe_exclusion to permanently unreachable, the same reasoning
# the Rust scorer's own NEVER_COVERS entry uses for its own whole-file gap.
NEVER_COVERS = {"implicit-runtime-dispatch-not-certified"}


def find_covering_claim(claims: list[dict], file: str, byte_offset: int) -> dict | None:
    covering = [
        c
        for c in claims
        if c["file"] == file
        and c["start_byte"] <= byte_offset < c["end_byte"]
        and c["reason"] not in NEVER_COVERS
    ]
    if not covering:
        return None
    covering.sort(key=lambda c: c["end_byte"] - c["start_byte"])
    return covering[0]


def cell_label(expected: str, observed: str) -> str:
    if expected == observed:
        return "exact"
    if observed == "excluded":
        return "unsafe_exclusion"
    if observed == "must":
        return "overclaim"
    if observed == "may" and expected in ("unknown", "excluded"):
        return "overclaim"
    return "conservative"


def score(
    sites_path: Path, extracted_roots: dict[str, Path], inspect_paths: dict[str, Path]
) -> dict:
    sites = json.loads(sites_path.read_text())["sites"]
    claims_by_package = {pkg: extract_call_claims(path) for pkg, path in inspect_paths.items()}
    results = []
    for i, s in enumerate(sites):
        entry = {
            "index": i,
            "package": s["package"],
            "file": s["file"],
            "line": s["line"],
            "shape": s["shape"],
            "true_class": s["true_class"],
            "confidence": s["confidence"],
        }
        if s["true_class"] == "not_a_call_site":
            entry["status"] = "not_a_call_site"
            results.append(entry)
            continue
        offset = site_byte_offset(extracted_roots[s["package"]], s)
        if offset is None:
            entry["status"] = "site_relocation_failed"
            results.append(entry)
            continue
        entry["byte_offset"] = offset
        claim = find_covering_claim(claims_by_package[s["package"]], s["file"], offset)
        observed_class = claim["class"] if claim else "excluded"
        entry["status"] = "scored"
        entry["observed_class"] = observed_class
        entry["observed_reason"] = claim["reason"] if claim else None
        entry["observed_caller"] = claim["caller"] if claim else None
        entry["cell"] = cell_label(s["true_class"], observed_class)
        results.append(entry)

    scored = [r for r in results if r["status"] == "scored"]
    from collections import Counter

    cell_counts = Counter(r["cell"] for r in scored)
    return {
        "schema_version": 1,
        "results": results,
        "cell_counts": dict(cell_counts),
        "not_a_call_site_count": sum(1 for r in results if r["status"] == "not_a_call_site"),
        "site_relocation_failed_count": sum(
            1 for r in results if r["status"] == "site_relocation_failed"
        ),
        "scored_count": len(scored),
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bitcode", type=Path, required=True)
    parser.add_argument("--sites", type=Path, required=True)
    parser.add_argument(
        "--package-root",
        action="append",
        required=True,
        help="package_id=/path/to/extracted/package, repeatable",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)

    extracted_roots = {}
    for entry in args.package_root:
        pkg_id, path = entry.split("=", 1)
        extracted_roots[pkg_id] = Path(path)

    inspect_paths = {}
    for pkg_id, root in extracted_roots.items():
        analyze = run(args.bitcode, ["analyze", str(root), "--json"])
        if analyze.returncode != 0:
            print(f"analyze failed for {pkg_id}: {analyze.stderr}", file=sys.stderr)
            return 1
        graph_path = json.loads(analyze.stdout)["graph_path"]
        inspect_out = args.output.parent / f"{pkg_id}-inspect.json"
        inspect = run(args.bitcode, ["inspect", graph_path, "--json"])
        if inspect.returncode != 0:
            print(f"inspect failed for {pkg_id}: {inspect.stderr}", file=sys.stderr)
            return 1
        inspect_out.write_text(inspect.stdout)
        inspect_paths[pkg_id] = inspect_out

    document = score(args.sites, extracted_roots, inspect_paths)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2) + "\n")
    print(
        f"wrote {args.output}: {document['scored_count']} scored, "
        f"{document['not_a_call_site_count']} not_a_call_site, "
        f"cells={document['cell_counts']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
