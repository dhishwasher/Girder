# Dispatch corpus after the lexical-binding repair

Product candidate `5733be6`; binary SHA-256
`fe6901793bb8055a66359ad235e631219ba87b3f6ab96a396e4741925a7deaef`.
[Commands and status](corpus-after-1-run.json),
[frozen input identities](corpus-after-1-inputs.json), and
[all case results](corpus-after-1.json) are retained. The serial run completed
in 34.016 seconds with exit zero; that exit means scoring completed, not that
every case passed.

All 49 cases were attempted. The complete published confusion matrix matches
the prior operator-correction observation: 22 exact test cells, 34 conservative
cells, zero unsafe exclusions or overclaims, and **one failed case**.
`typescript-structural-object-literal` still cannot resolve origin `name`.
Neither the failure nor its expectations were removed.

Rust remains 5 exact / 9 conservative, Python 7 / 6, TypeScript 5 / 10,
and Go 5 / 9 among scored test cells. There is **no dispatch-corpus improvement
from this repair**. The separate real-repository audit improved, but cannot be
substituted for this criterion. TypeScript remains IN PROGRESS; Go NOT STARTED.

## Reporting defect discovered in this run

The failed result row has `id`, `status`, and `reason`, but no `language`.
Consequently the pooled matrix records the failure while the per-language
TypeScript matrix reports `failed: 0`. The failed case is a TypeScript case;
that zero is misleading. It is already present in the earlier observation,
whose matrix is unchanged here. The raw records above are preserved verbatim.

Next: fix failure-language propagation and publish a corrected summary derived
from these unchanged case results. Do not rerun Girder or alter any observed
class to repair a reporting field. Then continue the unresolved structural
origin and remaining dispatch-proof work; common language gates remain due.
