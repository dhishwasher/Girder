# Expanded real-repository audit: failed Rust, passing Python sample

Measured at `3f961a5` with the binary SHA frozen in `../freeze-manifest.json`.
Labels were committed at that revision before any current candidate answer was
observed. Six pinned archives were acquired from the offline cache and freshly
analyzed/inspected serially. No resolver code changed in this campaign.

| Sample | Actual sites | Non-call entries | Exact | Conservative | Unsafe exclusions | Must precision |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Rust original | 52 | 53 | 28 | 24 | 0 | 1/1 |
| Rust expanded | 100 | 83 | 59 | 39 | **2** | 1/1 |
| Python original | 85 | 20 | 73 | 12 | 0 | 1/1 |
| Python expanded | 100 | 28 | 87 | 13 | 0 | 1/1 |

No overclaims or relocation failures occurred. Rust fails the frozen zero-unsound
criterion. Python passes this sample criterion; language progression still waits
for Rust and the common gates. One emitted Must per language is weak precision
evidence and does not establish population-wide correctness. These are static
per-site classifications, not measured test-impact recall or mutation results.

Rust's two omitted calls:

- `petgraph/src/algo/ford_fulkerson.rs:193`: addition on generic `N::EdgeWeight`.
- `petgraph/tests/graph.rs:124`: equality on derive-generated `NodeIndex`.

Both are ground-truth Unknown. A whole-file implicit-dispatch warning does not
count as a claim for either site under the unchanged scorer. Their absence must
be repaired with site-specific evidence, not a relaxed scorer.

The original cohorts' scores are unchanged. The one changed Rust answer is site
89 (`serde_json/src/ser.rs:504`): Unknown remains Unknown, but the caller is now
`Compound<'a, W, F>::serialize_element@SerializeSeq` and the reason is
`rust-binding-or-dispatch-unproven`, replacing the module's collision warning.
The sibling `@SerializeTuple` method has its own ID. See
`rust-serialize-element-identities.json` for all four distinct concrete methods
and `rust-changed-original-answers.json` for the complete before/after record.
This supplies the previously outstanding post-collision Rust re-score.

Reproduce the measurement from the frozen inputs using
`python3 -m tools.run_dispatch_audit_reconciliation` in a fresh checkout/scratch
area after the offline prepare step documented by the helper. Existing output
directories deliberately refuse overwrite. `run.json` records commands, exits,
duration, input identity, and acceptance; compressed analyze/inspect stdout plus
stderr are retained for every invocation. Common gates are deferred to the
subsequent operator correction candidate, not represented as run here.
