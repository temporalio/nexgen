# Python WIT Examples

`uv`-managed Python 3.10+ example suite for the WIT generator outputs, plus the
snapshot-only JSON-Schema **native-api** outputs.

- Authored WIT inputs live in `advanced/samples/inputs/*.wit`
- Checked-in generated packages live in `advanced/samples/python/wit/<name>/`,
  where `<name>` is the snake_case WIT input name
- Authored Temporal converters live in the shared `temporal_support/` package;
  generated WIT modules import that package as `_support` rather than copying
  it, and generated models are exposed as the public `models` module
- JSON-Schema native-api outputs (services + clients) live under
  `advanced/samples/python/json_schema/api/` and are snapshot-tested only
- Proto wire-compatibility fixtures live in `advanced/samples/wire/proto/`
- Pytest files (WIT round-trips + proto wire compatibility) live in
  `advanced/samples/python/tests/`
- `cargo test` validates the checked-in generated packages and does not rebuild them

Top-level rebuild command:

```bash
cargo build-examples --lang python
```

Current workflow:

```bash
cargo build-examples --lang python
cd advanced/samples/python
uv run pytest
uv run basedpyright
```

The beginner-facing JSON-Schema definitions samples live under `samples/python/`.

## Using a support package

In your own project, put hand-written helpers in a Python package **outside
the directory passed to `--output`**. Make that package importable by the
application and export the functions that generated code calls. Pass its
**Python module name**, not a source filename, as `--support-package`.
Generated modules use `import <module> as _support`; if generated code calls
`_support.retry_policy_to_proto(...)`, that function must be exported at the
package's top level with the expected signature. Regenerating output must
never delete the authored package.

For example, this repository puts generated packages under `wit/<name>/`
and keeps its authored converters in
[`temporal_support/`](temporal_support/). Its
[`__init__.py`](temporal_support/__init__.py) exposes the functions called by
generated code. From the repository root, the corresponding command is:

```sh
cargo run --features advanced -- python advanced/samples/inputs/type-roundtrip.wit \
  advanced/samples/inputs/deps \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --support-package temporal_support \
  --native-api --output advanced/samples/python/wit/type_roundtrip
```

To make your package importable, install it or put its parent directory on
Python's import path. In this sample, pytest uses `pythonpath = ["."]` in
[`pyproject.toml`](pyproject.toml). From `advanced/samples/python`, check this
sample with `uv run python -c 'import temporal_support'`, then run
`uv run ruff check .`, `uv run basedpyright`, and `uv run pytest`. In your own
project, check imports and generated code with its type checker and tests. See
[the WIT guide's Python instructions](../../../GUIDE.md#python) for sourced
expressions and default converter names. JSON Schema generation does not need
this user-owned support package.
