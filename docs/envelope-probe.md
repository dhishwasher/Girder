# Envelope capability probe

Gap #12 recorded that local plan authoring failed with 34 of 39 attempts
dying on plan-envelope shape errors. This probe tests whether that was a
property of the task or of the model.

Method: `tools/envelope_probe.py <model>` sends one schema-constrained
`/api/chat` request per seed (42+i, temperature 0), asking for a minimal
plan_version 2 envelope with one step and one `replace_node` edit whose
Python body is supplied verbatim in the prompt. Three seeds per model.

## Result, 2026-08-13

| Model | Size | Valid |
|---|---:|---:|
| tinyllama:1.1b | 637 MB | 0/3 |
| qwen2.5-coder:1.5b | 986 MB | 3/3 |

qwen2.5-coder:1.5b produced byte-identical correct output on all three
seeds and ran within the Crostini VM's 2.3 GB available memory.
qwen2.5-coder:3b (~1.9 GB) could not be evaluated: loading it wedged the VM.

## What this does and does not show

It shows the envelope-shape blocker in gap #12 was model selection, not an
intrinsic limit of graph-addressed authoring. It does not show the model can
author a plan for a real task: the probe supplied the edit content verbatim,
so only envelope emission was tested.

One observation with direct bearing on the authoring harness: the model
emitted `f'hi {name}'` where the prompt used double quotes. That is
semantically identical Python that the harness's byte-exact expected-tree
comparison would score as a failure.
