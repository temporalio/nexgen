# Go WIT samples

WIT-generated packages live in directories such as `protobackedshowcase/` and
`workflowservice/`. Hand-written converters live in [`support/`](support/) for
ordinary Nexus samples and [`systemsupport/`](systemsupport/) for System Nexus
samples. Generated facades live alongside their implementations in directories
such as `workflowservice_facade/` and `genericmodels_facade/`.

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
  --targets workflow --output advanced/samples/go/protobackedshowcase
```

The module path is under `go.temporal.io/sdk`, which permits importing the SDK's
internal packages. It does not make sample packages part of the SDK packages.
The System Nexus sample WIT omits a Go namespace override, so generated packages
take their names from their output directories and import the SDK's internal
APIs. The generated code imports authored converters from `systemsupport/`. For
SDK embedding, add `@nexus.namespace go="go.temporal.io/sdk/internal"` to the
WIT and use the SDK's internal package as both the implementation destination
and the support package.

The `genericmodels_facade/` package additionally exercises ordinary native APIs
with generic models. Its native tests call the facade instead of duplicating the
implementation tests.

A project outside the SDK module tree cannot copy the internal imports
unchanged. Adapt those conversions using APIs available to your module. For
`@nexus.source`, write the helper call without the package prefix. The sample
uses `go="WorkflowNamespace(ctx)"`, which is called as
`support.WorkflowNamespace(ctx)`. Run `go test ./...` from this sample module
after generation. See [the WIT guide](../../../GUIDE.md#go) for more detail.
JSON Schema generation does not need user support.
