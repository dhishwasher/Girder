#!/usr/bin/env python3
"""Stage 3 TypeScript real-repository audit scorer.

Mirrors tools/dispatch_audit_scorer_python.py's methodology, adapted to
TypeScript's own masker/shape/site schema. A parallel sibling, not a
parameterized rewrite (same reasoning the Python scorer used relative to
the Rust one): lower risk to either frozen scorer than genericizing across
three languages' different relocation logic.

Scores each labeled site in a TypeScript audit-sites-labeled file against
Girder's actual per-call-site answer, read from `girder analyze --json`
then `girder inspect <graph>.aether --json`.

Two differences from the Python scorer, both checked directly rather than
assumed to carry over unchanged:

1. **Target comparison.** Must-labeled sites carry a `true_target`
   (`file:line`) field the Python labeled-sites file doesn't have. When
   `true_class == observed_class == "must"`, this scorer additionally
   compares Girder's own claimed target NodeId's `file`/`span` against
   `true_target` -- a Must claim pointing at the wrong same-named
   definition (a real risk this repo's audit corpus has: zod pins both
   `src/` and `deno/lib/` copies of the same class names, and
   typescript-6.0.3 has four separate `TestSession` declarations) is
   scored `overclaim`, not `exact`, even though the CLASS matched. A Must
   site with no `true_target` recorded is a labeling gap, not a pass --
   scored `failed`, never silently treated as exact.
2. **`NEVER_COVERS`, built from this language's own inspect data, not
   copied from Python's.** Checked directly against a real
   `girder inspect` run on the smallest pinned repository
   (class-validator-0.15.1): `implicit-runtime-dispatch-not-certified` and
   `duplicate-semantic-path` both have whole-module byte spans (start_byte
   0 through the file's own length) -- treating either as "covering"
   collapses `unsafe_exclusion` to permanently unreachable, the same
   reasoning Python's own single-entry `NEVER_COVERS` uses.
   `unexpanded-macro-or-decorator` (TypeScript's decorator-specific
   coverage-gap reason) was checked and found NARROW (spans of a few
   hundred bytes at most, anchored to a specific decorator expression, not
   the whole file) -- left OUT of `NEVER_COVERS`, since excluding it would
   hide genuine per-site Unknown evidence the scorer should see and score
   against, not suppress. `duplicate-semantic-path` is emitted by the same
   shared `claims.rs` code path for every language (confirmed by reading
   the extractor directly, not assumed TypeScript-specific) -- neither the
   Rust nor the Python scorer's own `NEVER_COVERS` includes it, a latent
   gap in both already-DONE audits that happened not to matter because
   their specific sampled files never tripped `duplicate_paths`, recorded
   here rather than silently carried forward unfixed for this language too.

Sites whose true_class is "not_a_call_site" are not scored. For every
other site: if no CallClaim anywhere in the package contains the site's
byte offset, that is unsafe_exclusion, the most severe cell -- same policy
as Rust and Python, not renegotiated for TypeScript.
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
    from tools.dispatch_audit_site_selector_typescript import (
        SHAPE_PATTERNS,
        classify_line_with_match,
        mask_ts_source,
    )
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from dispatch_audit_site_selector_typescript import (
        SHAPE_PATTERNS,
        classify_line_with_match,
        mask_ts_source,
    )


def run(binary: Path, args: Sequence[str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(binary), *args], capture_output=True, text=True, timeout=600
    )


def line_text_matches(pkg_root: Path, site: dict) -> bool:
    """Whether `site["line"]`'s actual current text in the extracted package
    still matches the `text` the selector recorded when it picked this
    site -- see the Python scorer's own docstring for the same check."""
    path = pkg_root / site["file"]
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return False
    if site["line"] - 1 >= len(lines):
        return False
    return lines[site["line"] - 1].strip()[:200] == site["text"]


def _decorator_callee_column(masked_line: str, match_start: int, match_text: str) -> int:
    """For a `decorator` shape match (which starts at `@`), find the column
    of the CALLEE identifier's own start -- the last `.`-separated segment
    of the decorator name -- not the `@` itself. Per labeling-rubric-
    addendum.md §1: the selected site IS the factory call, and Girder's
    own claim for that call is expected to start at the callee's own name,
    the same way an ordinary `foo.bar.Baz(...)` call's claim starts at
    `Baz`, not at `foo`. Confirmed against the shape's own regex
    (`^@\\s*[A-Za-z_][A-Za-z0-9_.]*`): the match text is `@` plus optional
    whitespace plus a dotted name; strip the `@`/whitespace prefix, then
    advance past every `.` to the last segment.
    """
    name_part = match_text.lstrip("@").lstrip()
    prefix_len = len(match_text) - len(match_text.lstrip("@").lstrip())
    last_dot = name_part.rfind(".")
    segment_offset = last_dot + 1 if last_dot != -1 else 0
    return match_start + prefix_len + segment_offset


