# CI-gate design (proposal — not implemented, not approved)

> **Paused, 2026-10-01.** The user stopped all monetization-architecture
> work, explicitly including this design, to ship the current, free
> release first: "do NOT implement girder gate yet... prepare this release
> as the strongest free/public Girder release first" (`docs/roadmap.md`).
> Kept as the requested design record; not scheduled for implementation.

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
  policy; real, paired measurement shows a clean tree produces 0
  boundaries and a single one-line edit in a four-function fixture
  produces 82 (`docs/observations/ci-gate-design/`), so a silent
  default that fails on any nonzero Unknown count would redden most
  real PRs on most real codebases. Forcing an explicit, visible policy
  file is deliberate, not an oversight.
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
# Fail the gate unless every test in the must ∪ may ∪ unknown selection
# is confirmed to have run and passed, per --test-report. This is the
# real safety rule, not a Must-only check: the whole point of the
# classification policy is that Unknown means "include it," so a rule
# that only verifies Must-reachable tests ran would pass every gate on
# this program's own captured evidence (must=0 on both the clean and
# the edited demo-project runs, docs/observations/ci-gate-design/) while
# silently ignoring the 6 unknown tests that actually needed to run.
# Requires --test-report; without it this rule cannot be evaluated and
# the gate fails with a usage error (exit 2), not a silent pass.
require_selected_tests_ran = true

# Narrower, opt-in refinements for a team that wants to distinguish
# tiers of its own selection instead of treating must/may/unknown
# uniformly. Off by default; turning require_selected_tests_ran off and
# one of these on *weakens* the policy and should require an explicit,
# deliberate choice, not be a transitional default.
require_must_tests_ran = false
require_may_tests_ran = false

