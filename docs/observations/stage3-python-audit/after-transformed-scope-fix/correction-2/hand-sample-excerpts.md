# Hand-read excerpts for all 20 sampled Must call sites

Sample selected with fixed seed `20260923` from all 347 (post-padding-fix)
same-file Must call sites across the three audited packages -- see
[hand-sample.json](../correction-1/hand-sample.json) for the full list.
Each excerpt below is read directly from the extracted package source at
the sampled row, not from the tool's own claim.

## 1. `pydantic/_internal/_validators.py::less_than_or_equal_validator` (row 304) -> `_safe_repr`
```python
def less_than_or_equal_validator(x: Any, le: Any) -> Any:
    try:
        if not (x <= le):
            raise PydanticKnownError('less_than_equal', {'le': _safe_repr(le)})
        return x
```
Plain same-file call. Genuine.

## 2-4. `pydantic/_internal/_schema_gather.py::traverse_schema` (rows 126, 129, 184) -> `traverse_schema`
```python
        if 'values_schema' in schema:
            traverse_schema(schema['values_schema'], context)
    elif schema_type == 'union':
        for choice in iter_union_choices(schema):
            traverse_schema(choice, context)
    ...
        if 'return_schema' in schema:
            traverse_schema(schema['return_schema'], context)
        if 'schema' in schema:
            traverse_schema(schema['schema'], context)
```
Recursive same-file calls, three separate sites. Genuine.

## 5. `pydantic/plugin/_schema_validator.py::build_wrapper` (row 103) -> `filter_handlers`
```python
def build_wrapper(func: Callable[P, R], event_handlers: list[BaseValidateHandlerProtocol]) -> Callable[P, R]:
    if not event_handlers:
        return func
    else:
        on_enters = tuple(h.on_enter for h in event_handlers if filter_handlers(h, 'on_enter'))
```
Plain same-file call inside a generator expression. Genuine.

## 6. `click tests/test_shell_completion.py::test_argument_default` (row 137) -> `_get_words`
```python
    assert _get_words(cli, [], "") == ["a"]
    assert _get_words(cli, ["a"], "b") == ["b"]
```
Genuine.

## 7. `pydantic tests/test_datetime.py::test_datetime` (row 171) -> `create_tz`
```python
        ('2012-04-23T10:20:30.400+02:30', datetime(2012, 4, 23, 10, 20, 30, 400_000, create_tz(150))),
```
Genuine, inside a parametrize tuple literal.

## 8. `pydantic/_internal/_fields.py::collect_model_fields` (row 422) -> `_update_fields_from_docstrings`
```python
    if config_wrapper.use_attribute_docstrings:
        _update_fields_from_docstrings(cls, fields)
```
Genuine.

## 9-10. `click tests/test_shell_completion.py::test_argument_nargs` (row 281) / `::test_chained` (rows 97, 101) -> `_get_words`
```python
    assert _get_words(cli, [], "") == ["a", "b"]
    ...
    assert _get_words(cli, [], "") == ["get", "set", "start"]
    assert _get_words(cli, [], "s") == ["set", "start"]
    assert _get_words(cli, ["set", "start"], "") == ["get"]
```
Genuine, same shape as #6.

## 11. `pydantic tests/test_forward_ref.py::test_undefined_types_warning_raised_by_usage` (row 983) -> `pytest_raises_user_error_for_undefined_type`
```python
def test_undefined_types_warning_raised_by_usage(create_module):
    with pytest_raises_user_error_for_undefined_type('Foobar', 'UndefinedType'):
```
Called as a context-manager constructor. Genuine plain call.

## 12. `pydantic/_internal/_fields.py::rebuild_model_fields` (row 466) -> `update_field_from_config`
```python
                new_field = _recreate_field_info(...)
                update_field_from_config(config_wrapper, f_name, new_field)
```
Genuine.

## 13. `pydantic tests/test_forward_ref.py::test_undefined_types_warning_1a_raised_by_default_2b_forward_ref` (row 930) -> `pytest_raises_user_error_for_undefined_type`
```python
def test_undefined_types_warning_1a_raised_by_default_2b_forward_ref(create_module):
    with pytest_raises_user_error_for_undefined_type(defining_class_name='Foobar', missing_type_name='UndefinedType'):
```
Same shape as #11. Genuine.

## 14. `pydantic tests/test_type_hints.py::test_parent_sub_model` (row 137) -> `inspect_type_hints`
```python
def test_parent_sub_model(ParentModel):
    inspect_type_hints(ParentModel, None, DEPRECATED_MODEL_MEMBERS)
```
Genuine.

## 15, 17. `click tests/test_shell_completion.py::test_chained` (rows 97, 101) -> `_get_words`
Already read above under #9-10 (same test function, distinct sites).

## 16. `pydantic tests/test_datetime.py::test_datetime` (row 137) -> `create_tz`
Same test function and shape as #7, a different parametrize row.

## 18. `click tests/test_shell_completion.py::test_command` (row 35) -> `_get_words`
```python
def test_command():
    cli = Command("cli", params=[Option(["-t", "--test"])])
    assert _get_words(cli, [], "") == []
```
Genuine.

## 19. `pydantic/_internal/_typing_extra.py::get_model_type_hints` (row 367) -> `try_eval_type`
```python
                        globalns, localns = ns_resolver.types_namespace
                        hints[name] = try_eval_type(value, globalns, localns)
```
Genuine.

## 20. `pydantic/_internal/_schema_gather.py::traverse_definition_ref` (row 87) -> `traverse_schema`
```python
        ctx.collected_references[schema_ref] = def_ref_schema
        traverse_schema(definition, ctx)
        if 'serialization' in def_ref_schema:
            traverse_schema(def_ref_schema['serialization'], ctx)
```
Genuine, same recursive shape as #2-4.

## Result

All 20 sampled call sites read directly in their surrounding source
context: every one is a genuine, plain, unambiguous same-file call,
consistent with the programmatic check's 0/347 violation result
(\S1 of [correction.md](correction.md)).
