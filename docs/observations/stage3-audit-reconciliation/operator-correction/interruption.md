# Measurement interrupted by requested pause

At 2026-10-02 01:54:58 UTC the user requested a three-hour pause. The active
audit process and its analyzer child were interrupted with SIGINT. The runner's
finally block preserved `run.json`; KeyboardInterrupt bypassed its Exception
handler, so that original file has no final status. It is an interrupted run,
not an overall success. Its terminal traceback is preserved separately.

Rust completed before the pause: 100 actual sites, 61 exact, 39 conservative,
zero unsound cells, Must precision 1/1. Both earlier omissions now have Unknown
claims. Click analysis and inspection also completed; Pydantic analysis was
interrupted, and Requests had not started. No Python aggregate result exists
for this attempt. No command lacking an exit status is counted as completed.

On resumption, verify the same candidate binary, frozen labels/scorers, source
archives, and the completed Click extract's hash. Retain every file here without
replacement. Finish the remaining Python analysis in `../operator-resumption/`
using `python3 -m tools.resume_operator_audit`. Keep Rust's completed scores;
do not rerun the already-passing common gates on unchanged product code.