def site_byte_offset(pkg_root: Path, site: dict) -> int | None:
    """Byte offset (into the whole file, UTF-8) of the start of the shape
    pattern's match on the site's sampled line -- reuses
    `classify_line_with_match` directly (the same `finditer` + keyword-skip
    code path the selector itself uses to classify a line), so relocation
    can never disagree with the shape the site was originally selected
    under. Matches against the MASKED line for the same reason the Python
    scorer does: a string/comment/template-literal span earlier on the
    same line that happens to also look call-shaped must not be found
    instead of the real match.

    For a `decorator` site, the match itself starts at `@` (the shape
    pattern's own anchor), but the actual call site is the callee's own
    name -- see `_decorator_callee_column`.
    """
    path = pkg_root / site["file"]
    text = path.read_text(encoding="utf-8", errors="replace")
    lines = text.splitlines()
    masked_text = mask_ts_source(text)
    masked_lines = masked_text.splitlines()
    if site["line"] - 1 >= len(lines) or site["line"] - 1 >= len(masked_lines):
        return None
    line = lines[site["line"] - 1]
    masked_line = masked_lines[site["line"] - 1]
    stripped = masked_line.strip()
    result = classify_line_with_match(masked_line)
    if result is None:
        return None
    shape, match = result
    if shape != site["shape"]:
        return None
    lead = len(masked_line) - len(masked_line.lstrip())
    match_col = lead + stripped.index(match.group(), 0)
    if shape == "decorator":
        col = _decorator_callee_column(masked_line, match_col, match.group())
    else:
        col = match_col
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


def resolve_node_id(raw_target: str) -> str:
    """A `CallClaim.targets` entry from the RON `call_evidence_v1` text is a
    decimal u64, optionally parenthesized (e.g. `"(999)"`) -- NOT the
    16-character zero-padded lowercase hex string `girder inspect`'s own
    node `id` field uses. Converting with plain `hex()`/`format(x, "x")`
    silently drops leading zeros and fails to match roughly 1/16 of real
    ids -- the exact hex-padding bug found and fixed in
    docs/observations/stage3-python-audit/after-transformed-scope-fix/
    correction-2/common.py earlier in this program. Reused here rather
    than re-derived, per that correction's own stated lesson."""
    return format(int(raw_target.strip().strip("()")), "016x")


def extract_target_locations(inspect_path: Path) -> dict[str, tuple[str, int]]:
    """Map NodeId (hex string) -> (file, start_row+1) for every node, so an
    observed Must claim's target can be compared against a site's own
    `true_target` (`file:line`)."""
    doc = json.loads(inspect_path.read_text())
    locations = {}
    for node in doc["nodes"]:
        file = node.get("file")
        span = node.get("span")
        if not file or not span:
            continue
        locations[node["id"]] = (file, span["start_row"] + 1)
    return locations


# See the module docstring for how this was determined from real
# TypeScript inspect output, not copied from Python's own NEVER_COVERS.
NEVER_COVERS = {"implicit-runtime-dispatch-not-certified", "duplicate-semantic-path"}


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
    locations_by_package = {
        pkg: extract_target_locations(path) for pkg, path in inspect_paths.items()
    }
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
        if not line_text_matches(extracted_roots[s["package"]], s):
            entry["status"] = "site_relocation_failed"
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
        cell = cell_label(s["true_class"], observed_class)
        if s["true_class"] == "must" and observed_class == "must":
            true_target = s.get("true_target")
            if not true_target:
                entry["status"] = "failed"
                entry["failure_reason"] = "must site has no recorded true_target"
                results.append(entry)
                continue
            expected_file = true_target.split(" -> ")[-1].split(":")[0]
            expected_line = int(true_target.split(":")[-1])
            observed_targets = claim["targets"] if claim else []
            locations = locations_by_package[s["package"]]
            observed_locations = [locations.get(resolve_node_id(t)) for t in observed_targets]
            match_found = any(
                loc is not None and loc[0] == expected_file and loc[1] == expected_line
                for loc in observed_locations
            )
            entry["expected_target"] = f"{expected_file}:{expected_line}"
            entry["observed_targets"] = [
                f"{loc[0]}:{loc[1]}" if loc else "<unresolved-node-id>"
                for loc in observed_locations
            ]
            if not match_found:
                cell = "overclaim"
        entry["cell"] = cell
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
        "failed_count": sum(1 for r in results if r["status"] == "failed"),
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
