# Advanced samples

Advanced generation examples. Two flavors live here:

- **WIT-based generation** — each example starts from an authored WIT file in
  [`inputs/`](inputs/). Generated output is checked in for .NET, Go, Python, and
  TypeScript, with focused tests for the examples that exercise runtime or
  typechecking behavior.
- **JSON Schema native-api mode** — the service/client (`json_schema/api/`)
  outputs generated from the schemas in
  [`../../samples/schemas/`](../../samples/schemas/). These are snapshot-only
  (regenerated and diffed by `cargo test`); the runtime round-trip tests for
  JSON Schema live under [`../../samples/`](../../samples/) against the
  definitions.

For the beginner-friendly JSON Schema definitions samples, see
[`../../samples/`](../../samples/).

Inputs that intentionally fail compilation live in
[`invalid/`](invalid/README.md), with expected error kinds and diagnostic
fragments. They run as Rust integration tests rather than generated-language
samples.

To use hand-written support code with WIT generation, start with the
[support-package guide](../../GUIDE.md#integrating-a-support-package) and the
sample-project instructions for [Go](go/README.md), [Python](python/README.md),
[TypeScript](typescript/README.md), and [.NET](dotnet/README.md).

## WIT examples

### `function-execution`

- WIT: [`inputs/function-execution.wit`](inputs/function-execution.wit)
- .NET: [`dotnet/wit/function-execution/`](dotnet/wit/function-execution/)
- Go: [`go/functionexecution/`](go/functionexecution/)
- Python: [`python/wit/function_execution/`](python/wit/function_execution/)
- TypeScript:
  [`typescript/wit/function-execution/`](typescript/wit/function-execution/)

### `user-service`

- WIT: [`inputs/user-service.wit`](inputs/user-service.wit)
- .NET: [`dotnet/wit/user-service/`](dotnet/wit/user-service/)
- Go: [`go/userservice/`](go/userservice/),
  [`go/tests/userservice_test.go`](go/tests/userservice_test.go)
- Python: [`python/wit/user_service/`](python/wit/user_service/),
  [`python/tests/test_user_service.py`](python/tests/test_user_service.py)
- TypeScript: [`typescript/wit/user-service/`](typescript/wit/user-service/),
  [`typescript/tests/user-service.test.ts`](typescript/tests/user-service.test.ts)

### `type-showcase`

Includes flattened records that remain public because they are used as a
required input model or returned by an operation.

- WIT: [`inputs/type-showcase.wit`](inputs/type-showcase.wit)
- .NET: [`dotnet/wit/type-showcase/`](dotnet/wit/type-showcase/)
- Go: [`go/typeshowcase/`](go/typeshowcase/),
  [`go/tests/typeshowcase_test.go`](go/tests/typeshowcase_test.go)
- Python: [`python/wit/type_showcase/`](python/wit/type_showcase/),
  [`python/tests/test_type_showcase.py`](python/tests/test_type_showcase.py)
- TypeScript: [`typescript/wit/type-showcase/`](typescript/wit/type-showcase/),
  [`typescript/tests/type-showcase.test.ts`](typescript/tests/type-showcase.test.ts)

### `start-workflow`

- WIT: [`inputs/start-workflow.wit`](inputs/start-workflow.wit)
- .NET: [`dotnet/wit/start-workflow/`](dotnet/wit/start-workflow/)
- Go: [`go/startworkflow/`](go/startworkflow/)
- Python: [`python/wit/start_workflow/`](python/wit/start_workflow/),
  [`python/tests/test_start_workflow.py`](python/tests/test_start_workflow.py)
- TypeScript:
  [`typescript/wit/start-workflow/`](typescript/wit/start-workflow/),
  [`typescript/tests/start-workflow.test.ts`](typescript/tests/start-workflow.test.ts)

### `workflow-handle`

A System Nexus operation that returns a resource.

- WIT: [`inputs/workflow-handle.wit`](inputs/workflow-handle.wit)
- .NET: [`dotnet/wit/workflow-handle/`](dotnet/wit/workflow-handle/)
- Python: [`python/wit/workflow_handle/`](python/wit/workflow_handle/)
- TypeScript:
  [`typescript/wit/workflow-handle/`](typescript/wit/workflow-handle/)

### `workflow-service`

- WIT: [`inputs/workflow-service.wit`](inputs/workflow-service.wit)
- .NET: [`dotnet/wit/workflow-service/`](dotnet/wit/workflow-service/)
- Go: [`go/workflowservice/`](go/workflowservice/)
- Python: [`python/wit/workflow_service/`](python/wit/workflow_service/),
  [`python/tests/test_workflow_service.py`](python/tests/test_workflow_service.py)
- TypeScript:
  [`typescript/wit/workflow-service/`](typescript/wit/workflow-service/),
  [`typescript/tests/workflow-service.test.ts`](typescript/tests/workflow-service.test.ts)

### `proto-backed-showcase`

Protobuf-backed records, enums and defaults, byte lists, bare collections, and
sourced maps. Native tests exercise conversions through the generated APIs.

- WIT: [`inputs/proto-backed-showcase.wit`](inputs/proto-backed-showcase.wit)
- .NET: [`dotnet/wit/proto-backed-showcase/`](dotnet/wit/proto-backed-showcase/)
- Go: [`go/protobackedshowcase/`](go/protobackedshowcase/),
  [`go/tests/protobackedshowcase_test.go`](go/tests/protobackedshowcase_test.go)
- Python:
  [`python/wit/proto_backed_showcase/`](python/wit/proto_backed_showcase/),
  [`python/tests/test_proto_backed_showcase.py`](python/tests/test_proto_backed_showcase.py)
- TypeScript:
  [`typescript/wit/proto-backed-showcase/`](typescript/wit/proto-backed-showcase/),
  [`typescript/tests/proto-backed-showcase.test.ts`](typescript/tests/proto-backed-showcase.test.ts)

### `notification-service`

- WIT: [`inputs/notification-service.wit`](inputs/notification-service.wit)
- .NET: [`dotnet/wit/notification-service/`](dotnet/wit/notification-service/),
  [`dotnet/tests/NotificationServiceChecks.cs`](dotnet/tests/NotificationServiceChecks.cs)
- Python:
  [`python/wit/notification_service/`](python/wit/notification_service/),
  [`python/tests/test_notification_service.py`](python/tests/test_notification_service.py)
- TypeScript:
  [`typescript/wit/notification-service/`](typescript/wit/notification-service/),
  [`typescript/tests/notification-service.test.ts`](typescript/tests/notification-service.test.ts)

### `proto-oneof`

- WIT: [`inputs/proto-oneof.wit`](inputs/proto-oneof.wit)
- .NET: [`dotnet/wit/proto-oneof/`](dotnet/wit/proto-oneof/),
  [`dotnet/tests/ProtoOneofChecks.cs`](dotnet/tests/ProtoOneofChecks.cs)
- Python: [`python/wit/proto_oneof/`](python/wit/proto_oneof/),
  [`python/tests/test_proto_oneof.py`](python/tests/test_proto_oneof.py)
- TypeScript: [`typescript/wit/proto-oneof/`](typescript/wit/proto-oneof/),
  [`typescript/tests/proto-oneof.test.ts`](typescript/tests/proto-oneof.test.ts)

### `proto-generic`

- WIT: [`inputs/proto-generic.wit`](inputs/proto-generic.wit)
- .NET: [`dotnet/wit/proto-generic/`](dotnet/wit/proto-generic/),
  [`dotnet/tests/ProtoGenericChecks.cs`](dotnet/tests/ProtoGenericChecks.cs)
- Python: [`python/wit/proto_generic/`](python/wit/proto_generic/),
  [`python/tests/test_proto_generic.py`](python/tests/test_proto_generic.py)
- TypeScript: [`typescript/wit/proto-generic/`](typescript/wit/proto-generic/),
  [`typescript/tests/proto-generic.test.ts`](typescript/tests/proto-generic.test.ts)

### `stream-service`

The Stream service WIT, a nexgen input of the `temporalio-streams` crate. Its
records bind to the Stream protos of the API proposal (`temporalio/api#895` at
`44519d8a`), in
[`descriptors/streams/`](descriptors/streams/).

The sample is a System Nexus service, as in the crate. Its .NET output takes the
`Temporalio.Workflows` namespace, which `workflow-service` already has in the
main .NET project. So it builds in its own project,
[`dotnet/Nexgen.DotNetStreamService.csproj`](dotnet/Nexgen.DotNetStreamService.csproj),
and the tests refer to it through an alias. The header's proto paths are the
crate's.

nexgen can't generate all of the crate's WIT yet, so the sample takes the
nearest form that it can generate. For example, it has no `@nexus.source`,
`body-t` is a `payload`, and `Stream` has no type parameter. A field that the
crate omits from the API of a resource method's request is left out of the
model. nexgen ignores the directives that it doesn't know yet, such as
`@nexus.varargs` and `@nexus.close`.

Python and TypeScript don't generate the service. They panic on the read item, a
`oneof` of proto-bound records, and Python can't import the protos. .NET types
each case of the read item as its proto class.

- WIT: [`inputs/stream-service.wit`](inputs/stream-service.wit)
- .NET: [`dotnet/wit/stream-service/`](dotnet/wit/stream-service/),
  [`dotnet/tests/StreamServiceChecks.cs`](dotnet/tests/StreamServiceChecks.cs)

## Supporting files

- [`inputs/deps/`](inputs/deps/): reusable Temporal semantic/common type WIT
  inputs linked into proto-backed example generation.
- [`descriptors/temporal_api.bin`](descriptors/temporal_api.bin): Temporal API
  descriptor set. An example selects it with `descriptors = "temporal"` in
  [`inputs/examples.toml`](inputs/examples.toml). Descriptors are opt-in, as in
  the compiler rejection fixtures, so an example with no `descriptors` gets
  none. A list specifies files in `descriptors/`.
- [`descriptors/streams.bin`](descriptors/streams.bin): descriptor set of the
  stream protos. Only the stream example lists it in `descriptors`. No SDK has
  these protos yet, so the sample projects hold their bindings:
  [`python/streamprotos/`](python/streamprotos/),
  [`dotnet/proto/streams/`](dotnet/proto/streams/) and
  [`typescript/proto/streams.d.ts`](typescript/proto/streams.d.ts).
- [`wire/proto/`](wire/proto/): proto wire fixtures used by the
  proto-compatibility tests.
- Per-language notes: [`dotnet/README.md`](dotnet/README.md),
  [`go/README.md`](go/README.md), [`java/`](java/) (no user support package),
  [`python/README.md`](python/README.md),
  [`typescript/README.md`](typescript/README.md).

## Regenerating

From the repository root:

```sh
cargo build-examples                         # both input formats, all languages
cargo build-examples --format wit --lang go  # WIT for a single language
cargo build-examples --format json-schema    # JSON Schema outputs (definitions + api)
```

Each example's output directory is deleted before it is regenerated, so a file
whose definition was renamed or removed never lingers — and nothing hand-written
belongs in one. The per-language tests live in the sibling `tests/` directories.
(The `nexgen` CLI itself never deletes: it writes into an existing `--output`
directory and leaves everything already there alone.)

### The stream proto bindings

The protos in `descriptors/streams/` are the Stream protos of
`temporalio/api#895` at `44519d8a`. After a change to them, rebuild the
descriptor set and the bindings with `protoc` 33.1 and `mypy-protobuf` 5.1.0.
`API` is a checkout of the Temporal API protos that has
`temporal.api.common.v1.Execution`.

```sh
cd advanced/samples
set -- temporal/api/enums/v1/stream.proto temporal/api/stream/v1/message.proto \
  temporal/api/streamservice/v1/request_response.proto
protoc -I descriptors/streams -I "$API" \
  --descriptor_set_out=descriptors/streams.bin "$@"
protoc -I descriptors/streams -I "$API" --python_out=python/streamprotos \
  --mypy_out=python/streamprotos "$@"
perl -pi -e 's/^from temporal\.api\.(stream|streamservice)\./from streamprotos.temporal.api.$1./;
  s/^from temporal\.api\.enums\.v1 import stream_pb2/from streamprotos.temporal.api.enums.v1 import stream_pb2/;
  s/^from temporal\.api\./from temporalio.api./' \
  $(find python/streamprotos -name '*_pb2.py*')
protoc -I descriptors/streams -I "$API" --csharp_out=dotnet/proto/streams \
  --csharp_opt=base_namespace= "$@"
```

The TypeScript `typescript/proto/streams.d.ts` mirrors the protos with only the
`I<Message>` interfaces, oneof names and enums, as `protobufjs` declares them.
It adds them to `@temporalio/proto` with `declare module`, so the generated code
imports them as it does the other Temporal protos.
