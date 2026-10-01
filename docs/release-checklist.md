# Release-readiness checklist: first post-soundness Girder release

A checklist for cutting the first release after the Stage 3 Rust/Python
DONE milestones and the node-collision/combined-origin fixes in this
session (`docs/roadmap.md`, commits through `9ad6e4f`). Each item
states how to verify it, not just what to check — "done" means the
command was actually run, not assumed.

## Code correctness

- [ ] `cargo test --workspace -j1 --quiet` passes on a clean checkout
      of the release commit (not just in this session's working tree).
- [ ] `cargo clippy --workspace --all-targets -j1 -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `node --test npm/test/*.test.js` passes.
- [ ] `tools/core_trustworthiness_oracle.py --bitcode <release binary>`
      matches `docs/core-trustworthiness-baseline.json`.
- [ ] `tools/core_representative_mutations.py --bitcode <release binary>`
      matches `docs/core-representative-mutations.json`.
- [ ] GitHub Actions CI (`ci.yml`, `verify-builds.yml`) green on the
      release commit, not just locally.

## Known-state honesty (don't let the release notes drift from reality)

- [ ] `docs/roadmap.md`'s own checkpoint accurately reflects what's
      DONE vs IN PROGRESS as of the release commit — re-read it once,
      don't assume it's still accurate from memory.
- [ ] `docs/core-gap-analysis.md`'s "Prioritized open gaps" list
      matches what's actually still open (no gap silently closed
      without its own evidence commit, no new gap found but
      undocumented).
- [ ] Stage 3 TypeScript is still correctly described as IN PROGRESS
      everywhere it's mentioned (README, roadmap, any customer-facing
      doc) — do not let positioning imply it's DONE because Rust/Python
      are.
- [ ] The node-collision fix's own disclosed residuals (TypeScript's
      own `it()`/`describe()` collision shape unaddressed; Rust's
      `correction-3` re-score not done) are either closed with
      committed evidence, or still accurately described as open.

## Licensing

- [ ] `LICENSE` file matches what `README.md`'s "## License" section
      describes (BSL 1.1, conversion date, Apache 2.0 target).
- [ ] The Gumroad purchase link in `README.md` and
      `crates/aether-app/src/project/license.rs`'s `require_paid` help
      text actually resolves and matches the stated price.
- [ ] If a team tier ships in this release, `docs/pricing-audit.md` is
      updated to describe it (don't let the audit document go stale
      the moment it stops being accurate).

## Packaging and distribution

- [ ] `install.sh` (referenced from `README.md`) actually installs a
      working binary on a clean machine — not just assumed from past
      runs.
- [ ] `npm/` package (`npx -y girder-mcp setup`) tested against a real
      Claude Code, Codex, and Cursor install, not just the npm test
      suite — the README's own onboarding claim should be exercised at
      least once per release, not just unit-tested.
- [ ] Version numbers are consistent: `Cargo.toml` workspace version,
      `npm/package.json`, and any version string the binary itself
      reports (`girder --version`) all agree.

## Customer-facing material (if shipping alongside a commercial push)

- [ ] `docs/for-customers.md` (or equivalent) pricing matches
      `docs/pricing-audit.md` and `README.md` exactly — three sources
      of truth for the same number is three chances to drift.
- [ ] No claim anywhere (README, customer doc, evidence index) asserts
      formal soundness, perfect impact analysis, or token-based cost
      savings — the standing instruction from the 2026-10-01 directive
      in `docs/roadmap.md`. Spot-check by grepping for "sound",
      "guarantee", "token" across customer-facing files before
      publishing.

## Rollback plan

- [ ] The previous release's binary and `.aether` format compatibility
      are confirmed (or a documented breaking-change note exists) —
      check `crates/aether-graph/src/node.rs`'s serialization format
      hasn't changed incompatibly since the last release tag.
- [ ] A clear "how to roll back" note exists for anyone who installs
      this release and needs the previous one (pin the previous
      install.sh commit/tag).
