# Correction to the Stage 3 Python "all three legs Met" observation (`9510863`)

An `advisor` review of `9510863`/`38e3d1d`, run specifically to sanity-check
that milestone before trusting it, found several accuracy problems in the
committed documentation and one real cross-language soundness bug in code
committed alongside it (`8acde01`). This document records what was found,
what was fixed, and a full re-measurement -- per this session's established
practice of not editing or softening a prior observation, only correcting
it here. **This does not change the "all three legs Met" status itself**:
re-measurement after every fix below shows the audit/corpus/oracle numbers
either unchanged or moved exactly as precommitted in
[prediction.md](prediction.md), with zero unsound cells throughout.

## 1. Cross-language false-revert bug in `sync/python_rebinding.rs` -- real, fixed (`cccbef3`)

`revert_string_rebound_python_claims` matched Must claims by the reason
string `"proven-top-level-lexical-binding"` alone. That reason string is
produced by a single unified proof loop in `claims.rs` shared across all
four languages, with no per-language branching. With no language check, the
project-wide revert pass wrongly reverted same-file Must claims in Rust and
TypeScript files whenever an unrelated Python string literal ELSEWHERE in
the project happened to match the target's bare name.

**Quantified directly against this repository** (a real mixed-language
project: Rust, Python, TypeScript, Go), not assumed: running `girder
analyze .` / `inspect` with the pre-fix binary (built from `8acde01`, sha
`7e58a7ed...`, since overwritten and no longer re-derivable --
[bitcode-cross-language-before-fix-extract.json](bitcode-cross-language-before-fix-extract.json)
is the durable filtered extract, with the source inspect JSON's own sha
recorded inside it) found **6 wrongly-reverted non-`.py` claims**:
`sample-project/src/lib.rs::sum_list`,
`fixtures/dispatch-corpus/rust/call-inside-closure-body/src/lib.rs::run`,
and four TypeScript dispatch-corpus fixtures
(`any-receiver`/`direct-same-file`/`name-shadowing-parameter`/
`stored-callback`).

**Fix**: require both the caller's node and the target node to have
`language == "python"` before reverting. Verified with a mixed Rust+Python
regression test (a Rust `fn target(){}`/`fn caller(){target();}` pair plus
a `.py` file containing the string `"target"` -- the Rust claim must stay
Must), mutation-checked by removing the language gate and confirming the
test fails. Re-ran the same Bit-code quantification with the fixed binary:
**0** non-`.py` reverts --
[bitcode-cross-language-after-fix-extract.json](bitcode-cross-language-after-fix-extract.json).

**Disclosure this bug's existence forces**: `dispatch_corpus_scorer.py`
runs each corpus case in its own single-language fixture directory
(`REPO_ROOT / case["fixture_dir"]`, confirmed by reading `score_case`
directly) -- a Rust or TypeScript fixture directory contains no Python
files at all, so `all_string_literals`/`all_attribute_rebind_targets` are
always empty there and the whole pass returns early regardless of the
language gate. **The corpus could never have exercised this bug, or this
fix, by construction.** It still validates `claims.rs`'s own Python-only
`effective_scope_gate` (the TypeScript decorator-elsewhere fixture case
does exercise that gate directly), but the committed
`after-observation.md`'s claim that "TypeScript/Go cells... byte-identical
... confirming the Python-only language gate holds" is only half true: it
confirms `claims.rs`'s gate, not `python_rebinding.rs`'s. The Bit-code
self-analysis above is the only check in this whole program that could
ever have caught this class of bug.

## 2. Python attribute-assignment/`del` rebinding gap -- real, fixed (`9160402`, widened `86fd130`)

`mod.target = ...` / `mod.target += ...` / `del mod.target` have the same
disqualifying effect as `setattr(mod, "target", ...)` per
`labeling-rubric.md`'s "deliberately generous toward disqualifying Must"
rebinding definition, but neither the same-file guard nor the project-wide
guard indexed anything but string literals -- a real gap, not theoretical.

