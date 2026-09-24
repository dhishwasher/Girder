import ast
import json
import re
from pathlib import Path

PACKAGES = {
    "click-8.4.1": "/tmp/dispatch-audit-python-extracted/click-8.4.1/click-8.4.1",
    "pydantic-2.13.4": "/tmp/dispatch-audit-python-extracted/pydantic-2.13.4/pydantic-2.13.4",
    "requests-2.34.2": "/tmp/dispatch-audit-python-extracted/requests-2.34.2/requests-2.34.2",
}

claim_re = re.compile(
    r"\(site:\([^)]*\),class:(\w+),targets:\[([^\]]*)\],reason:\"([^\"]*)\",coverage_gap:(\w+)\)"
)


def must_claims(inspect_path):
    with open(inspect_path) as f:
        data = json.load(f)
    nodes = data["nodes"]
    by_hex = {n["id"]: n for n in nodes}
    out = []
    for n in nodes:
        for k, v in n.get("attributes", []):
            if k != "call_evidence_v1":
                continue
            for m in claim_re.finditer(v):
                cls, targets_str, reason, cov = m.groups()
                if cls == "must" and reason == "proven-top-level-lexical-binding":
                    for tid in targets_str.split(","):
                        tid = tid.strip().strip("()")
                        if not tid:
                            continue
                        tn = by_hex.get(format(int(tid), "x"))
                        if tn:
                            out.append((n.get("file"), n.get("name"), tn.get("file"), tn.get("name")))
    return out


total = 0
violations = []
for pkg, root in PACKAGES.items():
    claims = must_claims(
        f"/tmp/claude-1000/-home-corymaynard370/7daef8aa-b8bb-49b8-8f09-dce1989aa5f0/scratchpad/{pkg}-inspect-correction1.json"
    )
    # Build a per-file top-level-def-count and decorated-set via ast
    file_defs = {}
    for py_file in Path(root).rglob("*.py"):
        try:
            src = py_file.read_text(encoding="utf-8", errors="replace")
            tree = ast.parse(src, filename=str(py_file))
        except SyntaxError:
            continue
        rel = str(py_file.relative_to(root))
        defs = {}
        for node in tree.body:  # only true top-level (module body), matches "top_level" semantics
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                defs.setdefault(node.name, []).append(bool(node.decorator_list))
        file_defs[rel] = defs

    for caller_file, caller_name, target_file, target_name in claims:
        total += 1
        if caller_file != target_file:
            violations.append((pkg, caller_file, caller_name, target_file, target_name, "cross-file"))
            continue
        defs = file_defs.get(target_file, {})
        occurrences = defs.get(target_name)
        if occurrences is None:
            violations.append((pkg, caller_file, caller_name, target_file, target_name, "not-top-level-in-ast"))
            continue
        if len(occurrences) != 1:
            violations.append((pkg, caller_file, caller_name, target_file, target_name, f"not-unique ({len(occurrences)})"))
            continue
        if occurrences[0]:
            violations.append((pkg, caller_file, caller_name, target_file, target_name, "decorated"))
            continue

print("total must claims checked:", total)
print("violations:", len(violations))
for v in violations:
    print(" ", v)
