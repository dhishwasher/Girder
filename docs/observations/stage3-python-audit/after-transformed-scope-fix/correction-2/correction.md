# Correction-2: a hex-padding bug in correction-1's own checking scripts, plus remaining honesty fixes

A further `advisor` review of `correction-1` (`5cbe8af`) found that every
one of correction-1's Python scripts that resolves a `CallClaim`'s target
`NodeId` from `girder inspect`'s RON-encoded `call_evidence_v1` attribute
shared a silent-drop bug, undermining the two soundness checks correction-1
reported as clean. This document re-runs those checks correctly, confirms
the two blocking soundness checks are STILL clean on accurate data, and
corrects several remaining accuracy problems `correction-1` itself was
written to fix but partially repeated. Per this session's established
practice, `correction-1`/`5cbe8af` is not edited in place.

## 1. The bug: `format(int(tid), "x")` strips leading zeros

`girder inspect`'s node `id` field is a 16-character, zero-padded lowercase
hex string (`"243da3a4f6c47be7"`, confirmed directly:
`format(int(tid), "x")` on a decimal `NodeId` under 2^60 produces FEWER
than 16 hex digits, which never matches any key in the `by_id` dict built
from the JSON's own zero-padded strings -- a silent lookup miss, not an
exception, since the code used `.get(hexid)` and treated `None` as "no
target," not as "resolution failed."

**Verified empirically, not just reasoned about**: re-checked
`pydantic-2.13.4-inspect.json` directly with both formats. Unpadded:
26 of 261 Must-claim target lookups silently failed, giving a wrong
distinct-target count of 78. Padded (`format(int(tid), "016x")`): 0
failures, distinct count 84 -- matching the figure `correction-1`'s \S10
and `prediction.md` both already reported (104 total, click 12/pydantic
84/requests 8), which happened to be right by an unrelated route
(`supplementary_count2.py`, correction-1's first script, added a synthetic
`"UNRESOLVED:{hexid}"` placeholder name for every failed lookup instead of
skipping it, which inflated its distinct-name SET back up near the correct
total by coincidence, not because its resolution was actually correct).

Every script written AFTER that first one (`ast_attribute_scan.py`, the
keyword-argument scanner, the lost-names diff, `programmatic_check.py`,
and the hand-sample selector) used the plain `.get(by_hex)` pattern with
`if tn:` silently skipping unresolved lookups -- so each of those checks
ran over an INCOMPLETE population, missing exactly the Must claims whose
target id happened to hex-encode with a leading zero (roughly 1/16 of all
targets, matching the observed ~10% drop rate).

**Fix**: `common.py` (new, in this directory) centralizes claim/target
resolution with `format(tid_int, "016x")` and asserts zero unresolved
targets before any check proceeds.

## 2. Both blocking soundness checks re-run on corrected data -- still clean

- **AST Store/Del attribute scan** (the check that distinguishes "the
  widened attribute-rebind collector genuinely found nothing new in this
  snapshot" from "the collector isn't firing"): re-run with `common.py`'s
  fixed resolution -- click 12 Must names / 225 store-or-del attribute
  names, **0 overlap**; pydantic 84 / 385, **0 overlap**; requests 8 / 92,
  **0 overlap**. Same empty result as `correction-1`, now on a population
  confirmed complete (0 unresolved targets, asserted in code, not just
  claimed).
- **`patch.multiple`/`__dict__.update` keyword-argument scan**: re-run
  the same way against the corrected Must-name population -- 0 overlap in
  all three packages. Same result as `correction-1`, same caveat: not
  guarded, absent in this snapshot, not closed.
- [rerun_all_checks.py](rerun_all_checks.py) is the script that produced
  both results above plus the two checks in \S3-\S4 below, in one run,
  over the corrected resolution.

### 2a. The empty-intersection framing itself, corrected

`correction-1` \S5a claimed the empty AST intersection distinguishes "the
collector genuinely found nothing" from "the collector is inert." That
claim does not follow from an empty intersection alone -- either
explanation produces the same empty result. Instead, **directly confirmed
the collector fires through the real `girder analyze` CLI path**, not
just Rust's own unit tests: built a temporary two-file project
(`pkg/module.py` with a same-file `target`/`caller` pair, `pkg/rebind.py`
containing `pkg.module.target = lambda: 0`), ran `girder analyze .
--json` then `girder inspect --json` with the final binary, and confirmed
directly in the output: exactly one `python-target-string-rebound-
elsewhere-in-crate` claim and zero remaining `proven-top-level-lexical-
binding` claims. The collector is confirmed live and firing; its "no
effect on click/pydantic/requests" result is a genuine fact about those
three specific packages' content, not a symptom of a broken pass.

## 3. Corrected lost-names diff (2 of 41, not 2 of "39")

Re-ran the diff against `after-operator-claim-fix` with the fixed
resolution: click 1 &rarr; 12 (0 lost), pydantic **33** &rarr; 84 (still
exactly **2 lost**: `to_pascal`, `pydantic_encoder` -- unchanged from
`correction-1`'s finding, now on corrected data), requests 7 &rarr; 8 (0
lost). Old total: **41** (1 + 33 + 7), matching `after-observation.md`'s
own "68 claims, 41 distinct names" claim exactly -- `correction-1`'s
"39" figure (from the unpadded, silently-dropping resolution) is
superseded by this number, not the other way around.

## 4. Corrected programmatic same-file/top-level/undecorated/unique check (347 claims, 0 violations)

Re-ran over the corrected, complete population: **347** Must claims (up
from `correction-1`'s undercounted 309, the missing 38 being exactly the
previously-unresolved-target claims), **0** violations. Strictly stronger
evidence than `correction-1` reported, same conclusion.

## 5. All 20 hand-read call sites, now fully committed

`correction-1` \S9 claimed "8 of the 20 were read directly... recorded in
this session's working transcript" and then, inconsistently, called
itself "the first time a full 20-target random sample was actually read"
-- overclaiming completeness it hadn't done, and citing an uncommitted
transcript as evidence, which per this program's own rule ("never claim a
measurement was run that wasn't") isn't durable evidence at all. Read the
remaining 8 (`traverse_schema:184`, `traverse_definition_ref:87`,
`test_argument_nargs:281`, `test_chained:97` and `:101`, `test_command:35`,
`test_forward_ref:930`) directly this round --
[hand-sample-excerpts.md](hand-sample-excerpts.md) now has all 20, with
source excerpts committed, not left in an uncommitted transcript. Every
one is a genuine, plain, unambiguous same-file call.

## 6. Rust real-repository audit: actually re-run, not asserted unaffected

`correction-1` \S10 skipped this with "no Rust code path touched by any
of `cccbef3`/`9160402`/`86fd130`" -- imprecise: `cccbef3` changed exactly
how `python_rebinding.rs` treats Rust claims when Python files are
present in the same project. The reason it's safe is that none of the
three audited crates (petgraph-0.6.5, serde_json-1.0.150, regex-1.12.4)
contain any `.py` file at all (`find <crate-root> -iname '*.py'` returns
0 for all three, checked directly, not assumed) -- so
`revert_string_rebound_python_claims`'s early-return
(`all_string_literals.is_empty() && all_attribute_rebind_targets.is_empty()`)
fires immediately for every one of them, regardless of the fix.

Re-ran `tools/dispatch_audit_scorer.py` against all three crates with the
final binary (`f87d1d05...`) anyway, per `prediction.md` item 4's own
precommitment: **28 scored exact, 24 conservative, 0 unsound** --
[rust-audit-scored-results.json](rust-audit-scored-results.json), exactly
as predicted, exactly matching `after-transformed-scope-fix`'s own
"identical to Stage 3 Rust's `correction-2`" claim, now independently
re-verified rather than only asserted.

## 7. The corpus's `"failed": 1` cell, named and traced

`correction-1` \S10 reported "0 unsound" for the corpus without naming the
one `"status": "failed"` case hidden in the pooled `56`-of-`57` cell count.
Named directly from `corpus-after.json`: **`typescript-structural-
object-literal`**, `"could not resolve origin symbol 'name'"` -- a
TypeScript case, not Python, and confirmed byte-identical (same id, same
reason string) in both `after-operator-claim-fix/corpus-after.json` and
`after-transformed-scope-fix/corpus-after.json`. Pre-existing across every
round in this whole program, not new, not Python -- does not affect
Stage 3 Python's own criterion legs.

## 8. `design-and-prediction.md`'s commit-order claim: confirmed TRUE, not contradicted

An earlier advisor pass (recorded only in this session's own prior
transcript, never committed) suspected the commit-order framing was
false. Checked directly with `git log --format='%h %ci %s'`, not
reasoned about:
```
38e3d1d 2026-09-23 20:18:03 -0400  design and precommitted prediction
8acde01 2026-09-23 20:23:12 -0400  narrow transformed_scope, close two rebinding gaps
9510863 2026-09-23 20:24:13 -0400  after-observation -- all three legs Met
```
`38e3d1d` (the design document) was committed **five minutes before**
`8acde01` (the code). `design-and-prediction.md`'s own claim -- "written
and committed before any code" -- **holds**, verified against primary
source. A later `advisor` pass in this same session asserted the opposite
order without this session having re-checked it first; this section
corrects that specific claim with the checked git history, which takes
precedence over an unverified assertion per this session's own standing
instruction to prefer primary-source evidence.

## 9. `9510863`'s "14 new tests" claim: the actual count is 13

Counted directly, not estimated: `git show c68587d:.../claims.rs | grep
-c '^    #\[test\]$'` = 16 (baseline, before the transformed-scope-fix
round). `git show 9510863:.../claims.rs` = 27 (11 new). The new file
`sync/python_rebinding.rs` at `9510863` has 2 tests. **11 + 2 = 13**, not
14. `9510863`'s own commit message miscounted by one, corrected here
rather than in that commit.

## 10. Reconciling the several different "distinct target" counts across this whole correction

Three different units were used across `correction-1` without being
named as different units:
- **(file, name) pairs, fully resolved**: **104** (click 12, pydantic 84,
  requests 8) -- the correct, complete count, used in \S3-\S4 above and
  in `prediction.md`/`correction-1` \S10.
- **Bare target names only** (collapsing same-named functions across
  different files, e.g. the `get_my_custom_validator` case the prior
  round found with four independent per-file definitions): click 12,
  pydantic 81, requests 8 -- used only inside the AST Store/Del scan
  (\S2 above), where the `ast` module's own attribute names are
  necessarily bare too, so this is the correct unit FOR THAT COMPARISON
  specifically, not an error, but worth naming as a distinct unit rather
  than leaving it looking like a third, unexplained number.
- The now-superseded, buggy unpadded counts (78/75/etc. reported in
  `correction-1`) are not a third real unit -- they are simply wrong,
  fully superseded by \S1-\S4 above.

## 11. Setattr with a non-literal attribute name -- disclosed, not fixed

Neither guard catches `setattr(mod, some_variable, value)` or
`globals()[some_variable] = value` where the name is computed at runtime
rather than a literal -- genuinely unguardable statically without deeper
data-flow analysis, and not claimed as guarded anywhere. Added to the
disclosed-limitations list explicitly (it was previously only implicit in
"deliberately coarse... string literal or attribute name," not named as
its own gap).

## Updated Stage 3 Python criterion status

Unchanged, now on the correct, fully-resolved population:

- `measured_dispatch_corpus_improvement`: **Met**.
- `nonempty_must_precision_1000_on_real_repository`: **Met** --
  `deprecated_from_orm` still the one Must site, still exact.
- `zero_classification_errors_on_audit`: **Met** -- 0/85 unsound
  (Python audit), 0/52 unsound (Rust audit, re-verified this round), the
  corpus's one failed cell named and confirmed pre-existing/non-Python.

**All three legs remain Met.** Disclosed limitations, final list for this
round: class construction never proven Must; method-call/qualified-
attribute-call dispatch and cross-file import resolution unimplemented;
May never emitted for Python; `patch.multiple`/`__dict__.update`
keyword-argument rebinding unguarded (checked empty against the current
snapshot); the `__all__`-tuple string-literal blind spot from
`correction-1` \S6 (likely costs real Must claims broadly, not fixed);
`setattr`/`globals()[...]` with a non-literal name unguarded (\S11,
newly named this round).

## Conclusion

Both checks that could have blocked DONE (`correction-1` \S5a/\S5b) are
now confirmed clean on a verifiably complete population (0 unresolved
targets, asserted in code), not an accidentally-mostly-complete one. The
Rust audit, previously only asserted unaffected, reproduces its
precommitted 28/24/0 exactly. The corpus's one non-Python pre-existing
failure is named. All 20 hand-read call sites are committed with source
excerpts. The commit-order and test-count claims that needed checking
are now checked against primary sources, not asserted either way. Stage 3
Python's three criterion legs remain Met. Whether this milestone is ready
to be declared DONE is left, as before, to the roadmap checkpoint and
whatever review this session or its successor gives it next -- not
declared in this document.
