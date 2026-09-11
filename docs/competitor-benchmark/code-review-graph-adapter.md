# code-review-graph adapter checkpoint

The code-review-graph 2.3.8 adapter is frozen before its first scored campaign. It runs the pinned package as a persistent stdio MCP server, builds with `build_or_update_graph_tool(full_rebuild=true, postprocess="minimal")`, and synchronously invokes the same tool with `full_rebuild=false` after each mutation. This checkpoint is preflight evidence, not a competitive result.

## Reproducible installation and placement

The binary-only, hash-checked Python 3.11 installation uses [the committed requirements lock](locks/code-review-graph-requirements.txt). The pinned project wheel has SHA-256 `013ae3c119cc7de337f9e88fe36daef82e2d4def942a014edcf97f126e208547` and the executable reports `code-review-graph 2.3.8`.

The first install failed because the lock wrote `PyJWT==2.13.0` while `mcp` requests its `crypto` extra. pip therefore considered an unhashed dependency candidate. Revision 9 retained the version and hash while spelling the requirement as `PyJWT[crypto]==2.13.0`. A fresh retry completed in 59.308 seconds with 214,331,392 bytes peak process-tree RSS and no compiler fallback. Both attempts are retained.

The first graph build placed SQLite data beside the fixture on the ChromeOS removable 9p mount and failed with `disk I/O error`. Setting `CRG_DATA_DIR` to a fresh mode-0700 local temporary directory resolved that filesystem limitation; the matching CLI build completed in 2.150 seconds with 52,195,328 bytes peak RSS. Measured campaigns use a new local `HOME` and `CRG_DATA_DIR` while leaving fixture copies and raw artifacts on the removable drive.

## Frozen native mapping

| Benchmark operation | Native code-review-graph operation | Projection |
| --- | --- | --- |
| Definition | `semantic_search_nodes_tool`, then `get_review_context_tool(include_source=true)` for the unique returned file | Exact-name function identities plus the native source snippet |
| Callers | `query_graph_tool(pattern="callers_of")` | All returned identities |
| Callees | `query_graph_tool(pattern="callees_of")` | All returned identities |
| Impact | Exact symbol search, then `get_impact_radius_tool(changed_files=[target_file], max_depth=64)` | Non-file `impacted_nodes`; the seed is in `changed_nodes` |
| Tests | `query_graph_tool(pattern="tests_for")` | All returned identities |

The adapter does not discard unresolved identities. The base callee response contains native builtin `sum` alongside the correct `core.py::normalize_value`; `sum` remains a false positive. The native `tests_for` response is empty on the tiny fixture. File-scoped impact returns the static caller and both test functions, while the function-parameter callback remains absent.

The final adapter preflight completed full preparation, all five base queries, a synchronous body-only update, and all five updated queries. Preparation took 5.412 seconds and peaked at 109,031,424 bytes RSS in that preflight. Definition source changed to the current body marker after the update. These values are excluded from scored results.

The [artifact manifest](code-review-graph-adapter-manifest.json) pins the retained install, placement-failure, interface, and adapter-response files under [preflight artifacts](preflight-artifacts/code-review-graph). It also retains the incomplete MCP initialize request that omitted required client fields; revision 10 supplies the standard `capabilities` and `clientInfo` fields for every later campaign.

## Reproduction

```sh
python3 -m venv /tmp/code-review-graph-benchmark-venv
/tmp/code-review-graph-benchmark-venv/bin/python -m pip install \
  --require-hashes --only-binary=:all: \
  -r docs/competitor-benchmark/locks/code-review-graph-requirements.txt

PYTHONPATH=. python3 -m tools.competitor_benchmark.campaign \
  --product code-review-graph \
  --fixture tiny-python \
  --binary /tmp/code-review-graph-benchmark-venv/bin/code-review-graph \
  --work-root /external/work/code-review-graph-tiny \
  --output /external/runs/code-review-graph-tiny
```

The campaign runner creates the local private graph directory itself, checks memory before startup and during every request, records every MCP response byte, and removes the temporary graph after cleanup.
