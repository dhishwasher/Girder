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
    record["all_passed"] = ok
    OUT.write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps({"all_passed": ok, "node": record["node_version"]}))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
