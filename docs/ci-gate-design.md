# CI-gate design (proposal — not implemented, not approved)

Per the standing instruction, this is shown before any code changes to
license enforcement or runtime tier checks. Nothing in this document has
been built. It specifies the minimum implementation for the paid
boundary decided in `docs/roadmap.md` ("2026-10-01: product split
decided"): local/interactive `test-impact` becomes free; the paid
feature is CI/PR enforcement, as a small, composable CLI command.

## 0. What the paid feature actually has to be (the honest answer)

Free `test-impact --classified` already exposes Must/May/Unknown with a
boundary count as JSON (demonstrated with real captured output in
`docs/ci-pr-example.md`). A free user can already write a few lines of
shell or `jq` to exit nonzero on, say, "any Must-reachable test is
present." So the paid command cannot just be "the same data with an
exit code" — that's trivially scriptable for free, and charging for it
would contradict "do not charge for Girder merely telling a developer
which tests may be affected."

What a free user cannot easily script themselves:
- **Verifying the required tests actually ran and passed**, not just
  that Girder named them — cross-referencing the Must/May selection
  against the CI job's own test-runner report (e.g. a JUnit XML file,
  or cargo/pytest/jest's own machine-readable output).
- **A committed, per-repo policy file** — team-wide rules (which
  categories block a merge, thresholds, overrides) instead of everyone
  reinventing the same shell script slightly differently.
- **A stable, documented, versioned report artifact** for audit/
  compliance purposes — "we enforced this on every PR" evidence, not
  raw internal JSON meant for interactive/programmatic use.
- **Correct base-ref/merge-base handling for CI** (see §1) — this part
  is infrastructure, not itself chargeable on its own, but the gate
  command depends on it.

If none of the above turns out to matter to real users, the honest
conclusion is the paid feature is convenience/audit value on top of
already-free data — which is still worth charging for (team
configuration + enforcement + reporting is real product work), but the
docs should say so plainly rather than imply a hidden analytical
capability that doesn't exist.

## 1. A free prerequisite: `--base <ref>` (ships with the ungating, not the paid gate)

**Verified blocker, not assumed:** `semantic_changed_impact_with_config`
(`crates/aether-app/src/project/git.rs`) hardcodes
`build_baseline_graph_with_config(root, "HEAD", config)` — there is no
base-ref parameter anywhere in `test_impact.rs`'s CLI parsing. On a CI
checkout of a PR (working tree == HEAD, nothing uncommitted),
`test-impact` as it exists today produces an **empty diff**, not a
comparison against the PR's base branch. See `docs/ci-pr-example.md` for
the verified mechanism.

This needs a `--base <ref>` flag that threads an arbitrary git ref into
`build_baseline_graph_with_config` instead of the hardcoded `"HEAD"`
literal. This is part of the free `test-impact` ungation, not the paid
gate — it's the same "tell me what's affected" capability, just against
a different baseline. The paid `gate` command (below) depends on this
existing, but does not itself own it.

## 2. The command

```
girder gate <dir> --policy <policy.toml> [--base <ref>] [--test-report <path>] [--format json|text]
```

- `<dir>`: project root, same convention as every other `girder`
  subcommand.
- `--policy <policy.toml>`: required. A committed, per-repo policy file
  (schema in §3) — no policy, no run. There is no built-in default
  policy; given the measured prevalence of Unknown boundaries (82 in
  one `demo-project` edit, 472 in a real open-source repo per
  `docs/observations/stage3-typescript-audit/before-observation-
  addendum-9.md`), a silent default that fails on any Unknown would
  redden nearly every PR. Forcing an explicit, visible policy file is
  deliberate, not an oversight.
- `--base <ref>`: optional, defaults to `HEAD` (matching today's
  `test-impact` behavior for local use); in CI this should be the PR's
  base branch (e.g. `origin/main`).
- `--test-report <path>`: optional. If given, a JUnit-XML (or similar;
  format TBD) test-runner report to cross-reference Must/May selections
  against tests that actually ran and passed. Without it, the gate can
  only check Girder's own classification (e.g. "no Unknown boundaries
  above threshold N"), not whether the named tests were actually
  executed — this should be stated plainly in the command's own
  `--help` output, not left implicit.
- `--format json|text`: `json` emits the stable audit-report schema
  (§3); `text` is a human-readable CI-log summary. Both go to stdout;
  nothing is written to disk unless a future `--out` flag is added.

No GitHub App, no web dashboard, no daemon — a single CLI invocation
suitable for a GitHub Actions step, any other CI system's "run a
command and check its exit code" primitive, or a local pre-push hook.

## 3. Policy semantics

A committed TOML file, e.g. `.girder/gate-policy.toml`:

```toml
# Fail the gate if any Must-reachable test is present in the selection
# but is NOT confirmed passed in --test-report. Requires --test-report.
require_must_tests_ran = true

# Fail the gate if the count of Unknown-classified boundaries for this
# diff exceeds this number. Given measured real-world prevalence
# (82-472+ per single-function edit in this program's own measurements),
# a low default is not viable; teams must pick a number that reflects
# their own codebase, hence no shipped default.
max_unknown_boundaries = 500

# Fail the gate if any May-classified test is present but not confirmed
# run. Off by default — May is bounded ambiguity, not proof; teams opt
# in deliberately.
require_may_tests_ran = false
```

Evaluation order: parse policy → run classified impact analysis (reusing
the exact `classified_impact` function `test-impact --classified`
already uses) → optionally parse `--test-report` → evaluate each
configured rule → first failing rule determines the "policy violated"
exit code; all evaluated rules (not just the first) are listed in the
report for diagnosis, not truncated to one. An empty diff against
`--base` (no origin nodes at all) is itself a disclosed boundary case,
same as `test-impact`'s own existing fail-closed behavior
(`CLAUDE.md`'s documented Module-origin fix) — it must not be silently
treated as "policy satisfied."

## 4. Exit codes

No path exits 0 except a genuine, successfully-evaluated pass. Proposed
(open for adjustment — this is the one part of the design most worth
the user's own opinion):

| Code | Meaning | Why distinct |
|---|---|---|
| `0` | Ran successfully; policy satisfied | The only "merge is fine" signal |
| `1` | Ran successfully; policy **violated** | The common "fail this PR" case a CI YAML checks for |
| `2` | Usage/configuration error (bad args, missing/malformed policy file, `--base` ref doesn't resolve, not a git repo) | So "you misconfigured this" never looks like a real policy failure |
| `3` | License error (free tier, missing key, rejected/malformed key) | So a free user's CI gets a clear "you don't have this feature" rather than a confusing policy failure |
| `4` | Analysis error (graph build failed, couldn't read a git object at `--base`, internal error) | So a Girder-side failure is never silently treated as either "fine" (0) or "violated" (1) — nothing was actually checked |

This differs from every other `girder` command today: `main.rs::report()`
collapses every `io::Result::Err` to exit code 1 unconditionally. `gate`
cannot reuse `report()` as-is — it needs its own result → exit-code
mapping that does not go through the shared helper, or `report()` needs
a variant that accepts an explicit code.

## 5. Free/paid matrix

| Capability | Tier | Status |
|---|---|---|
| `test-impact` / `impacted_tests` — interactive Must/May/Unknown | Free | **Decided, not yet shipped** (currently still paid) |
| `--base <ref>` on `test-impact` | Free | **Not yet built** (new capability, ships with the ungating) |
| `orient` | Free | Already shipped |
| `plan run`'s `tests.impacted` check | Free | Already shipped (pre-existing; this decision resolves what was previously flagged as an inconsistency, not a new change) |
| `girder gate` command (exit-code policy enforcement) | Paid | **Not yet built** |
| Policy file parsing/evaluation | Paid | **Not yet built** |
| `--test-report` cross-referencing | Paid | **Not yet built** |
| Audit-report JSON schema | Paid | **Not yet built** |

## 6. Legacy-key behavior

**Proposed: no new tier string.** `Tier` stays exactly `Free`/`Paid`
(`crates/aether-app/src/project/license.rs`). `girder gate` calls the
same `require_paid("gate")` check `test-impact` calls today, just
pointed at the new command name. Any existing valid key — v1
(`girder-v1...`) or v2 (`girder-v2...`), tier `paid` — satisfies it
automatically, with **zero migration code**, trivially satisfying
"grandfather every existing valid key" and "treat those keys as
founding/legacy Pro entitlements."

Trade-off, named rather than silently chosen: a distinct tier string
(e.g. `Tier::Pro` or `Tier::Team`) was considered and rejected for this
proposal, because (a) it would make every already-issued key reject on
an unmodified binary as `UnknownTier` until reissued, and (b)
`license_keygen`'s `matches!(tier.as_str(), "free" | "paid")` would need
changing, with no way to retroactively relabel keys already sold. If the
user wants that distinction later (e.g. to eventually differentiate
"legacy Pro" from a future named tier), it's a separate, larger change —
not proposed here.

Required regression tests before this ships: a v1 `paid` key and a v2
`paid` key each unlock `gate`, mirroring the existing
`legacy_key_returns_its_tier` test.

## 7. Files to change (grep-verified, not from memory)

**Ungating `test-impact` (free):**
- `crates/aether-app/src/main.rs:320-325` (`paid_tool_for_command`) —
  remove `"test-impact"`.
- `crates/aether-app/src/project/commands/mcp/watch.rs:390-392,487-489` —
  the second, independent gate site; must change in the same commit.
- `crates/aether-app/src/project/commands/mcp.rs:464` area — the
  `impacted_tests` tool description/metadata.
- `crates/aether-app/tests/mcp.rs:125-148`
  (`unlicensed_orient_is_free_but_impacted_tests_remains_paid`) — name
  and assertions must flip.
- `crates/aether-app/tests/cli.rs:143-151` — same, CLI-side.
- `README.md` (Tiers line, "Buy a license" section),
  `npm/README.md:84,181-190` (tools table, buy section) — both ship
  separately (npm/README.md is what `npx` users actually see) and have
  drifted from each other before.
- `docs/pricing-audit.md`, `docs/for-customers.md`,
  `docs/first-customer-plan.md` — full rewrite from "decided, not
  shipped" to "shipped."

**New `--base <ref>` (free, ships alongside):**
- `crates/aether-app/src/project/git.rs::semantic_changed_impact_with_config`
  — accept a base-ref parameter instead of the hardcoded `"HEAD"`
  literal.
- `crates/aether-app/src/project/commands/test_impact.rs` — new
  `--base <ref>` flag, threaded through `resolve_origins`.
- New tests: diff against a non-HEAD commit; missing-object error path
  (shallow clone without the base ref fetched) gets a clear error, not a
  panic or a silent empty result.

**New paid `gate` command (not yet built):**
- New module, e.g. `crates/aether-app/src/project/commands/gate.rs`.
- `crates/aether-app/src/project.rs` — export it.
- `crates/aether-app/src/main.rs` — new `Some("gate") => ...` dispatch
  arm, with its own exit-code mapping (not `report()`), and `"gate"`
  added to `paid_tool_for_command`.
- New policy-file schema/parser module.
- New tests: policy parsing, each exit code's trigger condition, legacy
  v1/v2 key acceptance, interaction with `--base`.
- New docs: policy-file schema reference, a real (not hand-written) GitHub
  Actions example captured once `--base` exists, README/`npm/README.md`
  sections for the new paid feature.

## Open question for the user, not decided here

Should the Gumroad listing keep selling the current $39 key during the
transition, given the feature it gates is about to become free? See
`docs/for-customers.md` and `docs/first-customer-plan.md`.
