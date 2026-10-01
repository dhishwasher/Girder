# CI-gate design: captured evidence

Two real `girder test-impact demo-project --classified --quiet` runs,
captured for `docs/ci-gate-design.md` and `docs/ci-pr-example.md`, not
hand-written.

- **Binary:** `girder 0.2.7`, debug build at
  `/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder`.
- **HEAD at capture time:** `0e6b49eb4959964caacdf8dd756461a24eb514ad`.
- **License key used:** the `#[cfg(debug_assertions)]`-only debug test
  key from `crates/aether-app/src/project/license.rs`'s own test module
  (`DEBUG_TEST_LICENSE_KEY`), set only as a local `GIRDER_LICENSE_KEY`
  environment variable for these two commands, never written to any
  file. A release binary does not compile the public key this token
  verifies against and rejects it (see
  `release_build_rejects_the_debug_test_key` in `license.rs`'s own
  tests) — it has no value outside a local debug build and is not a
  secret worth protecting, but it is still never put in a doc or
  committed file, per instruction.

## `classified-clean-tree.json`

Captured with `demo-project/` byte-identical to `HEAD` (confirmed via
`git status --short demo-project/` showing no output immediately
before the run). Result: `must=0, may=0, unknown=0, boundaries=0`.

## `classified-one-line-edit.json`

Captured after changing exactly one line,
`demo-project/greeter.py::farewell`'s return value from
`f"goodbye {name}"` to `f"farewell, {name}"`, then reverted immediately
after capture with `git checkout -- demo-project/greeter.py` (confirmed
clean afterward). Result: `must=0, may=0, unknown=6, boundaries=82`
(`coverage_gap: 14, missing_or_invalid_evidence: 6,
unresolved_call_site: 62`).

## What this pair establishes

The empty/non-empty contrast is diff-scoped, not a graph-wide constant:
an unmodified tree against its own `HEAD` produces zero boundaries:
nothing to analyze, nothing reported. A single one-line edit in a small,
four-function fixture produces 82. This is real, paired evidence (not
assumption) for two claims in the CI-gate design: (1) a CI checkout with
no uncommitted changes genuinely does produce an empty, zero-boundary
result under the current (no-`--base`) `test-impact` — not a
hypothetical risk, an observed one; and (2) `boundaries.count` tracks
the diff, so a per-PR threshold policy is not degenerate against a
repo-wide constant.
