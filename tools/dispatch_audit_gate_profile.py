#!/usr/bin/env python3
"""Per-site resolver-gate profile for the Stage 3 Rust audit's conservative cells.

Reads Girder's own `call_evidence_v1` output (via `extract_call_claims`, the
same parser `dispatch_audit_scorer.py` uses) and the frozen audit's scored
results, and reports, for every conservative-cell site, which of the
extractor's gates block it: the identifier-only Must-proof filter, whether
the callee is same-file/top-level, `transformed_scope` (including inner
`#![...]` attributes, which an earlier ad hoc check missed), `duplicate_paths`,
parse errors, `macro_owners` for the site's specific caller function, and
whether the site sits inside a trusted assert macro's token tree. No build is
needed -- everything here is read from already-generated `girder inspect
--json` output and the crates' source text.

This does not re-run or alter the frozen audit; it explains site-by-site why
each conservative cell is conservative, as the basis for choosing the
smallest resolver change that could move at least one of them.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from tools.dispatch_audit_scorer import extract_call_claims

TEST_ATTRS = {"#[test]", "#[tokio::test]"}
TRUSTED_ASSERT_MACROS = {
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
}
GAP_REASONS = {
    "duplicate-semantic-path",
    "parse-error",
}


def attribute_or_macro_text(source_bytes: bytes, claim: dict) -> tuple[str, str]:
    """(kind, first line) for an 'unexpanded-macro-or-decorator' claim's span.

    kind is 'attr' for a `#[...]` or `#![...]` attribute (inner attributes
    start with `#!`, not `#[` -- an earlier ad hoc check treated them as
    macro invocations and silently dropped them), 'macro' otherwise.
    """
    text = source_bytes[claim["start_byte"] : claim["end_byte"]].decode(
        "utf-8", "replace"
    )
    stripped = text.strip()
    first_line = stripped.splitlines()[0] if stripped else ""
    if first_line.startswith("#[") or first_line.startswith("#!"):
        return "attr", first_line
    return "macro", first_line


def profile_site(
    site: dict,
    crate_root: Path,
    claims: list[dict],
) -> dict:
    file = site["file"]
    caller = site.get("observed_caller")
    file_claims = [c for c in claims if c["file"] == file]
    src_bytes = (crate_root / file).read_bytes()

    decorator_claims = [c for c in file_claims if c["reason"] == "unexpanded-macro-or-decorator"]
    attrs: set[str] = set()
    caller_macros: list[str] = []
    for c in decorator_claims:
        kind, first_line = attribute_or_macro_text(src_bytes, c)
        if kind == "attr":
            name = first_line.lstrip("#!").lstrip("[").split("(")[0].split("=")[0].strip(" ]")
            if first_line not in TEST_ATTRS:
                attrs.add(name)
        else:
            if c["caller"] == caller:
                caller_macros.append(first_line[:60])

    module_gap_reasons = sorted(
        {c["reason"] for c in file_claims if c["reason"] in GAP_REASONS}
    )

    non_test_attrs = sorted(attrs)
    cfg_like = sorted(a for a in non_test_attrs if a in ("cfg", "cfg_attr", "macro_use"))

    untrusted_caller_macros = [
        m for m in caller_macros if m.split("!")[0].split("(")[0].strip() not in TRUSTED_ASSERT_MACROS
    ]

    inside_trusted_assert = any(
        c["reason"] == "proven-call-inside-trusted-assertion-macro"
        and c["start_byte"] <= site.get("byte_offset", -1) < c["end_byte"]
        for c in file_claims
    ) or any(
        attribute_or_macro_text(src_bytes, c)[1].split("!")[0].strip() in TRUSTED_ASSERT_MACROS
        and c["start_byte"] <= site.get("byte_offset", -1) < c["end_byte"]
        for c in file_claims
        if c["reason"] == "unexpanded-macro-or-decorator"
    )

    return {
        "crate": site["crate"],
        "file": file,
        "line": site["line"],
        "shape": site["shape"],
        "true_class": site["true_class"],
        "caller": caller,
        "identifier_filter_pass": site["shape"] == "plain_call",
        "transformed_scope_all_attrs": non_test_attrs,
        "transformed_scope_cfg_like_attrs": cfg_like,
        "transformed_scope_trips_narrowly": bool(cfg_like),
        "transformed_scope_trips_currently": bool(non_test_attrs),
        "duplicate_paths": "duplicate-semantic-path" in module_gap_reasons,
        "parse_error": "parse-error" in module_gap_reasons,
        "macro_owners_blocks_caller": bool(untrusted_caller_macros),
        "untrusted_macros_in_caller": untrusted_caller_macros,
        "inside_trusted_assert_macro": inside_trusted_assert,
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, required=True, help="audit-scored-results-v3.json")
    parser.add_argument("--crate-root", action="append", required=True, help="crate_id=/path, repeatable")
    parser.add_argument("--inspect", action="append", required=True, help="crate_id=/path/to/inspect.json, repeatable")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)

    crate_roots = dict(entry.split("=", 1) for entry in args.crate_root)
    crate_roots = {k: Path(v) for k, v in crate_roots.items()}
    inspect_paths = dict(entry.split("=", 1) for entry in args.inspect)
    claims_by_crate = {c: extract_call_claims(Path(p)) for c, p in inspect_paths.items()}

    doc = json.loads(args.results.read_text())
    conservative = [r for r in doc["results"] if r.get("cell") == "conservative"]

    rows = [
        profile_site(site, crate_roots[site["crate"]], claims_by_crate[site["crate"]])
        for site in conservative
    ]
    args.output.write_text(json.dumps({"schema_version": 1, "rows": rows}, indent=2) + "\n")
    print(f"wrote {args.output}: {len(rows)} conservative sites profiled")
    return 0


if __name__ == "__main__":
    sys.exit(main())
