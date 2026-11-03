# Go WIT samples

WIT-generated packages live in directories such as `protobackedshowcase/` and
`workflowservice/`. Hand-written converters live in [`support/`](support/) for
ordinary Nexus samples. System Nexus implementations and their authored support
live together in [`internal/`](internal/); `workflowservice/` and
`multioperationservice/` are generated facades.

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
[`internal/sdk_api.go`](internal/sdk_api.go) supplies aliases and wrappers for
the existing SDK declarations; the same package contains authored support and
prefixed generated System Nexus declarations. Generation preserves these
authored files. The Go namespace and support package are both configured as
`go.temporal.io/sdk/internal`, so generated references and helper calls are
unqualified. Facades import the sample implementation's actual path,
`go.temporal.io/sdk/advanced/samples/go/internal`.

The `facade/` packages additionally exercise ordinary native APIs, including
generic models, variants, resource constructors and methods, and function
parameters. Their native tests call the facades instead of duplicating the
implementation tests.

A project outside the SDK module tree cannot copy the internal imports
unchanged. Adapt those conversions using APIs available to your module. For
`@nexus.source`, write the helper call without the package prefix. The System
Nexus sample uses `go="WorkflowNamespace(ctx)"`, which is called locally; an
ordinary imported support package is called as `support.WorkflowNamespace(ctx)`.
Run `go test ./...` from this sample module after generation. See
[the WIT guide](../../../GUIDE.md#go) for more detail. JSON Schema generation
does not need user support.
