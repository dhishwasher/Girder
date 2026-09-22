# Call classification policy v1

This policy is frozen before implementation and measurement. Changes require a
new policy revision and new observations; never rewrite an old result.

## Meaning and assumptions

Classify **call sites**, not just pairs of function names. Each record identifies
the caller, source location, candidate targets, evidence, and any unresolved
boundary. A static call is not a guarantee that a branch executes or a test fails.
All claims concern the indexed source snapshot and its declared configuration.
External/generated code, unindexed source, parser errors, and stale evidence are
boundaries, not proof of absence. Tests discovered by the graph are not necessarily
the full test runner inventory; report that scope explicitly.

- **Must:** the callee binding is proven and has no viable alternate target under
  the recorded dispatch assumptions. One same-spelled candidate is insufficient.
- **May:** each listed candidate is supported as viable, but target uniqueness is
  unproven or alternatives exist. A target is not viable merely because its name
  resembles the call. May can have one known candidate plus an Unknown boundary.
- **Unknown:** the target, candidate completeness, or extraction coverage cannot
  be established. Preserve source-boundary records even if no target node exists.

When Must versus May cannot be distinguished, choose May only if target viability
is established; otherwise Unknown. When the known candidate set is incomplete,
keep its proven May candidates AND an Unknown boundary. Missing or malformed
evidence, legacy graph edges, inferred impact/dataflow, and resolver heuristics
cannot silently become Must. Never turn a numeric edge weight into a proof.

## Language rules

| Language | Must requires | May requires | Unknown includes |
| --- | --- | --- | --- |
| Rust | Proven lexical/item binding or exact concrete dispatch, no shadowing/alternate implementation/configuration ambiguity | Proven trait/inherent implementation candidates for dyn, trait objects, generic-bound dispatch, callable alternatives, or conservative implicit destruction | Unexpanded/generated macros, FFI, unresolved function pointers/closures, unresolved trait bounds, unknown receiver adjustment, parse errors |
| Python | Proven binding with explicit source-snapshot/no-runtime-rebinding assumptions and validated scope; annotations alone do not establish exact runtime class | Proven possible inherited/overridden methods, super/MRO targets, or bounded receiver candidates | getattr/reflection, monkey patching, decorators with unproven effects, unconstrained duck typing, dynamic imports, unresolved fixtures and inheritance |
| TypeScript/TSX | Proven lexical binding/concrete implementation without viable structural/overload alternatives | Proven compatible structural/interface implementors, union alternatives, overload candidates | any/unknown receivers, computed properties, eval/reflection, unproven narrowing, unresolved declarations/generated/runtime code |
| Go | Proven package/local function binding or concrete method and receiver with resolved method set | Proven possible interface implementors, promoted methods, or bounded function-value alternatives | reflect, cgo/FFI, unresolved interfaces/embeddings/generics, missing build-tag configuration, unresolved function values |

TSX is TypeScript, not a fifth language. In every language, inability to prove one
of these conditions means Unknown, not an exemption. Stage 1 may expose existing
extractor limitations rather than closing them. Per-language dispatch reasoning
is developed in Stage 3; no unsupported language feature gets certified early.

## Reachability and presentation

A Must impact path contains only Must calls; a May impact path contains at least
one May call and otherwise Must/May calls. An uncertain segment yields Unknown.
For a node with multiple paths, retain its strongest supported positive class
(Must before May before Unknown), while independently retaining all relevant
Unknown boundary records. This precedence describes evidence of a path, not a
promise that execution follows it. Origins that are tests are directly selected.

Unknown targets cannot be traversed as imaginary edges. When a boundary might
hide a path to an origin, tests that cannot be excluded remain Unknown. If the
possible target scope is unbounded, conservatively consider the whole graph;
do not restrict by language where FFI or other cross-language calls are possible.
Distinguish individual unresolved call sites from file/extraction coverage gaps
and missing evidence. Report all three kinds with reasons. A file-level gap is
not presented as an invented count of missing call sites.

`impacted_tests` and `orient` expose a versioned classification response with
separate `must`, `may`, and `unknown` collections and separate Unknown-boundary
records. Orient's matching confidence remains distinct from dispatch certainty.
Normal and watched MCP answers use the same graph query and formatting logic.

Sort node sets lexicographically by exact semantic path. Sort boundary records
by file, byte position, caller, and reason. Deduplicate exact identities, never
distinct call sites. Display at most 50 entries per collection, with the full
count, a truncation flag, and a continuation offset. Subsequent pages must use
the same graph snapshot or explicitly refuse a stale continuation. Traverse and
score full sets; display limits must not limit reasoning. An explicit unbounded
output mode may be used by offline evaluators.

Quiet CLI test selection returns the conservative union of known test paths in
all three classes; uncertainty is also diagnosed on stderr. Empty output with
Unknown boundaries never means no tests need running. MCP uses labeled output,
not quiet name-only output. Missing explicit nodes are errors, not empty sets.
Removed-node baseline selections without classification evidence are Unknown.
Stale watched graphs are refused/rebuilt before classification.

Preserve old graph readability and existing plan formats, but require freshly
derived evidence for certified answers. Parser/resolver-owned evidence is removed
and regenerated after edits. Old graph-native annotations cannot override it.

## Frozen measurement

Use the existing Rust/Python core-trustworthiness fixture mutations and the
declared Click `group-invoke` representative mutation, preserving their sources,
declared tests, dynamic probes, and expected outcomes. Extend measurement output
without rewriting existing observations. No additional language accuracy is
claimed from this corpus. Run the dynamic oracle; do not label old observations
as fresh executions. Keep baseline and post-change records separately.

For each mutation, over its independently declared test inventory:

- Must precision = dynamically positive tests in Must / tests in Must.
- May-only recall = dynamically positive tests in May / dynamically positive tests.
- Must-or-May recall = dynamically positive tests in Must or May / positives.
- Unknown test count = declared tests in Unknown. Also report all discovered
  Unknown tests, unresolved sites, coverage gaps, and missing-evidence counts
  separately so a large graph cannot hide the measured inventory's results.
- Publish TP/FP/FN and raw numerator/denominator for each metric, per mutation and
  pooled totals. A zero denominator is null/undefined, never synthetic 1.000.
- Publish every missing classification, overlap, unexplained omission, timeout,
  command failure, and unavailable run. A failed execution has no fabricated score.

**Trustworthy Must gate:** precision 1.000 with a nonempty Must set and zero false
Must claims. Stage 1 has no minimum May recall; it must publish what occurred,
including zero. A failed trustworthy gate is FAILED-AND-PUBLISHED, not tuned away.
The interface/Unknown-integrity criterion also fails if an encountered unresolved
site disappears, a cap hides uncertainty, or normal/watch results disagree.

Stage 3 adds a separately precommitted real-repository audit of at least 100
call sites per language with zero classification errors and a nonempty correct
Must set; fixtures alone cannot satisfy that criterion. An observation is bounded
evidence, not proof that every possible program is handled soundly.

Run the four common stage gates once on the candidate, serially with retained
logs and statuses. Policy, corpus, binary, and candidate hashes accompany each
observation. Publish any regression without deleting or softening the old result.
