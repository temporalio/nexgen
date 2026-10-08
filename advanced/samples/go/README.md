# Go WIT samples

WIT-generated packages live in directories such as `typeroundtrip/` and
`workflowservice/`. The hand-written converter package lives separately in
[`support/`](support/), outside those regenerated directories.

## Using the support package

In your own project, create a **distinct Go package outside the directory
passed to `--output`**. Pass the package's **full import path** (from its Go
module and package directory), not a `.go` filename, as `--support-package`.
Generated code imports it as `support`; its converters must be exported and
accept/return the types used by generated calls such as
`support.DurationToProto(ctx, value)`. Implement both conversion directions.
An explicit `go-from`/`go-to` name in WIT is used exactly as spelled, so it
also must be exported. Do not import the generated package from support or
you will create an import cycle.

For example, this repository's [`go.mod`](go.mod) and [`support/`](support/)
make the import path `go.temporal.io/sdk/advanced/samples/go/support`. From the
repository root, generate its `type-roundtrip` example with:

```sh
cargo run --features advanced -- go advanced/samples/inputs/type-roundtrip.wit \
  advanced/samples/inputs/deps \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --support-package go.temporal.io/sdk/advanced/samples/go/support \
  --native-api --output advanced/samples/go/typeroundtrip
```

The sample support source imports
`go.temporal.io/sdk/internal` because it lives under the SDK module tree; a
project in another module cannot copy that import unchanged. Adapt those
conversions using APIs available to your module. For `@nexus.source`, write
the helper call without the package prefix. The sample uses
`go="WorkflowNamespace(ctx)"`; generated code calls
`support.WorkflowNamespace(ctx)`. The support helper calls
`workflow.GetInfo(ctx).Namespace`. Run `go test ./...` from this sample module
after generation. Run the same command from your own module root. See
[the WIT guide](../../../GUIDE.md#go) for more detail. JSON Schema generation
does not need user support.
