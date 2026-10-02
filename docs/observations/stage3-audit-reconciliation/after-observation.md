# Corrected 100-call-site Rust and Python audits

Candidate: `c441899`, binary SHA
`da59183f4543a51a48e584e43ed06fcb76f5d6fc21ce8ba4b4763d000442f552`.
The implementation and binary are unchanged across the requested pause.

| Language | Actual sites | Non-call entries retained | Exact | Conservative | Unsound | Must precision |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Rust | 100 | 83 | 61 | 39 | 0 | 1/1 |
| Python | 100 | 28 | 87 | 13 | 0 | 1/1 |

Every selected entry is accounted for; there are no relocation failures. Both
samples pass the unchanged size, zero-unsound, and nonempty-Must criteria.
Each has only **one emitted Must claim**: precision 1.000 on this sample is
limited evidence, not a guarantee for other calls or repositories. May recall
and remaining Unknown volume must not be inferred from the precision number;
full observed/ground-truth counts are in
[classification-counts.json](operator-resumption/classification-counts.json).

The original undersized audits remain historical measurements of 52 and 85
actual sites. This prospective extension cannot retroactively establish that
their earlier completion claims met the sample-size requirement. All original
sites, labels, exclusions, and failed observations are retained. Labels for the
extension were frozen in `3f961a5` before its first measurement; the subsequent
operator correction policy was committed at `96f7a11` before implementation.

## What changed

The [first expanded Rust audit](observation/summary.md) failed on two omitted
operator sites. Generic addition at `petgraph/src/algo/ford_fulkerson.rs:193`
and derive-generated equality at `petgraph/tests/graph.rs:124` now have local
Unknown claims. The implementation also covers compound assignment, unary
operations and indexing, retaining nested explicit calls. Short-circuit boolean
control flow receives no operator-trait claim. No operator is promoted to Must.

Rust improved from 59 exact / 39 conservative / 2 unsafe exclusions to
61 / 39 / 0. Two other Unknown answers changed only their reason/covering span.
All four changes are listed in
[rust-changed-answers.json](operator-correction/rust-changed-answers.json).
The original 52-site Rust cohort retains 28 exact / 24 conservative. Python's
100-site and original 85-site answers are unchanged.

Rust's previously outstanding post-collision check also completed: the formerly
merged `serialize_element` implementations have distinct trait-qualified paths
and IDs. The original affected site remains Unknown with the correct caller;
see [identities](observation/rust-serialize-element-identities.json) and the
[complete changed-answer record](observation/rust-changed-original-answers.json).

## Corpus and gates

The unchanged 49-case dispatch corpus was re-scored against this binary. The
entire confusion matrix and every observed test class match the last published
Python correction: 22 exact, 34 conservative, zero unsound, **one failed case**.
The existing `typescript-structural-object-literal` failure remains: origin
`name` cannot be resolved. It was not removed or counted as passing. Rust's
5 exact / 9 conservative and Python's 7 / 6 preserve their previously measured
dispatch improvements. Full results and input hashes are in
[corpus-after.json](operator-resumption/corpus-after.json) and
[corpus-run-inputs.json](operator-resumption/corpus-run-inputs.json).

All four common gates ran once, serially, on the correction candidate before
the pause: Cargo tests **753 passed, 2 ignored** across 24 suites; clippy and
fmt passed; npm tests **29 passed, 2 skipped**. All exit statuses are zero.
[Gate commands, hashes and logs](gates-operator-correction/summary.json) remain
the evidence for this unchanged product revision; resuming measurement did not
rerun those gates. This work made no dependency or license-text changes.

## Interrupted run and reproduction

The user-requested pause interrupted Pydantic analysis after Rust and Click
had completed. [The partial run and traceback](operator-correction/interruption.md)
are retained. Resume verified the candidate hash, frozen labels/scorers, source
archive contents, and Click output hash; it reused completed evidence and ran
only the unfinished Pydantic and Requests analyses serially, offline.

Use `python3 -m tools.run_operator_audit_correction` from the frozen prepared
snapshots for a fresh full run. For the preserved partial run, use
`python3 -m tools.resume_operator_audit`. Both deliberately refuse to overwrite
existing output. The [Rust run](operator-correction/run.json),
[Python resumed run](operator-resumption/run.json), and compressed raw extracts
record the measurement. A fresh reproduction needs new output/scratch locations;
do not remove a published observation merely to reuse its directory name.

Remaining limitations include unresolved generic/trait dispatch, macro expansion,
implicit destruction, dynamic Python binding and reflection. These audits are
static call-classification measurements, not new mutation-test or test-impact
recall measurements. TypeScript and Go are not certified by these results.
