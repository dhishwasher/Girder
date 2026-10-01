# Inbound customer funnel

> **Paused, 2026-10-01.** The user stopped all monetization-architecture
> work to focus on shipping the current, free release cleanly first
> (`docs/roadmap.md`). The paid "team feature" step 5 of this funnel refers
> to does not exist and is not being built yet.

This replaces an earlier draft of this document that assumed founder-led
outbound (identifying specific teams and contacting them directly). That
approach is explicitly out of scope: **no cold outreach, no prospect lists,
no sales calls, no founder-led pitching.** Everything below is something a
developer can go through entirely on their own, without talking to anyone.

The funnel: **discover → understand → install → experience value → encounter
paid team feature → buy → activate offline key.** Each section states what
already exists (verified by reading the actual file, not assumed) and what
is still a gap, so this stays a map of real work rather than a wish list.

## 1. Discover

A developer finds Girder through GitHub, npm, or organic search — not
through anyone reaching out to them.

**Already in place:**
- GitHub repo topics are set: `ai-agents`, `code-intelligence`,
  `developer-tools`, `ide`, `mcp`, `rust`, `semantic-search`, `tree-sitter`
  (verified via `gh repo view`).
- `npm/package.json` has a descriptive `description` and a relevant
  `keywords` list (`mcp`, `model-context-protocol`, `ai`, `coding-agent`,
  `code-search`, `semantic-graph`, `claude`, `cursor`, `context`), plus
  `mcpName: "io.github.dhishwasher/girder"` for MCP registry indexing.
