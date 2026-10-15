# .NET JSON-Schema samples

Generated C# for the JSON-Schema inputs in [`../schemas/`](../schemas/), in
**definitions** mode — plain data models (records + `System.Text.Json`
converters) with no service/endpoint scaffolding.

- `chat/`, `kb/` — generated models, one directory per schema.
- `tests/` — round-trip checks that serialize the shared wire fixtures in
  [`../wire/json_schema/`](../wire/json_schema/) through `System.Text.Json` and
  assert JSON-equality.

Regenerate with `cargo build-examples --format json-schema --lang dotnet` from
the repo root.

```bash
dotnet build Nexgen.DotNetExamples.csproj   # compile the models
dotnet test tests/                                # round-trip the fixtures
```

For the native-api (service/client) form of these same schemas and the WIT-based
examples, see
[`../../advanced/samples/dotnet/`](../../advanced/samples/dotnet/).