# Fail the gate if the count of unresolved-evidence boundaries for THIS
# DIFF (not the whole graph — confirmed diff-scoped, not a repo-wide
# constant, by the paired 0-vs-82 measurement in
# docs/observations/ci-gate-design/) exceeds this number. No shipped
# default; teams must pick a number that reflects their own codebase's
# baseline, established by running test-impact --classified on a few of
# their own representative diffs first.
max_unknown_boundaries = 500
```

Evaluation order: parse policy → determine whether the diff against
`--base` is empty (zero origin nodes — e.g. a docs-only PR touching no
analyzed source) → if empty, **pass** and say so explicitly in the
report, rather than running rules against nothing → if non-empty, run
classified impact analysis (reusing the exact `classified_impact`
function `test-impact --classified` already uses) → optionally parse
`--test-report` → evaluate each configured rule → first failing rule
determines the "policy violated" exit code; all evaluated rules (not
just the first) are listed in the report for diagnosis, not truncated
to one. A non-empty diff where origin resolution itself fails closed
(the existing Module-origin boundary behavior, `CLAUDE.md`) is NOT the
same as an empty diff and must not be treated as "policy satisfied" —
it should surface as boundaries requiring evaluation like any other
unresolved case, never silently skipped.

## 4. Exit codes

No path exits 0 except a genuine, successfully-evaluated pass. **Must
NOT reuse the existing pre-dispatch gate.** `main.rs:257-258` already
runs `report(project::require_paid(tool))` for any command
`paid_tool_for_command` names, before that command's own handler ever
runs, and `report()` exits 1 on any `Err`. If `"gate"` is added to
`paid_tool_for_command` the way `"test-impact"` is today, a free user
running `girder gate` gets exit 1 from that pre-dispatch path — which
this design defines below as "policy violated," not "you need a
license." That is a real contradiction, caught by re-reading the code
this design depends on, not assumed. Fix: `gate` is **not** added to
`paid_tool_for_command`; its own command handler calls the license
check itself and maps a rejection to its own distinct exit code (3
below), never falling through to `report()`.

Proposed table (open for adjustment — this is the one part of the
design most worth the user's own opinion, see the risk noted after it):

| Code | Meaning | Why distinct |
|---|---|---|
| `0` | Ran successfully; policy satisfied | The only "merge is fine" signal |
| `5` | Ran successfully; policy **violated** | The common "fail this PR" case a CI YAML checks for |
| `2` | Usage/configuration error (bad args, missing/malformed policy file, `--base` ref doesn't resolve, not a git repo) | So "you misconfigured this" never looks like a real policy failure |
| `3` | License error (free tier, missing key, rejected/malformed key) | So a free user's CI gets a clear "you don't have this feature" rather than a confusing policy failure |
| `4` | Analysis error (graph build failed, couldn't read a git object at `--base`, internal error) | So a Girder-side failure is never silently treated as either "fine" (0) or "violated" (5) — nothing was actually checked |

**Deliberately not `1` for "violated":** every other `girder` command's
error path already collapses to exit 1 via `main.rs::report()`. If
`gate`'s own internal code ever has a bug that lets an unhandled error
fall through to that same shared convention, it would silently read as
"policy violated" to a CI pipeline instead of "the tool itself broke" —
exactly the failure mode exit 4 exists to prevent. Picking a number
`report()` never emits (proposed: `5`) makes that class of bug visibly
wrong (an undocumented code) instead of silently plausible. A Rust
panic exits `101` by default — also worth keeping clear of. The
alternative — use `1` for "violated" to match common CI-tool convention
(lint-style tools often do) and accept the collision risk, documented
loudly — is a real option; this is the one number in the whole design
most worth the user picking directly rather than inheriting a proposal.

`gate` needs its own result → exit-code mapping regardless of which
numbers are chosen; it cannot go through `report()` unmodified since
that helper only ever emits `0` or `1`.

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
(`crates/aether-app/src/project/license.rs`). `girder gate`'s own
handler calls the same underlying `require_paid`-style check
`test-impact` uses today (see §4 on why this must happen inside `gate`'s
own code path, not via the shared pre-dispatch `paid_tool_for_command`),
just pointed at the new command name. Any existing valid key — v1
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

## 7. Files to change (grep-verified, not from memory — corrected after a first pass got one site wrong)

**Ungating `test-impact` (free):**
- `crates/aether-app/src/main.rs:320-325` (`paid_tool_for_command`) —
  remove `"test-impact"`. **Correction:** `crates/aether-app/src/
  project/commands/mcp/watch.rs:390,487` both call
  `crate::paid_tool_for_command` directly (checked twice in the same
  handler, once before queuing the job and once after graph validation,
  as a race-safety double-check — not two independent gates with their
  own logic). Changing `paid_tool_for_command` alone automatically
  ungates both watch.rs sites; no separate watch.rs edit is needed for
  the ungating itself.
- `crates/aether-app/src/project/license.rs:92-94` (`require_paid`'s
  `help` text) — names Gumroad and lists `get_source`/`find_definition`/
  `orient` as "free alternatives" to `test-impact` specifically; once
  `test-impact` is free this text has nothing left to apply to for the
  commands still using `require_paid`.
- `crates/aether-app/src/project/commands/mcp.rs:464` area — the
  `impacted_tests` tool description/metadata.
- `CLAUDE.md` — its test-impact guidance paragraphs currently describe
  it as the paid, advisory-gated tool; needs revision alongside the code
  change, not left stale.
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
- **`LICENSE`'s Additional Use Grant — a user-owned legal edit, not
  something to change without the user's own review.** Flagged here,
  not performed: the grant's free-tier list already omits `orient`
  relative to what the code gates (a pre-existing discrepancy, disclosed
  in `docs/objections-faq.md`), and once `test-impact` becomes free the
  grant's own text will need to describe that too. This is the one item
  in this whole list that is not a code or Claude-authored-doc change.

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
- New module, e.g. `crates/aether-app/src/project/commands/gate.rs`,
  including its own license-check call and its own denial/help text
  (not a reuse of `license.rs`'s `test-impact`-specific `help` string)
  and its own exit-code mapping — see §4 for why this cannot go through
  `main.rs`'s shared `paid_tool_for_command` pre-dispatch or `report()`.
- `crates/aether-app/src/project.rs` — export the new module.
- `crates/aether-app/src/main.rs` — new `Some("gate") => ...` dispatch
  arm calling the module's own entry point directly (NOT added to
  `paid_tool_for_command`, per §4).
- New policy-file schema/parser module.
- New tests: policy parsing, each exit code's trigger condition
  (including the free-user-gets-3-not-5 case specifically), legacy
  v1/v2 key acceptance, interaction with `--base`, the empty-diff-passes
  vs. origin-resolution-fails-closed distinction from §3.
- New docs: policy-file schema reference, a real (not hand-written) GitHub
  Actions example captured once `--base` exists, README/`npm/README.md`
  sections for the new paid feature.

## Open questions for the user, not decided here

- Should the Gumroad listing keep selling the current $39 key during
  the transition, given the feature it gates is about to become free?
  README's and `npm/README.md`'s "Buy a license" sections are the most
  visible places that key gets presented as the product today, so this
  question covers those CTAs too, not just the Gumroad listing itself.
  See `docs/for-customers.md` and `docs/first-customer-plan.md`.
- The exit-code numbering in §4, specifically whether "policy violated"
  should be `5` (as proposed, to avoid colliding with the ambient "1 =
  error" convention every other `girder` command already uses) or `1`
  (common CI-tool convention, with the collision risk accepted and
  documented).
- Whether `LICENSE`'s Additional Use Grant should be updated now (to
  add `orient` and describe the coming `test-impact` ungating) or held
  until the code change actually ships — this document does not touch
  `LICENSE` either way.