- The GitHub repo description itself names the concrete problem ("gives AI
  coding agents exactly the code they need instead of whole files") rather
  than a vague tagline.

**Gaps, not yet addressed:**
- No `homepageUrl` set on the GitHub repo (empty in `gh repo view` output) —
  a free discoverability signal left unused.
- No verification of whether Girder is listed in any third-party MCP
  server directory/registry (e.g. an "awesome-mcp-servers" list, Anthropic's
  own MCP directory if one exists) — worth checking, since listing there
  is a one-time submission, not outbound contact with a specific person.
- No GitHub "Social preview" image confirmed set — affects link-sharing
  appearance on platforms that render one.

## 2. Understand

A visitor reads the README and decides, unassisted, whether this solves
their problem.

**Already in place:** the README leads with the problem statement and a
two-command install in the first screen, states tiers and pricing plainly
(`README.md:21-24`), and cites a measured, reproducible comparison
(bytes saved, gated checks passing) rather than an unverifiable claim.
`docs/evidence-index.md` gives a skeptical technical reader a full,
reproducible trail (audits, mutation testing, CI) without needing to ask
anyone a question.

**Gaps:** no screenshots or short demo (terminal recording / GIF) showing
the actual CLI or MCP tool output — the whole pitch is currently text-only.
A visitor has to install it to see what it actually returns. See "Proof"
below.

## 3. Install

**Already in place:** `npx -y girder-mcp setup` auto-detects and configures
Claude Code, Codex, and Cursor; a `--dry-run` flag lets a cautious user see
what it would do first (`README.md:12-15`). No account, no signup, no
network dependency beyond the npm/binary download itself. A prebuilt
binary + `install.sh` covers the plain-CLI path. This step is already
fully self-serve.

## 4. Experience value (free tier)

**Already in place:** `get_source`, `find_definition`, `search_code`,
`ask_codebase`, `review_changes`, and `orient` are free, permanently, no
license key (`docs/pricing-audit.md`). A developer gets real, daily-usable
value — source lookup, call-graph navigation, per-node impact — before
ever hitting a paywall.

**Decided, not yet shipped:** `test-impact` / `impacted_tests` (local,
interactive Must/May/Unknown test selection) is also becoming part of
this free step — the product decision is that Girder does not charge for
telling a developer which tests a change may affect. As of this writing
the code is unchanged; see the boundary section below.

## 5. Encounter the paid team feature

**Today, in the shipped binary**, a free user hits this message calling
`test-impact` / `impacted_tests`
(`crates/aether-app/src/project/license.rs::require_paid`):

> "The `{tool}` tool needs a paid Girder license. Buy a perpetual Girder
> license at https://maynard42.gumroad.com/l/zwpsjl. Free alternatives:
> `get_source`, `find_definition`, and `orient` still answer
> code-navigation questions."

**This boundary is going away.** Once the ungating ships, this message
disappears for `test-impact` entirely — it becomes free, so there is
nothing left to gate there. The *new* paid-feature encounter will be a
CI/PR enforcement gate (a small CLI command for GitHub Actions and
similar, exiting nonzero per an explicit policy) that **does not exist
yet** — a design has been written (see `docs/roadmap.md`'s CI-gate
design entry) but nothing has been implemented or approved for build.
Until that command exists and is gated, there is no "paid team feature"
for a free user to encounter at all.

## 6. Buy

A real, live $39 one-time Gumroad listing exists
(`https://maynard42.gumroad.com/l/zwpsjl`) for the *current* `test-impact`
gate, which is becoming free. **Per explicit instruction, this listing
should not be presented as the main commercial product** until (a) the
new CI-gate feature is real, and (b) purchase fulfillment is verified.
Whether the Gumroad listing should keep selling during the transition is
an open question for the user, not decided here.

**Unresolved fulfillment question, found while re-checking this
document:** every valid license key must be signed with a private
Ed25519 key that never leaves this machine (`crates/aether-app/src/
project/license.rs::PUBLIC_KEY`, verified against
`crates/aether-app/examples/license_keygen.rs`, a local CLI that takes
the private key file and prints signed keys — it does not call Gumroad
or any network service). Gumroad cannot itself mint a key the binary
will accept. Two ways this still works without becoming a "speak to me"
step exist in principle — Gumroad's native "unique license key per sale"
feature, pre-loaded with a batch of keys generated in advance via
`license_keygen <private-key.pk8> paid <count>` (fully automatic once
loaded, no manual step per sale); or the Gumroad listing delivering one
shared key to every buyer (automatic, but not actually per-buyer) — but
**which one, if either, is how the live listing is actually configured
is Gumroad account configuration, outside this repository, and I have
not verified it.** This needs to be checked against the live Gumroad
listing before any version of this step is called self-serve.

## 7. Activate the offline key

**Already in place (`README.md:864-879`):** set `GIRDER_LICENSE_KEY` as an
environment variable, or save the key to a per-OS config file path. Fully
offline, Ed25519-verified, no network call, key never expires. This step
is already self-serve and already documented plainly, independent of
however step 6's fulfillment question resolves.

## What this funnel does NOT include

No prospect list, no list of specific teams or individuals to contact, no
email template, no sales call script, no "identify N leads and reach out"
step. If a developer never discovers Girder on their own, that is a
discoverability problem (section 1) to fix with better listings, SEO, and
proof — not a reason to contact them directly.

## Honest summary of where the funnel actually stands

Steps 1-4 and 7 either already work self-serve today or will continue to
once the ungating ships. Step 5 (today's paid-feature encounter) is
scheduled to disappear, to be replaced by a not-yet-built CI-gate
encounter. Step 6 (buy) is deliberately not being pushed as the main
commercial CTA right now, both because the feature it currently gates is
becoming free and because its Gumroad fulfillment path is unverified.
The remaining real work is: ship the ungating + the new CI gate (see the
design in `docs/roadmap.md`), verify Gumroad fulfillment, and close the
discoverability/proof gaps named in steps 1 and 2 (homepage URL,
third-party MCP directories, screenshots or a short recorded demo).

## The boundary decision (made; not yet shipped)

The user decided the product split (`docs/roadmap.md`, "2026-10-01:
product split decided"): local/interactive `test-impact` becomes free;
the paid boundary moves to a new CI/PR enforcement command, not yet
built. This also resolves two overlaps this document previously flagged
as open questions: `orient` (already free) no longer competes with a
paid `impacted_tests`, since both are free; and Plan Format v2's
`tests.impacted` check (`test_checks.rs::run_tests_impacted`), which
calls the same underlying `classified_impact` selection for free via
`plan run` with no license check, is no longer an inconsistency — it was
already giving away exactly what is now decided to be free.
