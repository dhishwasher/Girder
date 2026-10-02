# Lexical-binding proof slice passes its frozen contract

Product candidate `5733be6`; policy/corpus `954334d`; unchanged runner `0551237`.
[Builder checks](builder-tests-1.json): 146 tests passed.
[CLI build](cli-build-1.json): exit zero, binary SHA-256
`fe6901793bb8055a66359ad235e631219ba87b3f6ab96a396e4741925a7deaef`.

The [fresh CLI observation](after-1/run.json) scored **22/22 exact contract
answers**, compared with [10/22 before](before-observation.md). All six required
Must calls name the exact declaration. All sixteen required Unknown calls have
explicit evidence and no guessed target. The four runtime-disproved Must cases
now return Unknown; they have not been removed or relabeled.

All **19 executable runtime probes passed** again on Node `v22.23.1`; the same
three TypeScript-only cases remain skipped. Six executed marked Must calls now
reach their certified declaration, **6/6**, with zero runtime-disproved Must
claims. This is a hostile proof-contract fixture result, not repository-wide
precision. The named-expression-shadow probe still does not execute its marked
conditional call and is not used as runtime target evidence.

Both runs retain raw analyze/inspect/runtime output, command statuses, source
hashes, and the identical manifest, runner, and probe hashes. Nothing was run in
parallel with Cargo; the rebuilt CLI workloads were serial and offline. Rust,
Python, and Go use their previous proof paths.

This closes the demonstrated binding hazards for the frozen cases and adds
restricted nested lexical proofs. It does **not** establish TypeScript language
completion. The complete 100-call repository audit, dispatch corpus, structural
origin repair, and four common language gates remain outstanding. Script scopes,
escaping references, overloads, arbitrary block declarations, and dynamic
binding forms outside the certified subset remain Unknown.
