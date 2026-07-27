# Bit Code

Bit Code is a native Rust IDE prototype whose source of truth is a living
semantic graph of your codebase. It combines graph-aware editing, a parallel
agent workflow, semantic review/test impact, and time-travel debugging.

This repo is a runnable Rust prototype of that architecture. It is *not* a fork
of any existing editor. See [`BLUEPRINT.md`](./BLUEPRINT.md) for the full design.

## Quickstart

```bash
# Full end-to-end demo — no GPU, display, or API key required:
cargo run -p aether-app

# Run the test suite (graph, builder, AI router, agent swarm, debugger):
cargo test --workspace

# Optional: compile live providers and run the real debugpy adapter test:
cargo check -p aether-ai --features live-providers
python3 -m pip install debugpy
cargo test -p aether-dap --test debugpy -- --ignored
```

### Use it on a real project

Bit Code is also a CLI that operates on actual directories:

```bash
# Build the semantic graph from a project and save it as <dir>/project.aether:
cargo run -p aether-app -- analyze sample-project

# Inspect a saved graph and a node's impact set:
cargo run -p aether-app -- inspect sample-project/project.aether crate::lib::add

# Concept search: rank functions by relevance to a natural-language query:
cargo run -p aether-app -- search sample-project "sum numbers in a list"

# Graph-semantic rename — follows Calls edges (not text search) and rewrites
# only the real callers, then saves the updated graph:
cargo run -p aether-app -- refactor sample-project rename crate::lib::add plus

# Preview what the swarm would build — graph-aware Planner only, no code written:
cargo run -p aether-app -- plan sample-project "add user authentication"

# Dispatch the agent swarm on a project with a natural-language intent:
cargo run -p aether-app -- forge sample-project "add a subtract function"

# Semantic code review vs HEAD (typed mutations, not text diffs):
cargo run -p aether-app -- review sample-project --since HEAD~1

# Minimal test selection: find every test reachable from changed functions:
cargo run -p aether-app -- test-impact sample-project --run

# Knowledge-graph query — answer a question by traversing the semantic graph:
cargo run -p aether-app -- query sample-project "what would break if I change add?"
cargo run -p aether-app -- query sample-project "what calls sum_list?"
cargo run -p aether-app -- query sample-project  # interactive REPL (reads stdin)

# Start a graph-native collaboration history, give another replica its own actor,
# record that replica's current source graph, and deterministically merge it:
cargo run -p aether-app -- collab init sample-project alice alice.aetherc
cargo run -p aether-app -- collab fork alice.aetherc bob bob.aetherc --approve
cargo run -p aether-app -- collab sync sample-project bob.aetherc
cargo run -p aether-app -- collab merge alice.aetherc bob.aetherc merged.aethercb
cargo run -p aether-app -- collab materialize merged.aethercb merged.aether
# Membership changes are causal operations and require explicit approval:
cargo run -p aether-app -- collab member add alice.aetherc carol --approve
cargo run -p aether-app -- collab member remove alice.aetherc carol --approve

# Or exchange deltas in a mutually authenticated live loopback session.
# Secret contents are generated with private permissions and never printed.
# Group-secret-only operation remains available as a migration mode:
cargo run -p aether-app -- collab secret collaboration.secret

# Strict identity mode additionally pins each roster actor to an Ed25519 key.
# Generate each actor's private/shareable-public pair, compare the printed
# SHA-256 fingerprints out of band, and approve the exact peer fingerprint:
cargo run -p aether-app -- collab identity generate alice.aetherc \
  alice.identity alice.identity.pub
cargo run -p aether-app -- collab identity generate bob.aetherc \
  bob.identity bob.identity.pub
cargo run -p aether-app -- collab identity show bob.identity.pub
cargo run -p aether-app -- collab identity trust alice.trust \
  bob.identity.pub --approve <bob-fingerprint>
cargo run -p aether-app -- collab identity trust bob.trust \
  alice.identity.pub --approve <alice-fingerprint>
# Sign existing local-authored history now (strict host/join also does this
# in memory before exchange), then audit a fully attested bundle offline:
cargo run -p aether-app -- collab identity attest alice.aetherc alice.identity
cargo run -p aether-app -- collab identity attest bob.aetherc bob.identity

cargo run -p aether-app -- collab host alice.aetherc 127.0.0.1:7331 \
  --identity-file alice.identity --trust-store alice.trust \
  --secret-file collaboration.secret --discovery-dir .bitcode/peers \
  --presence "reviewing parser changes"
cargo run -p aether-app -- collab discover bob.aetherc .bitcode/peers \
  --secret-file collaboration.secret
cargo run -p aether-app -- collab join-peer bob.aetherc alice .bitcode/peers \
  --identity-file bob.identity --trust-store bob.trust \
  --secret-file collaboration.secret --presence "running transport tests"
# An explicit address remains available when local discovery is not in use:
cargo run -p aether-app -- collab join bob.aetherc 127.0.0.1:7331 \
  --secret-file collaboration.secret \
  --identity-file bob.identity --trust-store bob.trust
# Verify every retained non-bootstrap operation against the current local pins:
cargo run -p aether-app -- collab identity verify \
  alice.aetherc alice.identity alice.trust
# To rotate your own key, first generate a new pair, then record a dual-signed
# causal transition while both private keys are available. Peers can then rotate
# their current pin before the next strict session:
cargo run -p aether-app -- collab identity generate alice.aetherc \
  alice-new.identity alice-new.identity.pub
cargo run -p aether-app -- collab identity rotate-local alice.aetherc \
  alice.identity alice-new.identity \
  --from <old-alice-fingerprint> --approve <new-alice-fingerprint>
# Peer trust rotation/removal also requires the exact reviewed fingerprints:
cargo run -p aether-app -- collab identity rotate alice.trust \
  bob-new.identity.pub --from <old-bob-fingerprint> --approve <new-bob-fingerprint>
cargo run -p aether-app -- collab identity remove alice.trust bob \
  --approve <new-bob-fingerprint>
# Successful sessions persist both peers' causal acknowledgements. Once every
# active member has acknowledged superseded history, prune it conservatively:
cargo run -p aether-app -- collab compact alice.aetherc
# Rebuild remote whole-file projections, show semantic/file changes and
# conflicts, then explicitly validate and journal-commit the reviewed bytes:
cargo run -p aether-app -- collab review sample-project alice.aetherc
cargo run -p aether-app -- collab apply sample-project alice.aetherc --approve

# Real Python execution tracer — records every variable at every line/call/return:
cargo run -p aether-app -- debug script.py
cargo run -p aether-app -- debug script.py --what-if x=10 at 2

# DAP adapter dry-run: resolve graph breakpoints without launching an adapter:
cargo run -p aether-app -- dap script.py --dry-run

# Generate and review an extension recipe without changing the project:
cargo run -p aether-app -- extension sample-project generate "show call impact"

# Grant the exact recipe digest/capabilities, then manage its lifecycle:
cargo run -p aether-app -- extension sample-project generate "show call impact" --approve
cargo run -p aether-app -- extension sample-project list
cargo run -p aether-app -- extension sample-project disable dev.bitcode.generated.show-call-impact
cargo run -p aether-app -- extension sample-project remove dev.bitcode.generated.show-call-impact

# Install a hand-authored declarative recipe after the same explicit review:
cargo run -p aether-app -- extension sample-project install recipe.json --approve

# Browse the built-in reviewed marketplace and inspect a listing:
cargo run -p aether-app -- extension sample-project marketplace search impact
cargo run -p aether-app -- extension sample-project marketplace show org.bitcode.impact-navigator

# Regenerate a reviewed intent for this project, preview its capability delta,
# then explicitly approve the adapted recipe:
cargo run -p aether-app -- extension sample-project marketplace adapt org.bitcode.impact-navigator
cargo run -p aether-app -- extension sample-project marketplace adapt org.bitcode.impact-navigator --approve

# Portable catalogs use the same bounded parser and print a catalog fingerprint:
cargo run -p aether-app -- extension sample-project marketplace list \
  --catalog marketplace/bitcode-extensions.json

# Full help:
cargo run -p aether-app -- --help
```

