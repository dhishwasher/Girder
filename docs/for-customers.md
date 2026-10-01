# Girder for teams

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

## What's free, permanently

Source lookup, symbol search, call-graph navigation, and per-node
impact/test lookup (`orient`) — enough for an individual developer to
use Girder for real, every day, with no license key.

## What the team license adds

Automatic, git-diff-driven test selection (`impacted_tests` /
`test-impact`): point it at a change, get the conservative set of
tests that change could affect, without naming anything by hand — the
thing you'd want gating a PR or a CI job, not just an interactive
lookup.

**Pricing**: $39 one-time, perpetual, per developer — [buy directly on
Gumroad](https://maynard42.gumroad.com/l/zwpsjl), no account or sales
conversation required. For a team: buy one key per developer who needs
`impacted_tests`; there is no separate team SKU yet, so today "team
pricing" is simply $39 × the number of developers who need the gated
tool. Anyone who already has a free-tier install keeps using `get_source`,
`find_definition`, `search_code`, `ask_codebase`, `review_changes`, and
`orient` without buying anything.

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
