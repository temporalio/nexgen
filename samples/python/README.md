# Python JSON-Schema Samples

`uv`-managed Python 3.10+ sample suite for the JSON-Schema **definitions**
generator outputs.

- Authored JSON-Schema inputs live in `samples/schemas/`
- Checked-in generated definition packages live in `samples/python/<name>/`
  (`chat`, `kb`, `showcase`, `temporal`), where models are exposed as the public
  `models` module and re-exported from the package root
- Canonical wire fixtures live in `samples/wire/json_schema/`
- Pytest round-trip tests live in `samples/python/tests/`
- `cargo test` validates the checked-in packages and does not rebuild them

Top-level rebuild command:

```bash
cargo build-examples --format json-schema --lang python
```

Current workflow:

```bash
cargo build-examples --format json-schema --lang python
cd samples/python
uv run pytest
uv run basedpyright
```

The Workflow API variant of these schemas (services + clients) lives under
`advanced/samples/python/json_schema/api/`.
