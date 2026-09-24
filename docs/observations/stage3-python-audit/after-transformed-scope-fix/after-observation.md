# Stage 3 Python after-observation: narrowed `transformed_scope`

Source commit: `8acde01` (design/prediction in `38e3d1d`). Binary measured:
`/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder`,
sha256 `7e58a7edfc3af2b2b571cbfd10134390dc991fc3bf9fa653c39ad47c5a8f2316`.

## Result up front: prediction matched exactly, zero unsound cells, both rebinding gaps closed

[design-and-prediction.md](../gate-profile/design-and-prediction.md)
precommitted: **exactly two cells move, both conservative → exact,
nothing else** — one real audit site (`deprecated_from_orm`) and one
corpus cell (`decorator-elsewhere-in-file/test_direct`). Both moved,
nothing else did, on every measurement below.

## 1. Real-repository audit: 85 scored, 73 exact, 12 conservative, 0 unsound

[audit-scored-results.json](audit-scored-results.json) (was 72/13/0).
Diffed programmatically against the before-fix results by index: **exactly
one site changed**, index 24 (`pydantic-2.13.4
tests/test_deprecated.py:271`), `conservative -> exact`,
`observed_reason: proven-top-level-lexical-binding`,
`observed_caller: crate::tests::test_deprecated::test_nested_orm` — the
predicted site, target verified as `tests/test_deprecated.py:40`'s
`deprecated_from_orm` definition (confirmed undecorated, not nested,
directly). 0 overclaim.

## 2. Existing Python dispatch corpus: predicted cell moved, all others unchanged

[corpus-after.json](corpus-after.json): pooled 21/35 → **22/34**,
`must_true_positives` 4 → **5**, `must_false_positives: 0`.
`python-decorator-elsewhere-in-file`'s `test_direct` case moved
conservative → exact, the predicted site. Per-language breakdown:
Rust 5/9, TypeScript 5/10, Go 5/9 — **byte-identical to the pre-fix
baseline**, confirming the Python-only language gate holds. Python:
6/7 → 7/6.

## 3. Stage 1 oracle and Rust real-repository audit: unaffected

[oracle-after.json](oracle-after.json): Rust and Python both
`precision: 1.0, recall: 1.0` — unchanged, stderr empty. Python's
`classified.boundary_count` and `classified.must` were also checked (not
just precision/recall) and are unaffected by this specific change (unlike
the earlier operator-claim fix, this one didn't touch the oracle's own
fixture files).

Rust real-repository audit re-checked (shared `claims.rs` code, gated to
Python): identical 28 exact / 24 conservative, 0 unsound — matches Stage
3 Rust's `correction-2` exactly.

## 4. Common gates

All four, run once each: `cargo test --workspace -j1 --quiet`, `cargo
clippy --workspace --all-targets -j1 -- -D warnings`, `cargo fmt --all
--check`, `node --test npm/test/*.test.js` — [gates.log](gates.log), all
exit 0.

## 5. Unit tests, mutation-verified on genuinely decorated fixtures

14 new tests across `claims.rs` and the new `sync/python_rebinding.rs`.
This is the first time `top_level`'s decorated-function exclusion and the
`clean` scan's protections were exercised on a file that wasn't already
wholesale-rejected by `transformed_scope` before this change — each was
mutation-verified (break the specific check, confirm the relevant test
fails; restore, confirm it passes), not assumed to still work:
- Positive case (decorator elsewhere in file no longer blocks): fails
  under the pre-change code, passes after.
- `top_level` excludes a decorated target itself: fails when `top_level`
  is mutated to accept a `decorated_definition` parent.
- Redefinition, decorator-argument-value, bare-decorator-usage,
  global/del/import-as shadowing shapes: all rely on the pre-existing
  `clean` scan (not new code), verified to still hold.
- Wildcard import empties the whole file's proven map: fails when that
  check is disabled.
- Same-file and dotted-path string-literal rebinding: each fails when
  its specific check is disabled.
- `exec`/`eval` anywhere blocks the whole file: fails when disabled.
- TypeScript with the identical decorator-elsewhere shape stays
  unaffected: fails when the Python-only language gate is removed.
- Project-wide revert (`python_rebinding.rs`): a same-file Must claim is
  reverted when a DIFFERENT file rebinds the target by string (fails when
  the whole pass is disabled); an unrelated name in another file does NOT
  revert an unrelated target (fails when the name-matching check is made
  over-permissive).

