#!/usr/bin/env python3
"""Preimplementation checks for the TypeScript relative ESM named-import proof
policy v1. Node only: no Girder binary and no Cargo. Verifies every pinned
file hash and the frozen inputs, requires exactly one `/* claim */` marker per
case, checks the escaped-callee bytes, and runs each runnable case with
`node --test`. Writes preimplementation-checks.json next to this file and exits
nonzero on any failure."""
import hashlib, json, pathlib, subprocess, sys

REPO = pathlib.Path(__file__).resolve().parents[4]
ROOT = REPO / "fixtures/typescript-esm-import-proof/v1"
OUT = pathlib.Path(__file__).resolve().parent / "preimplementation-checks.json"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    manifest = json.loads((ROOT / "manifest.json").read_text())
    record = {"node_version": subprocess.run(["node", "--version"], capture_output=True, text=True).stdout.strip(),
              "manifest_sha256": sha(ROOT / "manifest.json"), "frozen_inputs": {}, "cases": []}
    ok = True
    for rel, pinned in manifest["frozen_inputs"].items():
        actual = sha(REPO / rel)
        record["frozen_inputs"][rel] = {"sha256": actual, "matches": actual == pinned}
        ok &= actual == pinned
    on_disk = {p.name for p in ROOT.iterdir() if p.is_dir()}
    record["case_dirs_match_manifest"] = on_disk == {c["id"] for c in manifest["cases"]}
    ok &= record["case_dirs_match_manifest"]
    for case in manifest["cases"]:
        base = ROOT / case["id"]
        files = {str(p.relative_to(base)) for p in base.rglob("*") if p.is_file()}
        entry = {"id": case["id"],
                 "file_set_matches": files == set(case["files"]),
                 "hashes_match": all(sha(base / f) == h for f, h in case["files"].items() if (base / f).exists()),
                 "single_claim_marker": sum((base / f).read_bytes().count(manifest["marker"].encode()) for f in files) == 1
                 and (base / case["importer"]).read_bytes().count(manifest["marker"].encode()) == 1}
        if case["expected_class"] == "must":
            target = (base / case["expected_target"]["file"]).read_bytes()
            entry["target_marker_unique"] = target.count(case["expected_target"]["marker"].encode()) == 1
            ok &= entry["target_marker_unique"]
        if case["id"] == "escaped-callee":
            text = (base / case["importer"]).read_bytes()
            entry["escaped_callee_bytes"] = b"/* claim */ t\x5cu0061rget(" in text
            ok &= entry["escaped_callee_bytes"]
        if case["runtime"]["mode"] == "node-test":
            proc = subprocess.run(["node", "--test", case["runtime"]["test_file"]], cwd=base,
                                  capture_output=True, text=True, timeout=120)
            tap = {k: int(v) for line in (proc.stdout + proc.stderr).splitlines()
                   if len(parts := line.split()) == 3 and parts[0] == "#" and parts[1] in ("tests", "pass", "fail")
                   for k, v in [(parts[1], parts[2])]}
            entry["runtime"] = {"exit": proc.returncode, "tap": tap,
                                "passed": proc.returncode == 0 and tap.get("fail") == 0 and tap.get("pass", 0) >= 1}
            ok &= entry["runtime"]["passed"]
        else:
            entry["runtime"] = {"skipped": case["runtime"]["reason"]}
        ok &= entry["file_set_matches"] and entry["hashes_match"] and entry["single_claim_marker"]
        record["cases"].append(entry)
    record["counts"] = {"cases": len(record["cases"]),
                        "runtime_passed": sum(1 for c in record["cases"] if c["runtime"].get("passed")),
                        "runtime_skipped": sum(1 for c in record["cases"] if "skipped" in c["runtime"])}
    record["all_passed"] = bool(ok)
    OUT.write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps({"all_passed": record["all_passed"], **record["counts"]}))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
