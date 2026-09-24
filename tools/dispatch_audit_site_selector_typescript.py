#!/usr/bin/env python3
"""Stage 3 TypeScript audit: deterministic, Girder-independent call-site sampler.

Mirrors tools/dispatch_audit_site_selector_python.py's methodology, adapted
to TypeScript-appropriate call shapes and masking. Per docs/roadmap.md's
frozen Stage 3 policy: site enumeration must not use Girder's own parser (a
site its tree-sitter grammar fails to see would never enter a
Girder-derived sample), so this uses a plain, hand-written character scanner
over the three already-cached, sha256-verified TypeScript repositories
(docs/stage3-typescript-corpus.json: typescript-6.0.3, zod-3.23.8,
date-fns-4.1.0 -- deliberately a SEPARATE manifest from
docs/core-representative-corpus.json, see that file's own "note" field for
why: adding these entries there directly broke that file's own
already-gated validate_manifest() test suite), not `girder query`.

Masking has no Python `tokenize`-equivalent for TypeScript/JavaScript in the
standard library, and this repo's Python tooling has no third-party
dependency infrastructure (no requirements.txt/pyproject.toml, confirmed by
inspection before choosing this approach over pip-installing
tree-sitter-typescript). `mask_ts_source` is therefore a hand-written,
single-pass character scanner -- independent of Girder's own parser, same
as the regex approach above it, and the same Girder-independence property
Python's own `tokenize`-based masker has for a different reason (stdlib,
not third-party). It masks line comments, block comments, string/template
literal text, and regex literals, while leaving `${...}` template
interpolation expressions unmasked (they are real code that may contain a
real call site, e.g. `` `Hello ${getName()}` ``).

Shapes, chosen for TypeScript's own dispatch-ambiguity landscape (no
operator overloading, unlike Rust/Python, so there is no `operator_dunder`
analogue; optional chaining is a TypeScript/JavaScript-specific dispatch
uncertainty neither other language has):
  decorator, dynamic_dispatch, qualified_attribute_call, method_call,
  optional_chaining_call, new_expression, plain_call.
"""
from __future__ import annotations

import argparse
import json
import os
import random
import re
import sys
import tarfile
import tempfile
from pathlib import Path, PurePosixPath
from typing import Sequence

try:
    from tools.core_representative_benchmark import (
        DEFAULT_CACHE,
        MAX_MEMBER_BYTES,
        MAX_UNCOMPRESSED_BYTES,
        acquire_artifact,
        safe_archive_name,
    )
except ModuleNotFoundError:
    from core_representative_benchmark import (
        DEFAULT_CACHE,
        MAX_MEMBER_BYTES,
        MAX_UNCOMPRESSED_BYTES,
        acquire_artifact,
        safe_archive_name,
    )

REPO_ROOT = Path(__file__).resolve().parents[1]
CORPUS_PATH = REPO_ROOT / "docs" / "stage3-typescript-corpus.json"
TYPESCRIPT_REPO_IDS = (
    "typescript-6.0.3",
    "zod-3.23.8",
    "date-fns-4.1.0",
    "class-validator-0.15.1",
)

# Applied to a single trimmed, MASKED source line. Order matters: more
# specific shapes are checked before the generic "plain_call" fallback so a
# line isn't double-counted under a broader pattern.
SHAPE_PATTERNS: list[tuple[str, re.Pattern[str]]] = [
    ("decorator", re.compile(r"^@\s*[A-Za-z_][A-Za-z0-9_.]*")),
    (
        "dynamic_dispatch",
        re.compile(
            r"\[[^\]]*\]\s*\("
            r"|\.(?:call|apply|bind)\s*\("
            r"|\bReflect\s*\.\s*(?:apply|construct)\s*\("
            r"|\bnew\s+Function\s*\("
        ),
    ),
    ("optional_chaining_call", re.compile(r"\?\.\s*\(|\?\.[A-Za-z_$][A-Za-z0-9_$]*\s*\(")),
    ("new_expression", re.compile(r"\bnew\s+[A-Za-z_$][A-Za-z0-9_$.]*\s*(?:<[^>]*>)?\s*\(")),
    (
        "qualified_attribute_call",
        re.compile(r"\b[A-Za-z_$][A-Za-z0-9_$]*(?:\.[A-Za-z_$][A-Za-z0-9_$]*){2,}\s*\("),
    ),
    ("method_call", re.compile(r"\.\s*[A-Za-z_$][A-Za-z0-9_$]*\s*(?:<[^>]*>)?\s*\(")),
    ("plain_call", re.compile(r"(?<![.\w$])[A-Za-z_$][A-Za-z0-9_$]*\s*(?:<[^>]*>)?\s*\(")),
]

