#!/usr/bin/env python3
"""Stage 3 Python audit: deterministic, Girder-independent call-site sampler.

Mirrors tools/dispatch_audit_site_selector.py's Rust methodology exactly,
adapted to Python-appropriate call shapes. Per docs/roadmap.md's frozen
Stage 3 policy: site enumeration must not use Girder's own parser (a site
its tree-sitter grammar fails to see would never enter a Girder-derived
sample), so this uses a plain regex over the three already-cached,
sha256-verified Python packages (docs/core-representative-corpus.json:
click-8.4.1, pydantic-2.13.4, requests-2.34.2). The regexes are
deliberately crude but independent of Girder; they exist to draw an
unbiased sample of call-site *shapes*, not to classify anything. Ground
truth (must/may/unknown) is authored by reading each selected site's
actual source afterward, as a separate step -- this script writes no
expected classification.

Shapes, per the roadmap's own list for Python: plain call, method call,
qualified-attribute call, decorator, dynamic dispatch (getattr/callable),
operator/dunder usage.
"""
from __future__ import annotations

import argparse
import io
import json
import random
import re
import sys
import tokenize
from pathlib import Path
from typing import Sequence

try:
    from tools.core_representative_benchmark import DEFAULT_CACHE, acquire_artifact, extract_archive
except ModuleNotFoundError:
    from core_representative_benchmark import DEFAULT_CACHE, acquire_artifact, extract_archive

REPO_ROOT = Path(__file__).resolve().parents[1]
CORPUS_PATH = REPO_ROOT / "docs" / "core-representative-corpus.json"
PYTHON_REPO_IDS = ("click-8.4.1", "pydantic-2.13.4", "requests-2.34.2")

# Applied to a single trimmed source line. Order matters: more specific
# shapes are checked before the generic "plain_call" fallback so a line
# isn't double-counted under a broader pattern.
SHAPE_PATTERNS: list[tuple[str, re.Pattern[str]]] = [
    ("decorator", re.compile(r"^@\s*[A-Za-z_][A-Za-z0-9_.]*")),
    (
        "dynamic_dispatch",
        re.compile(
            r"\b(?:getattr|setattr|hasattr|callable|__getattr__|__getattribute__|__call__)\s*\("
        ),
    ),
    (
        "qualified_attribute_call",
        re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*){2,}\s*\("),
    ),
    ("method_call", re.compile(r"\.\s*[A-Za-z_][A-Za-z0-9_]*\s*\(")),
    (
        "operator_dunder",
        re.compile(
            r"[A-Za-z_][A-Za-z0-9_]*\s*(?:\+|-(?!>)|\*\*?|/{1,2}|==|!=|<=|>=|%|\||&|\^)\s*[A-Za-z_0-9]"
        ),
    ),
    ("plain_call", re.compile(r"(?<![.\w])[A-Za-z_][A-Za-z0-9_]*\s*\(")),
]

KEYWORDS_NOT_CALLS = {
    "if", "while", "for", "def", "class", "return", "elif", "else", "with",
    "lambda", "yield", "raise", "except", "import", "from", "as", "assert",
    "del", "global", "nonlocal", "pass", "break", "continue", "and", "or",
    "not", "in", "is", "async", "await", "try", "finally",
}


def classify_line(line: str) -> str | None:
    stripped = line.strip()
    if not stripped or stripped.startswith("#"):
        return None
    for shape, pattern in SHAPE_PATTERNS:
        match = pattern.search(stripped)
        if not match:
            continue
        if shape in ("plain_call", "dynamic_dispatch"):
            word = re.match(r"[A-Za-z_][A-Za-z0-9_]*", match.group())
            if word and word.group() in KEYWORDS_NOT_CALLS:
                continue
        return shape
    return None


