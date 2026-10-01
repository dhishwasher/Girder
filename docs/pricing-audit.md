# Pricing audit: what is free, what is paid, as of 2026-10-01

This is a factual audit of the current, already-implemented license
gate — not a proposal. It exists so packaging/pricing decisions start
from what the code actually does, not from memory or assumption.
Verified by reading `crates/aether-app/src/project/license.rs`,
`main.rs`'s `paid_tool_for_command`, and every command module directly,
not inferred from documentation.

## The gate, exactly as implemented

One function decides tier: `crates/aether-app/src/project/license.rs::
require_paid`. One call site decides WHICH command it applies to:
`main.rs::paid_tool_for_command`, which currently returns `Some(...)`
for exactly one CLI command:

```rust
fn paid_tool_for_command(command: Option<&str>) -> Option<&'static str> {
    match command {
        Some("test-impact") => Some("impacted_tests"),
        _ => None,
    }
}
```

A `Tier` is `Free` or `Paid`, decided by a signed Ed25519 key (env var
`GIRDER_LICENSE_KEY` or a per-OS config-dir file), verified entirely
offline against a public key baked into the binary — no network call,
no phone-home, confirmed by reading `current_tier()` and `verify_key`
directly.

## Free (every CLI command except one)

`config`, `setup`, `analyze`, `search`, `names`, `context`, `new`,
`plan validate`/`explain`/`run`, `refactor`, `inspect`, `review`,
`orient`, `collab` (the collaboration/swarm projection feature),
`mcp`, `query`, `debug`. This includes `plan run` — graph-addressed,
verified edits with mandatory rollback and (per `CLAUDE.md`'s own
description) a mandatory `tests.impacted` check as part of plan
execution.

Of the 7 MCP tools the server exposes: `get_source`,
`find_definition`, `search_code`, `ask_codebase`, `review_changes`,
and `orient` are free. `orient` specifically bundles source + callers
+ callees + tests + impact for ONE named node per call — functionally
overlapping with what `impacted_tests` answers, just scoped to a node
you name rather than driven automatically off a git diff.

## Paid ($39, one-time, perpetual — already live on Gumroad)

Exactly one CLI command (`test-impact`) and its one corresponding MCP
tool (`impacted_tests`): the automatic, git-diff-driven conservative
test selection — "what changed, what needs testing," without the
caller naming a node.

## A gap worth disclosing, not silently fixed

Plan Format v2's own `tests.impacted` CHECK (`crates/aether-app/src/
project/planfile/checks/test_checks.rs::run_tests_impacted`, extensively
reworked this session for correctness) is **not** gated by
`require_paid` at all — grepped directly, confirmed absent. A plan step
can invoke the identical underlying `classified_impact` conservative
selection this whole session's work went into, for free, via `plan run`,
with no license check. Whether this is intentional (plan execution is
a different, already-free product surface) or an oversight is a product
decision, not a bug I'm fixing here — flagged for the packaging
decision below.

## What this suggests for a team tier

The current gate is binary (Free/Paid) with no seat count, no
organization identity, and no distinction between "one developer
bought a $39 key" and "a team shares one." Per-seat or org-scoped
licensing does not exist in `license.rs` today — `Tier` has exactly
two variants. Any team/pilot offering needs either (a) a documented
policy that one key may be used by N people on the honor system (zero
code change, ships today), or (b) an actual `Tier::Team`/seat-count
addition to `license.rs` (a real, small code change, not yet made,
not made without your sign-off per the standing directive).

## Evidence trail for this audit

- `crates/aether-app/src/project/license.rs` (the gate itself)
- `crates/aether-app/src/main.rs:258,320-325` (the one call site and
  the one gated command)
- `crates/aether-app/src/project/commands/mcp/watch.rs:390-392,487-489`
  (the same gate applied in the MCP watch path)
- `crates/aether-app/src/project/planfile/checks/test_checks.rs` (the
  ungated plan-execution path)
- `README.md:19-24,864-877` (current public pricing/tier statement,
  consistent with the code)
