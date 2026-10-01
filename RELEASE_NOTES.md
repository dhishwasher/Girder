# Girder 0.3.0

115 commits since 0.2.7, written from `git log v0.2.7..HEAD`, not from
memory. Girder's core product remains unchanged in this release: it saves
AI coding-agent context by giving the agent exactly the code it needs
instead of whole files, from a local semantic graph. See `README.md`.

As an additional capability on that same graph, `test-impact` /
`impacted_tests` is no longer paid-gated in this release. Telling a
developer which tests a change may affect is not a chargeable feature; see
"Pricing" in `README.md`. There is no current paid tier. A narrow,
CI/PR-focused paid feature is designed but not built
(`docs/ci-gate-design.md`).

**Machine-checked Must/May/Unknown test classification is now wired through
the CLI and MCP surface.** Every call site `test-impact`/`impacted_tests`
reasons about is labeled Must (proven reachable), May (bounded ambiguity),
or Unknown (disclosed, not guessed) — `--classified` shows why each test
was selected, and an unresolved dispatch includes the related test rather
than silently dropping it (`docs/call-classification-policy.md`). This
closed a real, found-and-fixed soundness program spanning Stage 1 through
Stage 3:

- **A real node-identity collision bug**, where two distinct semantic
  entities (e.g. two Rust trait impls providing a same-named method on the
  same type) could compute the identical internal graph id, silently
  overwriting the earlier one's entire node — including its own call
  evidence. Found via this program's own review discipline, fixed for
  Rust's trait-impl shape (commit `67dfae9`). TypeScript has a separate,
  disclosed instance of the same bug class (duplicate `it()`/`describe()`
  description strings) that remains open — see gap 28 in
  `docs/core-gap-analysis.md`.
