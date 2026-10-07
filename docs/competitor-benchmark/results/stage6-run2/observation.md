# Stage 6 recurring comparison: observation (run 2, policy revision 13)

One fresh, serial campaign: Girder 0.4.0 (normal and watch) against ripwire, codebase-memory-mcp and
code-review-graph, on the four modest fixtures (Rust, Python, TypeScript/TSX, Go). 20 campaigns, all
`COMPLETE`, all exit 0. The previous published campaign ([`../modest-final/`](../modest-final/)) ran
2026-09-11/12, so the 30-day limit is not an issue for this one; the next campaign is not due before
2026-11-06.

## Identity

| Item | Value |
| --- | --- |
| Girder candidate | release `v0.4.0`, commit `b38c427b7451814b5ea5a7fe5368ba7d806a2efe`; binary sha256 `81239a6be95992cd028f3e9bbeef51160f6c0a0eee7cdb0b27e041d9706652b7` (archive `80f24114a453dd023941abbfbc9dd47786f662af801aad7ca84c7d589309d555`, checked against the published `.sha256`); `girder --version` reports 0.4.0 |
| Harness commit (recorded in every result) | `e443952bf97e8aaa3f4e99a3bd5d1c15df086276` |
| Policy | `girder-competitor-benchmark-v1-revision-13`, sha256 `7a3b8e5308c7505d909fe3dd53a26d6e6537208e4a1220f2177e51c85e3505d2`; freeze manifest sha256 `458edf7a585c7e8f8cb570b044c8eef9a5c6ce09d8e8f7fb00fedff5676a8eff` |
| Pins | [`../../stage6-freeze-v2.json`](../../stage6-freeze-v2.json) (versions, commits, binary hashes, input and adapter hashes, limits), frozen and committed before the run |
| Externals | ripwire 0.5.0, codebase-memory-mcp 0.10.8 (run from a fresh temp copy), code-review-graph 2.3.8 (hash-locked venv in `/tmp`; needed pip 26.2.1, because pip 23.0.1 resolved `py-key-value-aio` 0.4.6 against the 0.4.5 lock) |
| Host | Linux x86_64; serial, one campaign at a time; memory floor 768 MiB checked before each |
| Driver | `tools/stage6_run_campaign.sh`; per-campaign limit 1800 s |

## Run 1 is invalid and is published, not deleted

Run 1 used the same Girder binary but the revision 12 Girder adapter, which stamped every Girder record
`version 0.2.6`, `commit 3f7d5ee` from a class constant and never checked the binary it launched. It would
have published 0.4.0 results labelled as 0.2.6. Found before any claim was drawn; the adapter now reads
`girder --version` and refuses unknown releases (tests added), policy revision 13 records the change, and
everything was refrozen and rerun. See [`../stage6-run1-INVALID/INVALID.md`](../stage6-run1-INVALID/INVALID.md).

## Result

Reports: [`report.md`](report.md), [`summary.csv`](summary.csv), [`summary.json`](summary.json),
[`deltas.md`](deltas.md), [`deltas.json`](deltas.json). Raw evidence: [`raw/`](raw/) with `artifacts.sha256`.

- **External runners reproduced the baseline exactly.** ripwire, codebase-memory-mcp and code-review-graph
  differ from the published baseline in **zero** metric cells (a version-identical rerun), so
  run-to-run variation on these fixtures is nil and any Girder difference below is a real change.
- **Girder 0.4.0 versus the 0.2.6 baseline: of 200 metric cells compared across all five runners, 188 are
  unchanged and 12 changed, all in Girder (6 in normal mode, 6 in watch mode).** Each mode's six are test
  selection on the Go, Python and Rust fixtures:
  - `tests_recall` 0.5 to **1.0** (improvement);
  - `tests_precision` 1.0 to **0.667** (**regression**).

  That is the intended trade: 0.4.0 selects every test it cannot prove unreachable (the conservative
  Must/May/Unknown union), so it stops missing a relevant test but also selects an unrelated one. Definition,
  callers, callees and impact metrics are unchanged. TypeScript has no changed cell.
- **Known weaknesses are unchanged, because those cells are unchanged:** Girder's callers and impact recall
  are still 0.5 on the Rust and Python fixtures, and Go definition lookup is still WRONG on the modest
  fixture (also WRONG in the baseline). See `summary.csv` for every cell.
- **Resource failures:** none in this run (every campaign completed). GitNexus was not rerun; its baseline
  `RESOURCE_BLOCKED` result (1.08 GB peak, above the 1 GiB limit) is retained in reporting. Peak RSS per
  campaign is in `summary.csv` (`peak_rss_bytes`).

## Honest limits

- Modest fixtures only (20 base tasks per runner). Results are specific to this setup and these pins, not general
  claims about the products.
- The reported precision/recall change is a trade, not a clean win; the regression is stated as one.
- The Girder candidate is the published release binary, not a build of the current branch head, so it does
  not include changes made after `v0.4.0` (for example the TypeScript function-collision fix).
- A reviewer correction: the first draft of the Girder rows came from the wrong adapter identity (run 1);
  none of those numbers are used.
- Gate logs for the committed candidate are in `../../../observations/stage6-comparative/` (see the roadmap
  checkpoint for the commit and CI run).
