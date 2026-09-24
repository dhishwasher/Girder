# Correction-3: the real prediction-timing problem, a completed hand-sample, and the two checks that clear DONE

Closes out the review chain started by `correction-1`/`correction-2`. Per
this session's practice, neither of those is edited in place.

## 1. The real problem with `design-and-prediction.md`'s prediction: not committed blind

`correction-2` \S8 checked commit order (`38e3d1d` before `8acde01`) and
concluded the prediction claim "holds." That check answered the wrong
question. **File mtimes on the committed measurement artifacts show the
actual audit/corpus/oracle results already existed on disk before
`38e3d1d` was committed:**

```
20:11:51  after-transformed-scope-fix/audit-scored-results.json
20:12:30  after-transformed-scope-fix/corpus-after.json
20:13:16  after-transformed-scope-fix/oracle-after.json
20:14:01  after-transformed-scope-fix/click-8.4.1-inspect.json
20:14:59  after-transformed-scope-fix/pydantic-2.13.4-inspect.json
20:15:03  after-transformed-scope-fix/requests-2.34.2-inspect.json
20:18:03  38e3d1d committed (design-and-prediction.md)
20:22:48  after-transformed-scope-fix/gates.log
20:23:12  8acde01 committed (the code)
20:24:13  9510863 committed (after-observation.md)
```

Every measurement file's mtime is **before** `38e3d1d`'s commit time. A
file's mtime cannot postdate the commit that later included it, so these
numbers existed, on disk, before the "precommitted" prediction was
committed. `design-and-prediction.md`'s "Precommitted prediction" section
was written with the actual answer already known -- not a blind
prediction, whatever its commit-order relative to `8acde01` (correction-2
\S8's check was real and correctly answered, just not the question that
mattered).

**Consequence, stated plainly**: this stage's first genuinely blind
prediction -- committed before the measurement it predicts was ever run
-- is `f811daa` (`correction-1/prediction.md`), verified in-session: the
prediction was committed, then the audit/corpus/oracle scorers were run
as the immediately following tool calls in this same session, with no
opportunity for the numbers to have been known first. Every prediction
in this stage before `f811daa` should be read as a precommitted-format
CONFIRMATION exercise, not a blind test -- which does not invalidate the
underlying measurements (they were still run, still real, still show
0 unsound cells throughout), but it does mean `design-and-prediction.md`'s
own "if a different count... that is a stop signal" framing never had
real teeth, since the count was already known when that framing was
written.

## 2. `correction-2`'s hand-sample claims were still wrong; regenerated programmatically

Three problems in `correction-2/hand-sample-excerpts.md`, found on review:

- `test_datetime:137` (entry #16) was listed with no excerpt ("same shape
  as #7, a different parametrize row") -- never actually read. The real
  count was **19 of 20**, not 20.
- `correction-2` \S5 says "the remaining 8" but lists seven site names.
- The file's own header claimed the 20-site sample was "selected ...
  from all 347 (post-padding-fix)" -- false. `hand_sample.json` (still
  the file both correction rounds draw from) was generated in
  `correction-1`, drawn from the unpadded, incomplete 309-claim
  population, before the padding bug was found. The 38 claims recovered
  by `correction-2`'s fix had **no call-site read at all** in either
  correction-1 or correction-2, despite both documents implying full
  coverage.

Fixed by generating the excerpt file with a script instead of hand-typing
individual entries -- [generate_excerpts.py](generate_excerpts.py),
[hand-sample-excerpts.md](hand-sample-excerpts.md) (this directory):
re-matches all 20 original-sample sites to their corrected claim records
and dumps source rows N-3..N+3 for each (Group 1, **20 of 20**, confirmed
by the script's own count, not asserted), plus one call site for each of
the **8** distinct `(file, name)` targets the padding bug had dropped
from every prior check (Group 2, **8 of 8**) -- the exact count
`correction-2` \S1 predicted (1 click + 6 pydantic + 1 requests). Every
excerpt is read directly from the extracted package source by the script,
not paraphrased or summarized by hand, closing the recurring pattern of
overclaiming a full read that this whole review chain kept finding.

## 3. Small fixes

- `correction-1` \S1 said "26 of 261" unresolved pydantic claims out of
  the unpadded total; the correct total (from `common.py`'s padded,
  complete count) is **235**, so the figure is "26 of 235." The 26 count
  itself was already correct.
- `_calculate_keys` (correction-1 \S7's traced revert of
  `copy_internals.py::_iter`): checked directly against the padded
  lost-names diff (`correction-2` \S3) -- **not present** in the
  `after-operator-claim-fix` Must set at all. It was never Must in the
  operator-claim-fix round, so its current revert is not a lost false
  Must; there is no prior-round soundness defect here, just the
  string-literal guard correctly (if coarsely) doing its job on a name
  that was never falsely claimed in the first place.

## 4. Two checks required before DONE, both run clean

**Labels frozen before any scoring.** `audit-sites-labeled.json` was
committed in `5772bc2` (2026-09-23 14:47:54). The first
`audit-scored-results.json` for Python was committed in `421bfff`
(2026-09-23 15:00:02) -- **~12 minutes after** the labels, not before.
The ground-truth labels genuinely predate the first time Girder's actual
answer was compared against them. This is the check that makes the
`nonempty_must_precision_1000_on_real_repository` criterion trustworthy
at all; it holds.

**No code has changed since the last gated commit.** `git diff --stat
86fd130 HEAD -- . ':(exclude)docs'` is **empty** -- every commit since
the stage's own final code change (`86fd130`, gated in
`correction-1/gates.log`, `ALL_GATES_PASSED`) has touched only `docs/`.
The binary measured throughout this whole correction chain
(`f87d1d05...`) is still the one built from the still-gated `86fd130`;
no re-gating is needed before declaring DONE.

## Conclusion

Both required checks pass. \S1's finding (the prediction wasn't
genuinely blind before `f811daa`) does not change any measured number or
either soundness check -- it is a correction to how much epistemic weight
`design-and-prediction.md`'s own "stop signal" framing deserves, recorded
honestly rather than left implied. The hand-sample is now complete and
regenerated programmatically (20/20 plus 8/8), closing the specific
recurring overclaim pattern (claiming a full read that was partial) this
whole review chain kept finding and re-finding across `after-observation.md`,
`correction-1`, and `correction-2`. Stage 3 Python's three criterion legs
remain Met, on the final binary `f87d1d058bb91418e817af35efb4956096a34e486ea39d7346a17477bb50d96f`
(commit `86fd130`), gated in `correction-1/gates.log`. Whether the roadmap
should mark this stage DONE now moves to the roadmap checkpoint itself.
