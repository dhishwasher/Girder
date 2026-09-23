#!/usr/bin/env python3
"""Per-site resolver-gate profile for the Stage 3 Python audit's conservative cells.

Mirrors tools/dispatch_audit_gate_profile.py's role for Rust: reads
Girder's own call_evidence_v1 output (already generated) and the frozen
Python audit's scored results, and reports, for every conservative-cell
site, which of the Python extractor's gates block it -- as the basis for
choosing the smallest resolver change that could move at least one of
them, BEFORE any Must-proof design work starts. No build is needed.

Python's Must-proof path (crates/aether-builder/src/mapper/claims.rs) is
simpler than Rust's (no macro_owners/trusted-assert-macro machinery), but
shares the same core gates: transformed_scope (here: ANY decorator or
decorated_definition ANYWHERE in the file -- Python's own transformed_scope
rule doesn't narrow this the way Rust's cfg-attribute check does),
duplicate_paths, parse_error, and an identifier-only filter (the callee
must be a bare name -- `function.kind() == "identifier"` in claims.rs --
so method_call/qualified_attribute_call shapes are categorically excluded
regardless of anything else, mirroring the exact same "92% of Rust's
conservative cells" cause).
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

try:
    from tools.dispatch_audit_scorer_python import extract_call_claims
except ModuleNotFoundError:
    from dispatch_audit_scorer_python import extract_call_claims

GAP_REASONS = {"duplicate-semantic-path", "parse-error"}


def profile_site(site: dict, pkg_root: Path, claims: list[dict]) -> dict:
    file = site["file"]
    caller = site.get("observed_caller")
    file_claims = [c for c in claims if c["file"] == file]
    src_bytes = (pkg_root / file).read_bytes()
    src_text = src_bytes.decode("utf-8", "replace")

    decorator_claims = [c for c in file_claims if c["reason"] == "unexpanded-macro-or-decorator"]
    # transformed_scope trips on ANY decorator/decorated_definition
    # anywhere in the file (crates/aether-builder/src/mapper/claims.rs's
    # `transformed_scope` closure), not just ones near this specific call
    # -- a file-wide gate, checked here by whether the file has ANY
    # unexpanded-macro-or-decorator claim at all (Python's decorator node
    # kind is the only thing that produces this reason for Python files).
    transformed_scope_trips = bool(decorator_claims)
    decorator_texts = []
    for c in decorator_claims[:5]:
        text = src_bytes[c["start_byte"] : c["end_byte"]].decode("utf-8", "replace")
        decorator_texts.append(text.strip().splitlines()[0][:60] if text.strip() else "")

    module_gap_reasons = sorted({c["reason"] for c in file_claims if c["reason"] in GAP_REASONS})

    # The callee name -- read starting at the site's own `byte_offset`
    # (the scorer's byte-precise match position, already computed against
    # the masked text the same way the selector chose this site), not
    # guessed from the whole line's text. Two earlier versions of this
    # function got this wrong in opposite ways: taking the FIRST
    # identifier-paren match on the line broke `MyModel(x=1234)
    # .model_dump_json()` (yielded the constructor, not the actual call);
    # taking the LAST match broke `deprecated_from_orm(State,
    # SimpleNamespace(...))` (a NESTED call inside the real one's
    # arguments matches later in the text, but isn't the sampled site).
    # Anchoring on `byte_offset` is correct for both shapes: `plain_call`'s
    # pattern starts at the identifier itself; `method_call`'s starts at
    # the `.`.
    line_text = site["text"]
    byte_offset = site.get("byte_offset")
    callee_name = None
    if byte_offset is not None:
        window = src_bytes[byte_offset : byte_offset + 200].decode("utf-8", "replace")
        m = re.match(r"\.?\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(", window)
        if m:
            callee_name = m.group(1)
    if callee_name is None:
        # Fallback when byte_offset isn't available (e.g. a direct unit
        # test constructing a site by hand): first match on the line, the
        # same simplification the selector's own classify_line uses.
        callee_match = re.search(r"([A-Za-z_][A-Za-z0-9_]*)\s*\(", line_text)
        callee_name = callee_match.group(1) if callee_match else None
    # claims.rs's `top` collection loop filters on
    # `"function_item" | "function_definition" | "function_declaration"`
    # ONLY -- `class_definition` is NOT included, so a same-file class
    # construction call (`MetaclassArgumentsWithDefault(i=None)`) can
    # NEVER be proven Must today, regardless of transformed_scope/
    # duplicate_paths/anything else. This is a distinct, third gate from
    # "needs cross-file resolution" -- an earlier version of this tool
    # folded class definitions into the same "same_file_top_level_def"
    # check as functions, which wrongly implied narrowing
    # transformed_scope alone would unblock a same-file class-construction
    # site; it would not, until class_definition is also added to
    # claims.rs's own `top` collection.
    fn_pattern = (
        rf"^(?:async\s+)?def\s+{re.escape(callee_name)}\s*\(" if callee_name else None
    )
    class_pattern = rf"^class\s+{re.escape(callee_name)}\s*[(:]" if callee_name else None
    same_file_top_level_def = bool(
        fn_pattern and re.search(fn_pattern, src_text, re.MULTILINE)
    )
    same_file_top_level_class_not_collected = bool(
        class_pattern and re.search(class_pattern, src_text, re.MULTILINE)
    )
    # A second same-file top-level def of the same name means claims.rs's
    # own uniqueness check (`candidates.len() != 1`) would ALSO block it,
    # independent of transformed_scope/duplicate_paths. Functions only,
    # matching what `top` actually collects.
    same_file_def_count = (
        len(re.findall(fn_pattern, src_text, re.MULTILINE)) if fn_pattern else 0
    )

    # self/cls dispatch: the receiver immediately before the callee's own
    # `.name(` -- searched anywhere on the line (not just at its start),
    # so `schema = self._apply_single_annotation(...)` and
    # `... or cls.is_true(value)` are both caught; an earlier version only
    # matched a receiver at column 0, missing both.
    is_self_or_cls_dispatch = bool(
        callee_name and re.search(rf"\b(?:self|cls)\.{re.escape(callee_name)}\s*\(", line_text)
    )

    return {
        "package": site["package"],
        "file": file,
        "line": site["line"],
        "shape": site["shape"],
        "true_class": site["true_class"],
        "caller": caller,
        "identifier_filter_pass": site["shape"] == "plain_call",
        "self_or_cls_method_dispatch": is_self_or_cls_dispatch,
        "transformed_scope_trips": transformed_scope_trips,
        "decorator_examples_in_file": decorator_texts,
        "duplicate_paths": "duplicate-semantic-path" in module_gap_reasons,
        "parse_error": "parse-error" in module_gap_reasons,
        "callee_name": callee_name,
        "same_file_top_level_def_exists": same_file_top_level_def,
        "same_file_top_level_def_count": same_file_def_count,
        "same_file_top_level_class_not_collected": same_file_top_level_class_not_collected,
        "would_need_cross_file_resolution": site["shape"] == "plain_call"
        and not same_file_top_level_def
        and not same_file_top_level_class_not_collected,
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument(
        "--labels",
        type=Path,
        required=True,
        help="audit-sites-labeled.json -- scored results don't carry `text`",
    )
    parser.add_argument("--package-root", action="append", required=True)
    parser.add_argument("--inspect", action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)

    pkg_roots = {k: Path(v) for k, v in (e.split("=", 1) for e in args.package_root)}
    inspect_paths = dict(e.split("=", 1) for e in args.inspect)
    claims_by_pkg = {p: extract_call_claims(Path(ip)) for p, ip in inspect_paths.items()}

    labels_doc = json.loads(args.labels.read_text())
    text_by_index = {i: s["text"] for i, s in enumerate(labels_doc["sites"])}

    doc = json.loads(args.results.read_text())
    conservative = [r for r in doc["results"] if r.get("cell") == "conservative"]
    for r in conservative:
        r["text"] = text_by_index[r["index"]]

    rows = [
        profile_site(site, pkg_roots[site["package"]], claims_by_pkg[site["package"]])
        for site in conservative
    ]
    args.output.write_text(json.dumps({"schema_version": 1, "rows": rows}, indent=2) + "\n")
    print(f"wrote {args.output}: {len(rows)} conservative sites profiled")
    return 0


if __name__ == "__main__":
    sys.exit(main())
