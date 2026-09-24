import ast
import json
import re
from pathlib import Path

PACKAGES = {
    "click-8.4.1": "/tmp/dispatch-audit-python-extracted/click-8.4.1/click-8.4.1",
    "pydantic-2.13.4": "/tmp/dispatch-audit-python-extracted/pydantic-2.13.4/pydantic-2.13.4",
    "requests-2.34.2": "/tmp/dispatch-audit-python-extracted/requests-2.34.2/requests-2.34.2",
}

MUST_REASON = "proven-top-level-lexical-binding"
claim_re = re.compile(
    r"\(site:\([^)]*\),class:(\w+),targets:\[([^\]]*)\],reason:\"([^\"]*)\",coverage_gap:(\w+)\)"
)


def must_target_names(inspect_path):
    with open(inspect_path) as f:
        data = json.load(f)
    nodes = data["nodes"]
    by_hex_id = {n["id"]: n for n in nodes}
    names = set()
    for n in nodes:
        for k, v in n.get("attributes", []):
            if k != "call_evidence_v1":
                continue
            for m in claim_re.finditer(v):
                cls, targets_str, reason, cov = m.groups()
                if cls == "must" and reason == MUST_REASON:
                    for tid_str in targets_str.split(","):
                        tid_str = tid_str.strip().strip("()")
                        if not tid_str:
                            continue
                        hexid = format(int(tid_str), "x")
                        tn = by_hex_id.get(hexid)
                        if tn:
                            names.add(tn.get("name"))
    return names


class StoreDelAttrCollector(ast.NodeVisitor):
    def __init__(self):
        self.names = set()

    def visit_Attribute(self, node):
        if isinstance(node.ctx, (ast.Store, ast.Del)):
            self.names.add(node.attr)
        self.generic_visit(node)


for pkg, root in PACKAGES.items():
    inspect_path = f"/tmp/claude-1000/-home-corymaynard370/7daef8aa-b8bb-49b8-8f09-dce1989aa5f0/scratchpad/{pkg}-inspect-correction1.json"
    must_names = must_target_names(inspect_path)
    collector = StoreDelAttrCollector()
    for py_file in Path(root).rglob("*.py"):
        try:
            src = py_file.read_text(encoding="utf-8", errors="replace")
            tree = ast.parse(src, filename=str(py_file))
        except SyntaxError:
            continue
        collector.visit(tree)
    hits = collector.names & must_names
    print(pkg, "must_names:", len(must_names), "store/del attr names:", len(collector.names), "INTERSECTION:", sorted(hits))
