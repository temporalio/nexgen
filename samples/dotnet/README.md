# .NET JSON-Schema samples

Generated C# for the JSON-Schema inputs in [`../schemas/`](../schemas/), in
**definitions** mode — models (records, each with a validating
`System.Text.Json` converter) and Nexus service interfaces, with no client
scaffolding.

- `chat/`, `kb/`, `showcase/`, `temporal/` — generated output, one directory per
  schema, each in its own `Nexgen.JsonSchema.<Example>` namespace.
- `tests/` — round-trips every shared wire fixture in
  [`../wire/json_schema/`](../wire/json_schema/) through Temporal's default data
  converter and asserts JSON-value equality, plus rejection and
  validate-before-write checks. `JsonSchemaKbNexusChecks.cs` implements the
  generated `IKnowledgeBaseService` and calls every operation from a workflow
  over a real Nexus endpoint on a local Temporal dev server: `GetPage` and
  `PutBlock` synchronously, and `GetCategoryTree` asynchronously, through a
  workflow that loads the tree in an activity. It also sends an invalid input
  the handler worker rejects as a non-retryable `BAD_REQUEST`.

Regenerate with `cargo build-examples --format json-schema --lang dotnet` from
the repo root.

```bash
dotnet build Nexgen.DotNetExamples.csproj   # compile the models
dotnet test tests/                          # round-trip the fixtures
```

For the native-api (service/client) form of these same schemas and the WIT-based
examples, see
[`../../advanced/samples/dotnet/`](../../advanced/samples/dotnet/).
