#!/usr/bin/env python3
"""Measure the frozen Go direct-call proof fixtures against a girder binary.

Cold CLI graphs only (`girder analyze` then `girder inspect`). Every case is
scored, none dropped. Cells: exact, conservative (honest Unknown where Must was
expected), overclaim (Must where Unknown was expected, or a Must naming the wrong
target) and failed (no unique marked claim). Overclaims are the unsound cells.

  python3 -m tools.measure_go_direct_calls --name baseline-1 --binary BIN --binary-revision SHA
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

from tools.dispatch_audit_scorer_typescript import extract_call_claims, resolve_node_id
from tools.go_audit_sites import blank, match_close

BASE = Path(__file__).resolve().parents[1]
CORPUS = BASE / "fixtures/go-direct-call-proof/v1"
OBS = BASE / "docs/observations/stage3-go-audit"
MARKER = "/* claim */"
CALLEE = re.compile(r"\s*([A-Za-z_][\w.]*)\s*(\[[^\]]*\])?\s*\(")


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def marked_call_end(source: bytes) -> int:
    text = blank(source).decode("latin-1")
    original = source.decode("latin-1")
    if original.count(MARKER) != 1:
        raise ValueError("expected exactly one call marker")
    after = original.index(MARKER) + len(MARKER)
    m = CALLEE.match(text, after)
    if not m:
        raise ValueError("no call after marker")
    close = match_close(text, m.end() - 1, "(", ")")
    if close < 0:
        raise ValueError("unbalanced call")
    return close + 1


def target_node(doc: dict, root: Path, target: str):
    file, symbol = target.split("::")
    want = (root / file).resolve()
    hits = []
    for node in doc["nodes"]:
        if node.get("kind") != "Function" or not node.get("file"):
            continue
        path = Path(node["file"])
        resolved = (path if path.is_absolute() else root / path).resolve()
        if resolved == want and node["path"].rsplit("::", 1)[-1] == symbol:
            hits.append(node["id"])
    return hits


def score(case: dict, root: Path, inspect: Path) -> dict:
    source = (root / case["importer"]).read_bytes()
    end = marked_call_end(source)
    importer = (root / case["importer"]).resolve()
    claims = [c for c in extract_call_claims(inspect)
              if c["end_byte"] == end
              and (Path(c["file"]) if Path(c["file"]).is_absolute() else root / c["file"]).resolve() == importer]
    row = {"marked_claims": claims, "cell": "failed"}
    if len(claims) != 1:
        row["failure"] = f"expected one marked claim, got {len(claims)}"
        return row
    claim = claims[0]
    targets = [resolve_node_id(t) for t in claim["targets"]]
    observed = claim["class"]
    row["observed_class"], row["observed_reason"] = observed, claim["reason"]
    if case["expected"] == "unknown":
        row["cell"] = "exact" if observed == "unknown" and not targets else "overclaim"
        return row
    expected_nodes = target_node(json.loads(inspect.read_text()), root, case["expected_target"])
    row["expected_target_nodes"] = expected_nodes
    if observed == "must" and not claim["coverage_gap"] and len(expected_nodes) == 1 and targets == expected_nodes:
        row["cell"] = "exact"
    elif observed == "must":
        row["cell"] = "overclaim"
    else:
        row["cell"] = "conservative"
    return row


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--name", required=True)
    ap.add_argument("--binary", required=True)
    ap.add_argument("--binary-revision", required=True)
    args = ap.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9-]+", args.name):
        raise SystemExit("bad name")
    binary = Path(args.binary).resolve(strict=True)
    manifest_path = CORPUS / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    dest = OBS / args.name
    dest.mkdir(parents=True, exist_ok=False)
    record = {
        "binary_revision": args.binary_revision,
        "binary_sha256": sha(binary),
        "manifest_sha256": sha(manifest_path),
        "runner_sha256": sha(Path(__file__)),
        "git_head": subprocess.run(["git", "rev-parse", "HEAD"], cwd=BASE, capture_output=True, text=True).stdout.strip(),
        "cases": [],
    }
    with tempfile.TemporaryDirectory(prefix=f"girder-go-contract-{args.name}-") as scratch:
        for case in manifest["cases"]:
            row = {"id": case["id"], "expected": case["expected"], "rule": case["rule"]}
            record["cases"].append(row)
            try:
                for rel, digest in case["files"].items():
                    if sha(BASE / case["path"] / rel) != digest:
                        raise ValueError("fixture hash mismatch: " + rel)
                root = Path(scratch) / case["id"]
                shutil.copytree(BASE / case["path"], root)
                a = subprocess.run([str(binary), "analyze", str(root), "--json"], capture_output=True, text=True, timeout=120)
                if a.returncode:
                    raise RuntimeError("analyze exit " + str(a.returncode))
                graph = json.loads(a.stdout)["graph_path"]
                i = subprocess.run([str(binary), "inspect", graph, "--json"], capture_output=True, text=True, timeout=120)
                if i.returncode:
                    raise RuntimeError("inspect exit " + str(i.returncode))
                inspect = Path(scratch) / (case["id"] + "-inspect.json")
                inspect.write_text(i.stdout)
                (dest / (case["id"] + "-inspect.json.gz")).write_bytes(gzip.compress(i.stdout.encode(), mtime=0))
                row.update(score(case, root, inspect))
            except Exception as error:  # recorded, never dropped
                row["error"] = f"{type(error).__name__}: {error}"
    cells = [c.get("cell", "failed") for c in record["cases"]]
    record["summary"] = {k: cells.count(k) for k in ("exact", "conservative", "overclaim", "failed")}
    record["total"] = len(cells)
    record["unsound"] = record["summary"]["overclaim"]
    must = [c for c in record["cases"] if c.get("observed_class") == "must"]
    record["must_claims"] = len(must)
    record["must_correct"] = sum(c["cell"] == "exact" and c["expected"] == "must" for c in must)
    (dest / "run.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps({k: v for k, v in record.items() if k != "cases"}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
