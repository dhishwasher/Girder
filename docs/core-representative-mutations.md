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
   `$GIRDER_ORACLE_PROBE` when called, mirroring the trustworthiness
   fixtures' `mark_probe()`) and a call to it as the first statement of the
   declared target function — a source change large enough for Girder's
   semantic diff to mark the function modified, small enough to be
   behavior-preserving;
3. runs `girder test-impact <checkout>` once against a fresh mutated
   checkout for the static prediction;
4. runs each declared test alone, in its own fresh identically-mutated
   checkout (`PYTHONPATH=src`, no install), and checks the probe file for
   dynamic ground truth;
5. classifies each declared test as TP/FP/FN/TN by comparing Girder's
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
  --girder /mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder \
  --offline --output docs/core-representative-mutations.json
```

Checked result: [`core-representative-mutations.json`](core-representative-mutations.json).

| Mutation | TP | FP | FN | TN | Precision | Recall |
|---|---:|---:|---:|---:|---:|---:|
| `group-invoke` | 2 | 1 | 0 | 0 | 0.667 | 1.000 |

**Updated 2026-10 after a `test-impact` fix** (see
`docs/observations/stage3-typescript-audit/before-observation-addendum-6.md`
and `-9.md`): the table above previously read `0 | 0 | 2 | 1 | 1.000 |
0.000`, with both dynamically-positive tests measured as false negatives.
That was not a regression Girder's own resolver introduced and then lost.
The origin here (`Group.invoke` itself, the mutated function) is a real,
ordinary Function origin -- origin resolution was never the problem for
this specific mutation. The gap was in `tests_for_nodes` itself
(resolved-Calls-reachability only): it found no proven edge for this
unresolved polymorphic-dispatch target and returned nothing, with no
fallback and no notice, for ANY unresolved-dispatch origin, not just this
one. That gap is now fixed: `test_impact.rs`'s bare (no-flag) invocation --
exactly the form this harness's own "static prediction" step uses -- falls
back to the classified must∪may∪unknown union whenever `tests_for_nodes`'s
narrow result is empty. Recall is now 1.000, at the cost of one
over-selected false positive (`test_other_command_invoke`) dragging
precision to 0.667 — the known, accepted direction this program's whole
"must∪may∪unknown conservative union" design always intended: missing a
real impact (false negative) is the dangerous direction; over-selecting
(false positive) is the safe one.

**Scale, disclosed rather than left implicit**: precision/recall above are
computed over only the 3 tests this mutation DECLARES (this harness's own
stated scope -- see "Protocol" above). The actual fallback this triggers is
graph-wide, not scoped to these 3: the new result's own `classified` field
reports `unknown_total_count: 472` for this Click checkout -- every node
the global "any Unknown boundary escalates every function" rule now
reaches, of which the 3 declared tests are a small sample. A real,
non-synthetic edit to `Group.invoke` (or any function reached only through
an equally-unresolved dispatch) would make `girder test-impact . --quiet`
select a potentially large fraction of Click's real test suite, not a
small number -- the exact, intended "over-select broadly rather than risk
missing one" trade-off, but a real cost worth knowing before relying on
`--quiet`'s selection size as a proxy for how targeted an answer it is.
The actual is-test-tagged subset of that 472 was not separately measured
in this session; only the total unknown-classified node count is
confirmed.

## Verified defect: fixture-mediated dispatch is still unresolved (recall fixed, resolution gap unchanged)

**This section's underlying technical claim is unchanged** -- only the
measured SELECTION OUTCOME above changed, not whether Girder can actually
resolve this dispatch. It still cannot, and this remains the real,
open gap recorded in `docs/core-gap-analysis.md`.

Both dynamically-positive tests reach `Group.invoke` through fixture-
mediated **polymorphic dispatch**: neither exercises it through a direct,
statically-typed call. `runner` is an untyped pytest fixture parameter, and
the real call chain is `test → runner.invoke(cli, args) → CliRunner.invoke →
Command.main → self.invoke(ctx)`, where `self.invoke`'s concrete target
(`Command.invoke` vs `Group.invoke`) depends on which subclass `cli` was
constructed as -- not on syntax visible at any single call site. Girder's
Rust/Python receiver-type inference (direct/dotted/quoted annotations,
constructor assignments, bounded nullable unions; see
`docs/core-gap-analysis.md`) has no mechanism for this: `runner`'s type is
unknown, and `self.invoke` inside `Command.main` is an ordinary unqualified
method call with no receiver hint at all. This is confirmed directly by the
new result's own `classified` breakdown: all three declared tests are
classified `unknown` (`unknown_count: 3`), not `must` or `may` -- Girder is
NOT claiming to have resolved this dispatch; it is correctly reporting it
as a boundary and conservatively including every test that boundary could
reach, which is why recall improved without the underlying resolution gap
closing.

The one true negative from the prior measurement
(`test_other_command_invoke`, plain `@click.command()`, no `Group`
involved) is now a false positive instead: the conservative union includes
it too, since it is also only reachable through the same unresolved
`self.invoke` dispatch boundary and the current classification has no way
to distinguish "reachable through this unresolved call, but not actually
this subclass" from "reachable and possibly this subclass." Recall is no
longer lost; the cost moved to precision instead, which is the designed
trade-off, not a new defect.

This is a real, newly-measured gap, not a regression: no fixture or gap
entry previously exercised polymorphic dispatch through an untyped test
fixture parameter on a real codebase. It is recorded as a prioritized gap
in `docs/core-gap-analysis.md`, unchanged by this update -- actually
resolving `self.invoke`'s receiver type remains future work; this update
only changes what Girder does when that resolution fails.
