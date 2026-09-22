#!/usr/bin/env python3
"""Stage 3 Rust audit: deterministic, Girder-independent call-site sampler.

Per docs/roadmap.md's frozen Stage 3 policy: site enumeration must not use
Girder's own parser (a site its tree-sitter grammar fails to see would
never enter a Girder-derived sample), so this uses a plain regex over the
three already-cached, sha256-verified crates
(docs/core-representative-corpus.json: petgraph-0.6.5, serde_json-1.0.150,
regex-1.12.4). The regexes are deliberately crude but independent of
Girder; they exist to draw an unbiased sample of call-site *shapes*, not to
classify anything. Ground truth (must/may/unknown) is authored by reading
each selected site's actual source afterward, as a separate step -- this
script writes no expected classification.
"""
from __future__ import annotations

import argparse
import json
import random
import re
import sys
import tarfile
from pathlib import Path
from typing import Sequence

try:
    from tools.core_representative_benchmark import DEFAULT_CACHE, acquire_artifact, extract_archive
except ModuleNotFoundError:
    from core_representative_benchmark import DEFAULT_CACHE, acquire_artifact, extract_archive

REPO_ROOT = Path(__file__).resolve().parents[1]
CORPUS_PATH = REPO_ROOT / "docs" / "core-representative-corpus.json"
RUST_CRATE_IDS = ("petgraph-0.6.5", "serde_json-1.0.150", "regex-1.12.4")

# Applied to a single trimmed source line. Order matters: more specific
# shapes are checked before the generic "plain call" fallback so a line
# isn't double-counted under a broader pattern.
SHAPE_PATTERNS: list[tuple[str, re.Pattern[str]]] = [
    ("macro_invocation", re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*!\s*[(\[{]")),
    (
        "path_or_associated_call",
        re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)+\s*\("),
    ),
    ("method_call", re.compile(r"\.\s*[A-Za-z_][A-Za-z0-9_]*\s*\(")),
    (
        "fn_pointer_or_closure",
        re.compile(r"Box<\s*dyn\s+Fn|impl\s+Fn(?:Mut|Once)?\s*\(|\)\s*\("),
    ),
    (
        "operator_usage",
        re.compile(
            r"[A-Za-z_][A-Za-z0-9_]*\s*(?:\+|-(?!>)|\*|/|==|!=|<=|>=)\s*[A-Za-z_][A-Za-z0-9_]*"
        ),
    ),
    ("plain_call", re.compile(r"(?<![.\w:])[A-Za-z_][A-Za-z0-9_]*\s*\(")),
]

KEYWORDS_NOT_CALLS = {
    "if", "while", "for", "match", "fn", "let", "return", "loop", "else",
    "impl", "struct", "enum", "trait", "pub", "use", "mod", "where", "as",
    "unsafe", "async", "move", "in",
}


def classify_line(line: str) -> str | None:
    stripped = line.strip()
    if not stripped or stripped.startswith("//"):
        return None
    for shape, pattern in SHAPE_PATTERNS:
        match = pattern.search(stripped)
        if not match:
            continue
        if shape == "plain_call":
            word = re.match(r"[A-Za-z_][A-Za-z0-9_]*", match.group())
            if word and word.group() in KEYWORDS_NOT_CALLS:
                continue
        return shape
    return None


def collect_sites(crate_root: Path, crate_id: str) -> list[dict[str, object]]:
    sites = []
    for rs_file in sorted(crate_root.rglob("*.rs")):
        try:
            lines = rs_file.read_text(encoding="utf-8", errors="strict").splitlines()
        except (UnicodeDecodeError, OSError):
            continue
        for lineno, line in enumerate(lines, start=1):
            shape = classify_line(line)
            if shape is None:
                continue
            sites.append(
                {
                    "crate": crate_id,
                    "file": str(rs_file.relative_to(crate_root)),
                    "line": lineno,
                    "shape": shape,
                    "text": line.strip()[:200],
                }
            )
    return sites


def stratified_sample(
    sites: Sequence[dict[str, object]], total: int, seed: int
) -> list[dict[str, object]]:
    by_shape: dict[str, list[dict[str, object]]] = {}
    for site in sites:
        by_shape.setdefault(site["shape"], []).append(site)  # type: ignore[index]
    shapes = sorted(by_shape)
    rng = random.Random(seed)
    for shape_sites in by_shape.values():
        rng.shuffle(shape_sites)
    per_shape = max(1, total // len(shapes))
    selected: list[dict[str, object]] = []
    for shape in shapes:
        selected.extend(by_shape[shape][:per_shape])
    # Top up to `total` round-robin across shapes with remaining sites, so a
    # shape with few real occurrences (e.g. fn_pointer_or_closure) doesn't
    # silently shrink the total below the precommitted minimum.
    leftovers = {shape: by_shape[shape][per_shape:] for shape in shapes}
    shape_cycle = [s for s in shapes if leftovers[s]]
    i = 0
    while len(selected) < total and shape_cycle:
        shape = shape_cycle[i % len(shape_cycle)]
        if leftovers[shape]:
            selected.append(leftovers[shape].pop(0))
        else:
            shape_cycle.remove(shape)
            continue
        i += 1
    rng.shuffle(selected)
    return selected


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-dir", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--offline", action="store_true", default=True)
    parser.add_argument("--seed", type=int, default=20260922)
    parser.add_argument("--total", type=int, default=105)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=float, default=60.0)
    args = parser.parse_args(argv)

    corpus = json.loads(CORPUS_PATH.read_text(encoding="utf-8"))
    repos_by_id = {r["id"]: r for r in corpus["repositories"]}

    all_sites: list[dict[str, object]] = []
    import tempfile

    with tempfile.TemporaryDirectory(prefix="dispatch-audit-sites-") as tmp:
        work_root = Path(tmp)
        for crate_id in RUST_CRATE_IDS:
            manifest = repos_by_id[crate_id]
            archive = acquire_artifact(
                manifest["artifact"],
                args.cache_dir,
                offline=args.offline,
                timeout_seconds=args.timeout_seconds,
            )
            crate_root = extract_archive(
                archive, work_root / crate_id, manifest["artifact"]["root"]
            )
            all_sites.extend(collect_sites(crate_root, crate_id))

    if not all_sites:
        raise SystemExit("no call sites found; cache extraction likely failed")

    selected = stratified_sample(all_sites, args.total, args.seed)
    document = {
        "schema_version": 1,
        "seed": args.seed,
        "requested_total": args.total,
        "selected_total": len(selected),
        "total_sites_discovered": len(all_sites),
        "crates": list(RUST_CRATE_IDS),
        "shape_counts_discovered": {
            shape: sum(1 for s in all_sites if s["shape"] == shape)
            for shape, _ in SHAPE_PATTERNS
        },
        "shape_counts_selected": {
            shape: sum(1 for s in selected if s["shape"] == shape)
            for shape, _ in SHAPE_PATTERNS
        },
        "sites": selected,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {len(selected)} sites to {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
