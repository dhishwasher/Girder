# `girder names` lookup cost

This measurement compares an exact-identifier lookup done via `grep` against
the same lookup done via `girder names`. It records an observed outcome,
not an architectural claim.

## Precommitted corrected method

- Policy: `docs/names-cost-policy.json`.
- Source commit: `b37731f218086c5d421cd8eae1534559659969e9` (the commit that
  added `girder names`).
- Ten identifiers, drawn from real exported functions/types already in this
  repo, fixed in the policy file before measurement: `build_from_dir_with_config`,
  `ProjectConfig`, `SemanticGraph`, `NodeId`, `invalid_input`,
  `git_worktree_clean`, `resolve_calls`, `tests_for`, `query_by_kind`,
  `select_candidate`.
- Before: `grep -rn "\b<identifier>\b" --include="*.rs" --include="*.py" .`
  — a word-boundary exact match, the honest analog to what `names` replaces.
  Not `girder search`, which is substring over name and path and answers a
  different, noisier question.
- After: `girder names . <identifier> --json`.
- Metric: raw stdout byte count of each command, per identifier, summed.
- Threshold: aggregate reduction ≥ 40%, matching the bar already established
  in `docs/authoring-cost.md`.

## Observation

| Identifier | grep bytes | `names` bytes |
|---|---:|---:|
| `build_from_dir_with_config` | 3,179 | 150 |
| `ProjectConfig` | 12,573 | 133 |
| `SemanticGraph` | 30,292 | 123 |
| `NodeId` | 51,727 | 117 |
| `invalid_input` | 13,985 | 1,052 |
| `git_worktree_clean` | 331 | 139 |
| `resolve_calls` | 318 | 144 |
| `tests_for` | 5,258 | 287 |
| `query_by_kind` | 2,014 | 144 |
| `select_candidate` | 319 | 133 |
| **Total** | **119,996** | **2,422** |

Aggregate reduction: **97.98%**. The gap is driven by `grep`'s nature — it
returns every occurrence of an identifier (declaration, every call site,
every mention in a comment or string), while `names` returns only the
declarations, which is what an exact-name lookup is actually asking for.
`invalid_input` shows the smallest reduction (92.5% for that identifier
alone) because it has few declarations relative to call sites, so even its
`names` output remains proportionally the largest of the ten.

The policy result is **PASS** (97.98% ≥ 40%).

## Relationship to earlier controls

This is the first cost measurement for `girder names`; there is no earlier
control run to compare against. The precommitted method mirrors
`docs/authoring-cost.md`'s structure (fixed inputs pinned before
measurement, one raw observation file) for consistency with the one other
cost measurement in this repo.

The complete per-identifier record is in `docs/names-cost-observation.json`.
