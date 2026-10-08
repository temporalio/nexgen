# Python WIT Examples

`uv`-managed Python 3.10+ example suite for the WIT generator outputs, plus the
snapshot-only JSON-Schema **native-api** outputs.

- Authored WIT inputs live in `advanced/samples/inputs/*.wit`
- Checked-in generated packages live in `advanced/samples/python/wit/<name>/`,
  where `<name>` is the snake_case WIT input name
- Authored Temporal converters live in the shared `temporal_support/` package;
  generated helpers remain under each package's private `_support/`, and
  generated models are exposed as the public `models` module
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
