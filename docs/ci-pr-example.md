# Using `test-impact` from CI or a PR — what works today, what doesn't yet

This documents the real, current behavior of `girder test-impact`, run
against this repository's own `demo-project/` fixture with the real
binary, not a hand-written example. It also states plainly what does
not work yet, rather than showing a CI snippet that would silently do
nothing.

## The command that exists today only diffs the working tree against HEAD

`crates/aether-app/src/project/git.rs::semantic_changed_impact_with_config`
calls `build_baseline_graph_with_config(root, "HEAD", config)` — the
baseline ref is hardcoded to `"HEAD"`, not configurable. There is no
`--base <ref>` flag anywhere in `test_impact.rs`'s argument parsing
(verified by reading it directly).

This matters for CI specifically: a typical `actions/checkout` on a pull
request leaves the working tree identical to `HEAD` — there is no
uncommitted change for `test-impact` to see. **Verified directly, not
assumed:** run `test-impact --classified` against `demo-project/` with
nothing modified (confirmed byte-identical to `HEAD` via `git status`
immediately before) and it reports `must=0, may=0, unknown=0,
boundaries=0` — a real empty result, captured in
`docs/observations/ci-gate-design/classified-clean-tree.json`. Run it in
a GitHub Actions job on a clean PR checkout today and you get exactly
this: nothing, not "these tests may be affected by this PR." **A
`--base <ref>` flag does not exist yet** — it is part of the CI-gate
design (`docs/ci-gate-design.md`), not yet built.

## What DOES work today: local, working-tree-vs-HEAD usage

This is real, captured output — see
`docs/observations/ci-gate-design/README.md` for exact provenance
(binary version, HEAD commit, the precise edit). A one-line edit to
`demo-project/greeter.py` (`farewell`'s return string) was made,
captured, and reverted immediately after.

```bash
girder test-impact demo-project --classified --quiet
```

produced (full, unedited output in
`docs/observations/ci-gate-design/classified-one-line-edit.json`;
`boundaries.items`, 82 entries, omitted here for length):

```json
{
  "schema_version": 1,
  "must": { "count": 0, "paths": [], "truncated": false },
  "may": { "count": 0, "paths": [], "truncated": false },
  "unknown": {
    "count": 6,
    "paths": [
      "crate::test_dupefinder::FindDuplicatesTests::test_groups_files_with_identical_content",
      "crate::test_dupefinder::FindDuplicatesTests::test_returns_nothing_when_all_files_differ",
      "crate::test_dupefinder::HashFileTests::test_same_content_hashes_the_same",
      "crate::test_greeter::GreeterTests::test_farewell",
      "crate::test_greeter::GreeterTests::test_format_greeting",
      "crate::test_greeter::GreeterTests::test_shout_greeting_delegates_to_format_greeting"
    ],
    "truncated": false
  },
  "boundaries": {
    "count": 82,
    "by_category": {
      "coverage_gap": 14,
      "missing_or_invalid_evidence": 6,
      "unresolved_call_site": 62
    }
  }
}
```

**Read this result carefully, because it is the exact reason a naive CI
policy is dangerous:** for a one-line string-literal edit to a single
function, Girder proved **zero** Must-reachable tests and put **every**
affected test in `unknown`, backed by 82 unresolved-evidence *boundary
entries* in this small fixture — paired against the clean tree's
verified 0, confirming this count tracks the diff, not the whole graph
(`docs/observations/ci-gate-design/README.md`). A different,
larger-scale measurement exists for a different quantity — not directly
comparable, but pointing the same direction: `docs/observations/
stage3-typescript-audit/before-observation-addendum-9.md` documents 472
*unknown-classified graph nodes* (not boundary entries) from one
unresolved-dispatch edit in a real open-source repository. Different
units, same conclusion: on real code, Unknown is frequently the common
case, not the exception.

## The CI mistake this rules out

`CLAUDE.md`'s own interactive guidance uses:

```bash
T=$(girder test-impact . --quiet); if [ -n "$T" ]; then cargo test -- $T; else echo "no impacted tests"; fi
```

**Do not put the `else echo "no impacted tests"` branch in CI.** An
empty selection with Unknown boundaries present does not mean nothing
needs testing — the frozen policy's own wording is: "Empty output with
Unknown boundaries never means no tests need running"
(`docs/call-classification-policy.md`). In CI, an empty selection, or
any run that reports Unknown boundaries, should fall back to running the
full test suite, not skip testing. This is exactly the gap a future
paid CI-gate command is meant to enforce correctly and auditably instead
of leaving every team to get this right in ad hoc shell.

## What a real CI example will need once `--base` ships

A working GitHub Actions step will need:

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0   # test-impact needs the base ref's objects, not just HEAD
```

`fetch-depth: 0` (or at minimum fetching the specific base SHA) is
required because `build_baseline_graph_with_config` reads source files
out of git objects at the baseline commit (`git show <ref>:<path>`) —
the default `fetch-depth: 1` checkout does not have those objects for
any ref other than the one checked out.

This document will be updated with a real, captured CI run once
`--base <ref>` ships — not before, so it never shows a command that
silently does nothing.
