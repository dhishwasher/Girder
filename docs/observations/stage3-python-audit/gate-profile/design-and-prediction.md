# Stage 3 Python resolver design: narrow `transformed_scope`'s Python effect

Written and committed before any code, per the same discipline every Rust
Stage 3 resolver round used (and per an `advisor` review's explicit
instruction this round). Precommits an exact prediction so any
divergence is a stop signal, not something to explain away after the
fact.

## What's being changed, precisely

`claims.rs`'s Python (and only Python — see "Language gating" below)
proven-map computation currently requires `!transformed_scope` as one of
four whole-file preconditions (`!root.has_error() && !duplicate_paths &&
!transformed_scope && ts_module`) before computing ANY same-file Must
binding. `transformed_scope` trips on any `decorator`/`decorated_definition`
node anywhere in the file — a single unrelated `@pytest.mark.parametrize`
on some other test function disqualifies every other Must-eligible call
in that file, which the gate-profile found blocks 12/13 real conservative
audit sites (though it is the *sole* blocker for only one).

**Change**: remove `transformed_scope` from gating Python's proven-map
computation. Rely instead on machinery that's already there and already
correctly scoped:
- The `top_level` check (`parent.id() == root.id()`) already excludes any
  function whose parent is `decorated_definition` — a decorated function
  was never eligible for `top` in the first place, independent of
  `transformed_scope`. **Checked directly against `claims.rs:140-145`,
  not assumed**: this means the design this document's own first draft
  proposed ("only exclude a candidate whose own node is decorated") is
  **redundant** — that exclusion already exists. There is nothing left
  to add on this specific point.
- The `clean` scan (every in-file `identifier`/`field_identifier`/
  `property_identifier`/`type_identifier` occurrence of the name must be
  either the declaration or a direct call) already rejects shadowing,
  reassignment, and escaping references to the name as an AST identifier.

## The soundness question this raises, investigated before writing code

If `transformed_scope`'s whole-file gate is removed, MORE calls become
eligible for the same-file Must path — which means any *existing* gap in
that path's soundness now has more surface area. One real gap: the
`clean` scan only inspects AST `identifier`-family nodes. A name rebound
via a **string literal** — `setattr(obj, "target", ...)`,
`monkeypatch.setattr(mod, "target", ...)`, `patch("pkg.mod.target")`,
`patch.object(mod, "target", ...)` — is invisible to it, since the string
`"target"` is a `string`/`string_content` node, never an `identifier`
node the `clean` scan looks at.

**This is not a new gap this change introduces — it already exists in
the same-file Must path Python has today** (every Python `CallEvidence`
already carries the explicit `no-runtime-rebinding-or-monkey-patching`
assumption string, disclosing exactly this). But widening what relies on
that assumption without checking it first would be careless.

**Checked empirically, not assumed absent — and this first check was
itself later found insufficient, corrected before trusting it (see the
four points above).** First pass: extracted every currently same-file-
Must-proven target name across all three real packages (68 claims, 41
distinct names) and grepped each package for
`setattr`/`monkeypatch`/`.patch(`/`patch.object` on the same LINE as each
name, word-boundary matched (an initial substring-only check produced
false positives on short names like `it` matching inside `setitem`/
`writer` — corrected before trusting even this first pass). Zero hits.
**This line-proximity check was too weak** — redone with Python's `ast`
module (point 3 above) across the WHOLE snapshot rather than just the 41
already-proven names, which is what found the real
`import_email_validator` cross-file case (point 4). The corrected,
`ast`-based, snapshot-wide check — re-run after implementing both guards,
against every name any new claim could touch — is what the after-
observation reports as the trustworthy result, not this first pass.

**Decision, revised after a second `advisor` review of this document's own
first draft: implement both a same-file guard AND a project-wide guard,
not just the same-file one.** The first draft proposed only a same-file
guard, matched narrowly against `setattr`/`patch`-looking call sites with
naive quote-stripped string arguments. That review found four real
problems with it, fixed here:

