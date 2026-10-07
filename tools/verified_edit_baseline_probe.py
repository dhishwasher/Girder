#!/usr/bin/env python3
"""Before-observation for the verified-edit fixtures, using the unmodified product.

For every manifest case, with today's `girder` only:
  1. For each step that carries `verify`, apply its replace_node edits textually to a copy
     (file bytes in a node's span equal Node.source, so a text splice equals the projection),
     run `analyze` and `inspect` before and after, compute the canonical delta (nodes keyed by
     path with changed `source`; edges as (source, target, kind) over all kinds), and compare it
     with the declared delta.
  2. Run the plan with `verify` stripped through today's `girder plan run --dry`, recording
     whether the existing pipeline accepts it (it must, for every case that has a replace_node
     edit: later refusals must come from the new layer, not the old one).

  python3 -m tools.verified_edit_baseline_probe --binary BIN --output probe.json
"""
from __future__ import annotations

import argparse
import copy
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "fixtures" / "verified-edits" / "v1"
GIT = ["git", "-c", "user.email=f@x", "-c", "user.name=fixture"]


def init_repo(path: Path) -> str:
    subprocess.run(["git", "init", "-q", str(path)], check=True)
    subprocess.run(GIT + ["-C", str(path), "add", "-A"], check=True)
    subprocess.run(GIT + ["-C", str(path), "commit", "-q", "-m", "fixture"], check=True)
    return subprocess.run(["git", "-C", str(path), "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()


def graph(binary: Path, root: Path) -> dict:
    """Nodes with their real Node.source (via `context`; `inspect` omits it), spans, and edges."""
    subprocess.run([str(binary), "analyze", str(root)], capture_output=True, check=True)
    out = subprocess.run([str(binary), "inspect", str(root / "project.aether"), "--json"],
                         capture_output=True, text=True, check=True).stdout
    doc = json.loads(out)
    paths = [n["path"] for n in doc["nodes"]]
    ctx = subprocess.run([str(binary), "context", str(root), "--nodes", ",".join(paths), "--json"],
                         capture_output=True, text=True, check=True).stdout
    source = {n["path"]: n["source"] for n in json.loads(ctx)["nodes"]}
    return {"nodes": {n["path"]: (source[n["path"]], n.get("file"), n["kind"],
                                  (n["span"]["start_byte"], n["span"]["end_byte"])) for n in doc["nodes"]},
            "edges": {(e["source"], e["target"], e["kind"]) for e in doc["edges"]}}


def canonical_delta(before: dict, after: dict) -> dict:
    # Module nodes hold whole-file text and are excluded (see docs/verified-edits-policy.md).
    b = {p: v for p, v in before["nodes"].items() if v[2] != "Module"}
    a = {p: v for p, v in after["nodes"].items() if v[2] != "Module"}
    changed = sorted(p for p in b if p in a and b[p][0] != a[p][0])
    modules_changed = sorted(p for p, v in before["nodes"].items()
                             if v[2] == "Module" and p in after["nodes"] and v[0] != after["nodes"][p][0])
    return {"modules_changed_excluded": modules_changed,
            "nodes": {"changed": changed, "added": sorted(set(a) - set(b)), "removed": sorted(set(b) - set(a))},
            "edges": {"added": sorted(map(list, after["edges"] - before["edges"])),
                      "removed": sorted(map(list, before["edges"] - after["edges"]))},
            "changed_kinds": {p: a[p][2] for p in changed}}


def apply_step(root: Path, before: dict, step: dict) -> None:
    for edit in step["edits"]:
        if "replace_node" in edit:
            source, file, _, (start, end) = before["nodes"][edit["node"]]
            data = (root / file).read_bytes()
            assert data[start:end].decode() == source, "span does not hold the node source"
            (root / file).write_bytes(data[:start] + edit["replace_node"].encode() + data[end:])
        elif "match" in edit:
            text = (root / edit["path"]).read_text()
            (root / edit["path"]).write_text(text.replace(edit["match"], edit["replace"], edit.get("occurrences", 1)))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", required=True)
    ap.add_argument("--output", required=True)
    a = ap.parse_args()
    binary = Path(a.binary).resolve(strict=True)
    manifest = json.loads((ROOT / "manifest.json").read_text())
    report = []
    for case in manifest["cases"]:
        plan = json.loads((ROOT / case["plan"]).read_text())
        row = {"id": case["id"], "expected": case["expected"], "steps": []}
        with tempfile.TemporaryDirectory() as scratch:
            work = Path(scratch) / "w"
            shutil.copytree(ROOT / case["fixture"], work)
            init_repo(work)
            cur = graph(binary, work)
            for step in plan["steps"]:
                verify = step.get("verify")
                supported = all("replace_node" in e for e in step["edits"]) and verify is not None
                info = {"step": step["id"], "has_verify": verify is not None, "delta_probed": supported}
                if supported:
                    apply_step(work, cur, step)
                    nxt = graph(binary, work)
                    actual = canonical_delta(cur, nxt)
                    declared = verify.get("delta", {})
                    d_nodes = declared.get("nodes", {})
                    info["actual"] = actual
                    info["declared_equals_actual"] = (
                        sorted(d_nodes.get("changed", [])) == actual["nodes"]["changed"]
                        and sorted(d_nodes.get("added", [])) == actual["nodes"]["added"]
                        and sorted(d_nodes.get("removed", [])) == actual["nodes"]["removed"]
                        and sorted(map(list, declared.get("edges", {}).get("added", []))) == actual["edges"]["added"]
                        and sorted(map(list, declared.get("edges", {}).get("removed", []))) == actual["edges"]["removed"])
                    cur = nxt
                row_steps = row["steps"]
                row_steps.append(info)
        # today's pipeline, verify stripped
        with tempfile.TemporaryDirectory() as scratch:
            work = Path(scratch) / "w"
            shutil.copytree(ROOT / case["fixture"], work)
            base = init_repo(work)
            stripped = copy.deepcopy(plan)
            stripped["base_commit"] = base
            for s in stripped["steps"]:
                s.pop("verify", None)
            (Path(scratch) / "plan.json").write_text(json.dumps(stripped))
            run = subprocess.run([str(binary), "plan", "run", str(Path(scratch) / "plan.json"), "--dry"],
                                 cwd=work, capture_output=True, text=True, timeout=300)
            row["today_unverified"] = {"exit": run.returncode, "head": (run.stdout + run.stderr).strip().splitlines()[:3]}
        report.append(row)
    Path(a.output).write_text(json.dumps({"cases": report}, indent=2, sort_keys=True) + "\n")
    bad = [r["id"] for r in report for s in r["steps"]
           if s["delta_probed"] and r["expected"]["outcome"].startswith("committed") and not s["declared_equals_actual"]]
    print(len(report), "cases; committed cases whose declared delta differs from today's actual delta:", bad)
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