- **Three independent false-empty `test-impact` symptoms**, each with the
  identical root pattern (a narrow, resolved-reachability-only selection
  returning nothing, with no fallback or notice, whenever evidence
  couldn't be resolved) — in the bare `--quiet` path, the full/`--run`/
  `--out` path, and independently in Plan Format v2's own mandatory
  `tests.impacted` check. All three now fall back to the conservative
  Must∪May∪Unknown union instead of returning nothing.
- As a disclosed side effect, not a claimed fix: a long-documented
  "representative mutation recall 0.000" defect (a Click polymorphic
  dispatch Girder still cannot resolve) now measures recall `1.000`,
  precision `0.667` — because the selection mechanism stopped silently
  dropping the affected test, not because the underlying dispatch
  resolution gap closed. That gap is still open
  (`docs/core-representative-mutations.md`).

**Language maturity, stated plainly:** Rust and Python Stage 3 audits
(hand-labeled ground truth against real open-source repositories) are both
DONE. TypeScript's Stage 3 audit is explicitly IN PROGRESS, not done. Go has
not had the same Stage 3 audit run against it at all (see gap 29 in
`docs/core-gap-analysis.md`) — its only committed measurement is the
narrower `docs/go-support.md` gate.

**CI note, not a feature:** this release also fixes a GitHub Actions
Clippy failure present since commit `67dfae9`, caused by an unpinned
toolchain picking up a newer `clippy::double_must_use` lint against
`async-trait`'s generated code, not by any bug in this program's own
commits (`docs/roadmap.md`, 2026-10-01 entries).

**Version note:** 0.3.0 rather than a patch bump because the Must/May/
Unknown classification surface and the test-impact ungating are both new,
user-visible capability on the additional-capability side, not just a fix.
Tagged and published: GitHub release, npm package, and MCP registry
listing.

# Girder 0.3.1

A positioning-only correction, no code or feature changes. 0.3.0's own
release notes above led with the test-impact ungating as "the headline
change" of that release — stated here plainly as a correction, not left
to stand: Girder's core product identity is, and remains, saving AI
coding-agent context by giving the agent exactly the code it needs
instead of whole files; test-impact/change-safety is a secondary
capability built on the same graph, not a co-equal part of what Girder
is. `README.md`, `npm/README.md`, the GitHub repository description, and
`server.json`'s MCP registry description are corrected to lead with
context-saving and the measured context-reduction numbers
(`get_source` 97.85% fewer bytes, `find_definition` 97.98% fewer bytes,
`orient` 48,814 vs 101,302 bytes / 15 vs 78 calls — output bytes, not
tokens; no tokenizer was run), with test-impact moved below as an
additional capability. Published so the corrected text reaches the public
npm page and package, not just the GitHub source tree.

# Girder 0.2.7

Girder 0.2.7 makes `orient` available without a license and adds `girder
setup` for installing advisory Claude Code and Codex hooks alongside the MCP
server. The read hook suggests Girder context before whole-file reads. After a
successful edit, the hook reports the saved graph's bounded blast radius and
which reached functions lack a covering test.

The edit advisory uses a precomputed, path-indexed cache and keeps its frozen
20 ms computation ceiling. On all 141 eligible events in the committed
cross-language mutation corpus, the home-local release binary recorded zero
timeouts and zero failures (p50 0.248 ms, p95 2.289 ms, max 3.509 ms). The hook
remains fail-open and makes no network calls.

# Unreleased benchmark work

The competitive benchmark now has a frozen, resource-bounded foundation for
comparing Girder with GitNexus, codebase-memory-mcp, code-review-graph, and
ripwire. It commits the corpus, mutation sequence, independent oracle, scoring
rules, version and artifact pins, adapter contract, resource limits, and narrow
unit tests before any comparative run. Measured results are added here only
after each adapter and campaign checkpoint is complete.

The first adapter checkpoint now covers both Girder 0.2.6 MCP modes through
the common harness. Its retained tiny-fixture validation publishes the known
dynamic-dispatch losses and keeps invalidated preflight runs separate. These
adapter checks are excluded from the later competitive aggregate.

Ripwire 0.5.0 is the first external adapter. Its checksum-pinned CLI mapping,
body-source check, symbol-scoped test-file projection, pagination boundaries,
and raw preflight failures are frozen before the scored small-fixture run.

The first valid external small-fixture comparison is now recorded under frozen
policy revision 5. Girder normal, Girder watch, and Ripwire each completed the
tiny Python campaign with 40 PASS and 60 WRONG query records. All found the
static definition and direct callee; all missed the frozen function-parameter
callback in callers and reverse impact. Girder normal returned 41,958 query
response bytes versus Ripwire's 122,926, while using 140 calls versus 120.
Ripwire recalled both relevant tests by selecting their whole file, with one
unrelated false positive; Girder returned one relevant test and no unrelated
test in the base warm query. This fixture is an adapter gate and is excluded
from the final competitive aggregate.

The codebase-memory-mcp 0.10.8 adapter is now frozen before its first scored
run. Native preflight corrected the not-yet-measured mapping to the pinned
server's inbound/outbound trace directions, finite depth 32, boolean
persistence, cursor pagination, and JSON group shapes. Two low-resource
placement failures remain published: the removable mount could not satisfy the
product's private-cache ancestry check, and a removable-drive executable missed
its fixed daemon admission window. A local mode-0700 copy of the verified
binary with a fresh local private cache passed all five adapter operations.
Before measurement, the freshness loop was also corrected so a stable known
wrong answer cannot terminate probing while another applicable answer remains
stale. Any stale result now resets the wrong-answer stability window.
The first scored attempt then exposed that a single native error still ended
probing before a multi-second watcher could recover. That complete run is
retained and excluded. Repeated identical errors now use the same frozen
three-probe, two-second stability bound as wrong answers; timeouts and resource
blocks remain immediately terminal.

The valid revision 8 rerun completed. Its warmed base result passed exact
definition/source and direct callees, while callers, impact, and tests each had
precision 1.000 and recall 0.500 from the retained dynamic-callback miss. The
body edit refreshed definition and callees. Rename then produced an empty exact
search and native `function not found` trace errors through the frozen
stability window; the five remaining mutation summaries ended `ERROR`. This
establishes no recovery inside that bound, not that a longer watcher wait could
never recover. The run recorded 88,414 query-response bytes and 170 calls,
including every failed probe, and remains excluded from the final aggregate.

The first code-review-graph 2.3.8 install failed before measurement because the
hash lock pinned PyJWT 2.13.0 without spelling the `crypto` extra required by
its MCP dependency. pip rejected the resulting unhashed candidate. The raw
failure and 184 MB peak process-tree RSS are retained. Revision 9 pins the same
2.13.0 wheel and hash with the required extra. A fresh binary-only install then
passed in 59.308 seconds without a compiler and peaked at 214,331,392 bytes.

Native preflight also retained a SQLite disk-I/O failure when graph data lived
on the ChromeOS removable mount. A fresh local private `CRG_DATA_DIR` passed.
Revision 10 freezes code-review-graph's persistent MCP adapter, complete MCP
initialization fields, exact-search plus native-source definition mapping,
direct relationship queries, file-scoped impact, native tests query, and
synchronous incremental update before its first score. The adapter preflight
completed the base state and body edit with a 109,031,424-byte peak.

Its first scored tiny campaign then completed under the committed revision 10
policy. The warmed definition passed. Callers were 1.000/0.500 precision and
recall, callees were 0.500/1.000 because native output retained builtin `sum`,
file-scoped impact was 1.000/0.750, and native `tests_for` returned no tests.
All six mutation summaries ended `WRONG`. The run recorded 869,360 query
response bytes in 140 calls and peaked at 193,798,144 bytes RSS. A failed
direct archive from the removable 9p mount is retained separately; the same
completed run was copied to local storage, byte-accounted, archived, extracted,
and replayed without rerunning the campaign. This tiny fixture remains excluded
from the final aggregate.

GitNexus 1.6.11 installation preflight retained two failures before any score.
The first exact-lock npm install ran on the removable 9p mount for 414.886
seconds before required `.bin` symlink creation failed with `EACCES`; its peak
RSS was 61,153,280 bytes. A fresh local retry then exposed a benchmark resource
sampler bug: splitting `/proc/<pid>/stat` on spaces shifted fixed fields for a
process name containing spaces and reported an impossible multi-terabyte RSS.
Revision 11 parses fields after the final parenthesized process-name delimiter
and adds a regression test. The false `RESOURCE_BLOCKED` record remains
published, and no threshold, corpus target, or completed competitor result was
changed.

The corrected third GitNexus install reached 1,076,068,352 bytes process-tree
RSS, 2,326,528 bytes above the frozen 1 GiB cap. The supervisor stopped it after
38.085 seconds. The benchmark keeps the limit and exact dependency lock intact,
records GitNexus as host-specific `RESOURCE_BLOCKED`, and assigns it no
correctness or comparative score.

The first complete modest-language matrix was audited before aggregation. That
audit found a Girder adapter error: conventional Rust paths such as
`crate::core::calculate_total` were not mapped to their unique `src/core.rs`
source identity. All eight affected Girder revision 11 campaigns are preserved
and excluded. Revision 12 froze the normalization correction and reran only
those campaigns; the twelve external revision 11 campaigns remain unchanged.

The [final cross-language report](docs/competitor-benchmark/results/modest-final/report.md)
now includes 20 complete campaigns and the GitNexus setup-only resource stop.
No product reached a fully correct result after any mutation. On the matched
Rust, Python, and TypeScript/TSX exact-definition attempts where Girder and
Ripwire both passed all 60 records, Girder returned 35,764 response bytes versus
158,514 with the same 120 calls. Girder lost the strict Go file-qualified
identity check even though its native response included the correct source.

See [the benchmark protocol](docs/competitor-benchmark/README.md) and the
[generated first-external result](docs/competitor-benchmark/results/tiny/report.md).

# Girder 0.2.6

Girder 0.2.6 ships the opt-in MCP watcher that was merged after the 0.2.5
release. Start it with:

```sh
girder mcp /path/to/project --watch
# or
npx -y girder-mcp /path/to/project --watch
```

The watch-enabled process owns a validated in-memory graph generation,
coalesces filesystem events, reparses changed files, runs full project-wide
resolution, persists the complete candidate atomically, and only then
publishes it to MCP calls. Calls wait while the graph is stale or rebuilding.
The existing invocation without `--watch` keeps its prior behavior.

The frozen watcher campaign matched fresh cold analysis for all 15 initial
graphs and all 45 consecutive mutations across Rust, Python, TypeScript, TSX,
and Go. It reused 4,930 of 4,995 file extractions (98.70%). Full parsing
occurred for 5 of 45 updates (11.11%), all because the dirty set exceeded half
of the owned source files. Small repositories therefore reuse less parsing:
the three-file fixtures fell back on 5 of 15 updates.

This release claims preservation of cold-analysis resolution while parsing is
reused. It does not claim broader semantic coverage or production-repository
latency. Existing intent-search, dynamic-dispatch, and Go function-value
limitations remain.

See the [watcher design, measurements, and limitations](https://github.com/dhishwasher/Girder/blob/v0.2.6/docs/mcp-watching.md)
and the [incremental equivalence record](https://github.com/dhishwasher/Girder/blob/v0.2.6/docs/incremental-updates.md).