1. **Narrow trigger set.** Matching only calls whose text contains
   `setattr`/`patch` misses `delattr`, `exec`/`eval`/`compile` (the
   frozen rubric names `exec`/`eval` explicitly), and
   `globals()["name"] = ...`/`vars()[...]`/`__dict__[...]` subscript
   assignment.
2. **Naive quote-stripping.** Breaks on raw strings, triple-quoted
   strings, f-strings, and `concatenated_string`. Fixed by reading the
   grammar's own `string_content` child node instead (confirmed against
   `node-types.json` before relying on it).
3. **Line-proximity checking was too weak in the investigation itself**
   (not the guard, the *check* that found "zero hits"): it required
   `setattr`/`monkeypatch`/`patch` on the same source LINE as the name, so
   a black-formatted multi-line call would be missed entirely. Redone with
   Python's own `ast` module, walking every `Call` and `Subscript` node in
   each file (not line-proximity), across the whole package.
4. **Per-file scope is not enough.** The frozen rubric disqualifies Must
   on *snapshot-wide* string rebinding, not just same-file. Re-running the
   corrected, `ast`-based check across the full snapshot (not just the 41
   previously-Must names, but every name any new claim could touch) found
   a **real, live case** the same-file guard alone cannot see: pydantic's
   `tests/test_networks.py:986` has `mocker.patch('pydantic.networks.
   import_email_validator', side_effect=ImportError)` — a STRING in a
   TEST file, naming a function DEFINED in `pydantic/networks.py`, which
   itself has three internal call sites to `import_email_validator()`
   (lines 1006, 1081, 1302, confirmed by reading the file directly) that
   would be same-file-eligible for Must under this design. A per-file
   guard cannot see a rebinding string in a different file; this is not
   hypothetical, it is the exact real shape this design is being built to
   avoid.

**Final design, both guards**:
- **Same-file guard** (`claims.rs`, unchanged in spirit from the first
  draft, fixed per points 1–2): for each candidate top-level name, collect
  every `string` node's `string_content` text anywhere in the file (not
  scoped to setattr/patch call arguments specifically — simpler and
  strictly more conservative, per point 1's broader trigger set); exclude
  the name from `proven` if any such string equals the name or ends
  `.{name}`. Additionally, empty `proven` entirely for the file if it
  contains any `wildcard_import` (`from m import *` can rebind a name with
  no `identifier` node `clean` would ever see) or any `exec`/`eval`/
  `compile` call.
- **Project-wide guard** (new module `sync/python_rebinding.rs`, run once
  after all files are extracted and Python same-file Must claims exist,
  mirroring `rust_methods.rs`'s post-processing architecture): collects
  every file's string literals into one crate-wide set, then for every
  `proven-top-level-lexical-binding` Must claim anywhere in the graph,
  resolves its target's name and reverts the claim to Unknown (reason
  `python-target-string-rebound-elsewhere-in-crate`) if that name is
  rebound by a string literal ANYWHERE in the indexed project — closing
  the same-file guard's blind spot, not just disclosing it.

Both guards are deliberately coarse (a string matching the name anywhere
in scope, not a precise binding/call-context resolution) — erring toward
excluding a name that's actually safe, never toward including one that
isn't, matching this whole module's stated design philosophy.

## Language gating

`AnnotateGates.transformed_scope` itself is **not changed** — it's still
computed exactly as before and still returned, because
`sync/rust_methods.rs` reads it for Rust's own guard logic. Only the
Python-specific `if !root.has_error() && !duplicate_paths &&
!transformed_scope && ts_module` condition that gates Python's `proven`
computation changes, and only when `lang == Lang::Python` — TypeScript
(which also has `decorator` nodes and currently shares this same
whole-file gate through the same code path) is explicitly **not**
touched by this round, to avoid moving Stage 3 TypeScript proofs before
Stage 3 TypeScript work has even started, which would violate the
roadmap's own language-order rule. Verified by re-running the pooled
dispatch corpus after implementing (Section "Verification", below) and
confirming TypeScript's and Go's cells don't change.

