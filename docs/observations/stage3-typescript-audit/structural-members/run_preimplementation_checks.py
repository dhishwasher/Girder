#!/usr/bin/env python3
"""Preimplementation runtime checks for the TypeScript structural member
identity policy v1. Runs Node only (no Girder, no Cargo): every identity
contract file is executed for its inline assertions, every validation fixture
and the frozen original fixture run under `node --test`, and frozen input
hashes are recorded. Writes preimplementation-checks.json next to this file."""
import hashlib, json, pathlib, subprocess, sys

REPO = pathlib.Path(__file__).resolve().parents[4]
OUT = pathlib.Path(__file__).resolve().parent / "preimplementation-checks.json"
IDENT = REPO / "fixtures/typescript-structural-member-corpus/v1"
VALID = REPO / "fixtures/typescript-structural-validation/v1"
ORIGINAL = REPO / "fixtures/dispatch-corpus/typescript/structural-object-literal"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(args, cwd):
    proc = subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=120)
    return proc.returncode, proc.stdout + proc.stderr


def tap_counts(text):
    counts = {}
    for line in text.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[0] == "#" and parts[1] in ("tests", "pass", "fail", "skipped"):
            counts[parts[1]] = int(parts[2])
    return counts


def check_escaped_site(path, site):
    """Byte-level check that a marked key really is an escaped identifier:
    the line must hold the marker, a space, then the spelling whose bytes
    include ASCII backslash (0x5c) followed by `u` and four hex digits, and
    decoding that escape must give the expected runtime key."""
    lines = path.read_bytes().split(b"\n")
    line = lines[site["line"] - 1] if site["line"] <= len(lines) else b""
    marker = site["marker"].encode()
    spelling = site["source_spelling"].encode()
    expected = marker + b" " + spelling + b":"
    index = spelling.find(b"\x5cu")
    hex_digits = spelling[index + 2:index + 6] if index >= 0 else b""
    decoded = None
    if index >= 0 and len(hex_digits) == 4 and all(c in b"0123456789abcdefABCDEF" for c in hex_digits):
        decoded = (spelling[:index] + chr(int(hex_digits, 16)).encode() + spelling[index + 6:]).decode()
    result = {
        "marker": site["marker"],
        "line": site["line"],
        "line_contains_marker_and_spelling": expected in line,
        "backslash_u_byte_offset_in_line": line.find(b"\x5cu"),
        "spelling_bytes_hex": spelling.hex(),
        "decoded_key": decoded,
        "decoded_matches_expected": decoded == site["decoded_key"],
    }
    result["passed"] = (
        result["line_contains_marker_and_spelling"]
        and result["backslash_u_byte_offset_in_line"] >= 0
        and result["decoded_matches_expected"]
    )
    return result


def main():
    record = {
        "node_version": run(["node", "--version"], REPO)[1].strip(),
        "frozen_inputs": {
            "docs/dispatch-corpus.json": sha(REPO / "docs/dispatch-corpus.json"),
            "fixtures/dispatch-corpus/typescript/structural-object-literal/app.test.ts": sha(ORIGINAL / "app.test.ts"),
        },
        "identity_corpus": [],
        "validation_fixtures": [],
    }
    manifest = json.loads((IDENT / "manifest.json").read_text())
    ok = True
    for case in manifest["cases"]:
        path = IDENT / case["file"]
        entry = {"file": case["file"], "sha256_matches_manifest": sha(path) == case["sha256"]}
        mode = manifest["runtime"][case["file"]]
        if mode == "executed":
            code, text = run(["node", case["file"]], IDENT)
            entry.update(exit=code, passed=code == 0)
            ok &= code == 0
        else:
            entry.update(skipped=mode)
        ok &= entry["sha256_matches_manifest"]
        text = path.read_text()
        markers = [m["marker"] for m in case.get("members", [])]
        markers += [d["marker"] for d in case.get("declaration_only", [])]
        entry["markers_unique"] = all(text.count(m) == 1 for m in markers)
        boundaries = case.get("refused_subtree_boundaries", [])
        entry["boundary_markers_unique"] = all(
            text.count(b["subtree_marker"]) == 1
            and all(text.count(c["marker"]) == 1 for c in b["calls_inside"])
            for b in boundaries
        )
        # Boundary pins need extractor output; they must never claim a check now.
        entry["boundaries_postimplementation_only"] = all(b["checked_now"] is False for b in boundaries)
        entry["refused_subtree_boundaries_pinned"] = len(boundaries)
        if case.get("escaped_key_sites"):
            entry["escaped_key_sites"] = [check_escaped_site(path, site) for site in case["escaped_key_sites"]]
            ok &= all(site["passed"] for site in entry["escaped_key_sites"])
        ok &= entry["markers_unique"] and entry["boundary_markers_unique"]
        ok &= entry["boundaries_postimplementation_only"]
        record["identity_corpus"].append(entry)
    vmanifest = json.loads((VALID / "manifest.json").read_text())
    for fixture in sorted({c["fixture_dir"] for c in vmanifest["cases"]}):
        code, text = run(["node", "--test", "app.test.ts"], VALID / fixture)
        expected = {c["fixture_sha256"] for c in vmanifest["cases"] if c["fixture_dir"] == fixture}
        entry = {"fixture_dir": fixture, "exit": code, "tap": tap_counts(text),
                 "sha256_matches_manifest": expected == {sha(VALID / fixture / "app.test.ts")}}
        ok &= code == 0 and entry["sha256_matches_manifest"]
        record["validation_fixtures"].append(entry)
    code, text = run(["node", "--test", "app.test.ts"], ORIGINAL)
    record["original_fixture_runtime"] = {"exit": code, "tap": tap_counts(text)}
    ok &= code == 0
    record["frozen_inputs_match_manifest"] = record["frozen_inputs"] == vmanifest["frozen_inputs"]
    ok &= record["frozen_inputs_match_manifest"]
    record["postimplementation_requirements_not_checked"] = {
        "refused_subtree_boundaries": sum(e["refused_subtree_boundaries_pinned"] for e in record["identity_corpus"]),
        "identity_contracts": "every expected_identity / null and declaration-only span (needs extractor output)",
        "validation_case_scores": "every validation case (needs a Girder build and a separate validation scorer)",
    }
    record["all_passed"] = ok
    OUT.write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps({"all_passed": ok, "node": record["node_version"]}))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
