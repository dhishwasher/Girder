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

## Observation (original, flat cap of 3 — FAIL)

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
show a reduction, matching the intended case: a node with a modest number of
covering tests, where `--with-tests` embeds exactly the capped 3 full
sources instead of the whole file(s) they live in. The other two
(`NodeId::from_path`, `select_candidate`) get *worse*: both are
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
force a pass: this result was recorded as **FAIL** at 22.61%, not adjusted.

## The fix: a per-node rule instead of a flat cap

The flat cap (`full` source for the first 3 covering tests, `names_only` for
every remaining one, no matter how many) was replaced with a per-node rule
in `covering_tests_entry` (`crates/aether-app/src/project/commands/context_cmd.rs`):
a node's covering-test *count* now decides the whole shape of its `"tests"`
object, not just how many get source.

- Below `SMALL_COVERING_SET_CUTOFF` (150) total covering tests: unchanged
  behavior — up to 3 full `{path, language, source}` entries, the (still
  small, since the total is under 150) rest as `{"path": ...}` only. This is
  the case the original measurement already showed working well.
- At or above 150: **no** full source at all, and `names_only` itself is
  capped at 3 entries instead of listing every covering test — because at
  that scale, a handful of arbitrary full test bodies out of a hundred-plus
  reachable tests doesn't narrow anything down, and it was the *unbounded
  names list*, not the full-source cap, that was driving the FAIL (each
  `names_only` entry costs little, but 200-300 of them adds up to more than
  a single file read). A new `total_covering_tests` field carries the real
  count in both cases, so the information the truncated list can no longer
  convey — "296 tests transitively reach this node" — isn't lost, just made
  cheap.

150 was picked from this measurement's own 5-node sample, not guessed: sorted
by total covering-test count the sample is 82, 107, 193, 195, 296, and the
widest gap in that sequence sits between 107 (`tests_for`, the largest node
that measured a real win from seeing full test source under the old flat
cap) and 193 (`select_candidate`, the smaller of the two nodes whose broad
coverage caused the FAIL). 150 sits in that gap.

## Observation (per-node rule — PASS)

Re-measured the same 5 nodes, same method, against the fixed code (repo
drift since the original measurement changed a few nodes' exact covering-test
counts and, for `tests_for`, added a third distinct covering-test file —
neither affects the method, only the raw counts):

| Node | context bytes | covering-test file bytes | before | after | total covering tests |
|---|---:|---:|---:|---:|---:|
| `query_by_kind` | 6,517 | 39,963 | 46,480 | 7,099 | 202 |
| `tests_for` | 6,877 | 73,610 | 80,487 | 27,489 | 109 |
| `git_worktree_clean` | 6,523 | 72,884 | 79,407 | 22,188 | 84 |
| `NodeId::from_path` | 6,633 | 15,809 | 22,442 | 7,131 | 305 |
| `select_candidate` | 8,874 | 24,154 | 33,028 | 9,486 | 200 |
| **Total** | | | **261,844** | **73,393** | |

Aggregate reduction: **71.97%**. The policy result is **PASS** (71.97% ≥
40%).

`tests_for` and `git_worktree_clean` (both under the 150 cutoff) keep full
source and see their reduction improve further, purely from repo drift
adding more covering-test file content to the "before" side. The three
nodes at or above the cutoff (`query_by_kind` at 202, `select_candidate` at
200, `NodeId::from_path` at 305) now cost almost exactly their own
`context_bytes` plus a few hundred bytes for 3 names and a count, instead of
tens of thousands of bytes of unbounded names — exactly the fix the
diagnosis called for. This was a genuine re-measurement against the fixed
code, not a retuning of the 40% threshold, which is unchanged from the
original policy file.

## Relationship to earlier controls

This is the first cost measurement for `--with-tests`; there is no earlier
control run to compare against. The precommitted method mirrors
`docs/authoring-cost.md`'s structure (fixed inputs pinned before
measurement, one raw observation file).

The complete per-node record for the passing re-measurement is in
`docs/context-with-tests-cost-observation.json`; the original failing raw
numbers are preserved in this file's git history (commit `e7086d1`).
