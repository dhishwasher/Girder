# Correction-1 re-measurement: precommitted prediction

Written and committed BEFORE running the real-repository audit, dispatch
corpus, or oracle against the final binary. Per this session's own
discipline (every Rust/Python resolver round precommits before
measuring): any divergence from this prediction is a stop signal, not
something to explain away after the fact.

## What changed since the committed `after-transformed-scope-fix/after-observation.md`

Three commits, each already gated and pushed:
- `cccbef3`: fixed `python_rebinding.rs`'s cross-language false-revert bug
  (it reverted non-Python same-file Must claims when an unrelated Python
  string elsewhere in the project matched the target's bare name).
  Quantified directly against Bit-code itself (a real mixed-language
  repo): 6 wrongly-reverted non-`.py` claims before, 0 after --
  [bitcode-cross-language-before-fix-extract.json](bitcode-cross-language-before-fix-extract.json)
  vs.
  [bitcode-cross-language-after-fix-extract.json](bitcode-cross-language-after-fix-extract.json).
  This fix can only ever ADD Must claims back (it narrows what gets
  reverted), and only for non-Python files -- it has **zero effect** on
  the three all-Python audited packages (click/pydantic/requests), so it
  predicts no change to any Python-only measurement below.
- `9160402` + `86fd130`: closed a real gap the same-file/project-wide
  rebinding guards had -- `mod.target = ...` / `del mod.target` /
  `mod.target += ...` / tuple targets / `for`/`with` targets / multi-target
  `del` were invisible to both guards, since neither indexed anything but
  string literals. `86fd130` widened the collector from matching only the
  simplest bare-attribute shape to walking the whole subtree of each
  target position (tuple assignment, `for`, `with ... as`, multi-target
  `del`), confirmed against the grammar directly, not assumed. This CAN
  reduce Must claims on the three real Python packages (an actual
  attribute-rebind hit would newly demote a same-file Must to Unknown via
  the project-wide pass) -- this is the one change with real discriminating
  power on the measurements below.

Final binary: `/mnt/chromeos/removable/MOVESPEED/aetherforge-target/debug/girder`,
built explicitly with `cargo build -p aether-app` from commit `86fd130`,
sha256 `f87d1d058bb91418e817af35efb4956096a34e486ea39d7346a17477bb50d96f`.

## Checked before predicting, not assumed

Grepped the pydantic snapshot directly for any attribute-rebind shape
touching `deprecated_from_orm` (the one predicted-Must real-audit
target and the one site with actual discriminating power over the
`nonempty_must_precision_1000_on_real_repository` criterion leg):
`\.deprecated_from_orm\s*=`, `del .*\.deprecated_from_orm\b`, `for
.*\.deprecated_from_orm\b`, `as .*\.deprecated_from_orm\b` -- zero hits
across the whole pydantic-2.13.4 snapshot.

## Precommitted prediction

1. **Real-repository audit**: **73 scored, 12 conservative, 0 unsound**,
   unchanged from the committed `after-observation.md` --
   `deprecated_from_orm` survives (no attribute-rebind hit found above).
   If it reverts instead, the result is 72/13/0 with **zero** Must sites in
   the frozen sample -- that fails the nonempty-precision leg outright and
   is a stop, not a re-explanation.
2. **Dispatch corpus**: pooled **22/34** unchanged, per-language cells
   (Rust 5/9, TypeScript 5/10, Go 5/9, Python 7/6) unchanged -- the
   corpus's own fixture dirs contain no attribute-rebind shape (checked by
   reading all 12 Python corpus cases directly in the earlier design
   round) and the cross-language fix cannot affect single-language
   fixture directories at all (see disclosure below on why the corpus
   could never have exercised that fix's language-gating in the first
   place).
3. **Stage 1 oracle**: `precision`/`recall` unchanged (`1.0`/`1.0` for
   both Rust and Python), stderr empty. `classified.must`'s exact count
   may shrink if the oracle's own fixtures contain any attribute-rebind
   shape (not checked in advance the way the audit target was, since the
   oracle fixtures are small and reading them during scoring is
   sufficient); direction only predicted, not a number.
4. **Rust real-repository audit**: unchanged (28 exact / 24 conservative,
   0 unsound) -- none of this round's three commits touch any
   Rust-specific code path.
5. **Supplementary distinct-target count** (crate-wide, not just the
   frozen sample): predicted to **decrease** from the committed round's
   104 (click 12, pydantic 84, requests 8), because the widened collector
   can now flag `self.<name> = ...`/`self.<name>[key] = ...`-shaped
   attribute assignments whose `<name>` happens to collide with an
   unrelated top-level function's bare name -- a known, disclosed
   over-conservatism (per the "err toward excluding a name that's
   actually safe" design), not a bug. Exact numbers reported after
   measuring, not predicted, since this count was never itself a
   frozen criterion.
6. **Bit-code cross-language non-`.py` reverts**: **0**, already measured
   above with the final binary (not a prediction -- already run and
   recorded).

If anything beyond `deprecated_from_orm`/the corpus's two
already-predicted cells (from the original `design-and-prediction.md`)
moves on the audit or corpus, or the supplementary count *increases*, or
the oracle's precision/recall changes at all, that is a stop signal to
investigate before trusting the result.
