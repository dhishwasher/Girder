import json
import re

MUST_REASON = "proven-top-level-lexical-binding"
claim_re = re.compile(
    r"\(site:\(start_byte:(\d+),end_byte:(\d+),start_row:(\d+),start_col:(\d+)\),class:(\w+),targets:\[([^\]]*)\],reason:\"([^\"]*)\",coverage_gap:(\w+)\)"
)


def load_nodes(path):
    with open(path) as f:
        data = json.load(f)
    nodes = data["nodes"]
    by_hex = {n["id"]: n for n in nodes}
    return nodes, by_hex


def resolve_target(by_hex, tid_str):
    tid_str = tid_str.strip().strip("()")
    if not tid_str:
        return None, None
    tid_int = int(tid_str)
    hexid = format(tid_int, "016x")
    node = by_hex.get(hexid)
    return hexid, node


def must_claims(path):
    """Returns list of dicts: caller_file, caller, site info, target_file, target_name, unresolved(bool)."""
    nodes, by_hex = load_nodes(path)
    out = []
    for n in nodes:
        for k, v in n.get("attributes", []):
            if k != "call_evidence_v1":
                continue
            for m in claim_re.finditer(v):
                sb, eb, sr, sc, cls, targets_str, reason, cov = m.groups()
                if cls == "must" and reason == MUST_REASON:
                    for tid_str in targets_str.split(","):
                        hexid, node = resolve_target(by_hex, tid_str)
                        if hexid is None:
                            continue
                        out.append(
                            {
                                "caller_file": n.get("file"),
                                "caller": n.get("name"),
                                "site_start": int(sb),
                                "site_end": int(eb),
                                "row": int(sr),
                                "target_hexid": hexid,
                                "target_file": node.get("file") if node else None,
                                "target_name": node.get("name") if node else None,
                                "unresolved": node is None,
                            }
                        )
    return out
