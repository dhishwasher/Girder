# Technical evidence index

A pointer index for technical readers evaluating Girder — every item
here is a committed, reproducible artifact in this repository, not a
marketing claim. Nothing in this document is new measurement; it links
to what already exists and states plainly what each one does and
doesn't prove.

## Soundness program (the core claim: what Girder will and won't assert)

`docs/call-classification-policy.md` is the frozen policy: every call
site Girder's graph can see is classified Must (proven), May (bounded
ambiguity), or Unknown (disclosed, not guessed) — never silently
dropped. `docs/roadmap.md` is the full, append-only resume log of the
multi-stage program that built and is still extending this:

- **Stage 1** (`docs/observations/stage1/`): the baseline measurement
  on real code, including the honestly-published fact that it was
  frequently near the full test suite at first, not a flattering
  number chased after the fact.
- **Stage 3, per language** (`docs/observations/stage3-rust-audit/`,
  `stage3-python-audit/`, `stage3-typescript-audit/`): hand-labeled
  ground-truth audits (105 sites each, frozen rubrics) scored against
  real pinned open-source repositories (petgraph, regex, serde_json;
  click, pydantic, requests; the TypeScript compiler itself, zod,
  date-fns, class-validator), with every correction, every found bug,
  and every measurement re-run committed in order — not just the final
  number. Rust, Python, TypeScript and Go are all marked DONE, each
  against its own precommitted criterion with committed passing evidence
  (TypeScript's closed with the ESM named-import proof, 73/73 contract and
  a 100-site audit with zero unsound cells; Go's under
  `stage3-go-audit/`, audited against a pinned Go 1.27.1 standard
  library). Failed attempts are kept alongside the passing ones.
- **Stage 4** (`docs/observations/stage4-clients/`): `girder setup` for
  Claude Code, Codex and Cursor, with verified client-documentation
  formats, isolated install and MCP smoke checks, and the limits
  (no live model session loaded the instruction; Cursor's pre-read hook
  is not built).
- **Stage 5** (`docs/observations/stage5-verified-edits/`, policy in
  `docs/verified-edits-policy.md`): certified `replace_node` edits with a
  path-bound fingerprint and a declared delta. 19 frozen plans, nine
  mutation checks that each break a guard test, and a failed first gate
  run (`gates-beae7c2/`) published next to the passing one.
- **Stage 6** (`docs/competitor-benchmark/results/stage6-run2/`): a fresh
  serial campaign of Girder 0.4.0 against ripwire, codebase-memory-mcp and
  code-review-graph. Its first run is kept as
  `stage6-run1-INVALID/` because the harness mislabeled the Girder
  binary; Girder's test-selection precision dropping from 1.0 to 0.667
  (recall 0.5 to 1.0) is published as a regression, not hidden.
- **A real extractor bug, found and fixed in this program, not hidden**:
  `docs/observations/stage3-typescript-audit/before-observation-
  addendum-4.md` through `-14.md` document a node-identity collision
  bug found via this program's own review discipline, its full
  investigation (including a wrong first fix caught by its own
  regression test and corrected before landing), and the eventual fixes
  — commits `ba57090` through `9ad6e4f` for Rust, and `-14.md` for
  TypeScript function and method collisions (which changed no measured
  number, because the pinned corpora contain none; its tests carry the
  proof, and 6 of its 8 fail on the old code). This is offered as evidence
  of the program's own rigor: bugs get found, disclosed, and fixed in
  public commit history, not quietly smoothed over.

## Trustworthiness and mutation testing

- `docs/core-trustworthiness-measurement.md` +
  `docs/core-trustworthiness-baseline.json`: a dynamic-proof oracle
  (a probe write that only fires when mutated code actually executes)
  measuring real precision/recall on representative Rust and Python
  fixtures — currently 1.000/1.000, with the historical defect that
  was closed (a precision regression at 0.667) left in the document
  rather than deleted from the record.
- `docs/core-representative-mutations.md` +
  `docs/core-representative-mutations.json`: the SAME dynamic-proof
  technique against one real, cached open-source repository (Click).
  Documents a genuine, still-open limitation (polymorphic dispatch
  through an untyped test fixture parameter is not resolved) rather
  than hiding it — and documents, with the same honesty, when a later
  fix changed the SELECTION outcome without claiming to have solved
  the underlying resolution gap (`docs/core-gap-analysis.md` item 11).
- `docs/core-gap-analysis.md`: the standing, prioritized list of known
  gaps and adversarial findings — read this before anyone else does,
  for exactly the items a skeptical technical reader would ask about.

## Cost/efficiency measurements (bytes, not tokens, stated as such)

`docs/context-vs-read-cost.md`, `docs/names-cost.md`,
`docs/orient-tool.md` (and the root `README.md`'s own citation of it):
every efficiency number in this repository is measured in bytes with
no tokenizer run, and the documents say so explicitly rather than
implying a token-cost claim. `docs/context-with-tests-cost.md` extends
the same measurement to test-selection cost specifically.

## CI and build verification

`.github/workflows/ci.yml` (fmt + clippy `-D warnings` + full test
suite on every push/PR), `.github/workflows/verify-builds.yml`
(cross-platform build verification), `.github/workflows/release.yml`
(release packaging). All run on GitHub's own infrastructure, publicly
visible on this repo's Actions tab — not a private, unverifiable claim.

## Adversarial and gap-analysis documents

`docs/core-gap-analysis.md`'s "Prioritized open gaps" section and
every numbered item above it in the same document is the adversarial
record: specific, named scenarios (macro token-tree precision,
match-arm receiver resolution, let-else/let-chain resolution, Python
annotated/constructor/import-alias receiver resolution, cargo binary
subprocess entrypoints, concurrent analysis correctness, transaction
recoverability) that were tested, found wanting at some point, and
either fixed (with the fix's own evidence linked) or left open and
named as open.

## Licensing

`LICENSE` (Apache License 2.0; earlier releases remain under the terms
they shipped with) and `docs/pricing-audit.md`, a **historical** audit
(2026-10-01) of what the former license gate covered. It is not a
current paid offering: there is no current paid tier.

## What this index does NOT claim

No formal verification, no mathematical soundness proof, and no claim
that Must/May/Unknown classification covers every possible dispatch
shape in every supported language — the open items above are real,
not residual. This index exists to make the EXISTING evidence easy to
find, not to assert more than that evidence supports.
