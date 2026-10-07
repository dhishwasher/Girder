#!/usr/bin/env python3
"""Measure the frozen Go real-source audit against a girder binary.

Verifies every frozen input hash, extracts a fresh copy of the pinned standard
library subset, runs `girder analyze` then `girder inspect`, and scores each
labeled site. A site matches a claim when claim.site.end_byte equals the site's
call_end_byte in the same file. A labeled actual call with no matching claim is
`unsafe_exclusion`. Cells per actual call: exact, conservative, overclaim,
unsafe_exclusion. not_a_call_site labels are reported, not scored.

  python3 -m tools.measure_go_real_audit --name baseline-1 --binary BIN --binary-revision SHA
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import re
import subprocess
import tempfile
from pathlib import Path

from tools.dispatch_audit_scorer_typescript import extract_call_claims, resolve_node_id
from tools.go_audit_inventory import ARCHIVE, selected
import tarfile

BASE = Path(__file__).resolve().parents[1]
OBS = BASE / "docs/observations/stage3-go-audit"


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_freeze() -> dict:
    freeze = json.loads((OBS / "labels-freeze.json").read_text())
    pins = {
        "sites-initial-140.json": freeze["inputs"]["sites_initial_140"],
        "sites-context.json": freeze["inputs"]["sites_context"],
        "labels-final.json": freeze["labels_final_sha256"],
        "label-audit-log.json": freeze["audit_log_sha256"],
        "policy.md": freeze["inputs"]["policy"],
        "methodology.md": freeze["inputs"]["methodology"],
    }
    for name, digest in pins.items():
        if sha(OBS / name) != digest:
            raise SystemExit(f"frozen input changed: {name}")
    return freeze


def method_nodes(doc: dict, root: Path, file: str, symbol: str) -> list[str]:
    want = (root / file).resolve()
    name = symbol.rsplit(".", 1)[-1]
    owner = symbol.split(".")[0] if "." in symbol else None
    hits = []
    for node in doc["nodes"]:
        if node.get("kind") != "Function" or not node.get("file"):
            continue
        path = Path(node["file"])
        if (path if path.is_absolute() else root / path).resolve() != want:
            continue
        if node["path"].rsplit("::", 1)[-1] == name:
            hits.append((node["id"], node["path"]))
    if owner and len(hits) > 1:
        refined = [h for h in hits if owner in h[1]]
        hits = refined or hits
    elif not owner and len(hits) > 1:
        shallow = min(h[1].count("::") for h in hits)
        hits = [h for h in hits if h[1].count("::") == shallow]
    return [h[0] for h in hits]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--name", required=True)
    ap.add_argument("--binary", required=True)
    ap.add_argument("--binary-revision", required=True)
    args = ap.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9-]+", args.name):
        raise SystemExit("bad name")
    freeze = verify_freeze()
    binary = Path(args.binary).resolve(strict=True)
    labels = json.loads((OBS / "labels-final.json").read_text())
    dest = OBS / args.name
    dest.mkdir(parents=True, exist_ok=False)
    record = {"binary_revision": args.binary_revision, "binary_sha256": sha(binary),
              "runner_sha256": sha(Path(__file__)), "labels_final_sha256": freeze["labels_final_sha256"],
              "git_head": subprocess.run(["git", "rev-parse", "HEAD"], cwd=BASE, capture_output=True, text=True).stdout.strip(),
              "status": "started"}
    (dest / "start.json").write_text(json.dumps(record, indent=2) + "\n")
    with tempfile.TemporaryDirectory(prefix=f"girder-go-audit-{args.name}-") as scratch:
        root = Path(scratch) / "tree"
        with tarfile.open(ARCHIVE) as archive:
            for rel, data in selected(archive):
                (root / rel).parent.mkdir(parents=True, exist_ok=True)
                (root / rel).write_bytes(data)
        a = subprocess.run([str(binary), "analyze", str(root), "--json"], capture_output=True, text=True, timeout=900)
        (dest / "analyze.stderr.txt").write_text(a.stderr)
        if a.returncode:
            raise SystemExit("analyze failed: " + str(a.returncode))
        (dest / "analyze.json.gz").write_bytes(gzip.compress(a.stdout.encode(), mtime=0))
        graph = json.loads(a.stdout)["graph_path"]
        i = subprocess.run([str(binary), "inspect", graph, "--json"], capture_output=True, text=True, timeout=900)
        (dest / "inspect.stderr.txt").write_text(i.stderr)
        if i.returncode:
            raise SystemExit("inspect failed: " + str(i.returncode))
        (dest / "inspect.json.gz").write_bytes(gzip.compress(i.stdout.encode(), mtime=0))
        inspect = Path(scratch) / "inspect.json"
        inspect.write_text(i.stdout)
        doc = json.loads(i.stdout)
        claims = {}
        for c in extract_call_claims(inspect):
            f = Path(c["file"])
            rel = (f if f.is_absolute() else root / f).resolve().relative_to(root.resolve()).as_posix()
            claims.setdefault((rel, c["end_byte"]), []).append(c)
        rows = []
        for label in labels:
            row = {"id": label["id"], "file": label["file"], "line": label["line"], "stratum": label["stratum"],
                   "callee_text": label["callee_text"], "label": label["class"]}
            rows.append(row)
            if label["class"] == "not_a_call_site":
                row["cell"] = "not_scored"
                continue
            found = claims.get((label["file"], label["call_end_byte"]), [])
            if len(found) != 1:
                row["cell"] = "unsafe_exclusion"
                row["failure"] = f"{len(found)} matching claims"
                continue
            claim = found[0]
            targets = [resolve_node_id(t) for t in claim["targets"]]
            row.update(observed_class=claim["class"], observed_reason=claim["reason"], observed_targets=targets)
            if label["class"] == "unknown":
                row["cell"] = "exact" if claim["class"] == "unknown" else "overclaim"
            elif claim["class"] != "must":
                row["cell"] = "conservative"
            else:
                expected = []
                if label["target_in_snapshot"] and label["target"] and "::" in label["target"]:
                    f, sym = label["target"].split("::", 1)
                    expected = method_nodes(doc, root, f, sym)
                row["expected_target_nodes"] = expected
                row["cell"] = "exact" if expected and targets and set(targets) <= set(expected) and not claim["coverage_gap"] else "overclaim"
    calls = [r for r in rows if r["cell"] != "not_scored"]
    cells = [r["cell"] for r in calls]
    summary = {k: cells.count(k) for k in ("exact", "conservative", "overclaim", "unsafe_exclusion")}
    must_claims = [r for r in calls if r.get("observed_class") == "must"]
    record.update(status="measured", rows=rows, summary=summary, actual_calls=len(calls),
                  not_a_call_sites=len(rows) - len(calls), unsound=summary["overclaim"] + summary["unsafe_exclusion"],
                  girder_must_claims=len(must_claims), girder_must_correct=sum(r["cell"] == "exact" for r in must_claims),
                  must_precision=(sum(r["cell"] == "exact" for r in must_claims) / len(must_claims)) if must_claims else None)
    (dest / "run.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps({k: v for k, v in record.items() if k != "rows"}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
