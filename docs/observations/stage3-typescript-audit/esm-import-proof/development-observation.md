# ESM resolver development observation, 2026-10-06

**The builder implementation passes the frozen 73-case contract and the
19-step incremental sequence. This is not CLI or language-stage acceptance.**

Candidate `d0cf78beb064131ec6b8217ae7be6dfa8b6c932d` passed
`cargo test --locked --offline -p aether-builder -j1 --quiet`: 164 tests,
zero failures, zero ignored, nine test-result groups including the empty
doc-test group. The serial command took 722.122 seconds including compilation.
[Command, environment and status](development-2/run.json) and
[complete test log](development-2/test.log) are retained.

The implementation upgrades only conditional relative ESM import claims and
attaches the three frozen execution assumptions. It checks canonical root and
indexed source identity, symlink components, case and module-path collisions,
binding uses, import/export/call forms, cycles, and mock/hook hazards. Every
resolve revokes old import certificates before deriving new ones. Source-only
loads have no root attestation; projections differing from disk cannot receive
these certificates. The CLI loader opts in only when symlink following is off.

## What was tested

- All 73 frozen cases through a fresh builder graph: seven conditional Musts
  with exactly the expected file and semantic target, 66 Unknowns with no
  target. Required assumptions are checked on every new import certificate.
- All 19 frozen filesystem mutations through `GraphBuilder::update_files`,
  including empty source-change batches for non-source configuration changes.
  After every step, the marked class and target identities match a cold
  rebuild and the pinned expected class. Missing/deleted targets fail the test.
- Source-only loads, differing in-memory projections, and a manually ingested
  symlinked library source refuse the import certificate.
- Seven additional static contract cases cover computed and renamed mock APIs,
  an escaped hook package spelling, import attributes, and tagged calls. They
  are preserved in `crates/aether-builder/tests/typescript_esm.rs`; they are
  not Node runtime observations or additions to the 49-case denominator.
- Existing builder tests, including lexical and structural TypeScript tests,
  passed. No fixture or expected answer in the frozen manifest changed.

Manifest SHA-256 remains
`daa7310248f8e2adc3bdbbae340a3301dcca6b18983ca29ef3c6738bb97b94cd`;
dispatch-corpus SHA-256 remains
`9e3208a8f3fdc8faaee55cece63c1b5e25526322ebebbba915652f4f526ccd0a`.

## Retained failed run and correction

Candidate `25d7c88` failed its first development run: two tests passed and the
73-case aggregate test failed at `mock-unindexed-file` ([record](development-1/run.json),
[log](development-1/test.log)). The test helper indexed `target/helper.ts`,
although the frozen case and default CLI configuration exclude `target/`.
That changed the input universe: the resolver could establish a different mock
identity instead of refusing an unindexed identity. `9863d91` corrected the
helper to match default CLI exclusions. The original failure is retained.
This was not evidence that the runtime called a different target; the frozen
fixture itself records its expected refusal as conservative. A fresh CLI run
is still required to verify actual application ingestion independently.

## Conservative limits and pending acceptance

The current implementation refuses more inputs than the minimum eligibility
rules require. In particular, any indexed TypeScript subscript expression or
escaped string refuses these new import proofs; literal mock API names and
hook-library strings are detected without proving they are used as APIs.
`package.json` checks conservatively reject escapes and matching flag/key text
outside script fields too. These checks can mistake harmless data for a hazard.
They must not be described as exact API/configuration resolution. They leave
Unknown and do not change the frozen criteria. Resolving these extra refusals
requires separate evidence, not weaker expectations.

No new CLI binary was built for this candidate. No after-CLI observation,
watched-MCP replay, full ingestion-route audit, unchanged 49-case dispatch
measurement, or unchanged 100-call real-repository audit has run. The four
common stage gates have not run on this implementation. Library tests do not
prove that a watcher delivers every non-source or directory-deletion event.
Plan projection ingestion and configured symlink-following behavior still
need application-level checks. Must remains conditional on the frozen ESM
execution and no-unmodeled-hook assumptions.

Next: complete those application checks, build a fresh CLI, publish the
after-contract and unchanged benchmark/audit results, and run the common gates.
TypeScript remains IN PROGRESS; Go remains NOT STARTED.
