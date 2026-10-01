# First-customer plan

## Who to target

A small (3-10 engineer) team already using Claude Code, Codex, or
Cursor heavily for real code changes — not evaluating AI coding tools,
already living with them daily, because they're the ones who've
already felt an AI-authored change miss a test. Best found where this
pain is already being talked about publicly: teams posting about AI
agent workflows, CI reliability, or "the agent broke prod again"
incidents.

## What problem to pitch

Not "faster AI coding" — "your agent can silently skip the tests a
change actually needed, and you have no systematic way to catch it
before it ships." Lead with the specific failure mode (a change looks
isolated, the agent doesn't check far enough, wrong tests run), not
with Girder's feature list.

## What they receive

- A founding-team license (3-5 seats, exact terms TBD — see
  `docs/pricing-audit.md`'s note that per-seat/org licensing doesn't
  exist in code yet; a founding pilot can ship on the honor system
  first, a real `Tier::Team` second).
- `npx -y girder-mcp setup` onboarding (same as the free tier — no
  separate install path to build).
- Direct access to whoever's driving this (you) for the first few
  weeks — founding customers buy the relationship as much as the tool.

## What to charge

A founding-team pilot in the $200-500 range (one-time), explicitly
framed as "founding" — lower than a mature per-seat price will be,
in exchange for their feedback shaping what ships next. Manually
invoiced; no billing infrastructure needed for the first sale.

## What evidence to show them

`docs/for-customers.md` for the pitch itself; `docs/evidence-index.md`
if they want to go deeper technically. Do NOT lead with Stage 3
TypeScript or anything still in progress unless they specifically use
TypeScript and ask — lead with Rust/Python, which have committed,
DONE audits.

## Immediate next step

Identify 5-10 specific teams/individuals matching the target profile
and reach out directly — this plan is only useful once it has names
attached, which is a decision for you, not something to generate
speculatively here.
