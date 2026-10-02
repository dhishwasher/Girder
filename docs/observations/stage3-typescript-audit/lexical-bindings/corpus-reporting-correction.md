# Failure-language reporting correction

The [original dispatch observation](corpus-after-1.json) remains unchanged.
Its unresolved `typescript-structural-object-literal` row lacked `language`,
and the aggregator skipped case failures before counting by language.

The scorer now keeps the declared language for all three case-failure exits
(missing origin, ambiguous origin, selection failure), and the matrix counts
that failure under its language. Historical rows that still lack language are
counted explicitly as `unattributed_failed_cases`; no language is guessed.
[Fourteen unit tests passed](corpus-reporting-tests-1.json), covering those exits,
legacy rows, and unchanged precision/recall accounting for scored tests.

The [corrected report](corpus-after-1-reporting-correction.json) was derived from
the existing observation without executing Girder again. Exactly one metadata
field was supplied from the unchanged corpus by exact case ID: the failed case's
language is TypeScript. The corrected matrix therefore reports **one TypeScript
failure**, with zero unattributed failures. The pooled matrix, all 49 case
statuses, every observed test class, and all precision/recall values are unchanged.
The structural origin is still unresolved; this is no product improvement.

Reproduction of the matrix, using the retained original observation:

```python
import copy, json
from pathlib import Path
from tools.dispatch_corpus_scorer import confusion_matrix

root = Path('docs/observations/stage3-typescript-audit/lexical-bindings')
original = json.loads((root / 'corpus-after-1.json').read_text())
cases = json.loads(Path('docs/dispatch-corpus.json').read_text())['cases']
languages = {case['id']: case['language'] for case in cases}
rows = copy.deepcopy(original['results'])
for row in rows:
    if 'language' not in row:
        assert row['status'] == 'failed'
        row['language'] = languages[row['id']]
matrix = confusion_matrix(rows)
```

Input/output identities and the assertion that zero observed test classes changed
are stored in the corrected artifact. Previous observations and failures were
neither rewritten nor softened.
