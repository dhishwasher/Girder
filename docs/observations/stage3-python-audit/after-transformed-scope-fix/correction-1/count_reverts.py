import json, sys

with open("/tmp/claude-1000/-home-corymaynard370/7daef8aa-b8bb-49b8-8f09-dce1989aa5f0/scratchpad/inspect-after-fix.json") as f:
    data = json.load(f)

def walk(obj):
    if isinstance(obj, dict):
        yield obj
        for v in obj.values():
            yield from walk(v)
    elif isinstance(obj, list):
        for v in obj:
            yield from walk(v)

py_hits = 0
non_py_hits = []
REASON = "python-target-string-rebound-elsewhere-in-crate"

nodes = data.get("nodes") if isinstance(data, dict) else None
if nodes is None:
    # fall back to generic walk looking for objects with 'file' and 'call_evidence' style shape
    print("top-level keys:", list(data.keys()) if isinstance(data, dict) else type(data))
    sys.exit(0)

for n in nodes:
    file = n.get("file") or ""
    ev = n.get("call_evidence") or n.get("attributes", {}).get("call_evidence_v1")
    calls = []
    if isinstance(ev, dict):
        calls = ev.get("calls", [])
    for c in calls:
        if c.get("reason") == REASON:
            if file.endswith(".py"):
                py_hits += 1
            else:
                non_py_hits.append((file, n.get("name")))

print("py_hits:", py_hits)
print("non_py_hits:", len(non_py_hits))
for h in non_py_hits:
    print(" ", h)
