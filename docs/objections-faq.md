# Objections FAQ

Answers to the questions a skeptical evaluator would ask before
installing or paying, sourced from the actual code and license text
rather than restated from memory. If a question isn't answered here,
that's a documentation gap, not a reason to ask — file it as one.

## Does Girder phone home, collect telemetry, or need an account?

No. `license.rs`'s license check (`current_tier()` in
`crates/aether-app/src/project/license.rs`) reads an environment
variable or a local key file and verifies the signature with a public
key compiled into the binary — no network call, confirmed by reading
`current_tier()` and `verify_key_with_public_key` directly. The binary
makes no network request for anything related to licensing, in either
tier. There is no account, no login, and no telemetry collection
anywhere in the license or usage path.

## Can my company use the free tier commercially?

Yes. `LICENSE`'s Additional Use Grant states: "You may make production
use of the Licensed Work for any purpose, including internal commercial
use," with two conditions: (1) you cannot resell Girder itself as a
hosted/managed service whose primary value is Girder's own
functionality, and (2) you cannot disable or circumvent the license-key
gate on functionality that requires one. Condition 2 explicitly does
not require a key for the free-tier tools.

(The license text's own list of free-tier functionality — `get_source`,
`find_definition`, `search_code`, `ask_codebase`, `review_changes`,
"on one repository" — is narrower and stricter than what the code
actually gates, which also includes `orient` free with no repository
count limit, per `docs/pricing-audit.md`. This is a real discrepancy
between the legal text and the shipped behavior, flagged here rather
than silently resolved — `LICENSE` is a legal document and hasn't been
edited as part of this pass.)

## Is Girder's test selection sound — will it ever miss a test I needed?

No formal soundness proof exists, and the documentation does not claim
one — it can and does miss tests. What exists is a disclosed, three-way
classification — Must (proven reachable), May (bounded ambiguity),
Unknown (can't be proven either way) — where the stated policy is that
Unknown is never *silently* dropped: an unprovable call site is
supposed to make Girder include the related tests rather than guess
they're safe to skip, and that policy is enforced by code with its own
regression tests (`docs/call-classification-policy.md`,
`docs/core-representative-mutations.md`).

That policy does not cover every omission mode, and the honest list is:
`npm/README.md` states plainly that `impacted_tests` "misses tests
reached only through dynamic dispatch" — a real, currently open gap
(e.g. polymorphic method resolution through an untyped parameter,
`docs/core-representative-mutations.md`). Separately, this program found
and fixed a node-identity collision bug where two distinct code
entities could compute the same internal id and silently overwrite each
other's graph node (`docs/observations/stage3-typescript-audit/`) —
fixed for Rust's trait-impl shape, but TypeScript's own equivalent
collision shape (duplicate `it()`/`describe()` description strings) is
a different mechanism and remains unaddressed. See
`docs/core-gap-analysis.md`'s "Prioritized open gaps" for the complete,
maintained list. A full test run remains the authority before calling
any change safe — Girder's own documentation says this explicitly
(`CLAUDE.md`, `npm/README.md`).

## Which languages are actually ready?

Rust and Python have completed, published audits (hand-labeled ground
truth against real open-source repositories, "DONE" status in
`docs/roadmap.md`). TypeScript and Go are measured and gated but
explicitly marked in-progress, with their specific limits written down
separately (`docs/typescript-support.md`, `docs/go-support.md`) rather
than folded into a single "supported" claim.

## What happens to my $39 key if the pricing model changes?

Per explicit decision (`docs/roadmap.md`, "2026-10-01: product split
decided"): every existing valid key keeps working, with no loss of
functionality, and is treated as a legacy/founding entitlement once a
new paid tier ships. No promise is made here about which *future*
paid features a legacy key will or won't unlock beyond "nothing you
have today is taken away" — that's a decision for whenever the new tier
actually exists, not invented in advance.

## Why is a feature I'd expect to be paid (test-impact) currently free / becoming free?

Local, interactive test-impact lookup — "which tests might this change
affect" — was decided not to be a chargeable feature on its own
(`docs/roadmap.md`). The commercial boundary is moving to CI/PR
enforcement: machine-enforced policy, merge/CI gating, and auditable
reports for a team — not yet built. See `docs/pricing-audit.md` for
what the code gates today and `docs/for-customers.md` for where it's
headed.

## Where's the evidence behind the accuracy and cost numbers?

`docs/evidence-index.md` is the full pointer index: audits, dynamic-proof
mutation testing against real repositories, and the bugs found and fixed
along the way, not just a final number. Nothing in Girder's own
documentation cites a token-based cost claim — every efficiency number
is measured in output bytes, stated as such.