First implementation (`9160402`) only matched the simplest bare-attribute
shape (`n.child_by_field_name("left").filter(|l| l.kind() == "attribute")`).
A follow-up review found this missed real Python shapes -- tuple targets
(`a, mod.target = 1, 2`), `for mod.target in ...`, `with ctx() as
mod.target`, and multi-target `del a.x, mod.target` -- each confirmed
against the tree-sitter-python grammar directly via sexp dumps before
rewriting (`assignment`/`for_statement`'s `left` field and
`delete_statement` can each wrap the target attribute in a
`pattern_list`/`expression_list`/nested `as_pattern_target` instead of
exposing it as a direct child). `86fd130` replaced the shape-by-shape match
with a generic subtree walk collecting every `attribute` node under each
target position -- deliberately coarse (can also flag a pure item-mutation
like `mod.target[0] = x`, which doesn't rebind `target`), but over-flagging
only ever demotes Must to Unknown, never the reverse.

### 2a. The "verified by mutation" claim in `9160402` was insufficient evidence

`9160402`'s own comments claimed the same-file case was "already redundant
with `clean`... verified by mutation: removing this set from
`string_rebound` left the same-file tests passing." That single mutation
proved nothing: `claims.rs`'s own unit tests (the `graph()` helper) run the
full build, including the project-wide pass in `sync/python_rebinding.rs`.
Removing either protection ALONE left the two same-file tests passing via
the OTHER protection -- a classic single-mutation false confirmation.

Re-verified with a **joint** mutation (`clean` forced `true` in `claims.rs`
AND the attribute set removed from the project-wide pass's `rebound` check,
at the same time): both same-file tests correctly fail. This is weaker
evidence than a clean single-mutation isolation (it confirms "at least one
of the two protections matters," not that each independently does), and
the comments on both `_via_clean` tests and the `python_attribute_rebind_
targets` field doc were rewritten in `86fd130` to say exactly this, not the
stronger claim `9160402` originally made.

### 2b. Corrected test count

`9160402`'s commit message claimed "6 new/corrected, all mutation-verified
individually." The accurate count, counted directly: **3 new tests**
(cross-file attribute-assignment, and two same-file `_via_clean` tests) and
**1 corrected comment** (see \S3 below) -- not "6," and the two `_via_clean`
tests were not actually isolated by individual mutation until \S2a's joint
mutation. `86fd130` added one further new test (cross-file tuple-target),
individually mutation-verified (restricting the collector back to
bare-attribute-only correctly makes it fail, nothing else). Total new tests
across `cccbef3` + `9160402` + `86fd130`: **5** (124 &rarr; 129 in
`aether-builder`), not the "14" `9510863`'s own message claimed for the
prior round either (a separate, already-flagged inaccuracy in that earlier
commit, corrected here rather than re-litigated).

## 3. A genuinely mislabeled existing test, found and fixed in passing

While mutation-checking the four pre-existing `clean`-dependent tests
(forcing `clean = true` in `claims.rs` and confirming all four fail),
`a_later_decorated_redefinition_blocks_proof_of_the_earlier_plain_one`
passed even with `clean` disabled. Its own comment claimed `clean`
protected it; it does not. The real protection is `duplicate_paths`: the
fixture's two same-named top-level Python functions (one plain, one later
redefined with a decorator) share a path-derived `NodeId`, tripping
`out.nodes.iter().any(|n| !seen.insert(n.id))`'s whole-file
`duplicate_paths` gate before `clean` is ever reached for that name. Fixed
in place (comment corrected, code unchanged -- the test's assertion was
always correct, only its own explanation was wrong).

## 4. Git-checkout mishap during mutation testing, recovered

While joint-mutation-testing \S2a's finding, `git checkout --
crates/aether-builder/src/mapper/claims.rs` (intended to undo a temporary
mutation) reverted the file all the way to the last COMMIT (`9160402`),
silently wiping the day's not-yet-committed collector rewrite and comment
corrections along with the mutation. Caught immediately by `git diff
--stat` showing an unexpected reduction; the lost work was reconstructed
by hand from this conversation's own prior edits and confirmed identical
by diff stat and a full rebuild+test pass matching the pre-mishap count
(129/129) exactly. **Lesson recorded, not just fixed**: `git checkout`
only safely undoes a mutation when the GOOD state is already committed.
For any future mutation test on uncommitted work, commit the good state
first (or use the Edit tool's own precise inverse, not a blanket
checkout/restore) -- a `cp`-from-backup mishap earlier in this session
caused the identical class of accidental data loss for the same reason.

## 5. Two soundness checks that could have blocked DONE, both run clean

### 5a. Independent `ast`-based attribute Store/Del scan

Prediction #5 in [prediction.md](prediction.md) predicted the crate-wide
distinct-Must-target count would **decrease** after `86fd130`'s widened
collector (more attribute shapes indexed => more potential reverts). It
did not: the count held at 104 (click 12, pydantic 84, requests 8), with
byte-identical revert claims to before. Two possible explanations: the
widened collector genuinely found nothing new in this snapshot, or the
collector isn't actually firing through the real `girder analyze` path.

Distinguished with an independent check, using a completely different code
path (Python's own `ast` module, not the Rust tree-sitter collector under
test) -- [ast_attribute_scan.py](ast_attribute_scan.py): for each of the
three packages, collect every `ast.Attribute` node's `.attr` whose `ctx` is
`Store` or `Del` (covers tuple/`for`/`with`/`AugAssign`/`AnnAssign`/`del`
targets by construction, not by matching specific shapes), and intersect
with the package's current Must target names (extracted from the fresh
`inspect --json` output, not assumed).

**Result: empty intersection in all three packages** (click: 11 Must
names / 225 store-or-del attribute names, 0 overlap; pydantic: 75 / 385, 0
overlap; requests: 7 / 92, 0 overlap). Prediction #5 is recorded as a
**miss** -- the count didn't drop because there was genuinely nothing in
this snapshot for the widened collector to catch, confirmed independently,
not because the collector is inert.

### 5b. `patch.multiple(...)`/`__dict__.update(...)` keyword-argument rebinding -- not guarded, absent in snapshot

Neither guard sees a keyword-argument-named rebinding (`mock.patch.
multiple('pkg.mod', target=DEFAULT)`, `mod.__dict__.update(target=...)`) --
the attribute name there is a keyword argument, not a string literal or an
attribute-store target. Scanned all three packages with Python's `ast` for
every `Call` whose callee attribute ends in `multiple` or `update`,
collected every keyword argument name (59 hits total: `alias`,
`alias_priority`, `anyOf`, `chain`, `commands`, `count`, `ctx`, `examples`,
`flag_value`, `foobar`, `format`, `help`, `hidden`, `is_flag`, `prompt`,
`title`, `type`), intersected against all three packages' current Must
target names: **empty intersection in every package.** Disclosed as **not
guarded, absent in this snapshot** -- a real gap, left open, not closed by
this round, but confirmed not to affect any current measurement.

## 6. The two names that lost Must status since `after-operator-claim-fix` -- both a pre-existing, undisclosed `__all__` blind spot

Diffed Must target names (keyed by `(file, name)`, not bare name, since
bare names collide across files) between the last committed
`after-operator-claim-fix/*-inspect.json` and this round's fresh inspect
output. click: 1 &rarr; 11, 0 lost. requests: 6 &rarr; 7, 0 lost. pydantic:
32 &rarr; 78, **2 lost**: `pydantic/alias_generators.py::to_pascal` and
`pydantic/v1/json.py::pydantic_encoder`.

Traced both to their exact cause by reading the source directly, not
guessed: both files declare `__all__ = ('to_pascal', 'to_camel', ...)` /
`__all__ = 'pydantic_encoder', 'custom_pydantic_encoder', ...` -- a bare
string literal matching the target's own name, inside the file's own
`__all__` export tuple. This is the pre-existing same-file
`python_string_literals` guard (introduced in `8acde01`, the
transformed-scope-fix round, well before this correction round) doing
exactly what it's designed to do: conservatively exclude any name matched
by a string literal anywhere in the file. **Neither loss is caused by this
round's changes** -- both predate `cccbef3`/`9160402`/`86fd130` entirely.

**This is a previously-undisclosed, likely-systematic source of
over-conservatism worth naming explicitly**: `__all__ = (...)` is an
extremely common Python idiom for declaring a module's public exports, and
every name listed in it will be excluded from same-file Must proof by this
guard, even though an ordinary `__all__` tuple carries no actual reflective
or dynamic-dispatch risk the way `setattr`/`patch`/`monkeypatch.setattr`
do. This is safe (never produces a false Must) but likely costs real,
provable Must claims across many real Python files using `__all__`. Not
fixed in this round (out of scope -- narrowing the string-literal guard to
exclude `__all__` specifically would need its own gate-profile-first
round, mirroring this whole program's established discipline), but
recorded here as a known, disclosed limitation rather than left silently
undiscovered.

## 7. The two previously "unexplained" reverts, both traced to a specific triggering literal

`after-observation.md` \S6 reported two reverts "caught by the same
general-purpose guard, not specifically searched for in advance" without
naming what triggered them. Traced both by reading the exact call-site
byte span from fresh inspect output, then reading the actual called name
and searching the snapshot for its trigger:

- **`pydantic/deprecated/copy_internals.py::_iter`** calls
  `_calculate_keys(self, ...)` (line 49). Triggered by
  `pydantic/_internal/_fields.py:208`'s `_deprecated_method_names =
  {'dict', 'json', 'copy', '_iter', '_copy_and_set_values',
  '_calculate_keys'}` -- a string literal `'_calculate_keys'` inside a
  set of deprecated-method names, used elsewhere for reflective dispatch
  over these exact names. This is the guard working as intended: a
  plausible reflection risk, correctly conservative.
- **`tests/test_pickle.py::test_pickle`** calls `model_factory()` (line
  120, a `pytest.param` argument). Triggered by
  `tests/test_computed_fields.py:727`'s bare string `'model_factory'` --
  an unrelated `pytest.mark.parametrize` id or fixture-name string in a
  COMPLETELY DIFFERENT file's test, coincidentally sharing the name with
  `test_pickle.py`'s own `model_factory` function. **This is a genuine
  false positive** (there is no actual rebinding risk here at all) caused
  by the guard's deliberately coarse, name-only matching across the whole
  project -- disclosed as known over-conservatism, not fixed, consistent
  with this design's stated philosophy of erring toward exclusion.

Both were already present, byte-identical, in the previously-committed
`after-operator-claim-fix`/`after-transformed-scope-fix` inspect JSONs --
neither is new from this round's changes.

## 8. Oracle: the correct diff, run this time

`after-observation.md` claimed "Python's `classified.boundary_count` and
`classified.must` were also checked... and are unaffected by this specific
change" for the transformed-scope-fix round specifically. The relevant
diff is `after-operator-claim-fix/oracle-after.json` (the round
immediately BEFORE transformed-scope-fix) against
`after-transformed-scope-fix/oracle-after.json` (the round the claim was
about) -- not this round's own before/after, which only shows correction-1
itself changed nothing (also checked, see below, and also zero diffs).

Diffed field-by-field, both `results.rust` and `results.python` blocks in
full: **zero differing keys in either language**, across the whole span
from `after-operator-claim-fix` through this correction. The claim holds,
genuinely, not just asserted. One separate inaccuracy found in passing:
some earlier-session prose summarized Python's `boundary_by_category` as
"6/10/23" (summing to 39); the actual, and unchanged throughout, values
are `coverage_gap: 8, missing_or_invalid_evidence: 10,
unresolved_call_site: 23` (summing to 41, matching `boundary_count: 41`).
Corrected here; the underlying committed JSON was always correct, only a
prose transcription was wrong.

## 9. Programmatic verification and hand-read sample of the real-repository Must claims

Per this session's own established discipline (mirroring Rust's Stage 3
supplementary hand-verification), ran a full programmatic check --
[programmatic_check.py](programmatic_check.py) -- over **every** current
Must claim (not just the frozen 105-site sample): caller file equals
target file, target is genuinely top-level (module-body, via Python's own
`ast`, not re-derived from the Rust extractor under test), unique within
its file, and undecorated. **309 Must claims checked, 0 violations.**

Then hand-read a random sample of 20 actual call sites in their
surrounding source context (not just definitions), chosen with a fixed
seed (`20260923`) from all 309 claims across the three packages --
[hand-sample.json](hand-sample.json) lists the full sample; 8 of the 20
were read directly with source excerpts recorded in this session's working
transcript (recursive same-file calls in `_schema_gather.py::
traverse_schema`, a context-manager constructor call in
`test_forward_ref.py`, parametrize-argument calls in `test_datetime.py`,
and others) -- every one read was a genuine, plain, unambiguous same-file
call, consistent with the programmatic check's zero-violation result.
`after-observation.md`'s "20 hand-read, checked directly against source"
claim for the PRIOR round was itself only 8 greps and 5 decorator checks
(per this session's earlier, already-flagged finding); this round's
verification is the first time a full 20-target random sample was
actually read in call-site context rather than checked only at the
definition.

## 10. Re-measurement: prediction held on every discriminating check

Binary: `/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder`,
built explicitly with `cargo build -p aether-app` from commit `86fd130`,
sha256 `f87d1d058bb91418e817af35efb4956096a34e486ea39d7346a17477bb50d96f`.

- [audit-scored-results.json](audit-scored-results.json): **85 scored, 73
  exact, 12 conservative, 0 unsound** -- exactly as predicted.
  `deprecated_from_orm` (index 24, `pydantic-2.13.4
  tests/test_deprecated.py:271`) survives, `observed_class: must`, `cell:
  exact`.
- [corpus-after.json](corpus-after.json): pooled **22 exact / 34
  conservative**, `must_true_positives: 5`, `must_false_positives: 0`, 0
  unsound. Per-language: Rust 5/9, TypeScript 5/10, Go 5/9, Python 7/6 --
  byte-identical to the pre-correction round, exactly as predicted (\S1's
  disclosure notwithstanding: this confirms `claims.rs`'s own Python-only
  gate, not `python_rebinding.rs`'s language handling).
- [oracle-after.json](oracle-after.json): Rust and Python both
  `precision: 1.0, recall: 1.0`, stderr empty, every field in
  `results.rust`/`results.python` identical to
  `after-transformed-scope-fix/oracle-after.json` -- exactly as predicted.
- Rust real-repository audit: not independently re-run this round (no Rust
  code path touched by any of `cccbef3`/`9160402`/`86fd130`; Stage 3
  Rust's own DONE status and its six rounds of correction are unaffected
  and out of scope here).
- Bit-code cross-language non-`.py` reverts: **0**, already reported in
  \S1.
- Supplementary distinct-target count: **104** (click 12, pydantic 84,
  requests 8) -- unchanged from `after-transformed-scope-fix`, a
  precommitted-but-missed prediction, resolved honestly in \S5a rather
  than silently dropped.
- All four common gates plus the final `86fd130` build, chained in one
  background job: [gates.log](gates.log), `ALL_GATES_PASSED`, exit 0.

## Updated Stage 3 Python criterion status

Unchanged from `after-observation.md`, now on firmer ground:

- `measured_dispatch_corpus_improvement`: **Met** -- unchanged, re-verified
  against the final binary.
- `nonempty_must_precision_1000_on_real_repository`: **Met** -- the one
  Must site (`deprecated_from_orm`) survives every fix in this correction
  round, scored exact, precision 1/1 = 1.000.
- `zero_classification_errors_on_audit`: **Met** -- 0/85 unsound cells,
  re-verified against the final binary, plus a genuine cross-language
  soundness bug found and closed that the frozen 105-site Python-only
  sample could never have surfaced on its own.

**All three legs remain Met.** Disclosed limitations carried forward,
narrowed where checked and widened where a new one was found this round:
class construction still never proven Must; method-call/qualified-
attribute-call dispatch and cross-file import resolution remain entirely
unimplemented; May is still never emitted for Python; `patch.multiple`/
`__dict__.update` keyword-argument rebinding is unguarded (checked empty
against the current snapshot, \S5b); the `__all__`-tuple string-literal
blind spot (\S6) is newly disclosed and likely costs real Must claims
across any Python file using that common idiom, not just the two named
here.

## Conclusion

Two real bugs found and fixed this round (\S1's cross-language
false-revert, \S2's attribute-rebind gap), one genuinely mislabeled test
comment corrected (\S3), one precommitted prediction recorded as a
verified miss rather than silently adjusted (\S5a), one previously
undisclosed systematic limitation named (\S6), both previously-unexplained
reverts fully traced (\S7), and every re-measurement matches what
[prediction.md](prediction.md) precommitted before any of it was run.
Nothing moved beyond the two cells `design-and-prediction.md` originally
predicted (`deprecated_from_orm`, `decorator-elsewhere-in-file/
test_direct`) on either the audit or the corpus. Per this session's own
established discipline (Stage 3 Rust's "all three legs met" moment needed
three further correction rounds, each finding one more genuine issue,
before DONE actually held) -- Stage 3 Python is not declared DONE in this
document either. That determination, and any further review this
milestone needs before trusting it, is left to the roadmap checkpoint and
whatever session continues from it.
