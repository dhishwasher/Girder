#!/usr/bin/env python3
"""Stage 3 Rust real-repository audit scorer.

Scores each labeled site in audit-sites-labeled.json (or a later revision)
against Girder's actual per-call-site answer, read from `girder analyze
--json` (builds+saves a graph) then `girder inspect <graph>.aether --json`
(dumps every node's raw call_evidence_v1 RON attribute).

Matching is byte-offset precise, not row-fuzzy: a site's byte offset (the
start of its regex match on its sampled line) must fall within a CallClaim's
[start_byte, end_byte) span. A line with more than one call in the sampled
window, or a call whose claim landed on a neighboring line due to a
multi-line expression, cannot silently pick the wrong claim this way.

Sites whose true_class is "not_a_call_site" are not scored (no Girder
answer is meaningful for them). For every other site: if no CallClaim
anywhere in the crate contains the site's byte offset, that is not a
neutral "couldn't check" outcome -- per the frozen Stage 3 policy
(docs/roadmap.md), it means the call is unreachable through any classified
query surface Girder exposes, which is exactly unsafe_exclusion, the most
severe cell. A module-level gap (e.g. duplicate-semantic-path) recorded
elsewhere in the same file does not change this: it discloses that
*something* is wrong in the file, but does not make this specific call
appear in any impact/test-impact/orient answer.
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

# Mirrors dispatch_audit_site_selector.py's SHAPE_PATTERNS, used here only to
# relocate each site's original match span within its sampled line so a
# byte offset can be computed; site *selection* itself is not redone.
try:
    from tools.dispatch_audit_site_selector import SHAPE_PATTERNS
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from dispatch_audit_site_selector import SHAPE_PATTERNS


def run(binary: Path, args: Sequence[str]) -> subprocess.CompletedProcess:
    return subprocess.run([str(binary), *args], capture_output=True, text=True, timeout=120)


def site_byte_offset(crate_root: Path, site: dict) -> int | None:
    """Byte offset (into the whole file) of the start of the shape pattern's
    match on the site's sampled line."""
    path = crate_root / site["file"]
    text = path.read_text(encoding="utf-8", errors="replace")
    lines = text.split("\n")
    line = lines[site["line"] - 1]
    pattern = dict(SHAPE_PATTERNS)[site["shape"]]
    match = pattern.search(line.strip())
    if match is None:
        return None
    # match was found against the *stripped* line; recover its offset in the
    # original (unstripped) line.
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


def find_covering_claim(claims: list[dict], file: str, byte_offset: int) -> dict | None:
    covering = [
        c for c in claims if c["file"] == file and c["start_byte"] <= byte_offset < c["end_byte"]
    ]
    if not covering:
        return None
    # Prefer the tightest (smallest span) covering claim if more than one.
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


def score(sites_path: Path, extracted_roots: dict[str, Path], inspect_paths: dict[str, Path]) -> dict:
    sites = json.loads(sites_path.read_text())["sites"]
    claims_by_crate = {crate: extract_call_claims(path) for crate, path in inspect_paths.items()}
    results = []
    for i, s in enumerate(sites):
        entry = {
            "index": i,
            "crate": s["crate"],
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
        offset = site_byte_offset(extracted_roots[s["crate"]], s)
        if offset is None:
            entry["status"] = "site_relocation_failed"
            results.append(entry)
            continue
        entry["byte_offset"] = offset
        claim = find_covering_claim(claims_by_crate[s["crate"]], s["file"], offset)
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
    parser.add_argument("--crate-root", action="append", required=True,
                         help="crate_id=/path/to/extracted/crate, repeatable")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)

    extracted_roots = {}
    for entry in args.crate_root:
        crate_id, path = entry.split("=", 1)
        extracted_roots[crate_id] = Path(path)

    inspect_paths = {}
    for crate_id, root in extracted_roots.items():
        analyze = run(args.bitcode, ["analyze", str(root), "--json"])
        if analyze.returncode != 0:
            print(f"analyze failed for {crate_id}: {analyze.stderr}", file=sys.stderr)
            return 1
        graph_path = json.loads(analyze.stdout)["graph_path"]
        inspect_out = args.output.parent / f"{crate_id}-inspect.json"
        inspect = run(args.bitcode, ["inspect", graph_path, "--json"])
        if inspect.returncode != 0:
            print(f"inspect failed for {crate_id}: {inspect.stderr}", file=sys.stderr)
            return 1
        inspect_out.write_text(inspect.stdout)
        inspect_paths[crate_id] = inspect_out

    document = score(args.sites, extracted_roots, inspect_paths)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2) + "\n")
    print(f"wrote {args.output}: {document['scored_count']} scored, "
          f"{document['not_a_call_site_count']} not_a_call_site, "
          f"cells={document['cell_counts']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
