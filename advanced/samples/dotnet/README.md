# .NET advanced samples

Generated C# for the WIT inputs in [`../inputs/`](../inputs/), plus the
snapshot-only native-api form of the JSON-Schema inputs.

- `TemporalSupport/` — authored converters and workflow-service helpers shared
  by the generated projects.
- `wit/` — generated output per WIT example (`workflow-service`,
  `user-service`, `type-showcase`, `type-roundtrip`, `start-workflow`,
  `function-execution`): models, operations, and Nexus service interfaces.
- `json_schema/api/` — native-api (service + client) output for the
  JSON-Schema inputs. Snapshot-only: regenerated and diffed by the Rust tests,
  not exercised by runtime tests here.
- `tests/` — endpoint runtime checks (`WorkflowService`, `UserService`), the
  generated-api compile check, and proto-wire compatibility checks against the
  fixtures in [`../wire/proto/`](../wire/proto/).

Regenerate WIT output with `cargo build-examples --format wit --lang dotnet` from the repo
root.

```bash
dotnet build Nexgen.DotNetExamples.csproj
dotnet test tests/
```

For the beginner-facing JSON-Schema definitions models, see
[`../../../samples/dotnet/`](../../../samples/dotnet/).
