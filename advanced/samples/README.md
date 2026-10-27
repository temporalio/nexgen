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

A stream API shape: a resource built from a ref and a read with offsets. The
records are proto-backed, as a stream service's are, on the sample proto in
[`descriptors/streamsample/`](descriptors/streamsample/).

- WIT: [`inputs/stream-service.wit`](inputs/stream-service.wit)
- .NET: [`dotnet/wit/stream-service/`](dotnet/wit/stream-service/)
- Python: [`python/wit/stream_service/`](python/wit/stream_service/)
- TypeScript:
  [`typescript/wit/stream-service/`](typescript/wit/stream-service/),
  [`typescript/tests/stream-service.test.ts`](typescript/tests/stream-service.test.ts)

## Supporting files

- [`inputs/deps/`](inputs/deps/): reusable Temporal semantic/common type WIT
  inputs linked into proto-backed example generation.
- [`descriptors/temporal_api.bin`](descriptors/temporal_api.bin): Temporal API
  descriptor set. An example selects it with `descriptors = "temporal"` in
  [`inputs/examples.toml`](inputs/examples.toml). Descriptors are opt-in, as in
  the compiler rejection fixtures, so an example with no `descriptors` gets
  none. A list specifies files in `descriptors/`.
- [`descriptors/stream_service.bin`](descriptors/stream_service.bin): descriptor
  set of the `stream-service` sample proto. Only `stream-service` lists it in
  `descriptors`. The sample projects hold its bindings:
  [`python/streamsample/`](python/streamsample/),
  [`dotnet/proto/`](dotnet/proto/) and
  [`typescript/proto/streamsample.d.ts`](typescript/proto/streamsample.d.ts).
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

### The `stream-service` proto bindings

Upstream has no stream protos, so the sample has its own. After a change to
`descriptors/streamsample/v1/stream_service.proto`, rebuild its descriptor set
and bindings with `protoc` 33.1 and `mypy-protobuf` 5.1.0. `API` is a checkout
of the Temporal API protos (`temporalio/api`), for
`temporal/api/common/v1/message.proto`.

```sh
cd advanced/samples
protoc -I descriptors -I "$API" --descriptor_set_out=descriptors/stream_service.bin \
  streamsample/v1/stream_service.proto
protoc -I descriptors -I "$API" --python_out=python --mypy_out=python \
  streamsample/v1/stream_service.proto
perl -pi -e 's/^from temporal\.api\./from temporalio.api./' \
  python/streamsample/v1/stream_service_pb2.py python/streamsample/v1/stream_service_pb2.pyi
protoc -I descriptors -I "$API" --csharp_out=dotnet/proto streamsample/v1/stream_service.proto
```

The TypeScript `typescript/proto/streamsample.d.ts` mirrors the proto by hand,
with only the `I<Message>` interfaces and the enums that the generated code
imports.
