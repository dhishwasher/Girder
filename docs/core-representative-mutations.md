# Core Representative Mutations

This extends the trustworthiness oracle's per-test dynamic-proof technique —
a probe write that only fires when the mutated code actually executes — to
one cached representative repository ([`click-8.4.1`](core-representative-corpus.json)),
for a small hand-declared set of mutations and their real, unmodified test
callers.

It is not a claim of general test-impact accuracy across Click's behavior:
only the declared mutations and declared tests are measured. A defect this
finds is a recorded gap, not a failure of the harness. Rust representative
mutations are deliberately out of scope for this milestone — a compiled
mutation-per-checkout cycle across representative-sized Rust crates is not
honest to run repeatedly on this host's build latency; that remains future
work.

## Protocol

For each declared mutation, the harness:

1. extracts Click from the digest-verified benchmark cache
   (`.benchmark-cache/core-representative-v1/`), `git init`s and commits a
   baseline;
2. inserts a probe helper (writes the mutation's id to
   `$BITCODE_ORACLE_PROBE` when called, mirroring the trustworthiness
   fixtures' `mark_probe()`) and a call to it as the first statement of the
   declared target function — a source change large enough for Bit Code's
   semantic diff to mark the function modified, small enough to be
   behavior-preserving;
3. runs `bitcode test-impact <checkout>` once against a fresh mutated
   checkout for the static prediction;
4. runs each declared test alone, in its own fresh identically-mutated
   checkout (`PYTHONPATH=src`, no install), and checks the probe file for
   dynamic ground truth;
5. classifies each declared test as TP/FP/FN/TN by comparing Bit Code's
   static selection (the test's own graph path present in the printed
   "Impacted tests" section) against the dynamic probe result.

Every declared test's expected dynamic outcome is asserted at run time
(`measure_mutation` raises if a declared positive doesn't execute the probe,
or a declared negative does) — the ground truth is checked, not assumed.

## Declared mutation: `Group.invoke`

`click.core.Group.invoke` overrides `Command.invoke` and is reached only
when the dispatched command object is a `click.Group` (a multi-command CLI),
not a plain `@click.command()`. Three real, unmodified Click tests are
declared:

| Test | Uses | Expected |
|---|---|---|
| `tests/test_commands.py::test_other_command_forward` | `click.Group()` | positive |
| `tests/test_chain.py::test_basic_chaining` | `@click.group(chain=True)` | positive |
| `tests/test_commands.py::test_other_command_invoke` | `@click.command()` | negative |

## Result

Reproduce with:

```sh
python3 -m unittest -v tools.test_core_representative_mutations
python3 tools/core_representative_mutations.py \
  --bitcode /mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/bitcode \
  --offline --output docs/core-representative-mutations.json
```

Checked result: [`core-representative-mutations.json`](core-representative-mutations.json).

| Mutation | TP | FP | FN | TN | Precision | Recall |
|---|---:|---:|---:|---:|---:|---:|
| `group-invoke` | 0 | 0 | 2 | 1 | 1.000 | 0.000 |

## Verified defect: fixture-mediated dispatch is unresolved

Both dynamically-positive tests are false negatives. Neither exercises
`Group.invoke` through a direct, statically-typed call: `runner` is an
untyped pytest fixture parameter, and the real call chain is
`test → runner.invoke(cli, args) → CliRunner.invoke → Command.main →
self.invoke(ctx)`, where `self.invoke` is a **polymorphic dispatch** whose
concrete target (`Command.invoke` vs `Group.invoke`) depends on which
subclass `cli` was constructed as — not on syntax visible at any single call
site. Bit Code's Rust/Python receiver-type inference (direct/dotted/quoted
annotations, constructor assignments, bounded nullable unions; see
`docs/core-gap-analysis.md`) has no mechanism for this: `runner`'s type is
unknown, and `self.invoke` inside `Command.main` is an ordinary unqualified
method call with no receiver hint at all.

The one true negative (`test_other_command_invoke`, plain `@click.command()`,
no `Group` involved) is correctly excluded — recall is lost, not precision.

This is a real, newly-measured gap, not a regression: no fixture or gap
entry previously exercised polymorphic dispatch through an untyped test
fixture parameter on a real codebase. It is recorded as a new prioritized
gap in `docs/core-gap-analysis.md` rather than silently accepted.