def mask_strings_and_comments(text: str) -> tuple[str, bool]:
    """Replace STRING and COMMENT token spans with spaces, preserving every
    line/column position so downstream line numbers stay correct. Uses
    Python's own `tokenize` stdlib module -- independent of Girder's parser,
    matching the same Girder-independence requirement the Rust site
    selector's regex approach already satisfies. A whole-pool check (not
    just the sampled 105 sites) found 33% of this tool's call-shaped line
    matches fell inside a string or comment token before this mask was
    added -- club/pydantic/requests docstrings are full of executable-
    looking code examples -- so masking, not just disclosure, is required
    here where it wasn't needed for Rust's own audit.

    Returns `(masked_text, tokenize_succeeded)`. On a tokenize failure
    (encountered zero times across the actual three-package pool, but kept
    as a documented fallback rather than an unhandled exception), returns
    the original, unmasked text with `False`, and the caller counts this.
    """
    lines = text.splitlines(keepends=True)
    try:
        for tok in tokenize.generate_tokens(io.StringIO(text).readline):
            if tok.type not in (tokenize.STRING, tokenize.COMMENT):
                continue
            (start_row, start_col), (end_row, end_col) = tok.start, tok.end
            if start_row == end_row:
                line = lines[start_row - 1]
                lines[start_row - 1] = (
                    line[:start_col] + " " * (end_col - start_col) + line[end_col:]
                )
            else:
                first = lines[start_row - 1]
                first_nl = "\n" if first.endswith("\n") else ""
                first_body_len = len(first) - len(first_nl)
                lines[start_row - 1] = (
                    first[:start_col] + " " * (first_body_len - start_col) + first_nl
                )
                for mid in range(start_row, end_row - 1):
                    line = lines[mid]
                    nl = "\n" if line.endswith("\n") else ""
                    lines[mid] = " " * (len(line) - len(nl)) + nl
                last = lines[end_row - 1]
                lines[end_row - 1] = " " * end_col + last[end_col:]
    except (tokenize.TokenError, IndentationError, SyntaxError, ValueError):
        return text, False
    return "".join(lines), True


def collect_sites(
    pkg_root: Path, pkg_id: str, tokenize_failures: list[str]
) -> list[dict[str, object]]:
    sites = []
    for py_file in sorted(pkg_root.rglob("*.py")):
        try:
            raw_text = py_file.read_text(encoding="utf-8", errors="strict")
        except (UnicodeDecodeError, OSError):
            continue
        masked_text, ok = mask_strings_and_comments(raw_text)
        if not ok:
            tokenize_failures.append(str(py_file.relative_to(pkg_root)))
        masked_lines = masked_text.splitlines()
        raw_lines = raw_text.splitlines()
        for lineno, masked_line in enumerate(masked_lines, start=1):
            shape = classify_line(masked_line)
            if shape is None:
                continue
            original_line = raw_lines[lineno - 1] if lineno - 1 < len(raw_lines) else masked_line
            sites.append(
                {
                    "package": pkg_id,
                    "file": str(py_file.relative_to(pkg_root)),
                    "line": lineno,
                    "shape": shape,
                    "text": original_line.strip()[:200],
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
    # shape with few real occurrences doesn't silently shrink the total
    # below the precommitted minimum.
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
    parser.add_argument("--seed", type=int, default=20260923)
    parser.add_argument("--total", type=int, default=105)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=float, default=60.0)
    args = parser.parse_args(argv)

    corpus = json.loads(CORPUS_PATH.read_text(encoding="utf-8"))
    repos_by_id = {r["id"]: r for r in corpus["repositories"]}

    all_sites: list[dict[str, object]] = []
    tokenize_failures: list[str] = []
    import tempfile

    with tempfile.TemporaryDirectory(prefix="dispatch-audit-sites-py-") as tmp:
        work_root = Path(tmp)
        for pkg_id in PYTHON_REPO_IDS:
            manifest = repos_by_id[pkg_id]
            archive = acquire_artifact(
                manifest["artifact"],
                args.cache_dir,
                offline=args.offline,
                timeout_seconds=args.timeout_seconds,
            )
            pkg_root = extract_archive(
                archive, work_root / pkg_id, manifest["artifact"]["root"]
            )
            all_sites.extend(collect_sites(pkg_root, pkg_id, tokenize_failures))

    if not all_sites:
        raise SystemExit("no call sites found; cache extraction likely failed")

    selected = stratified_sample(all_sites, args.total, args.seed)
    document = {
        "schema_version": 1,
        "seed": args.seed,
        "requested_total": args.total,
        "selected_total": len(selected),
        "total_sites_discovered": len(all_sites),
        "packages": list(PYTHON_REPO_IDS),
        "masking": {
            "method": "tokenize-based STRING/COMMENT span masking before line "
            "classification (see mask_strings_and_comments); a whole-pool "
            "check before this was added found 33% of matched lines fell "
            "inside a string/comment token",
            "tokenize_failure_count": len(tokenize_failures),
            "tokenize_failure_files": tokenize_failures,
        },
        "shape_counts_discovered": {
            shape: sum(1 for s in all_sites if s["shape"] == shape)
            for shape, _ in SHAPE_PATTERNS
        },
        "shape_counts_selected": {
            shape: sum(1 for s in selected if s["shape"] == shape)
            for shape, _ in SHAPE_PATTERNS
        },
        "package_counts_discovered": {
            pkg: sum(1 for s in all_sites if s["package"] == pkg) for pkg in PYTHON_REPO_IDS
        },
        "package_counts_selected": {
            pkg: sum(1 for s in selected if s["package"] == pkg) for pkg in PYTHON_REPO_IDS
        },
        "sites": selected,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {len(selected)} sites to {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
