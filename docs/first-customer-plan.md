# Inbound customer funnel

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
ever hitting a paywall. This is deliberate: the free tier has to be good
enough to build trust before the paid feature is ever offered.

## 5. Encounter the paid team feature

This is the moment a free user discovers there's something more, without
being sold to — the product itself should say it.

**Already in place:** `crates/aether-app/src/project/license.rs::
require_paid` returns this exact message when a free user calls
`test-impact` / `impacted_tests`:

> "The `{tool}` tool needs a paid Girder license. Buy a perpetual Girder
> license at https://maynard42.gumroad.com/l/zwpsjl. Free alternatives:
> `get_source`, `find_definition`, and `orient` still answer
> code-navigation questions."

This already satisfies "an obvious upgrade message when a user reaches
paid functionality": it names the gated tool, gives the buy link directly
in the error, and tells the user what still works for free instead of
just blocking them. No code change identified as necessary here.

**Gap:** the same message is not yet confirmed to appear through every
path a user could hit it from (MCP tool-call error surfaced inside
Claude Code / Codex / Cursor's own UI, not just the raw CLI stderr) —
worth a manual check, not a code change, before relying on it as the
primary upgrade touchpoint.

## 6. Buy

**Already in place:** a real, live $39 one-time Gumroad listing
(`https://maynard42.gumroad.com/l/zwpsjl`), linked both from the README's
"Buy a license" section and from the in-product error message above.
Purchase requires no conversation — Gumroad is checkout-only.

## 7. Activate the offline key

**Already in place (`README.md:864-879`):** set `GIRDER_LICENSE_KEY` as an
environment variable, or save the key to a per-OS config file path. Fully
offline, Ed25519-verified, no network call, key never expires. This step
is already self-serve and already documented plainly.

## What this funnel does NOT include

No prospect list, no list of specific teams or individuals to contact, no
email template, no sales call script, no "identify N leads and reach out"
step. If a developer never discovers Girder on their own, that is a
discoverability problem (section 1) to fix with better listings, SEO, and
proof — not a reason to contact them directly.

## Honest summary of where the funnel actually stands

Steps 3, 4, 6, and 7 are already fully self-serve and require no further
work to function. Step 5's message already exists and already does the
job asked of it. The two real gaps are in steps 1 and 2: discoverability
listing opportunities (homepage URL, third-party MCP directories) and the
complete absence of any visual proof (screenshots or a short recorded
demo) that a skimming visitor can see before installing anything.
