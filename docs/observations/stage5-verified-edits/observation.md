# Stage 5 verified edits: after-observation

Candidate: `300fff7` (implementation `0d1cc38`, guard unit tests, authoring surface, harness lint fix).
Policy: [verified-edits-policy.md](../../verified-edits-policy.md), frozen in `9809713`/`b9d35da` before any
implementation. Fixtures: `fixtures/verified-edits/v1/` (19 plans, 4 fixture projects, 26 pinned file hashes).

## Result against the precommitted criterion

- All 19 frozen plans meet their expected outcome through the real `girder plan run`
  (`crates/aether-app/tests/verified_edits.rs::every_frozen_plan_meets_its_expected_outcome`).
- The wrong-overload fixture is refused (by fingerprint pre-apply, and by declared delta post-apply) with every
  project file hashed before and after identical (excluding `.git` and `.girder/reports`, which `plan run`
  always writes), no saved graph, and no journal. The correct-target counterpart commits, and the other
  overload is byte-untouched. The same holds for the identical-bodies variants, which only the path-bound
  fingerprint can tell apart.
- Stale input, unexpected edge, a declared change that does not happen, four insufficient-evidence cases, a
  two-step rollback, and the Python pair behave as frozen. An uncertified (no `verify`) plan still runs and is
  reported uncertified; v1 plans are untouched.
- The report keeps `certification` (with the actual delta) and "predicted, not execution evidence" impact
  separate from executed `checks` (asserted in `the_report_separates_certification_and_predicted_impact_from_executed_checks`).
- Gates on `300fff7`: `gates-300fff7/` has `cargo test --workspace -j1`, clippy `-D warnings`, `fmt --check`
  and the npm tests, all exit 0. A first candidate `beae7c2` failed clippy (two collapsible ifs in the test
  harness); that failure is kept in `gates-beae7c2/` with `FAILED.txt`.

## Guard proof (mutation checks)

Each guard was disabled, the tests were run, and the tree restored from git or a byte-identical backup.
Every mutation made at least one test fail:

- M1 to M6 against the frozen fixtures (`mutations-m1-m6.txt`): pre-apply sibling rule, post-apply wrong overload,
  edge comparison, baseline check, Module-node inclusion, path-free fingerprint.
- M7 to M9 against unit tests (`mutations-m7-m9.txt`): projection byte-exactness, incremental-vs-cold comparison,
  parse/duplicate-path gap. No frozen fixture reaches these three guards, so they are proven by the unit tests in
  `verify.rs` using a doctored workspace.

## Authoring surface

`girder context` without `--source-only` adds a `fingerprint` per node through the same function the verifier
uses; `--source-only` still returns exactly `{path, language, source}` (test asserts the key set). The emitted
plan schema documents the optional `verify` block.

## Honest limits

- Certified scope is `replace_node` on Rust and Python only. Rename, delete, insert and text edits inside a
  certified step are refused as `insufficient_evidence`.
- The delta is structural; it does not prove behavior. `Module` nodes are excluded from the node comparison
  because their source is the whole file; soundness there rests on the projection-exactness check.
- Edits that change an unresolved call change no `Calls` edge and are invisible to the delta.
- Predicted reachability is conservative and dominated by Unknown on real code.
- No live agent authored a certified plan; fixtures are hand-authored with independently computed fingerprints.
- Gates ran on this VM only; the real CI result is recorded in the roadmap after the push.
