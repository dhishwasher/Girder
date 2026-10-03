#!/usr/bin/env python3
"""Preimplementation checks for the TypeScript relative ESM named-import proof
policy v1. Node only: no Girder binary and no Cargo.

The script checks:
- every pinned file hash, and every pinned symlink's link text (symlinks are
  never followed when pinning);
- the frozen inputs;
- exactly one `/* claim */` marker per case;
- the raw specifier bytes of the spelling-hazard cases.

It runs each runnable case with `node --test`, passing any per-case Node
arguments, and replays the incremental sequence on a temporary copy, checking
Node's runtime outcome after every step. Results go to
preimplementation-checks.json next to this file, and the script exits nonzero
on any failure."""
import hashlib, json, os, pathlib, shutil, subprocess, sys, tempfile

REPO = pathlib.Path(__file__).resolve().parents[4]
ROOT = REPO / "fixtures/typescript-esm-import-proof/v1"
OUT = pathlib.Path(__file__).resolve().parent / "preimplementation-checks.json"

# Exact raw bytes that must appear in each spelling-hazard importer.
RAW_SPECIFIERS = {
    "percent-encoded-specifier": b"from './%61pp.ts'",
    "encoded-dot-segment": b"from './%2e%2e/app.ts'",
    "encoded-slash-specifier": b"from './sub%2Fapp.ts'",
    "backslash-specifier": b"from './sub\x5c\x5capp.ts'",
    "tab-escape-specifier": b"from './ap\x5ctp.ts'",
    "raw-tab-byte-specifier": b"from './ap\x09p.ts'",
    "trailing-space-specifier": b"from './app.ts '",
    "hex-escape-specifier": b"from './\x5cx61pp.ts'",
    "line-continuation-specifier": b"from './ap\x5c\np.ts'",
    "non-ascii-specifier": "from './äpp.ts'".encode(),
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pins(base):
    out = {}
    for dirpath, dirnames, filenames in os.walk(base, followlinks=False):
        for name in dirnames + filenames:
            path = pathlib.Path(dirpath) / name
            rel = str(path.relative_to(base))
            if path.is_symlink():
                out[rel] = {"symlink": os.readlink(path)}
            elif path.is_file():
                out[rel] = sha(path)
    return out


def node_test(cwd, test_file, args=()):
    proc = subprocess.run(["node", "--test", *args, test_file], cwd=cwd,
                          capture_output=True, text=True, timeout=120)
    tap = {}
    for line in (proc.stdout + proc.stderr).splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[0] == "#" and parts[1] in ("tests", "pass", "fail"):
            tap[parts[1]] = int(parts[2])
    passed = proc.returncode == 0 and tap.get("fail") == 0 and tap.get("pass", 0) >= 1
    return {"exit": proc.returncode, "tap": tap, "passed": passed}


def main():
    manifest = json.loads((ROOT / "manifest.json").read_text())
    marker = manifest["marker"].encode()
    record = {"node_version": subprocess.run(["node", "--version"], capture_output=True, text=True).stdout.strip(),
              "manifest_sha256": sha(ROOT / "manifest.json"), "frozen_inputs": {}, "cases": []}
    ok = True
    for rel, pinned in manifest["frozen_inputs"].items():
        actual = sha(REPO / rel)
        record["frozen_inputs"][rel] = {"sha256": actual, "matches": actual == pinned}
        ok &= actual == pinned
    record["case_dirs_match_manifest"] = {p.name for p in ROOT.iterdir() if p.is_dir()} == {c["id"] for c in manifest["cases"]}
    ok &= record["case_dirs_match_manifest"]
    for case in manifest["cases"]:
        base = ROOT / case["id"]
        actual = pins(base)
        regular = [f for f, v in actual.items() if isinstance(v, str)]
        entry = {"id": case["id"], "pins_match": actual == case["files"],
                 "single_claim_marker": sum((base / f).read_bytes().count(marker) for f in regular) == 1
                 and (base / case["importer"]).read_bytes().count(marker) == 1}
        if case["expected_class"] == "must":
            target = (base / case["expected_target"]["file"]).read_bytes()
            entry["target_marker_unique"] = target.count(case["expected_target"]["marker"].encode()) == 1
            ok &= entry["target_marker_unique"]
        if case["id"] in RAW_SPECIFIERS:
            entry["raw_specifier_bytes"] = RAW_SPECIFIERS[case["id"]] in (base / case["importer"]).read_bytes()
            ok &= entry["raw_specifier_bytes"]
        if case["runtime"]["mode"] == "node-test":
            entry["runtime"] = node_test(base, case["runtime"]["test_file"], case["runtime"].get("node_args", []))
            ok &= entry["runtime"]["passed"]
        else:
            entry["runtime"] = {"skipped": case["runtime"]["reason"]}
        ok &= entry["pins_match"] and entry["single_claim_marker"]
        record["cases"].append(entry)
    record["incremental"] = []
    for sequence in manifest.get("incremental_sequences", []):
        inc_root = REPO / sequence["base_dir"]
        parent = inc_root.parent
        actual = {"base/" + k: v for k, v in pins(inc_root).items()} | {"steps/" + k: v for k, v in pins(parent / "steps").items()}
        seq = {"id": sequence["id"], "pins_match": actual == sequence["files"], "steps": []}
        ok &= seq["pins_match"]
        with tempfile.TemporaryDirectory() as tmp:
            work = pathlib.Path(tmp) / "snapshot"
            shutil.copytree(inc_root, work)
            seq["base_runtime"] = node_test(work, sequence["importer"])
            ok &= seq["base_runtime"]["passed"]
            for step in sequence["steps"]:
                target = work / step["path"]
                if step["op"] == "write":
                    shutil.copyfile(parent / step["source"], target)
                else:
                    target.unlink()
                result = node_test(work, sequence["importer"])
                matched = result["passed"] == (step["runtime_expect"] == "pass")
                seq["steps"].append({"op": step["op"], "path": step["path"], "expected_class": step["expected_class"],
                                     "runtime_expect": step["runtime_expect"], "runtime": result, "matched": matched})
                ok &= matched
        record["incremental"].append(seq)
    record["counts"] = {"cases": len(record["cases"]),
                        "runtime_passed": sum(1 for c in record["cases"] if c["runtime"].get("passed")),
                        "runtime_skipped": sum(1 for c in record["cases"] if "skipped" in c["runtime"]),
                        "incremental_steps_matched": sum(s["matched"] for q in record["incremental"] for s in q["steps"])}
    record["all_passed"] = bool(ok)
    OUT.write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps({"all_passed": record["all_passed"], **record["counts"]}))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