`analyze`/`forge` walk every `.rs`/`.py` file (skipping `target`, `.git`, …),
build the graph with directory-aware module paths, resolve free and
receiver-qualified method calls across files, and persist the `.aether` graph.
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

Bit Code works without configuration. To materialize and inspect the validated
defaults for a project:

```bash
cargo run -p aether-app -- config sample-project --init
cargo run -p aether-app -- config sample-project
```

`bitcode.toml` controls source roots and ignore globs, symlink policy, graph
storage, structured impacted-test commands, the agent output module/file, and
candidate-validation commands, timeouts, diagnostic limits, and copy budgets.
Commands are argv arrays rather than interpolated shell strings. On Linux,
available `bubblewrap` support mounts the host filesystem read-only while the
candidate and approved build caches remain writable; other platforms still run
inside the disposable candidate copy. Invalid keys, escaping paths, broken
globs, unsupported graph extensions, malformed commands, and unsupported config
versions fail before project analysis starts.

### The pipeline demo

`cargo run -p aether-app` (no args) walks the entire pipeline over stdout:

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
| `aether-app` | egui/wgpu GUI (feature `gui`) + the headless `smoke` demo binary. |

## The GUI

```bash
cargo run -p aether-app --features gui -- --gui sample-project
```

The native workspace opens a configured project, indexes its Rust/Python source,
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

