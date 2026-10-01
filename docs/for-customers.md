# Girder for teams

> **Paused, 2026-10-01.** The user stopped all monetization-architecture
> work to focus on shipping the current, free release cleanly first
> (`docs/roadmap.md`). This document describes a paid-tier narrative that
> does not currently exist and is not being built yet — not linked from
> `README.md`, kept for its factual free/paid audit trail, not as
> customer-facing copy.

## The problem

AI coding agents (Claude Code, Codex, Cursor, and others) move fast
and can silently miss what a change actually affects. A change that
looks small can touch code the agent never checked, and standard
practice — grepping, reading nearby files, or asking the agent to
"figure out what tests to run" — has no guarantee behind it. The
result: an AI-authored change ships, the wrong tests (or no tests) run,
and the gap surfaces in review or production instead of before commit.

## What Girder does

Girder parses your repository into a semantic graph — real functions,
real call edges, not text search — and answers two kinds of questions
against it:

1. **Where is this, and what does it touch?** Exact function source,
   callers, callees, blast radius — without reading whole files.
2. **What does this change actually need tested?** A conservative
   selection of tests reachable from what changed, with every call
   site labeled Must (proven), May (bounded ambiguity), or Unknown
   (disclosed, never silently dropped) — so an unresolved dispatch
   shows up as "we don't know, so include it," not as nothing.

It's one static binary any MCP-capable agent can drive, plus a CLI for
scripting and CI.

## Installation

```bash
npx -y girder-mcp setup
```

Auto-detects and configures Claude Code, Codex, and Cursor. For any
other MCP client, a documented raw JSON config works the same way. No
account, no cloud dependency — the binary runs entirely on your
machine or your CI runner.

## What's free

Source lookup, symbol search, call-graph navigation, and per-node
impact/test lookup (`orient`) — enough for an individual developer to
use Girder for real, every day, with no license key.

**Decided, not yet shipped:** `impacted_tests` / `test-impact` — the
automatic, git-diff-driven Must/May/Unknown test selection — is also
becoming free. Telling a developer which tests a change may affect is
not something Girder charges for. **As of this writing the code has
not changed yet**: `test-impact` is still gated behind a paid key today
(`docs/pricing-audit.md`). This document describes where things are
headed; it will be corrected the moment the ungating actually ships.

## What the paid tier is becoming

Not individual test-impact lookup — a CI/PR enforcement feature: a
small, composable command for GitHub Actions and other CI systems that
consumes Girder's existing Must/May/Unknown classification and exits
nonzero according to an explicit, auditable policy, plus configuration
suitable for a team (a committed policy file, not a per-developer
setting). **This does not exist yet.** A design has been written
(`docs/roadmap.md`'s CI-gate design entry) and is awaiting approval
before any implementation starts.

**Until that feature is real and purchase fulfillment is verified
end-to-end, Girder does not have a commercial product to sell as "the"
paid tier.** The $39 key that exists today gated `test-impact`, which
is becoming free — existing buyers are not losing anything (every valid
key, old or new, will keep working and will carry forward as a legacy
entitlement once the new tier ships), but a *new* purchase today would
be buying access to a feature about to become free. Whether new sales
of that key continue during the transition is an open question, not
decided here.

Anyone who already has a free-tier install keeps using `get_source`,
`find_definition`, `search_code`, `ask_codebase`, `review_changes`, and
`orient` without buying anything, regardless of how the paid tier
evolves.

## Current limitations, stated plainly

- TypeScript support is measured and explicitly marked **in progress**,
  not done — Rust and Python have completed, published audits against
  real open-source repositories; TypeScript's own audit is ongoing and
  its gaps are disclosed, not hidden (`docs/roadmap.md`,
  `docs/observations/stage3-typescript-audit/`).
- Some dynamic-dispatch shapes (e.g. polymorphic method resolution
  through an untyped parameter) aren't resolved yet — when Girder
  can't prove a call site, it says so (Unknown) rather than guessing,
  which means it over-includes tests in that case rather than
  under-including them.
- No formal soundness proof. The claim is disclosed, measured
  precision/recall on real code (see `docs/evidence-index.md`), not a
  mathematical guarantee.

## Why trust the numbers

Every efficiency and accuracy claim in Girder's own documentation is
backed by a committed, reproducible measurement — not a marketing
number. See `docs/evidence-index.md` for the full trail: real
open-source repositories, hand-labeled ground truth, dynamic-proof
mutation testing, and a public commit history that includes the bugs
found and fixed along the way, not just the final state.
