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

## Using a support namespace

In your own project, keep hand-written `.cs` files **outside the directory
passed to `--output`**. Give them a namespace and pass that **namespace** as
`--support-package`, not a filename or assembly path. Compile the authored
support sources **in the same project/assembly** as the generated files so
their `internal` helpers and extension methods remain accessible. A separate
referenced DLL is not equivalent to compiling them together.

For example, this repository keeps support sources in
[`TemporalSupport/`](TemporalSupport/), outside generated `wit/<example>/`
directories. They declare `namespace Nexgen.Support`:

```sh
cargo run --features advanced -- dotnet advanced/samples/inputs/type-roundtrip.wit \
  advanced/samples/inputs/deps \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --support-package Nexgen.Support \
  --native-api --output advanced/samples/dotnet/wit/type-roundtrip
```

If your project disables default compile items or excludes the support
directory, explicitly include those authored sources. For example,
[`Nexgen.DotNetMultiOperationService.csproj`](Nexgen.DotNetMultiOperationService.csproj)
uses `<Compile Include="TemporalSupport/*.cs" />`. Run
`dotnet build Nexgen.DotNetExamples.csproj` and `dotnet test tests/` from this
directory to check this sample; build and test your own project similarly. See
[the WIT guide](../../../GUIDE.md#net) for the required helper contract.