The right workspace has separate **Agents** and **Extensions** views. Extensions
contains Generate, Marketplace, and Installed tabs. Extension generation returns
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

![Bit Code GUI](docs/bitcode-gui.png)

> The **default headless build, all tests, and the `cargo run` demo require none
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
time-travel debug) are real, tested, and runnable. Implemented features:

| Feature | What it does |
|---|---|
| Semantic graph | Nodes/edges, Rust alias/return/scoped-pattern-aware and Python annotation-aware cross-file call resolution, literal-aware macro calls, impact BFS, similarity edges, strict versioned `.aether` persistence and source reconciliation |
| Graph explorer | Retained force layout, pan/zoom, search and typed filters, neighborhood focus, LOD/culling, metadata inspection, source navigation |
| Agent swarm | Planner + Coder + Tester + Documenter + Refactorer + Optimizer + SecurityAuditor + QueryAgent |
| Intent-first planning | Planner reads the graph before planning; generates ordered `FeatureSpec` with Calls-edge wiring |
| Real Python tracer | `sys.settrace` execution recording, what-if branching via `PyFrame_LocalsToFast` |
| Knowledge-graph queries | Natural-language → concept / impact / callers / callees / explain / neighbourhood |
| Semantic review | Typed diff (added/modified/removed nodes + edges), impact radius, test gap report |
| Minimal test selection | Call-graph reachability from changed functions, optional `--run` |
| Graph collaboration | Deterministic operation-set CRDT, causal membership/deltas/tombstones, atomic RON/bincode bundles, roster-gated authenticated loopback host/join with optional downgrade-resistant pinned Ed25519 actor identities, private authenticated local discovery leases, all-member acknowledgement compaction, and reviewed whole-file source projection |
| Project contract | Validated `bitcode.toml` for source scope, graph path, test runners, and agent output |
| Source projection | GUI/CLI agent output and graph rename commit validated source plus graph through recoverable journaled transactions |
| Candidate validation | Disposable project copy, optional bubblewrap isolation, Cargo build/tests, configured checks, cancellation/timeouts, bounded diagnostics, snapshot-bound commit gate |
| DAP integration | Two-phase DAP launch, graph-node breakpoints, stop/stack inspection, and real `debugpy` coverage |
| Declarative extensions | AI/JSON recipe generation, exact digest-bound approval, parameterized capabilities, graph-native records/contributions, GUI/CLI lifecycle, reversible validated projections |
| Generative marketplace | Bounded portable catalogs, deterministic fingerprints/search, digest-bound reviews, project-aware regeneration, exact capability deltas, CLI and native browser |

Optional DAP adapter smoke test:

```bash
python3 -m pip install debugpy
cargo test -p aether-dap --test debugpy -- --ignored --nocapture
```

The production roadmap (continuous/network collaboration discovery and
presence, encrypted remote transport, key transparency, self-optimization,
web/mobile projections, and deeper tracing) is in `BLUEPRINT.md §9`.

The measured table-stakes comparison, current correctness evidence, and
prioritized open risks are maintained in
[`docs/core-gap-analysis.md`](docs/core-gap-analysis.md).
