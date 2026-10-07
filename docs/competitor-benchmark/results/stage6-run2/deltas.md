# Stage 6 deltas against the published baseline

Baseline: results/modest-final (Girder 0.2.6 revision 12; externals revision 11). Run: results/stage6-run2 (policy revision 13). Unchanged metric cells: 188.

A *regression* is a lower precision/recall or a PASS that became non-PASS. External-runner differences are reported too; they are version-identical reruns, so they measure run-to-run variation, not product change.

## girder (0.4.0)

| fixture | metric | baseline | now | change |
|---|---|---|---|---|
| modest-go | tests_precision | 1.0 | 0.6666666666666666 | regression |
| modest-go | tests_recall | 0.5 | 1.0 | improvement |
| modest-python | tests_precision | 1.0 | 0.6666666666666666 | regression |
| modest-python | tests_recall | 0.5 | 1.0 | improvement |
| modest-rust | tests_precision | 1.0 | 0.6666666666666666 | regression |
| modest-rust | tests_recall | 0.5 | 1.0 | improvement |

## girder-watch (0.4.0)

| fixture | metric | baseline | now | change |
|---|---|---|---|---|
| modest-go | tests_precision | 1.0 | 0.6666666666666666 | regression |
| modest-go | tests_recall | 0.5 | 1.0 | improvement |
| modest-python | tests_precision | 1.0 | 0.6666666666666666 | regression |
| modest-python | tests_recall | 0.5 | 1.0 | improvement |
| modest-rust | tests_precision | 1.0 | 0.6666666666666666 | regression |
| modest-rust | tests_recall | 0.5 | 1.0 | improvement |

## ripwire (0.5.0)

No metric differs from the baseline.

## codebase-memory-mcp (0.10.8)

No metric differs from the baseline.

## code-review-graph (2.3.8)

No metric differs from the baseline.

