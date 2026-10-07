# Stage 6 run 1 is INVALID (published, not deleted)

All 20 campaigns completed with exit 0 on the Girder 0.4.0 release binary, but the revision 12 Girder adapter
stamped every Girder record `version 0.2.6`, `commit 3f7d5ee` from a class constant and never checked the binary
it launched. The report therefore labels 0.4.0 results as 0.2.6. No delta or claim is drawn from this run.
The external-runner rows are unaffected in substance but are also not used: run 2 reruns everything on
policy revision 13. Raw outputs remain on the drive under `runs/stage6-run1/`. Fix: policy revision 13
(see `revision_history` in `policy.json`).
