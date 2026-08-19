# `bitcode context --with-tests` cost

This measurement compares the two-step "get context, then open each
covering test's file" workflow against one `bitcode context --with-tests`
call. It records an observed outcome, not an architectural claim.

## Precommitted corrected method

- Policy: `docs/context-with-tests-cost-policy.json`.
- Source commit: `b38893b6c2600ebaae9c177404b7996a4e42079e` (the commit that
  added `--with-tests`).
- Five nodes, fixed in the policy file before measurement, chosen for
  diversity across crates: `SemanticGraph::query_by_kind`,
  `SemanticGraph::tests_for`, `git_worktree_clean`, `NodeId::from_path`,
  `select_candidate`.
- Before: `bitcode context <dir> --nodes <node> --json` (no `--with-tests`)
  stdout bytes, plus the full byte size of every distinct source file
  containing one of that node's (at most 3, matching the `--with-tests` cap)
  covering tests.
- After: `bitcode context <dir> --nodes <node> --json --with-tests` stdout
  bytes, in one call.
- Threshold: aggregate reduction ≥ 40%, matching the bar carried over from
  `docs/authoring-cost.md` and used for `docs/names-cost.md`.

## Observation

| Node | context bytes | covering-test file bytes | before | after |
|---|---:|---:|---:|---:|
| `query_by_kind` | 6,518 | 39,963 | 46,481 | 39,762 |
| `tests_for` | 6,878 | 55,750 | 62,628 | 27,163 |
| `git_worktree_clean` | 6,524 | 68,969 | 75,493 | 21,863 |
| `NodeId::from_path` | 6,634 | 15,809 | 22,443 | 56,053 |
| `select_candidate` | 8,875 | 24,154 | 33,029 | 40,939 |
| **Total** | | | **240,074** | **185,780** |

Aggregate reduction: **22.61%**. The policy result is **FAIL** (22.61% <
40%).

Three of the five nodes (`tests_for`, `git_worktree_clean`, `query_by_kind`)
show a large reduction (57-71%), matching the intended case: a node with a
modest number of covering tests, where `--with-tests` embeds exactly the
capped 3 full sources instead of the whole file(s) they live in. The other
two (`NodeId::from_path`, `select_candidate`) get *worse*: both are
foundational, high-fan-in utilities reachable from 193-296 tests via this
codebase's dense call graph. `--with-tests` reports every one of those as a
`names_only` entry (not just the capped 3 full ones), and at nearly 300
entries the pretty-printed JSON overhead for that list alone exceeds the
before workflow's single-file read. This is not a bug in the cap (the cap on
*full* sources held correctly in every case — never more than 3) — it is the
`names_only` tail growing with real transitive test-reachability, which the
before workflow has no equivalent of at all: reading one covering test's
file never told an agent that 296 tests transitively reach this node, so the
two are not measuring quite the same capability once fan-in gets this high.

Per the project's standing rule against tuning a precommitted threshold to
force a pass: this result is recorded as **FAIL** at 22.61%, not adjusted.
`--with-tests` clearly earns its place for the common case (a specific,
moderately-tested function) but this sample includes two adversarial,
maximally-connected utility functions that are not representative of what
an agent would typically pin `--nodes` to, and the aggregate reflects that.

## Relationship to earlier controls

This is the first cost measurement for `--with-tests`; there is no earlier
control run to compare against. The precommitted method mirrors
`docs/authoring-cost.md`'s structure (fixed inputs pinned before
measurement, one raw observation file).

The complete per-node record is in
`docs/context-with-tests-cost-observation.json`.
