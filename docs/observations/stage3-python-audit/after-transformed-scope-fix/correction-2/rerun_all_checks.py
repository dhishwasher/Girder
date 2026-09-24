import ast
import sys
from pathlib import Path

sys.path.insert(0, "/tmp/claude-1000/-home-corymaynard370/7daef8aa-b8bb-49b8-8f09-dce1989aa5f0/scratchpad")
from common import must_claims, load_nodes, resolve_target

PACKAGES = {
    "click-8.4.1": "/tmp/dispatch-audit-python-extracted/click-8.4.1/click-8.4.1",
    "pydantic-2.13.4": "/tmp/dispatch-audit-python-extracted/pydantic-2.13.4/pydantic-2.13.4",
    "requests-2.34.2": "/tmp/dispatch-audit-python-extracted/requests-2.34.2/requests-2.34.2",
}

SCRATCH = "/tmp/claude-1000/-home-corymaynard370/7daef8aa-b8bb-49b8-8f09-dce1989aa5f0/scratchpad"

all_claims_by_pkg = {}
all_names_by_pkg = {}
for pkg, root in PACKAGES.items():
    claims = must_claims(f"{SCRATCH}/{pkg}-inspect-correction1.json")
    assert all(not c["unresolved"] for c in claims), f"{pkg}: unresolved targets present"
    all_claims_by_pkg[pkg] = claims
    all_names_by_pkg[pkg] = set(c["target_name"] for c in claims)

print("=== 1. AST Store/Del attribute scan (check 1) ===")


class StoreDelAttrCollector(ast.NodeVisitor):
    def __init__(self):
        self.names = set()

    def visit_Attribute(self, node):
        if isinstance(node.ctx, (ast.Store, ast.Del)):
            self.names.add(node.attr)
        self.generic_visit(node)


for pkg, root in PACKAGES.items():
    collector = StoreDelAttrCollector()
    for py_file in Path(root).rglob("*.py"):
        try:
            tree = ast.parse(py_file.read_text(encoding="utf-8", errors="replace"), filename=str(py_file))
        except SyntaxError:
            continue
        collector.visit(tree)
    hits = collector.names & all_names_by_pkg[pkg]
    print(pkg, "must_names:", len(all_names_by_pkg[pkg]), "store/del attr names:", len(collector.names), "INTERSECTION:", sorted(hits))

print("=== 2. patch.multiple/__dict__.update keyword intersection (check 2) ===")


def callee_ends_with(call, suffix):
    return isinstance(call.func, ast.Attribute) and call.func.attr == suffix


for pkg, root in PACKAGES.items():
    kw_names = set()
    for py_file in Path(root).rglob("*.py"):
        try:
            tree = ast.parse(py_file.read_text(encoding="utf-8", errors="replace"), filename=str(py_file))
        except SyntaxError:
            continue
        for node in ast.walk(tree):
            if isinstance(node, ast.Call) and (callee_ends_with(node, "multiple") or callee_ends_with(node, "update")):
                for kw in node.keywords:
                    if kw.arg:
                        kw_names.add(kw.arg)
    hits = kw_names & all_names_by_pkg[pkg]
    print(pkg, "kw_names:", len(kw_names), "INTERSECTION:", sorted(hits))

print("=== 3. Lost-names diff vs after-operator-claim-fix (check 4) ===")
for pkg in PACKAGES:
    old_claims = must_claims(
        f"/home/corymaynard370/Bit-code/docs/observations/stage3-python-audit/after-operator-claim-fix/{pkg}-inspect.json"
    )
    if any(c["unresolved"] for c in old_claims):
        print(pkg, "OLD ROUND HAS UNRESOLVED TARGETS TOO -- count:", sum(1 for c in old_claims if c["unresolved"]))
    old_names = set((c["target_file"], c["target_name"]) for c in old_claims if not c["unresolved"])
    new_names = set((c["target_file"], c["target_name"]) for c in all_claims_by_pkg[pkg])
    lost = old_names - new_names
    gained = new_names - old_names
    print(pkg, "old:", len(old_names), "new:", len(new_names), "lost:", sorted(lost), "gained_count:", len(gained))

print("=== 4. Programmatic same-file/top-level/undecorated/unique check (check 5) ===")
total = 0
violations = []
for pkg, root in PACKAGES.items():
    file_defs = {}
    for py_file in Path(root).rglob("*.py"):
        try:
            tree = ast.parse(py_file.read_text(encoding="utf-8", errors="replace"), filename=str(py_file))
        except SyntaxError:
            continue
        rel = str(py_file.relative_to(root))
        defs = {}
        for node in tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                defs.setdefault(node.name, []).append(bool(node.decorator_list))
        file_defs[rel] = defs

    for c in all_claims_by_pkg[pkg]:
        total += 1
        if c["caller_file"] != c["target_file"]:
            violations.append((pkg, c, "cross-file"))
            continue
        defs = file_defs.get(c["target_file"], {})
        occ = defs.get(c["target_name"])
        if occ is None:
            violations.append((pkg, c, "not-top-level-in-ast"))
        elif len(occ) != 1:
            violations.append((pkg, c, f"not-unique({len(occ)})"))
        elif occ[0]:
            violations.append((pkg, c, "decorated"))

print("total must claims checked:", total, "violations:", len(violations))
for v in violations:
    print(" ", v)
