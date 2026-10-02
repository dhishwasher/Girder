# Rust operator boundary correction

Precommitted after the failed expanded audit at `25bdb9a`, before implementation.
The initial campaign, labels, reserves, and scorers remain immutable. This is a
separate implementation/candidate observation, not a replacement result.

Problem: a whole-file warning leaves generic addition and derived equality
without a call-site claim. Add conservative, site-specific Unknown evidence for
Rust expressions capable of invoking operator traits. Do not infer a concrete
target from syntax or promote an operator to Must. Keep the whole-file warning
for implicit drop and other unmodeled behavior. Preserve explicit nested calls.

Validation: regression coverage for the two exposed shapes and harder nested,
assignment, unary/deref and indexing shapes as supported by the Rust grammar.
Built-in-only short-circuit boolean control flow must not become an operator
trait claim. Rerun the same 100 actual Rust sites and Python regression sample
with unchanged labels/scorers. Acceptance remains >=100 actual sites, no
unsound cells, nonempty Must precision exactly 1.000, no lost old-cohort sites.
Publish any new failures and each changed answer. Record the new binary hash,
revision, commands, raw output, and all four common gates, once each, serially.
Neither sample size nor label selection changes in response to these results.
