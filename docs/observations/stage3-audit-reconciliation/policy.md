# Prospective correction of audit sample size

Frozen before extension sites are labeled or current Girder answers are observed.
This supplements, never rewrites, the old Rust/Python audits. The old completion
claims failed the original >=100 actual-call-site requirement (52/85 scored).
A new audit cannot retroactively precommit the earlier implementation changes.
It can establish a correctly sized current-candidate audit prospectively.

- Preserve every original entry and label (Rust labeled-v2, Python original).
  Non-call entries remain in the ledger but NEVER count toward the minimum.
- Reuse the pinned three-repository snapshots per language and existing independent
  selectors/rubrics. Do not select sites from Girder's own output.
- Seed 20261001; freeze a 256-entry Rust and 128-entry Python reserve, excluding
  any original (repository,file,line). Traverse each reserve in its stored order.
- Read and label each successive site against source. Preserve non-call exclusions
  with reasons. Stop immediately when old plus new ACTUAL call sites reaches 100.
  No skipped/reordered sites, no selecting by observed class, no discarded failure.
  If a reserve is exhausted, publish the shortage before extending the policy.
- Publish source excerpts and rationales, freeze labels in a separate commit, then
  freshly analyze and inspect all snapshots serially using the pinned binary.
- Score the old cohort and combined cohort separately on the same fresh extracts.
  Preserve all results, including failures. Compare old-cohort answers with the
  previous committed observation; name every changed site.
- Gate: >=100 scored actual sites, all selected entries accounted for, 0 unsound
  cells, nonempty Must precision exactly 1.000. Undefined precision fails.
  Keep corpus-improvement evidence from its original fixed cohort separately;
  additional audit samples do not revise prior improvement denominators.
- Specifically recheck Rust's serde_json serialize_element node identities after
  the collision fix. Record exact paths and distinct node IDs, not just counts.
- Missing artifacts, timeouts, or scorer failures are recorded as unavailable.
  No new resolver work or label changes in response to scores in this campaign.
- Use existing dependencies only, offline workloads, serial execution, and the
  four common stage gates. No subagents or work alongside Cargo.
