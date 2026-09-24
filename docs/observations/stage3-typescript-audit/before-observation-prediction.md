# Stage 3 TypeScript before-observation: precommitted prediction

Written and committed BEFORE running the before-observation measurement.
This is the stage's first genuinely blind prediction (see
`labeling-rubric-addendum-2.md`'s discussion of `_testConvertToAsyncFunctionFailed`
and the wider session-level finding that some earlier "precommitted"
predictions in this program's history were written after the measurement
they predicted already existed on disk -- this one is checked against
that failure mode directly: written and committed here, then the
scorer is run as the immediately following step, with no measurement
output existing anywhere in the repo or scratchpad before this commit).

## Derived from Girder's actual current capability, not guessed

`crates/aether-builder/src/mapper/claims.rs`'s Python-only fix
(`8acde01`) removed `transformed_scope` from gating Python's proven-map
computation specifically -- confirmed directly in that file: `let
effective_scope_gate = if lang == Lang::Python { python_whole_file_blocked
} else { transformed_scope };`. **TypeScript still uses the unmodified,
original `transformed_scope` gate**: any decorator (`@Something`)
anywhere in a `.ts` file blocks the WHOLE FILE's same-file Must proof
computation, the same way it did for Python before that language's own
resolver round. No cross-file import resolution exists for ANY language's
resolver yet (Rust, Python, and TypeScript's own design all disclose this
as an open limitation) -- a Must claim can currently only be proven for a
same-file, top-level, undecorated, unrebound, unshadowed binding.

## Checked per-candidate, not assumed at the whole-corpus level

Of the 46 Must-labeled sites (after `511fe77`'s correction), checked
directly which are (a) SAME-FILE (the call and its `true_target` are in
the identical file, not resolved through an import) and (b) in a file
with **zero** decorator lines anywhere (`grep -cE '^\s*@[A-Za-z_]'`,
checked per file, not assumed from the corpus-level `shape_counts_pool`):

- **8 same-file, decorator-free**: sites 29, 32, 34 (`checker.ts`, 0
  decorators), 37, 38, 39 (`emitter.ts`, 0 decorators), 83
  (`compileOnSave.ts`, 0 decorators), 91 (`projectReferences.ts`, 0
  decorators).
- **1 same-file, but decorator-blocked**: site 3 (`nested-validation.spec.ts`,
  23 decorator lines -- `transformed_scope` trips for the whole file).
- **37 cross-file**: every decorator-factory Must (0, 1, 2, 4-14, 16 --
  even the two same-file-local ones, 1 and 2, are in `custom-decorators.spec.ts`,
  itself decorator-containing, so blocked the same way site 3 is) plus
  the remaining `typescript-6.0.3`/`zod-3.23.8` sites whose `true_target`
  is in a different file than the call site (40, 44, 49, 54, 55, 65, 66,
  67, 69, 75, 76, 80, 81, 85, 86, 87, 90, 92, 94, 96, 97) -- none of these
  can be proven by a resolver with no cross-file import resolution,
  regardless of decorators.

## Precommitted prediction

**8 of 46 Must sites score `must` (exact); the remaining 38 score
`unknown` (conservative, per `cell_label`'s existing rule -- a
Must-labeled site Girder reports as unknown is `conservative`, not
`overclaim`). 0 unsound cells (no `overclaim`, no `unsafe_exclusion`)
predicted anywhere in the 46 Must sites.** The 51 Unknown-labeled and 8
`not_a_call_site`-labeled sites are not part of this specific prediction
(their expected scoring already follows directly from their own true
class and don't depend on the same-file/cross-file distinction the same
way).

If the actual scored count of exact Must cells differs from 8, or if any
unsound cell appears among the 46 Musts, that is a stop signal to
investigate before trusting the result -- the same discipline every prior
language's Stage 3 resolver round in this program used.
