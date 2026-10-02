# Lexical-binding baseline: false Must claims confirmed

Policy/corpus: `954334d`. Runner: `0551237`. Product binary built from
`67d65f6`, SHA-256
`13976d976157318878512ae6eb5458f0cf5ca4f19b5c642a9bf16bbde54381f7`.
The [complete run](before-1/run.json) preserves commands, input hashes, raw CLI
outputs, runtime outputs, and every case. All 22 cases were scored. No fixture
or expectation was changed after measurement.

**The precommitted contract criterion failed: 10/22 exact answers.**

- Five required nested Must cases remain Unknown: nested declaration, closure,
  hoisted call, recursive call, and method-local declaration.
- Four incorrect Must targets were contradicted by executed probes:
  `destructuring`, `escaped-assignment`, `direct-eval`, and `escaped-eval`.
  Each certified the original declaration, whose body returns `original`;
  the marked call actually ran the replacement and returned `alternate`.
- Three further Must answers violate the stricter proof contract:
  `shorthand-escape`, `namespace-export`, and `escaped-unrelated-identifier`.
  These are **not three more demonstrated wrong targets**. The two executable
  probes return `original`; namespace syntax was not executed.

All 19 runnable probes matched their frozen outcomes on Node `v22.23.1`.
Three TypeScript-only files were explicitly skipped at runtime. Among the seven
runtime-executed marked calls that Girder classified Must, three called the
certified original and four contradicted it: **3/7**, approximately **0.429**.
One additional Must answer had no runtime probe. This intentionally hostile
proof corpus is not an estimate of repository-wide precision; the four concrete
counterexamples are sufficient to reject the current proof's soundness.

The named-expression-shadow probe returned its frozen `named` result, but did
not execute its marked conditional call. It is not used as runtime target
evidence. No absent claim was accepted as an Unknown answer.

The frozen 100-site real-repository baseline still has an empty Must set; it
does not expose or excuse these independent counterexamples. TypeScript remains
IN PROGRESS and Go remains NOT STARTED. Next: implement the precommitted scope,
identifier-use, and dynamic-binding guards, preserving this failed observation.