## 6. Supplementary check: every new Must claim, snapshot-wide rebinding re-scan, and a hand-verified sample

**Claim volume**: same-file Must claims (`proven-top-level-lexical-
binding`) went from 68 (41 distinct targets: click 1, pydantic 33,
requests 7) to 352 (107 distinct targets: click 12, pydantic 87,
requests 8) before the project-wide guard, and to a slightly smaller
distinct-target count after it reverted several (click 12, pydantic 84,
requests 8) — the expected shape of a real resolver improvement across
the whole codebase, not just the frozen 105-site sample, mirroring how
Rust's Stage 3 method-call resolver produced 49 supplementary claims
beyond its own frozen audit.

**Snapshot-wide rebinding re-scan, corrected methodology**: the design
document's own first-pass check (line-proximity, 41 names) was found
insufficient by a later review and is not what this result relies on.
Re-ran the corrected, Python-`ast`-based, whole-package scan (walking
every `Call`/`Subscript` node, not scanning lines) against **every**
distinct Must target after both guards (104 total: click 12, pydantic 84,
requests 8): **zero remaining string-rebinding hits.** The one real case
this design's own investigation found
(`pydantic.networks.import_email_validator`, see design-and-prediction.md)
is confirmed reverted: `pydantic/networks.py`'s three internal call sites
to `import_email_validator` (lines 1006, 1081, 1302) all now carry
`reason: python-target-string-rebound-elsewhere-in-crate` instead of a
Must claim, read directly from the regenerated inspect JSON. The
project-wide pass also reverted two further, unrelated cases (a target in
`pydantic/deprecated/copy_internals.py` and one in
`tests/test_pickle.py`) caught by the same general-purpose guard, not
specifically searched for in advance.

**Hand-verified sample**: 20 of the 62 newly-Must-proven distinct target
names, chosen by a fixed random seed (`20260923`) from the full new-target
set, checked directly against source (not trusted from the tool's own
claim): every sampled target has exactly one same-file top-level
definition (confirmed via `grep -rn "^def NAME("`/`"^async def NAME("` —
one sample, `get_my_custom_validator`, has four SEPARATE definitions
across four independent `tests/mypy/outputs/*.py` fixture files, each
file self-contained with its own definition and call site, which is
correctly per-file-unique, not crate-wide-ambiguous) and is genuinely
undecorated (no `@...` line immediately preceding the `def`, confirmed by
reading the two lines above each definition directly).

## Updated Stage 3 Python criterion status

- `measured_dispatch_corpus_improvement`: **Met** — pooled 21/35 → 22/34,
  `must_true_positives` 4 → 5, the predicted corpus cell moved, 0 unsound.
- `nonempty_must_precision_1000_on_real_repository`: **Met** — 1 Must site
  in the frozen 105-site sample (`deprecated_from_orm`), scored exact,
  precision 1/1 = 1.000.
- `zero_classification_errors_on_audit`: **Met** — 0/85 unsound cells (0
  overclaim, 0 unsafe_exclusion), holding from the operator-claim fix and
  unaffected by this change.

**All three Stage 3 Python criterion legs are now Met on the frozen
105-site real-repository audit and the existing dispatch corpus.**

## Whether this means Stage 3 Python is DONE

Not yet declared here. Per this session's own established discipline for
exactly this situation (Stage 3 Rust's own "all three legs met" moment
needed a further `advisor` review before trusting a DONE declaration, and
that review found three more genuine unsoundnesses across three further
rounds before DONE actually held) — this after-observation reports the
measurement honestly and stops short of the DONE claim itself pending that
same review discipline, rather than repeat the exact overclaiming pattern
this program has already caught itself doing multiple times this session.
Disclosed limitations carried forward, unclosed by this round: class
construction is never proven Must (`claims.rs`'s `top` never collects
`class_definition`, gate-profile Finding 3); method-call/qualified-
attribute-call dispatch (9/13 of the original conservative sites) and
cross-file import resolution (2/13) are entirely unimplemented for
Python; May is never emitted for Python at all (0 May observed anywhere
in this whole program's Python measurements to date, including the
corpus's own `override-via-subclass`/`super-mro-diamond` cases, which
remain conservative).
