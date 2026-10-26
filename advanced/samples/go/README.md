# Go WIT samples

WIT-generated packages live in directories such as `protobackedshowcase/` and
`workflowservice/`. Hand-written converters live in [`support/`](support/) for
ordinary Nexus samples and [`systemsupport/`](systemsupport/) for system Nexus
samples, outside the regenerated directories.

## Using the support package

In your own project, create a
**distinct Go package outside the directory passed to `--output`**. Pass the
package's **full import path** (from its Go module and package directory), not a
`.go` filename, as `--support-package`. Generated code imports it as `support`;
its converters must be exported and accept/return the types used by generated
calls such as `support.DurationToProto(value)`. Implement both conversion
directions. Ordinary protobuf-backed models use the public transfer converter
API; their conversion callbacks receive no data converter. Converter-dependent
sample support, such as failure conversion, uses a fixed default encoding
policy. With `--system-nexus`, generated callbacks receive the SDK's system
Nexus configuration and pass its data converter to support functions, such as
`support.DurationToProto(cfg.DataConverter, value)`. An explicit
`go-from`/`go-to` name in WIT is used exactly as spelled, so it also must be
exported. Do not import the generated package from support or you will create an
import cycle.

For example, this repository's [`go.mod`](go.mod) and [`support/`](support/)
make the import path `go.temporal.io/sdk/advanced/samples/go/support`. From the
repository root, generate its `proto-backed-showcase` example with:

```sh
cargo run --features advanced -- go advanced/samples/inputs/proto-backed-showcase.wit \
  advanced/samples/inputs/deps \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --support-package go.temporal.io/sdk/advanced/samples/go/support \
  --native-api --output advanced/samples/go/protobackedshowcase
```

The system Nexus sample support source imports `go.temporal.io/sdk/internal`
because it lives under the SDK module tree; a project in another module cannot
copy that import unchanged. Adapt those conversions using APIs available to your
module. For `@nexus.source`, write the helper call without the package prefix.
The sample uses `go="WorkflowNamespace(ctx)"`; generated code calls
`support.WorkflowNamespace(ctx)`. The support helper calls
`workflow.GetInfo(ctx).Namespace`. Run `go test ./...` from this sample module
after generation. Run the same command from your own module root. See
[the WIT guide](../../../GUIDE.md#go) for more detail. JSON Schema generation
does not need user support.
