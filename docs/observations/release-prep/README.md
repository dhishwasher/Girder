# README demo capture: provenance

Two real command outputs, captured against `demo-project/` with the debug
binary (`girder 0.2.7`, HEAD `6693c0a17df4335a69074cbab66e46158175bb8f`),
used verbatim in `README.md`'s top section. Not hand-written.

## `context --source-only`

```
girder context demo-project --nodes crate::greeter::shout_greeting --json --source-only
```

```json
{
  "intent": "",
  "nodes": [
    {
      "language": "python",
      "path": "crate::greeter::shout_greeting",
      "source": "def shout_greeting(name):\n    return format_greeting(name).upper()"
    }
  ]
}
```

## `test-impact --quiet`

Captured in a disposable copy of `demo-project/` with a fresh `git init`,
an initial commit, then one real uncommitted edit to
`greeter.py::farewell`'s return string — the same edit used in
`docs/observations/ci-gate-design/`. No license key set.

```
girder test-impact . --quiet
```

```
test-impact: 82 unresolved call-evidence boundaries found; this selection is the conservative must∪may∪unknown union, not a targeted answer — pass --classified to see why each test is included (docs/call-classification-policy.md)
test_groups_files_with_identical_content
test_returns_nothing_when_all_files_differ
test_same_content_hashes_the_same
test_farewell
test_format_greeting
test_shout_greeting_delegates_to_format_greeting
```