KEYWORDS_NOT_CALLS = {
    # `super` and `import` are deliberately NOT here, unlike a first draft
    # of this set: `super(args)` (a parent-constructor call) and
    # `import("x")` (a dynamic import expression) are both real call
    # sites, not syntax this selector should reject. Static
    # `import { x } from 'y'` has no `identifier(` shape at all (no paren
    # directly follows "import" in that form), so this doesn't cause it
    # to be wrongly picked up either.
    "if", "while", "for", "function", "return", "switch", "else", "with",
    "catch", "do", "try", "finally", "throw", "export", "from",
    "as", "default", "extends", "implements", "this", "typeof",
    "instanceof", "in", "of", "yield", "await", "async", "const", "let",
    "var", "case", "break", "continue", "delete", "void", "static",
    "public", "private", "protected", "readonly", "abstract", "declare",
    "module", "namespace", "get", "set", "class", "interface", "type",
    "enum", "new",
}

# Last-significant-token classes that make a following `/` a DIVISION
# operator rather than the start of a regex literal. Anything else (start
# of expression, after an operator/punctuation, after one of these
# keywords) is treated as a regex start. This is the same context-based
# heuristic real JS tokenizers use; it is an approximation, empirically
# checked against the three pinned repositories (see the methodology doc),
# not assumed correct.
REGEX_CONTEXT_KEYWORDS = {
    "return", "typeof", "instanceof", "in", "of", "new", "delete", "void",
    "throw", "yield", "case", "do", "else", "await", "default",
}
IDENT_CHAR = re.compile(r"[A-Za-z0-9_$]")


def _is_ident_char(ch: str) -> bool:
    return bool(IDENT_CHAR.match(ch))


