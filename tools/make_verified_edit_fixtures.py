#!/usr/bin/env python3
"""Generate the frozen Stage 5 verified-edit fixtures, plans, and manifest.

Writes fixtures/verified-edits/v1/. Plans carry independently computed node
fingerprints: sha256(b"girder-node-fingerprint-v1\\0" + path + b"\\0" + source), where
path and source are a node's semantic path and source text as `girder context`
reports them. Plans use the placeholder base_commit "<HEAD>", which the test
harness replaces with the commit of a fresh git repository holding the fixture.

  python3 -m tools.make_verified_edit_fixtures --binary PATH_TO_GIRDER
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "fixtures" / "verified-edits" / "v1"
PRELUDE = b"girder-node-fingerprint-v1\0"
def cargo(name):
    return f'[package]\nname = "fixture_{name}"\nversion = "0.1.0"\nedition = "2021"\n\n[workspace]\n'


OVERLOAD = '''pub trait Render {
    fn render(&self) -> String;
}

pub struct Widget;

impl Widget {
    pub fn render(&self) -> String {
        String::from("inherent")
    }
}

impl Render for Widget {
    fn render(&self) -> String {
        String::from("trait")
    }
}

pub fn draw(w: &Widget) -> String {
    w.render()
}

pub fn draw_via_trait<T: Render>(t: &T) -> String {
    Render::render(t)
}

pub fn helper() -> usize {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_caller_reaches_its_overload() {
        assert_eq!(draw(&Widget), "inherent");
        assert_eq!(draw_via_trait(&Widget), "trait");
    }
}
'''
IDENTICAL = OVERLOAD.replace('"inherent"', '"same"').replace('"trait"', '"same"').replace(
    '''        assert_eq!(draw(&Widget), "same");
        assert_eq!(draw_via_trait(&Widget), "same");''', '''        assert_eq!(draw(&Widget), draw_via_trait(&Widget));''')
IDENTICAL = IDENTICAL.replace("    pub fn render(&self)", "    fn render(&self)")  # keep both node sources byte-identical
BASIC = '''pub fn alpha() -> i32 {
    1
}

pub fn beta() -> i32 {
    2
}

pub fn helper() -> i32 {
    3
}
'''
PYBASIC = '''def alpha():
    return 1


def beta():
    return 2
'''

FIXTURES = {
    "overload": {"Cargo.toml": cargo("overload"), "src/lib.rs": OVERLOAD},
    "overload-identical": {"Cargo.toml": cargo("overload_identical"), "src/lib.rs": IDENTICAL},
    "basic": {"Cargo.toml": cargo("basic"), "src/lib.rs": BASIC},
    "pybasic": {"app.py": PYBASIC},
}
TRAIT = "crate::lib::Widget::render@Render"
INHERENT = "crate::lib::Widget::render"
A, B_, H = "crate::lib::alpha", "crate::lib::beta", "crate::lib::helper"
PYA, PYB = "crate::app::alpha", "crate::app::beta"


def fp(path: str, source: str) -> str:
    return hashlib.sha256(PRELUDE + path.encode() + b"\0" + source.encode()).hexdigest()


def sources(binary: Path, fixture: str, paths: list[str]) -> dict[str, str]:
    with tempfile.TemporaryDirectory() as scratch:
        work = Path(scratch) / fixture
        shutil.copytree(ROOT / fixture, work)
        git = ["git", "-c", "user.email=f@x", "-c", "user.name=fixture"]
        subprocess.run(["git", "init", "-q", str(work)], check=True)
        subprocess.run(git + ["-C", str(work), "add", "-A"], check=True)
        subprocess.run(git + ["-C", str(work), "commit", "-q", "-m", "fixture"], check=True)
        subprocess.run([str(binary), "analyze", str(work)], capture_output=True, check=True)
        out = subprocess.run([str(binary), "context", str(work), "--nodes", ",".join(paths), "--json"],
                             capture_output=True, text=True, check=True).stdout
        return {n["path"]: n["source"] for n in json.loads(out)["nodes"]}


def delta(changed=(), added=(), removed=(), edges_added=(), edges_removed=()):
    return {"nodes": {"changed": list(changed), "added": list(added), "removed": list(removed)},
            "edges": {"added": [list(e) for e in edges_added], "removed": [list(e) for e in edges_removed]}}


def plan(steps, on_failure="rollback_plan"):
    return {"plan_version": 2, "plan_id": "verified-edit-fixture", "intent": "fixture",
            "base_commit": "<HEAD>", "on_failure": on_failure, "steps": steps}


def step(sid, edits, verify=None):
    s = {"id": sid, "description": "", "edits": edits, "checks": []}
    if verify is not None:
        s["verify"] = verify
    return s


def replace(node, body):
    return {"node": node, "replace_node": body}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", required=True)
    binary = Path(ap.parse_args().binary).resolve(strict=True)
    if ROOT.exists():
        shutil.rmtree(ROOT)
    for name, files in FIXTURES.items():
        for rel, text in files.items():
            (ROOT / name / rel).parent.mkdir(parents=True, exist_ok=True)
            (ROOT / name / rel).write_text(text)
    ov = sources(binary, "overload", [TRAIT, INHERENT])
    oi = sources(binary, "overload-identical", [TRAIT, INHERENT])
    assert oi[TRAIT] == oi[INHERENT], "the identical-bodies fixture must have byte-identical node sources"
    ba = sources(binary, "basic", [A, B_, H])
    py = sources(binary, "pybasic", [PYA, PYB])
    trait_v2 = ov[TRAIT].replace('"trait"', '"trait v2"')
    inh_v2 = ov[INHERENT].replace('"inherent"', '"inherent v2"')
    same_trait_v2 = oi[TRAIT].replace('"same"', '"same trait v2"')
    alpha_v2 = ba[A].replace("1", "10")
    alpha_calls = ba[A].replace("    1\n", "    helper() + 1\n")
    py_alpha_v2 = py[PYA].replace("return 1", "return 10")
    cases = []

    def case(cid, fixture, plan_doc, outcome, category=None, step_id="s1", note=""):
        cases.append({"id": cid, "fixture": fixture, "plan": f"plans/{cid}.json", "expected": {
            "outcome": outcome, "category": category, "failed_step": step_id if outcome == "refused" else None},
            "note": note})
        (ROOT / "plans").mkdir(parents=True, exist_ok=True)
        (ROOT / "plans" / f"{cid}.json").write_text(json.dumps(plan_doc, indent=2) + "\n")

    v = lambda base, d: {"baseline": base, "delta": d}
    # --- wrong overload (distinct bodies)
    case("wrong-overload-by-fingerprint", "overload", plan([step("s1", [replace(INHERENT, inh_v2)], v(
        {INHERENT: fp(TRAIT, ov[TRAIT])}, delta(changed=[TRAIT])))]), "refused", "wrong_overload",
        note="addresses the inherent method but pins the trait impl's fingerprint")
    case("wrong-overload-by-delta", "overload", plan([step("s1", [replace(INHERENT, inh_v2)], v(
        {INHERENT: fp(INHERENT, ov[INHERENT])}, delta(changed=[TRAIT])))]), "refused", "wrong_overload",
        note="baseline is true of the addressed node but the declared change names the same-named sibling")
    case("correct-overload-applies", "overload", plan([step("s1", [replace(TRAIT, trait_v2)], v(
        {TRAIT: fp(TRAIT, ov[TRAIT])}, delta(changed=[TRAIT])))]), "committed",
        note="the counterpart of the wrong-overload plans: the trait impl, its own fingerprint and delta")
    # --- wrong overload with identical bodies (path binding)
    case("identical-bodies-wrong-by-fingerprint", "overload-identical", plan([step("s1", [replace(
        INHERENT, oi[INHERENT].replace('"same"', '"same v2"'))], v({INHERENT: fp(TRAIT, oi[TRAIT])},
        delta(changed=[TRAIT])))]), "refused", "wrong_overload",
        note="both bodies are identical; only the path-bound fingerprint tells the overloads apart")
    case("identical-bodies-wrong-by-delta", "overload-identical", plan([step("s1", [replace(
        INHERENT, oi[INHERENT].replace('"same"', '"same v2"'))], v({INHERENT: fp(INHERENT, oi[INHERENT])},
        delta(changed=[TRAIT])))]), "refused", "wrong_overload")
    case("identical-bodies-correct-applies", "overload-identical", plan([step("s1", [replace(
        TRAIT, same_trait_v2)], v({TRAIT: fp(TRAIT, oi[TRAIT])}, delta(changed=[TRAIT])))]), "committed")
    # --- stale input
    case("stale-baseline", "basic", plan([step("s1", [replace(A, alpha_v2)], v(
        {A: fp(A, ba[A].replace("1", "7"))}, delta(changed=[A])))]), "refused", "stale_input",
        note="the pinned fingerprint is of source that is not the current source")
    # --- unexpected / missing edge
    case("unexpected-edge-refused", "basic", plan([step("s1", [replace(A, alpha_calls)], v(
        {A: fp(A, ba[A])}, delta(changed=[A])))]), "refused", "unexpected_edge",
        note="the replacement adds a call to helper, a Calls edge the delta does not declare")
    case("declared-edge-applies", "basic", plan([step("s1", [replace(A, alpha_calls)], v(
        {A: fp(A, ba[A])}, delta(changed=[A], edges_added=[(A, H, "Calls")])))]), "committed")
    # --- delta mismatch
    case("declared-extra-change-refused", "basic", plan([step("s1", [replace(A, alpha_v2)], v(
        {A: fp(A, ba[A])}, delta(changed=[A, B_])))]), "refused", "delta_mismatch",
        note="the delta promises beta changes too; it does not")
    # --- no-op replacement has an empty delta
    case("no-op-replacement-applies", "basic", plan([step("s1", [replace(A, ba[A])], v(
        {A: fp(A, ba[A])}, delta()))]), "committed", note="identical replacement: empty delta")
    # --- insufficient evidence
    case("insufficient-missing-baseline", "basic", plan([step("s1", [replace(A, alpha_v2)], v(
        {}, delta(changed=[A])))]), "refused", "insufficient_evidence")
    case("insufficient-missing-edge-delta", "basic", plan([step("s1", [replace(A, alpha_v2)], {
        "baseline": {A: fp(A, ba[A])}, "delta": {"nodes": {"changed": [A], "added": [], "removed": []}}})]),
        "refused", "insufficient_evidence", note="a complete delta must state edges explicitly")
    case("insufficient-text-edit-in-verified-step", "basic", plan([step("s1", [
        replace(A, alpha_v2), {"path": "src/lib.rs", "match": "    3\n", "replace": "    4\n", "occurrences": 1}], v(
        {A: fp(A, ba[A])}, delta(changed=[A])))]), "refused", "insufficient_evidence")
    case("insufficient-unsupported-edit-kind", "basic", plan([step("s1", [{"node": B_, "rename_node": "gamma"}],
        v({B_: fp(B_, ba[B_])}, delta(removed=[B_], added=["crate::lib::gamma"])))]), "refused",
        "insufficient_evidence", note="rename_node is outside the certified scope in v1")
    # --- compatibility: no verify block
    case("uncertified-plan-runs-as-before", "basic", plan([step("s1", [replace(A, alpha_v2)])]),
         "committed-uncertified", note="no verify block: runs exactly as today and is reported uncertified")
    # --- multi-step rollback
    case("second-step-refused-rolls-back", "basic", plan([
        step("s1", [replace(A, alpha_v2)], v({A: fp(A, ba[A])}, delta(changed=[A]))),
        step("s2", [replace(B_, ba[B_].replace("    2\n", "    20\n"))], v({B_: fp(B_, ba[B_])}, delta(changed=[A])))]),
        "refused", "delta_mismatch", step_id="s2", note="step 1 commits, step 2 is refused, rollback_plan restores the base")
    # --- Python
    case("python-correct-applies", "pybasic", plan([step("s1", [replace(PYA, py_alpha_v2)], v(
        {PYA: fp(PYA, py[PYA])}, delta(changed=[PYA])))]), "committed")
    case("python-stale-refused", "pybasic", plan([step("s1", [replace(PYA, py_alpha_v2)], v(
        {PYA: fp(PYA, py[PYA].replace("1", "5"))}, delta(changed=[PYA])))]), "refused", "stale_input")
    manifest = {"schema_version": 1, "policy": "docs/verified-edits-policy.md",
                "fingerprint": "sha256(b'girder-node-fingerprint-v1\\0' + path + b'\\0' + source)",
                "placeholder": "base_commit '<HEAD>' is replaced by the harness",
                "fixtures": {n: sorted(f) for n, f in FIXTURES.items()}, "cases": cases}
    # Pin every generated file so the frozen inputs cannot drift unnoticed.
    manifest["files_sha256"] = {
        str(f.relative_to(ROOT)): hashlib.sha256(f.read_bytes()).hexdigest()
        for f in sorted(ROOT.rglob("*")) if f.is_file() and f.name != "manifest.json"}
    (ROOT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(len(cases), "cases")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