## Reverse-direction guard check: would this create any new false Must?

Checked every non-`not_a_call_site` `plain_call` site with `true_class:
unknown` in the 105-site sample (the only shape this change could ever
affect) and every dispatch-corpus case that isn't already Must-eligible,
by reading the fixture/site source directly:

- **Real audit**: 2 `plain_call` sites are labeled `unknown` —
  `click-8.4.1 tests/test_arguments.py:473` (`frozenset(...)`) and
  `pydantic-2.13.4 tests/test_networks_ipaddress.py:83`
  (`IPv4Address(...)`) — both call builtins/stdlib imports with no
  same-file `def`/`class` of that name, so `same_file_top_level_def_exists`
  is false for both regardless of `transformed_scope`; neither could ever
  enter `proven`. No risk.
- **Corpus** (`fixtures/dispatch-corpus/python/`, all 12 cases read
  directly): `name-shadowing-parameter` (a parameter named `target`
  shadows the global `target` inside `run` — already correctly excluded
  by the existing `clean` scan, which rejects the parameter occurrence as
  neither the declaration nor a direct call; no decorator in this file
  either, so untouched either way); `decorator-on-call-site` (`target`
  itself is decorated by `@log` — excluded from `top` by the existing
  `top_level`/parent check, unchanged by this design); `dunder-call`
  (`add` is a local variable, never a module-level `def`/`class`, never
  in `top`); `getattr-dynamic`, `unconstrained-duck-typing` (method-call
  shape, blocked by the identifier filter regardless);
  `negative-unreachable`, `direct-same-file` (no decorator present,
  unaffected by this change either way). **No case produces a new false
  Must.**

## Precommitted prediction

**Exactly two cells move, both conservative → exact, nothing else
changes:**
1. Real audit: `pydantic-2.13.4 tests/test_deprecated.py:271`
   (`deprecated_from_orm(...)`), target
   `tests/test_deprecated.py:40`'s `deprecated_from_orm` definition.
2. Dispatch corpus: `python-decorator-elsewhere-in-file`'s `test_direct`
   case.

If the implementation produces a different count, a different target, or
moves anything else (including any TypeScript/Go cell), that is a stop
signal to investigate before trusting the result — the same discipline
every Rust resolver round in this program used. If either predicted move
doesn't happen, or a different site moves instead, that is itself useful
information and gets recorded, not silently adjusted to match.

## Supplementary check: run after implementing, results in the after-observation

This change makes more same-file Python Must claims possible across real
click/pydantic/requests code (any undecorated top-level helper called
from a file that has a decorator anywhere else in it — a meaningful
fraction of test files, which commonly mix `@pytest.mark.*`-decorated and
undecorated test functions). Full counts, the `ast`-based snapshot-wide
rebinding re-scan (confirming zero remaining hits after both guards), and
a hand-verified random sample against `labeling-rubric.md` are in
[../after-transformed-scope-fix/after-observation.md](../after-transformed-scope-fix/after-observation.md),
mirroring Rust's own supplementary-hand-verification discipline.

## Order

1. Implement (this document, committed first).
2. Unit tests, each mutation-verified (revert the specific guard,
   confirm the test that depends on it fails; restore, confirm it
   passes) — planned: the `top_level`-already-excludes-decorated-functions
   claim itself (a passing test today, before any change, confirms this
   isn't a new dependency); the string-rebinding guard (a fixture with a
   same-file `monkeypatch.setattr(mod, "name", ...)` must NOT be proven
   Must); the language gate (a TypeScript fixture with the same
   decorator-elsewhere shape must NOT newly prove Must).
3. Rebuild, re-measure: real audit, dispatch corpus (Python cells AND
   confirm TypeScript/Go cells unchanged), Stage 1 oracle, all four
   common gates.
4. Supplementary hand-verification of new real-repository claims.
5. After-observation, committed separately.
6. Roadmap, last.