def mask_ts_source(text: str) -> str:
    """Replace comment and string/template-literal-text spans with spaces,
    preserving every character position so line/column numbers stay
    correct. Template interpolation expressions (`${...}`) are NOT masked
    -- they are real code. Regex literals are masked in full (their content
    is pattern syntax, not TypeScript call syntax, and can otherwise
    produce false shape matches, e.g. `/foo(bar)/` looking like a call).

    Always returns a masked string -- there is no failure mode analogous to
    `tokenize.TokenError`, since this scanner has no ambiguous end state: an
    unterminated string/template/regex/comment at EOF is masked to the end
    of the file rather than raising.
    """
    out = list(text)
    n = len(text)
    i = 0
    # Stack of frames: "code" (implicit base, never popped) or
    # ("template",) or ("interp", brace_depth).
    stack: list[tuple] = [("code",)]
    last_significant = ""  # last non-whitespace char seen in code/interp mode
    last_word = ""  # last identifier/keyword run seen in code/interp mode

    def in_code_like() -> bool:
        return stack[-1][0] in ("code", "interp")

    while i < n:
        ch = text[i]
        frame = stack[-1]

        if frame[0] == "template":
            if ch == "\\" and i + 1 < n:
                out[i] = " "
                out[i + 1] = " "
                i += 2
                continue
            if ch == "`":
                stack.pop()
                i += 1
                continue
            if ch == "$" and i + 1 < n and text[i + 1] == "{":
                stack.append(("interp", 0))
                i += 2
                continue
            out[i] = " " if ch != "\n" else "\n"
            i += 1
            continue

        # frame is "code" or ("interp", depth)
        if ch == "/" and i + 1 < n and text[i + 1] == "/":
            j = i
            while j < n and text[j] != "\n":
                out[j] = " "
                j += 1
            i = j
            continue
        if ch == "/" and i + 1 < n and text[i + 1] == "*":
            j = i
            while j < n - 1 and not (text[j] == "*" and text[j + 1] == "/"):
                out[j] = " " if text[j] != "\n" else "\n"
                j += 1
            if j < n - 1:
                out[j] = " "
                out[j + 1] = " "
                j += 2
            else:
                out[j] = " " if j < n and text[j] != "\n" else out[j]
                j = n
            i = j
            continue
        if ch in ("'", '"'):
            quote = ch
            j = i + 1
            while j < n and text[j] != quote:
                if text[j] == "\\" and j + 1 < n:
                    j += 2
                    continue
                j += 1
            end = min(j + 1, n)
            for k in range(i, end):
                out[k] = " " if text[k] != "\n" else "\n"
            i = end
            last_significant = quote
            last_word = ""
            continue
        if ch == "`":
            out[i] = " "
            stack.append(("template",))
            i += 1
            last_significant = "`"
            last_word = ""
            continue
        if ch == "/":
            # Regex-vs-division disambiguation.
            is_regex = True
            if last_significant and (
                _is_ident_char(last_significant)
                or last_significant in ")]"
            ):
                is_regex = last_word in REGEX_CONTEXT_KEYWORDS
            if is_regex:
                j = i + 1
                in_class = False
                while j < n and text[j] != "\n":
                    if text[j] == "\\" and j + 1 < n:
                        j += 2
                        continue
                    if text[j] == "[":
                        in_class = True
                    elif text[j] == "]":
                        in_class = False
                    elif text[j] == "/" and not in_class:
                        j += 1
                        break
                    j += 1
                while j < n and text[j].isalpha():  # regex flags
                    j += 1
                end = min(j, n)
                for k in range(i, end):
                    out[k] = " " if text[k] != "\n" else "\n"
                i = end
                last_significant = "/"
                last_word = ""
                continue
            # division: fall through as an ordinary character
        if ch == "{" and frame[0] == "interp":
            stack[-1] = ("interp", frame[1] + 1)
            i += 1
            last_significant = "{"
            last_word = ""
            continue
        if ch == "}" and frame[0] == "interp":
            if frame[1] > 0:
                stack[-1] = ("interp", frame[1] - 1)
            else:
                stack.pop()  # back to the enclosing "template" frame
            i += 1
            last_significant = "}"
            last_word = ""
            continue

        # Ordinary code character: track last-significant-token state.
        if not ch.isspace():
            if _is_ident_char(ch):
                if last_word and _is_ident_char(last_significant):
                    last_word += ch
                else:
                    last_word = ch
            else:
                last_word = ""
            last_significant = ch
        i += 1

    return "".join(out)


def classify_line_with_match(line: str) -> tuple[str, "re.Match[str]"] | None:
    """Like `classify_line`, but also returns the winning regex match --
    needed by the scorer to relocate the exact byte offset a site's shape
    was matched at, not just its name. `classify_line` is defined in terms
    of this function so the two can never disagree (the scorer's own
    relocation test re-classifies every frozen site and asserts the shape
    still matches, which only means something if both code paths share one
    implementation)."""
    stripped = line.strip()
    if not stripped:
        return None
    for shape, pattern in SHAPE_PATTERNS:
        for match in pattern.finditer(stripped):
            if shape == "plain_call":
                word = re.match(r"[A-Za-z_$][A-Za-z0-9_$]*", match.group())
                if word and word.group() in KEYWORDS_NOT_CALLS:
                    continue
            return shape, match
    return None


def classify_line(line: str) -> str | None:
    result = classify_line_with_match(line)
    return result[0] if result else None


