# Technical due-diligence / evidence index

A pointer index for anyone evaluating Girder technically before buying
or investing — every item here is a committed, reproducible artifact
in this repository, not a marketing claim. Nothing in this document is
new measurement; it links to what already exists and states plainly
what each one does and doesn't prove.

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
  number. Rust and Python are marked DONE (committed passing evidence
  against their own precommitted criteria); TypeScript is explicitly
  IN PROGRESS, stated as such in the roadmap, not glossed over.
- **A real extractor bug, found and fixed in this program, not hidden**:
  `docs/observations/stage3-typescript-audit/before-observation-
  addendum-4.md` through `-13.md` document a node-identity collision
  bug found via this program's own review discipline, its full
  investigation (including a wrong first fix caught by its own
  regression test and corrected before landing), and the eventual fix
  — commits `ba57090` through `9ad6e4f`. This is offered as evidence
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
  for exactly the items a skeptical buyer would ask about.

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

## Licensing and commercial terms

`LICENSE` (Business Source License 1.1, converting to Apache License
2.0 on 2030-09-04) and `docs/pricing-audit.md` (this session's factual
audit of exactly what the license gate does and doesn't cover, verified
by reading the gating code directly).

## What this index does NOT claim

No formal verification, no mathematical soundness proof, and no claim
that Must/May/Unknown classification covers every possible dispatch
shape in every supported language — the open items above are real,
not residual. This index exists to make the EXISTING evidence easy to
find, not to assert more than that evidence supports.
