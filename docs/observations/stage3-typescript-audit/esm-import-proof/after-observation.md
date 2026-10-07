# ESM named-import proof: after-observation (candidate `cb2aa11`)

Measured 2026-10-06 against the frozen policy
([policy.md](policy.md)), with nothing in the corpus, labels, manifest, or
thresholds changed. The earlier [before-observation](before-observation.md) and its
`before-1/` directory are untouched; these runs use new names.

## Identity

| Item | Value |
| --- | --- |
| Candidate commit | `cb2aa116585d80f9d096f8aeef81fba39e76c4e0` (tracked tree clean) |
| Binary | debug `girder` built `cargo build --locked --offline -p aether-app --bin girder -j1` on the same revision |
| Binary SHA-256 | `cbc20b346129f4256d29e79bd0c644e17c9ba2f979010eb2da1731bb82e1c2b1` (differs from the baseline binary `469f2519…` built at `c209a22`) |
| ESM manifest SHA-256 | `daa7310248f8e2adc3bdbbae340a3301dcca6b18983ca29ef3c6738bb97b94cd` (matches the frozen pin) |
| Dispatch corpus SHA-256 | `9e3208a8f3fdc8faaee55cece63c1b5e25526322ebebbba915652f4f526ccd0a` (matches the frozen pin) |
| Environment | Linux x86_64, 2 CPUs, 2.7 GB RAM, rustc/cargo 1.97.1, node v22.23.1, serial, offline, `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`; see [gates-cb2aa11/environment.txt](gates-cb2aa11/environment.txt) |

## Results

| Measurement | Command | Before | After |
| --- | --- | --- | --- |
| 73-case ESM contract | `python3 -m tools.measure_typescript_esm_imports --name after-1 --binary <bin> --binary-revision cb2aa11…` | 66/73 exact, 7 conservative misses | **73/73 exact, 0 errors, `cold_contract_met: true`** ([after-1/](after-1/)) |
| 49-case dispatch corpus | `python3 -m tools.dispatch_corpus_scorer --bitcode <bin> --corpus docs/dispatch-corpus.json --output after-1/dispatch-corpus-scoring.json --json` | 22 exact / 34 conservative / 0 unsound / 1 failed | **23 exact / 33 conservative / 0 unsafe_exclusion / 0 overclaim / 1 failed**; Must precision 6/6 = 1.0 (was 5/5); `typescript-direct-cross-file` conservative to **exact** (expected `must`, observed `must`) |
| 100-call real audit | `python3 -m tools.measure_typescript_real_audit --name esm-after-1 --binary <bin>` | 56 exact / 44 conservative / 0 unsound, Must 3/3 | **56 exact / 44 conservative / 0 unsound, Must 3/3 = 1.0**, acceptance `passed: true` ([../lexical-bindings/esm-after-1/](../lexical-bindings/esm-after-1/)) |

## What this does and does not show

- The resolver closes the frozen cross-file named-import case end to end: the
  one corpus case it targets flips to exact, and every Must is correct.
- **Real-repository audit, two comparisons (both checked site by site, not
  inferred from totals).**
  - *Against the immediate predecessor* (`structural-members-after-local-1`): all
    110 rows (100 calls plus 10 noncalls) are identical in observed class,
    reason, caller, and cell. There are 0 differences, so the audit is unchanged
    there.
  - *Against the frozen 100-site baseline* (`extension/before/combined-cohort.json`,
    the file `changed-answers.json` diffs; sha256 prefix `bf6aab7f951dfa7c`):
    53 exact / 47 conservative becomes **56 exact / 44 conservative**. Exactly
    three scored sites change class, all `unknown` to `must` (audit indices 32,
    37, 91); Must goes from 0 to 3, and all 3 are correct (3/3). The other
    changed rows in `changed-answers.json` are detail-only. This is the
    convention the Rust leg `audit_shows_fewer_conservative_more_exact_cells`
    used (27/25 baseline to 28/24), so that leg is **met** here.
- The corpus failure `typescript-structural-object-literal` (ambiguous
  `alice`/`bob` origin) is unchanged and retained on purpose.
- Must claims are conditional on the policy's recorded ESM execution and
  no-unmodeled-hook assumptions.
- **Stage 2 obligation outstanding.** `typescript-direct-cross-file` started
  passing. Stage 2's rule (keep the case, append a harder one, publish fixed-cohort
  scores separately) is not yet satisfied: the changelog has no precedent for a
  flipped case, and the policy pins the frozen corpus hash, so a harder successor
  must be a new versioned corpus file, not an edit. Still owed.
- **Policy C-3 (every ingestion route skips a symlink or marks the snapshot
  uncertain, and never yields a Must), what the evidence shows.**
  - CLI analyze: the 73-case contract includes `symlinked-target-file` and
    `symlinked-directory`; both pass.
  - Watch/MCP watched graphs: application tests cover a symlinked target file,
    a symlinked target directory, and `follow_symlinks = true`; each stays Unknown
    with watched equal to cold.
  - Library `load_file` / modified projections: the builder test
    `source_only_and_modified_projection_loads_have_no_import_certificate` pins
    that a graph with no attested root, or with bytes differing from disk, stays
    Unknown.
  - **Not covered at the application level:** plan projections and workspace
    buffers (no application route attests a root and also accepts non-disk bytes,
    so no test can reach the byte-difference rule; these routes stay Unknown
    because the root is never attested). Disclosed, not claimed.
- Application tests overall: non-source inputs, subdirectory deletion,
  symlinked target file, symlinked directory, `follow_symlinks`, stale
  environment, and the 19-step replay. The stale-environment and `follow_symlinks`
  tests were mutation-checked. The subdirectory-deletion, symlinked-file, and
  symlinked-directory tests were not mutation-checked.

## Common gates on the committed candidate

The measurements ran on the binary built at `cb2aa11`. A later test-only commit
`b0c09b6` (adds the symlinked-directory watch test; no product code changed)
is the final candidate. All four gates ran serially on both revisions; logs and
exit codes are in [gates-cb2aa11/](gates-cb2aa11/) and
[gates-b0c09b6/](gates-b0c09b6/).

| Gate | `cb2aa11` | `b0c09b6` |
| --- | --- | --- |
| `cargo test --workspace -j1 --quiet` | exit 0, 782 passed, 0 failed, 2 ignored | exit 0, **783 passed**, 0 failed, 2 ignored (27 suites) |
| `cargo clippy --workspace --all-targets -j1 -- -D warnings` | exit 0 | exit 0 |
| `cargo fmt --all --check` | exit 0 | exit 0 |
| `node --test npm/test/*.test.js` | exit 0, 29 passed, 2 skipped | exit 0, 29 passed, 2 skipped |

GitHub Actions on `cb2aa11` (run 37532911297): success on both jobs. The run on
the final pushed head is checked and recorded separately.