def extract_archive_subset(
    archive_path: Path, destination: Path, expected_root: str, allowed_prefixes: tuple[str, ...]
) -> Path:
    """Like `core_representative_benchmark.extract_archive`, but only
    extracts members whose path (relative to `expected_root`) starts with
    one of `allowed_prefixes` -- needed because the upstream TypeScript
    repository's full tarball (84,334 members, mostly its own enormous
    conformance-test baseline corpus) exceeds
    `core_representative_benchmark.MAX_ARCHIVE_MEMBERS` (50,000) long
    before reaching the ~700 real compiler source files this audit
    actually wants. The archive itself is still the exact, unmodified,
    sha256-pinned upstream tarball (verified by `acquire_artifact` before
    this function ever runs) -- only the LOCAL extraction is scoped down,
    the same safety checks `preflight_archive` applies (path traversal,
    symlink/special-file rejection, size limits) are re-applied here to
    every member this function actually extracts, just without needing to
    enumerate and bound-check all 84,334 members up front to do it.
    """
    if destination.exists():
        raise RuntimeError(f"refusing to merge extraction into existing path: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=f".{destination.name}.", dir=destination.parent))
    os.chmod(temporary, 0o700)
    total_bytes = 0
    extracted = 0
    try:
        with tarfile.open(archive_path, mode="r:gz") as archive:
            for member in archive:
                if not safe_archive_name(member.name):
                    raise RuntimeError(f"unsafe archive member path: {member.name!r}")
                normalized = member.name.rstrip("/")
                parts = tuple(PurePosixPath(normalized).parts)
                if parts[0] != expected_root:
                    raise RuntimeError(
                        f"archive member is outside exact root {expected_root!r}: {member.name!r}"
                    )
                relative = "/".join(parts[1:])
                if not relative.startswith(allowed_prefixes) and relative not in allowed_prefixes:
                    continue
                if not (member.isdir() or member.isreg()):
                    raise RuntimeError(f"unsupported archive member type: {member.name!r}")
                if member.isdir():
                    continue
                if member.size < 0 or member.size > MAX_MEMBER_BYTES:
                    raise RuntimeError(f"archive member size limit exceeded: {member.name!r}")
                total_bytes += member.size
                if total_bytes > MAX_UNCOMPRESSED_BYTES:
                    raise RuntimeError("archive uncompressed-byte limit exceeded")
                target = temporary / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                extracted_file = archive.extractfile(member)
                if extracted_file is None:
                    raise RuntimeError(f"could not read archive member: {member.name!r}")
                with extracted_file, open(target, "wb") as out:
                    out.write(extracted_file.read())
                extracted += 1
        if extracted == 0:
            raise RuntimeError(
                f"no archive members matched {allowed_prefixes!r} under root {expected_root!r}"
            )
        os.rename(temporary, destination)
    except BaseException:
        import shutil

        shutil.rmtree(temporary, ignore_errors=True)
        raise
    return destination


def collect_sites(repo_root: Path, repo_id: str) -> list[dict[str, object]]:
    sites = []
    for ts_file in sorted(repo_root.rglob("*.ts")):
        try:
            raw_text = ts_file.read_text(encoding="utf-8", errors="strict")
        except (UnicodeDecodeError, OSError):
            continue
        masked_text = mask_ts_source(raw_text)
        masked_lines = masked_text.splitlines()
        raw_lines = raw_text.splitlines()
        for lineno, (raw_line, masked_line) in enumerate(zip(raw_lines, masked_lines), start=1):
            shape = classify_line(masked_line)
            if shape is None:
                continue
            sites.append(
                {
                    "package": repo_id,
                    "file": str(ts_file.relative_to(repo_root)),
                    "line": lineno,
                    "shape": shape,
                    "text": raw_line.strip(),
                }
            )
    return sites


def stratified_sample(
    sites: Sequence[dict[str, object]], total: int, seed: int
) -> list[dict[str, object]]:
    by_shape: dict[str, list[dict[str, object]]] = {}
    for site in sites:
        by_shape.setdefault(site["shape"], []).append(site)
    shapes = sorted(by_shape)
    rng = random.Random(seed)
    for shape in shapes:
        rng.shuffle(by_shape[shape])
    per_shape = max(1, total // len(shapes)) if shapes else 0
    selected: list[dict[str, object]] = []
    remaining: dict[str, list[dict[str, object]]] = {}
    for shape in shapes:
        take = by_shape[shape][:per_shape]
        selected.extend(take)
        remaining[shape] = by_shape[shape][per_shape:]
    idx = 0
    shape_cycle = [s for s in shapes if remaining[s]]
    while len(selected) < total and shape_cycle:
        shape = shape_cycle[idx % len(shape_cycle)]
        if remaining[shape]:
            selected.append(remaining[shape].pop(0))
        if not remaining[shape]:
            shape_cycle.remove(shape)
            if shape_cycle:
                idx %= len(shape_cycle)
            continue
        idx += 1
    selected.sort(key=lambda s: (s["package"], s["file"], s["line"]))
    return selected[:total]


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--total", type=int, default=105)
    parser.add_argument("--seed", type=int, default=20260924)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cache-dir", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--timeout-seconds", type=float, default=120.0)
    args = parser.parse_args(argv)

    with open(CORPUS_PATH) as f:
        corpus = json.load(f)
    repos = {r["id"]: r for r in corpus["repositories"]}

    import tempfile

    all_sites: list[dict[str, object]] = []
    package_counts_pool: dict[str, int] = {}
    with tempfile.TemporaryDirectory(prefix="dispatch-audit-sites-ts-") as tmp:
        work_root = Path(tmp)
        for repo_id in TYPESCRIPT_REPO_IDS:
            repo = repos[repo_id]
            archive = acquire_artifact(
                repo["artifact"],
                args.cache_dir,
                offline=args.offline,
                timeout_seconds=args.timeout_seconds,
            )
            # typescript-6.0.3's full upstream tarball (84,334 members) is
            # mostly its own conformance-test baseline corpus and exceeds
            # core_representative_benchmark.MAX_ARCHIVE_MEMBERS long before
            # reaching the ~700 real compiler .ts files this audit wants --
            # scoped to src/ only (see extract_archive_subset's own
            # docstring). zod and date-fns are small enough that the whole
            # repository is extracted, matching Rust/Python's own
            # whole-package precedent.
            prefixes = ("src", "LICENSE.txt", "package.json") if repo_id == "typescript-6.0.3" else ("",)
            root = extract_archive_subset(
                archive, work_root / repo_id, repo["artifact"]["root"], prefixes
            )
            sites = collect_sites(root, repo_id)
            package_counts_pool[repo_id] = len(sites)
            all_sites.extend(sites)

    shape_counts_pool: dict[str, int] = {}
    for site in all_sites:
        shape_counts_pool[site["shape"]] = shape_counts_pool.get(site["shape"], 0) + 1

    selected = stratified_sample(all_sites, args.total, args.seed)
    package_counts_selected: dict[str, int] = {}
    shape_counts_selected: dict[str, int] = {}
    for site in selected:
        package_counts_selected[site["package"]] = package_counts_selected.get(site["package"], 0) + 1
        shape_counts_selected[site["shape"]] = shape_counts_selected.get(site["shape"], 0) + 1

    output = {
        "schema_version": 1,
        "seed": args.seed,
        "pool_size": len(all_sites),
        "package_counts_pool": package_counts_pool,
        "shape_counts_pool": shape_counts_pool,
        "selected_count": len(selected),
        "package_counts_selected": package_counts_selected,
        "shape_counts_selected": shape_counts_selected,
        "sites": selected,
    }
    args.output.write_text(json.dumps(output, indent=2) + "\n", encoding="utf-8")
    print(
        f"wrote {args.output}: pool {len(all_sites)}, selected {len(selected)}, "
        f"packages {package_counts_selected}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
