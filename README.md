# Girder

**Girder gives coding agents exactly the code they need, instead of whole
files.** It parses your repository into a living semantic graph — functions,
definitions, call edges — and answers questions against that graph: exact
function source, callers and callees, impact analysis, minimal test selection,
and verified graph-addressed edits. It is one static Rust binary that any agent
can drive over MCP, plus an optional native IDE.

```bash
curl -fsSL https://raw.githubusercontent.com/dhishwasher/Girder/main/install.sh | sh
```

**Languages:** Rust, Python, TypeScript, and Go. Rust and Python are the most
mature; TypeScript and Go are measured and gated, with their limits written
down ([TypeScript](./docs/typescript-support.md), [Go](./docs/go-support.md)).

**Tiers:** the free tier is permanent and needs no account — `get_source`,
`find_definition`, `search_code`, `ask_codebase`, and `review_changes` on a
single repository. The `orient` and `impacted_tests` tools need a
[paid license](#license). Keys are verified offline; the binary never phones
home.

On this repository's committed ten-node measurement, `girder context
--source-only` returned 8,765 bytes where full-file reads returned 408,137 — a
[97.85% reduction](./docs/context-vs-read-cost-observation.json). That counts
bytes, not tokens.

## Install

A prebuilt binary, no Rust toolchain needed:

```bash
curl -fsSL https://raw.githubusercontent.com/dhishwasher/Girder/main/install.sh | sh
girder --version
```

Or from source:

```bash
cargo install --path crates/aether-app
girder --help
```

This builds the default headless profile and installs the `girder` binary to
`~/.cargo/bin` (make sure it's on your `PATH`). No GPU, display, network, or API
key is required — the default AI provider is an offline `MockProvider`. The GUI
and live AI providers are opt-in Cargo features not included in a plain
install; see [The GUI](#the-gui) and [Local-first AI](#local-first-ai) below.

### Windows installer

Each Windows release also includes `Girder-<version>-setup.exe`. It installs
for the current user under `%LOCALAPPDATA%\Programs\Girder`, adds Girder to the
user `PATH`, creates a Start Menu shortcut, and does not request administrator
access. Open a new terminal after installation so it sees the updated `PATH`.
The installer includes the desktop GUI, and its Start Menu shortcut opens it.
The archives and npm installation continue to provide the headless CLI.

The installer is **not code-signed yet**, so Windows SmartScreen will warn on
first run. After downloading the installer from the GitHub release, double-click
it, choose **More info** on the “Windows protected your PC” dialog, verify that
the app is Girder and the publisher is shown as unknown, then choose **Run
anyway**. If those details do not match, cancel instead.

## Search accuracy

Exact-symbol lookup is Girder's strongest search path. Natural-language intent
search is experimental: it reached **41.9% top-1** and **77.4% top-5** accuracy
on the committed 31-item corpus, below the precommitted 75% and 90% thresholds.
See the [observation](./docs/description-search-accuracy-observation.json) and
[policy](./docs/description-search-accuracy-policy.json).

## Use it from an AI coding agent

`girder mcp` serves the read-only graph commands over the
[Model Context Protocol](https://modelcontextprotocol.io), so an agent can ask
about your codebase instead of reading files into its context window.

Claude Code:

```bash
claude mcp add girder -- npx -y girder-mcp .
```

Any MCP client config:

```json
{
  "mcpServers": {
    "girder": {
      "command": "npx",
      "args": ["-y", "girder-mcp", "."]
    }
  }
}
```

With a binary already installed, `"command": "girder", "args": ["mcp", "."]`
skips npm entirely.

Seven tools, all read-only:

| Tool | What it answers |
|---|---|
| `get_source` | The source of specific functions, without the file around them. |
| `find_definition` | Where an exact identifier is declared. Not a substring search. |
| `search_code` | Which functions match a description, when you don't know the name. |
| `ask_codebase` | Callers, callees, and blast radius, by graph traversal. |
| `impacted_tests` | Only the tests that can reach what changed. |
| `review_changes` | What changed in the working tree, as semantics rather than text. |
| `orient` | Source, callers, callees, tests, and impact for one node, in one call. |

### What that saves, and what it doesn't

Two precommitted measurements, both counting **bytes of command output rather
than tokens** (no tokenizer was run):

- `get_source` against reading the whole file: **97.85% fewer bytes** across ten
  functions sampled by source-size decile, cheaper on all ten
  ([`docs/context-vs-read-cost.md`](./docs/context-vs-read-cost.md)).
- `find_definition` against `grep`: **97.98% fewer bytes** across ten
  identifiers ([`docs/names-cost.md`](./docs/names-cost.md)).

Both are single-repository measurements. The direction is structural — files
are much larger than the functions in them, and grep returns every mention
where `find_definition` returns only declarations — but the exact percentages
are not portable.

`impacted_tests` is **advisory**. It over-selects unrelated tests, and it
misses tests reached only through dynamic dispatch (measured: recall 0.000 on a
polymorphic-dispatch case, [`docs/core-representative-mutations.md`](./docs/core-representative-mutations.md)).
A full test run remains the authority before calling a change safe.

`orient` bundles what `get_source` + `ask_codebase` (callers, callees, and
impact) + `impacted_tests` otherwise answer across 5-6 separate calls into
one. On a 15-task corpus spanning ten pinned repositories, that one call used
**fewer aggregate bytes than the chain it replaces** (48,814 vs 101,302,
a 0.48 ratio) while cutting 78 round trips to 15 — one per task — and, after
two disclosed defects were fixed, **37 of 37 gated checks pass**. The first
run found `impacted_tests --quiet` silently dropping non-Rust/Python test
names (`orient`'s own test-coverage section did not share the bug, which is
how it was found); that filter is now removed. Its natural-language `intent`
input still inherits `search_code`'s accuracy — all three intent tasks in
this corpus resolved to the wrong node, unchanged and out of scope for this
fix — but `orient`'s confidence heuristic, which originally caught none of
the three, now flags all three `"confidence": "low"` with candidate scores
attached, at the cost of also flagging some correct resolutions when a
runner-up is close. See [`docs/orient-tool.md`](./docs/orient-tool.md) and
the committed [policy](./docs/orient-tool-policy.json) /
[original observation](./docs/orient-tool-observation.json) /
[post-fix observation](./docs/orient-tool-observation-post-fix.json).

The project root is fixed when the server starts, so no tool call can reach
another directory. `GIRDER_MCP_TIMEOUT_SECONDS` (default 120) bounds each
call; raise it for a very large repository.

Everything below assumes `girder` is on your `PATH`. Building from a source
checkout without installing works the same way with `cargo run -p aether-app --`
in place of `girder`.

## Quickstart

```bash
# Full end-to-end demo — no GPU, display, or API key required:
girder

# Run the test suite (graph, builder, AI router, agent swarm, debugger):
cargo test --workspace

# Optional: compile live providers and run the real debugpy adapter test:
cargo check -p aether-ai --features live-providers
python3 -m pip install debugpy
cargo test -p aether-dap --test debugpy -- --ignored
```

### Use it on a real project

Girder is also a CLI that operates on actual directories:

```bash
# Build the semantic graph from a project and save it as <dir>/project.aether:
girder analyze sample-project

# Inspect a saved graph and a node's impact set:
girder inspect sample-project/project.aether crate::lib::add

# Concept search: rank functions by relevance to a natural-language query:
girder search sample-project "sum numbers in a list"

# Graph-semantic rename — follows Calls edges (not text search) and rewrites
# only the real callers, then saves the updated graph:
girder refactor sample-project rename crate::lib::add plus

# Preview what the swarm would build — graph-aware Planner only, no code written:
girder swarm-plan sample-project "add user authentication"

# Dispatch the agent swarm on a project with a natural-language intent:
girder forge sample-project "add a subtract function"

# Author a single graph-addressed plan step via a wired-in model (the
# offline MockProvider by default) and execute it through the same
# verified plan executor as `plan run` below, repairing from check
# failures automatically. Never point this — or `plan run --authored` —
# at sample-project/: it is a pinned measurement fixture, not a demo
# target, and both refuse it outright (see "Demo target" below):
girder do demo-project "add an exclamation mark to the farewell"

# Emit graph context, a real Plan Format v2 authoring schema, and a plan
# skeleton as one JSON object — for pasting into any external chat model
# that isn't wired in as a provider. See "External authoring" below for
# the full loop from here to a verified, applied edit:
girder context demo-project --nodes crate::greeter::farewell "add an exclamation mark to the farewell" --json

# Validate/inspect/execute a plan file directly. `--authored` is for a plan
# an external model wrote by hand (see "External authoring" below); without
# it, `run` executes a plan exactly as authored (used internally by `do`):
girder plan validate my-plan.json
girder plan explain my-plan.json
girder plan run my-plan.json --dry

# Semantic code review vs HEAD (typed mutations, not text diffs):
girder review sample-project --since HEAD~1

# Minimal test selection: find every test reachable from changed functions:
girder test-impact sample-project --run

# Knowledge-graph query — answer a question by traversing the semantic graph:
girder query sample-project "what would break if I change add?"
girder query sample-project "what calls sum_list?"
girder query sample-project  # interactive REPL (reads stdin)

# Start a graph-native collaboration history, give another replica its own actor,
# record that replica's current source graph, and deterministically merge it:
girder collab init sample-project alice alice.aetherc
girder collab fork alice.aetherc bob bob.aetherc --approve
girder collab sync sample-project bob.aetherc
girder collab merge alice.aetherc bob.aetherc merged.aethercb
girder collab materialize merged.aethercb merged.aether
# Membership changes are causal operations and require explicit approval:
girder collab member add alice.aetherc carol --approve
girder collab member remove alice.aetherc carol --approve

# Or exchange deltas in a mutually authenticated live loopback session.
# Secret contents are generated with private permissions and never printed.
# Group-secret-only operation remains available as a migration mode:
girder collab secret collaboration.secret

# Strict identity mode additionally pins each roster actor to an Ed25519 key.
# Generate each actor's private/shareable-public pair, compare the printed
# SHA-256 fingerprints out of band, and approve the exact peer fingerprint:
girder collab identity generate alice.aetherc \
  alice.identity alice.identity.pub
girder collab identity generate bob.aetherc \
  bob.identity bob.identity.pub
girder collab identity show bob.identity.pub
girder collab identity trust alice.trust \
  bob.identity.pub --approve <bob-fingerprint>
girder collab identity trust bob.trust \
  alice.identity.pub --approve <alice-fingerprint>
# Sign existing local-authored history now (strict host/join also does this
# in memory before exchange), then audit a fully attested bundle offline:
girder collab identity attest alice.aetherc alice.identity
girder collab identity attest bob.aetherc bob.identity

girder collab host alice.aetherc 127.0.0.1:7331 \
  --identity-file alice.identity --trust-store alice.trust \
  --secret-file collaboration.secret --discovery-dir .bitcode/peers \
  --presence "reviewing parser changes"
girder collab discover bob.aetherc .bitcode/peers \
  --secret-file collaboration.secret
girder collab join-peer bob.aetherc alice .bitcode/peers \
  --identity-file bob.identity --trust-store bob.trust \
  --secret-file collaboration.secret --presence "running transport tests"
# An explicit address remains available when local discovery is not in use:
girder collab join bob.aetherc 127.0.0.1:7331 \
  --secret-file collaboration.secret \
  --identity-file bob.identity --trust-store bob.trust
# Verify every retained non-bootstrap operation against the current local pins:
girder collab identity verify \
  alice.aetherc alice.identity alice.trust
# To rotate your own key, first generate a new pair, then record a dual-signed
# causal transition while both private keys are available. Peers can then rotate
# their current pin before the next strict session:
girder collab identity generate alice.aetherc \
  alice-new.identity alice-new.identity.pub
girder collab identity rotate-local alice.aetherc \
  alice.identity alice-new.identity \
  --from <old-alice-fingerprint> --approve <new-alice-fingerprint>
# Peer trust rotation/removal also requires the exact reviewed fingerprints:
girder collab identity rotate alice.trust \
  bob-new.identity.pub --from <old-bob-fingerprint> --approve <new-bob-fingerprint>
girder collab identity remove alice.trust bob \
  --approve <new-bob-fingerprint>
# Successful sessions persist both peers' causal acknowledgements. Once every
# active member has acknowledged superseded history, prune it conservatively:
girder collab compact alice.aetherc
# Rebuild remote whole-file projections, show semantic/file changes and
# conflicts, then explicitly validate and journal-commit the reviewed bytes:
girder collab review sample-project alice.aetherc
girder collab apply sample-project alice.aetherc --approve

# Real Python execution tracer — records every variable at every line/call/return:
girder debug script.py
girder debug script.py --what-if x=10 at 2

# DAP adapter dry-run: resolve graph breakpoints without launching an adapter:
girder dap script.py --dry-run

# Generate and review an extension recipe without changing the project:
girder extension sample-project generate "show call impact"

# Grant the exact recipe digest/capabilities, then manage its lifecycle:
girder extension sample-project generate "show call impact" --approve
girder extension sample-project list
girder extension sample-project disable dev.bitcode.generated.show-call-impact
girder extension sample-project remove dev.bitcode.generated.show-call-impact

# Install a hand-authored declarative recipe after the same explicit review:
girder extension sample-project install recipe.json --approve

# Browse the built-in reviewed marketplace and inspect a listing:
girder extension sample-project marketplace search impact
girder extension sample-project marketplace show org.bitcode.impact-navigator

# Regenerate a reviewed intent for this project, preview its capability delta,
# then explicitly approve the adapted recipe:
girder extension sample-project marketplace adapt org.bitcode.impact-navigator
girder extension sample-project marketplace adapt org.bitcode.impact-navigator --approve

# Portable catalogs use the same bounded parser and print a catalog fingerprint:
girder extension sample-project marketplace list \
  --catalog marketplace/girder-extensions.json

# Full help:
girder --help
```

`analyze`/`forge` walk every `.rs`, `.py`, `.ts`, `.tsx`, `.mts`, `.cts`, and
`.go` file (skipping `target`, `.git`, …),
build the graph with directory-aware module paths, resolve free and
receiver-qualified method calls across files, and persist the `.aether` graph.
Supported languages are Rust, Python, TypeScript, and Go. The TypeScript and Go
graph surfaces are measured and gated, but remain less mature than Rust and
Python: TypeScript meets its precision and recall gates, while Go currently
records one false negative (micro-recall 0.954545 against a 1.0 gate). See the
honest support boundaries and results for
[TypeScript](docs/typescript-support.md) and [Go](docs/go-support.md), with the
committed observations for
[TypeScript](docs/typescript-support-observation.json) and
[Go](docs/go-support-observation.json).
Rust module-scope imports retain renamed symbol identity across bounded public
re-export chains, including crate-root and `mod.rs` facades, so collisions are
resolved by exact path while ambiguous or cyclic aliases stay unlinked.
Rust parameter annotations and direct type-qualified local constructors provide
bounded receiver types, including inside macro token trees. Function signatures
also supply parser-owned return types for local factory bindings through `?`,
`unwrap`/`expect`, and result-preserving error adapters. Instance factories
returning `Self` resolve to their owning type, while single-argument generic
wrappers propagate an inner receiver only when their signatures prove the same
direct type parameter flows through. Recursive factory hints have a hard size
budget. Unknown receiver types remain unresolved rather than being linked to an
unrelated same-named method.
`forge` plans every candidate byte, checks conflict
baselines, validates the candidate in a copied workspace, runs Cargo build/tests
when a manifest is present plus configured validation commands, and only then
journal-commits the source projection and graph together.

`collab` exchanges semantic graph operations rather than text ranges. Each
human or agent replica has a validated actor id and causal version vector;
minimal idempotent deltas converge regardless of delivery order. Concurrent
deletes win, concurrent updates have a deterministic tie-break, and deleting
then recreating a node cannot resurrect edges from its old generation. RON
`.aetherc` bundles are reviewable; `.aethercb` bundles use compact bincode.
Init/sync reconciles source with the durable graph so graph-owned agent and
extension metadata participates instead of being discarded. Bundle saves use a
synced atomic replacement.

Membership is part of the causal operation history rather than a local address
book. An approved `collab fork` registers the invited actor in both the source
and forked bundles; if writing the fork fails, the source roster is rolled back.
`collab member add|remove ... --approve` records convergent add/remove
operations, concurrent removal wins, normal replica APIs reject new operations
after the local actor is removed, and membership changes invalidate stale
acknowledgements. A history has one genesis self-membership root: separately
initialized actors cannot self-invite through a relayed delta. Removal records
the highest counter observed for that actor; unseen later counters fail until
their context observes a causal re-add, so a removed offline actor cannot keep
extending a stale membership epoch. Version
1 and 2 bundles migrate conservatively by retaining the local actor, previously
acknowledged peers, and non-bootstrap actors already present in the causal
clock. Use `fork` to allocate a new actor replica; direct `member add` is for
re-authorizing an already allocated unique actor, since it does not create that
actor's bundle. Version 1-3 bundles load as explicit unsigned legacy history;
version 4 stores operation attestations without changing existing CRDT dots.

Live host/join uses fresh random challenges, mutual HMAC-SHA256 group
authentication, direction- and sequence-bound message integrity, bounded frames
checked before allocation, and socket timeouts. Secrets are read from
non-symlink regular files owned by the current user with private permissions.
Optional identity mode adds transcript-bound Ed25519 proofs and a local
actor-to-key trust store: both endpoints must configure it, each public key must
match the peer actor's exact pinned fingerprint, and either attempted downgrade
to group-secret-only mode is refused. The signed transcript also authenticates
fresh X25519 keys, deriving a session-integrity key that another group-secret
holder cannot calculate from captured traffic. Private identity files receive
the same ownership, symlink, and permission checks. New trust, rotation, and
removal are explicit fingerprint-approved operations, and identity files are
actor-bound to their collaboration bundle.

Strict identity sessions also give every retained non-bootstrap CRDT operation a
durable Ed25519 attestation. A delta importing a new operation, or a new
retroactive attestation for an already-known dot, triggers verification of that
actor's complete retained history before any bundle is replaced. The signature
binds the dot, causal context, and exact action, so action tampering, actor
forgery, unsigned relay, conflicting proofs, and replayed dot changes fail
atomically. Local legacy operations can be upgraded with `identity attest`;
`identity verify` audits a whole bundle against the local private identity and
peer trust store.

Key rotation is a causal operation signed by the previous key with a second
proof from the successor key. Verification walks this chain backward from the
currently pinned fingerprint, so a retired key remains valid for its historical
counters but cannot authorize later ones. Rotation operations are never removed
by compaction. Deterministic bootstrap snapshot operations remain an explicitly
pre-shared bundle baseline rather than pretending to have a human signature;
strict deltas cannot introduce new bootstrap history. Group-secret-only live
sessions and the unpinned `collab merge` workflow ignore incoming attestation
metadata, preventing an unauthenticated path from poisoning later strict
verification. Actor-key transparency beyond exact local pins is still a
separate layer.

The transport deliberately binds loopback only: graph payloads are authenticated
but not encrypted, so remote peers must connect through an encrypted tunnel such
as SSH. After both sides verify that the other actor is active in the roster and
durably persist a converged version, they persist monotonic peer
acknowledgements. A delta that would remove either authenticated endpoint is
rejected before persistence. A session claiming an unlisted actor is rejected
even with a valid group-secret proof. Host and join may explicitly share a
single-line status of at most 256 UTF-8 bytes. Both statuses and optional public
keys are bound into the authenticated handshake. Status is reported to the peer
and discarded after that synchronization; it is never written to graph
operations, acknowledgements, discovery tickets, or collaboration bundles.

An optional `--discovery-dir` publishes an atomic, HMAC-authenticated,
process-bound lease for the loopback host. The current-user directory and
tickets must be private (new paths are mode 700 and 600 on Unix), symlinks are
rejected, each scan is capped at 256 entries, dead process ids on Unix and
actors outside the local bundle's active remote roster are ignored, and
`join-peer` refuses ambiguous same-actor tickets. A normal host shutdown removes
its unchanged ticket.
Tickets are only endpoint hints: PID reuse or a stale ticket cannot authorize a
session because the existing roster-bound mutual-authentication handshake still
decides every join.

`collab compact`
requires an acknowledgement from every active remote member, then prunes only
causally superseded operations while retaining concurrent winners, membership
removal barriers, and node-generation tombstones. Peers older than the recorded
history floor fail safely and need a current bundle. Network/continuous
discovery, continuous presence/subscriptions, encrypted remote transport, and
operation-level signatures/key transparency remain future work.
Group-secret-only migration mode remains a group credential: roster checks
reject an unlisted claimed actor, but any secret holder can impersonate an
active actor. Pinned identity mode prevents that endpoint impersonation after
fingerprints have been verified. It does not retroactively prove the author of
every historical CRDT operation: authenticated peers can relay the existing
multi-actor operation set, so provenance of stored history remains trusted at
the collaboration-group boundary until operations themselves are signed.

Every parsed module carries a bounded `file-v1` whole-file projection in the
semantic graph. `collab review` compares the remote and freshly reconciled local
graphs, lists file and semantic changes, and reparses every remote file to prove
its nodes and projection-derived edges agree with the claimed graph. Missing
modules, path escapes, oversized files, inconsistent concurrent winners, and
stale local baselines are conflicts. `collab apply --approve` reruns the plan,
executes configured validation in an isolated candidate, then journal-commits
added/modified/deleted files and the reconciled graph together. Native approval
is SHA-256-bound to every candidate and expected baseline byte, so any project
or bundle change forces another review.

### Project configuration

Girder works without configuration. To materialize and inspect the validated
defaults for a project:

```bash
girder config sample-project --init
girder config sample-project
```

`girder.toml` controls source roots and ignore globs, symlink policy, graph
storage, structured impacted-test commands, the agent output module/file, and
candidate-validation commands, timeouts, diagnostic limits, and copy budgets.
Commands are argv arrays rather than interpolated shell strings. On Linux,
available `bubblewrap` support mounts the host filesystem read-only while the
candidate and approved build caches remain writable; other platforms still run
inside the disposable candidate copy. Invalid keys, escaping paths, broken
globs, unsupported graph extensions, malformed commands, and unsupported config
versions fail before project analysis starts.

## External authoring: `girder context` + `plan run --authored`

`girder do` drives a wired-in provider (OpenAI, Anthropic, Ollama, or the
offline `MockProvider`) automatically. `girder context` and `plan run
--authored` split that same workflow at the model boundary, so *any* chat
model — one with no API integration in this codebase at all — can author a
verified graph edit.

> **Reading, not authoring?** Add `--source-only`. The default output below
> carries a Plan Format v2 schema and plan skeleton, which is a fixed ~6 KB
> that a model authoring an edit needs and a model merely *reading* code does
> not — it measured *more expensive* than reading the whole file on 2 of 10
> nodes, and 15.6× the file for a small one. `--source-only` drops the
> envelope and measured 97.85% cheaper than a file read across the same ten
> nodes ([`docs/context-vs-read-cost.md`](./docs/context-vs-read-cost.md)).
> It also needs no git repository, having no `base_commit` to pin.

The authoring loop:

```bash
# 1. Emit graph context, a real Plan Format v2 authoring schema, and a plan
#    skeleton (base_commit, on_failure, the mandatory tests.impacted check)
#    as one JSON object:
girder context demo-project --nodes crate::greeter::farewell \
  "add an exclamation mark to the farewell" --json > context.json

# 2. Paste context.json into any chat model. Ask it to fill in the plan
#    skeleton's step (id, description, edits) so the step satisfies the
#    schema exactly, and save its response as plan.json — the schema is the
#    literal Plan Format v2 the executor loads, not an approximation of it,
#    so a model that follows it produces a file that loads without
#    translation.

# 3. Run and verify it. `plan run` has no project-directory argument — the
#    root is always the current directory, so run this from inside
#    demo-project/. --authored applies the same harness guarantees `do`
#    applies internally to a model-authored plan: on_failure is forced to
#    rollback_plan, a tests.impacted check is injected if the plan doesn't
#    already carry one, and a zero-step plan is refused outright rather than
#    passing vacuously. --authored-by <name> records who authored it in the
#    written report:
(cd demo-project && girder plan run ../plan.json --authored --authored-by claude-opus-5)
```

A passing run applies the edit to the real tree and writes a report under
`.girder/reports/`; a failing check rolls the tree back to `base_commit`
automatically, so a bad edit from an untrusted external model never lands
half-applied. `plan_schema()`'s exact shape (a `oneOf` discriminated union
per edit/check kind, matching `planfile::schema`'s deserializer field for
field) is what makes step 2 reliable — see gap 17 in
[`docs/core-gap-analysis.md`](docs/core-gap-analysis.md) for the defect this
closed and the round-trip test that proves it.

### Demo target: `demo-project/`, never `sample-project/`

`girder do` and `plan run --authored` both refuse `sample-project/` as a
target outright, with an error naming `demo-project/` as the place to run
instead:

```
$ girder do sample-project "uppercase greet's return value"
error: sample-project/ is a pinned measurement fixture (see gap 18/21 in
docs/core-gap-analysis.md) and refuses authored writes; run demos against
demo-project/ instead
```

`sample-project/` is a pinned baseline `tools/authoring_task_check.py` and
`tools/plan_executor_oracle.py` read against a specific clean source commit
for the graph-addressed-authoring-cost measurement corpus, not a scratch
target. A real (non-dry) authored write there mutates the same file the
measurement harness depends on — this happened twice, three days apart, and
cost a full day of debugging a broken referee before the fixture drift was
found; see gap 18 in `docs/core-gap-analysis.md`. `demo-project/` exists so
that never has to happen again: a small, disposable Python project nothing
under `tools/` or `docs/` reads, safe for real (non-dry) authored writes.
Read-only commands (`context`, `search`, `analyze`, `test-impact`) still work
against `sample-project/` — only the two commands that write are refused.

### The pipeline demo

`girder` (no args) walks the entire pipeline over stdout:

1. **Builds a semantic graph** from source with tree-sitter (the graph is the
   source of truth; text is a projection).
2. **Dispatches the agent swarm** on a natural-language intent
   (*"Add a multiply function to the math module"*). The graph-aware Planner reads
   existing nodes before planning; the Coder generates each function, wires Calls
   edges, and emits a FeatureComplete summary; Tester, Documenter, Refactorer,
   SecurityAuditor, and Optimizer annotate in parallel.
3. **Runs predictive impact analysis** over the graph.
4. **Time-travels a deliberate bug**: records an execution trace via `sys.settrace`,
   branches a what-if alternative (variable overridden at a specific step via
   CPython's `PyFrame_LocalsToFast`), shows exactly where the two timelines diverge.

## Workspace

| Crate | Role |
|---|---|
| `aether-graph` | The living semantic graph: nodes/edges, impact analysis, `.aether` serialization, and convergent collaboration replicas. **Source of truth.** |
| `aether-builder` | tree-sitter → graph mapping, incremental edit sync, syntax-highlight spans. |
| `aether-ai` | `AiProvider` trait, offline `MockProvider`, OpenAI/Anthropic live providers, multi-provider `Router`, provider extension points. |
| `aether-agents` | The parallel swarm: message bus, orchestrator, 8 specialized agents. |
| `aether-debugger` | Recording interpreter, execution trace, branching timeline, what-if + AI root-cause. |
| `aether-dap` | Debug Adapter Protocol client/session layer with graph-aware breakpoint support. |
| `aether-extensions` | Strict declarative recipes, digest-bound grants, graph-native lifecycle, bounded UI and project contributions. |
| `aether-app` | The `girder` binary: every CLI subcommand, plus the egui/wgpu GUI behind feature `gui`. |

## The GUI

```bash
cargo run -p aether-app --features gui -- --gui sample-project
```

The native workspace opens a configured project, indexes its Rust, Python, and
TypeScript source,
provides file navigation and language-aware editing, folds edits into the graph,
and reconciles those fresh projections with the durable graph. Agent summaries,
graph-owned nodes, and inferred relationships survive reopen and editor refresh
while source-derived structure follows the files on disk.

The semantic graph is an interactive navigation surface rather than a static
diagram. Its retained force layout keeps positions stable while the graph
changes; pan/zoom, fit-to-view, text search, node/edge type filters, one- and
two-hop focus, viewport culling, and zoom-dependent labels keep large graphs
legible. At overview scale, implementation nodes and their relationships
collapse into weighted module-level connections; zooming in restores exact
types, functions, and enabled relationships. The overview ranks and bounds
module connections so the strongest architectural signals remain readable.
Selecting a node exposes its source location and agent metadata, and
double-clicking or choosing **Open source** moves the editor cursor to its span.

Editor saves use a recoverable journaled transaction for the source projection
and semantic graph. Dirty buffers guard project/file switches, saves detect
external source or graph changes, and startup rolls back an interrupted
multi-file commit before indexing. GUI agent runs mutate a checkpointed graph
and remain visibly pending while validation runs off the render thread. Commit
stays disabled until the exact graph snapshot passes candidate build/tests;
diagnostics remain inspectable and failed candidates can be revalidated or
rolled back. **Commit** projects generated functions to the configured output
file and persists the graph, while **Roll back** cancels validation and restores
the pre-run graph without touching source files.

The right workspace has separate **Agents**, **Extensions**, **Collaboration**,
and **Author** views. Extensions contains Generate, Marketplace, and Installed
tabs. Extension generation returns
a strict JSON recipe; installation stays disabled until the user reviews its
exact SHA-256 digest, capabilities, contributions, projections, and full JSON.
The marketplace searches bounded declarative catalogs, displays the catalog and
listing/reference-recipe fingerprints plus reviews bound to both, and
regenerates a listing intent against a bounded sample of the current semantic graph. Adapted
recipes preserve the listing ID and show every added/removed capability scope.
A catalog review never grants installation authority: the adapted recipe still
requires a fresh approval bound to its own exact digest.
Reviewer identities are catalog metadata rather than cryptographic signatures;
verify an external catalog's printed SHA-256 fingerprint through the channel
that distributed it.

The **Collaboration** view initializes, inspects, and synchronizes the full
workspace graph, generates private secrets, verifies bounded local discovery
tickets, and lets the user select an authenticated active-roster endpoint
before joining on a background thread so rendering never blocks. It shows
causal version/operation counts and the deterministic conflict policy, durable
acknowledgements, and compacted history floor; it can conservatively compact
acknowledged history. A live join updates the collaboration bundle only.
Optional identity controls generate an actor-bound key, display a peer's exact
fingerprint for out-of-band review, pin the unchanged public file, list trusted
actors, and make strict identity mode visibly distinct from legacy group-secret
mode. Explicit rotation/removal remains available in the CLI.
Separate Review and Apply controls keep remote graph-to-source projection
explicit, consistency-checked, digest-bound, validated, and atomic. CLI `host`
is the persistent serving surface.

The **Author** view exposes both authoring paths from "External authoring"
above without a terminal: type an intent, click **Search** to run the same
concept search `girder do`/`context` use (only the top-scored hit starts
checked — narrower than the CLI default on purpose, since gap 15 in
`docs/core-gap-analysis.md` exists to shrink what a model can touch), and
adjust the checkboxes to pin the exact nodes a plan may edit — the same
`--nodes` a terminal invocation would otherwise require typing full paths
for. **Local model** mode mirrors `girder do`: **Run** streams each
provider attempt into a live log and shows pass/fail plus the report.
**External model** mode mirrors `girder context` + `plan run --authored`:
**Copy context JSON** puts exactly what the CLI command would print onto the
clipboard, paste a model's plan response back in, and **Run authored**
applies the same guarantees (forced `rollback_plan`, the mandatory
`tests.impacted` check, zero-step refusal). A non-dry Run in either mode
requires a second, explicit confirming click — a GUI button that silently
writes to the tree has no command line to review first. Open `demo-project/`
to try the full loop for real:

```bash
cargo run -p aether-app --features gui -- --gui demo-project
```

Then, in the Author tab: type "add an exclamation mark to the farewell",
click Search, leave `crate::greeter::farewell` checked, and either click Run
(Local model) or use Copy Context JSON / paste a plan back in / Run authored
(External model) — Dry run stays checked by default in both, so nothing
writes to the tree until it's unchecked and confirmed. Like `girder do`
against a terminal, both refuse `sample-project/` outright; only
`demo-project/` (or another project you point it at) accepts a real,
non-dry Run.

Installed records and their contribution nodes live in the semantic graph and
survive source reconciliation. Enable/disable affects only contribution
visibility. Removal conflict-checks every installed projection, restores
replaced files, deletes files created by the extension, and commits the project
plus graph as one recoverable transaction. Model output is never loaded as
native code. Contributed validation commands resolve back to an installed,
enabled recipe and require Bubblewrap; they run with networking disabled and a
cleared environment in the disposable candidate workspace.

CI type-checks the `gui` feature on the stable Rust toolchain. Rendering needs a
GPU — or a software Vulkan adapter (Mesa **lavapipe**) plus the usual X11 libs
(e.g. `libxkbcommon-x11`). If no surface can be created the binary logs the wgpu
error and **falls back to the headless demo** automatically, so it never
hard-fails. It has been verified headlessly under `Xvfb` + lavapipe:

> The **default headless build, all tests, and `girder`'s demo require none
> of this** — no GPU, display, or extra system libraries.

## Local-first AI

The default provider is a deterministic, offline `MockProvider`, so everything
runs with no network and no API key. OpenAI and Anthropic are implemented live
providers behind the `live-providers` feature. OpenAI uses the Responses API and
is selected when `OPENAI_API_KEY` is set:

```bash
export OPENAI_API_KEY=sk-...
export OPENAI_MODEL=gpt-5.6   # optional; this is the default
cargo run -p aether-app --features live-providers -- forge sample-project "add a divide function"
```

Anthropic remains available as a second live provider:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
cargo run -p aether-app --features live-providers -- forge sample-project "add a divide function"
```

The `Router` tries OpenAI first when configured, uses Anthropic as a secondary
live provider for planning/codegen, and falls back to the mock otherwise. Gemini,
xAI Grok, and local Ollama remain compile-clean extension-point structs; they
deliberately do not enter routing until their HTTP bodies are implemented.

## Status

A focused, honest prototype: the three pillars (graph-as-truth, agent swarm,
time-travel debug) are real, tested, and runnable.

The checked
[Core Trustworthiness Measurement](docs/core-trustworthiness-measurement.md)
compares affected-test selection with isolated runtime execution. Its bounded
baseline measures both Rust and Python precision/recall at `1.000/1.000`
(the earlier `0.667` Rust precision defect is closed). That is a result on
small fixtures, not a representative-repository superiority claim — and the
counter-evidence is checked in alongside it: on a real dependency, a
polymorphic-dispatch mutation measured recall `0.000`
([`docs/core-representative-mutations.md`](docs/core-representative-mutations.md)),
which is why test selection is documented as advisory rather than
authoritative.

Implemented features:

| Feature | What it does |
|---|---|
| Semantic graph | Nodes/edges, Rust alias/return/scoped-pattern/Cargo-entrypoint-aware and Python alias/nullable-annotation/constructor-aware cross-file call resolution, literal-aware macro calls, impact BFS, similarity edges, strict versioned `.aether` persistence and source reconciliation |
| Graph explorer | Retained force layout, pan/zoom, search and typed filters, neighborhood focus, LOD/culling, metadata inspection, source navigation |
| Agent swarm | Planner + Coder + Tester + Documenter + Refactorer + Optimizer + SecurityAuditor + QueryAgent |
| Intent-first planning | Planner reads the graph before planning; generates ordered `FeatureSpec` with Calls-edge wiring |
| Real Python tracer | `sys.settrace` execution recording, what-if branching via `PyFrame_LocalsToFast` |
| Knowledge-graph queries | Natural-language → concept / impact / callers / callees / explain / neighbourhood |
| Semantic review | Typed diff (added/modified/removed nodes + edges), impact radius, test gap report |
| Minimal test selection | Call-graph reachability from changed functions, optional `--run` |
| Graph collaboration | Deterministic operation-set CRDT, causal membership/deltas/tombstones, atomic RON/bincode bundles, roster-gated authenticated loopback host/join with optional downgrade-resistant pinned Ed25519 actor identities, private authenticated local discovery leases, all-member acknowledgement compaction, and reviewed whole-file source projection |
| Project contract | Validated `girder.toml` for source scope, graph path, test runners, and agent output |
| Source projection | GUI/CLI agent output and graph rename commit validated source plus graph through recoverable journaled transactions |
| Candidate validation | Disposable project copy, optional bubblewrap isolation, Cargo build/tests, configured checks, cancellation/timeouts, bounded diagnostics, snapshot-bound commit gate |
| DAP integration | Two-phase DAP launch, graph-node breakpoints, stop/stack inspection, and real `debugpy` coverage |
| Declarative extensions | AI/JSON recipe generation, exact digest-bound approval, parameterized capabilities, graph-native records/contributions, GUI/CLI lifecycle, reversible validated projections |
| Generative marketplace | Bounded portable catalogs, deterministic fingerprints/search, digest-bound reviews, project-aware regeneration, exact capability deltas, CLI and native browser |
| External authoring | `girder context` emits graph context plus a real Plan Format v2 schema for any external chat model; `plan run --authored` re-enforces rollback/impacted-test guarantees on the result |

Optional DAP adapter smoke test:

```bash
python3 -m pip install debugpy
cargo test -p aether-dap --test debugpy -- --ignored --nocapture
```

The measured table-stakes comparison, current correctness evidence, and
prioritized open risks are maintained in
[`docs/core-gap-analysis.md`](docs/core-gap-analysis.md).

## License

Girder is **source-available**, not open source, under the
[Business Source License 1.1](./LICENSE).

The free tier is genuinely free and permanent: it has no expiry and requires no
account. For a single repository it includes `get_source`, `find_definition`,
`search_code`, `ask_codebase`, and `review_changes`. The `orient` and
`impacted_tests` tools require a paid license.

Licenses are signed keys verified locally by the Girder binary. The binary
never phones home, makes no network call for licensing, and works fully offline
in both tiers.

On September 4, 2030, the license converts to the Apache License, Version 2.0.
See [`LICENSE`](./LICENSE) for the authoritative terms and
[`CONTRIBUTING.md`](./CONTRIBUTING.md) for contribution terms.
