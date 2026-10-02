# TypeScript audit-size extension

Frozen before selecting or reading additional sites, and before further
TypeScript resolver changes. Preserve all 105 original entries and labels:
97 actual calls, eight noncalls. The minimum remains 100 actual calls.

- Reuse the four exact archives in `docs/stage3-typescript-corpus.json`, the
  existing TypeScript selector, and the original rubric with both addenda.
  Compiler extraction remains scoped to src, LICENSE.txt and package.json;
  the other three archives remain whole-repository. No new repository/language.
- Acquire missing pinned archives separately from the measurement, verifying
  their existing size and SHA. All extraction, selection and scoring are offline.
- Exclude every original (package, file, line). Select a reserve of 64 using
  the existing shape-stratified sampler with seed 20261002. That sampler sorts
  its output by package; shuffle the reserve with a fresh Random(20261002)
  before freezing its traversal order, so the short extension is not simply
  the first package alphabetically. Do not inspect Girder answers to select it.
- Commit the reserve and hashes before labeling. Traverse its stored order;
  retain noncall exclusions and stop immediately at 100 actual old-plus-new
  calls. No skipped, reordered or discarded sites. If exhausted, publish the
  shortage before extending. Record source excerpts and rationale for every
  added entry; every Must needs a checked source target and rebinding review.
- Commit labels before scoring. Pin the candidate binary and scorer hashes.
  Analyze/inspect all four snapshots serially; score original and combined
  cohorts on the same extracts, retaining raw outputs and every changed answer.
- Acceptance is unchanged: at least 100 actual scored calls, all entries
  accounted for, zero overclaims/unsafe exclusions, and nonempty Must precision
  exactly 1.000. Undefined Must precision fails. Corpus improvement and common
  gates are additional language-completion requirements, not waived by this
  extension. The existing unresolved-origin corpus failure stays published.
- This is a before-observation and sample repair, not a completed language
  checkpoint. Do not rerun Cargo gates for source-free sampling work; run all
  four once on a subsequent implementation candidate at its language checkpoint.
  No threshold, label, scorer rule or old observation changes to improve results.
