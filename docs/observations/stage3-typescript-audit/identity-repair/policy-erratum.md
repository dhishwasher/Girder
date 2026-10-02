# Identity policy file-location correction

Policy `69b4c7f` calls the files containing audit indices 23 and 91 "both real
compiler files." That repository description is incorrect. The unchanged
zero-based indices in the frozen combined labels locate:

- 23: `date-fns-4.1.0/src/intlFormatDistance/test.ts:109`.
- 91: `typescript-6.0.3/src/testRunner/unittests/tsserver/projectReferences.ts:1189`.

The original collision report named these same indices. Measure both files
from their pinned snapshots; do not substitute a second compiler file. No site,
label, expected answer, identity criterion, or test threshold changes. The
original policy remains present so the mistaken description is visible.
