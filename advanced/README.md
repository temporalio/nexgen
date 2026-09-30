# Advanced: WIT-direct generation

> [!WARNING]
> This repository is experimental. Generated APIs, input formats, and CLI behavior
> may change without compatibility guarantees.

This is the advanced guide. For the JSON Schema getting-started guide, see the
[root README](../README.md).

`nexgen` can also generate language-specific Nexus operation bindings directly
from WIT. The WIT definition is the source of truth for the public API. Protobuf
descriptor sets are optional and are only needed when the WIT opts into
proto-backed models or when using `add-rpc` to scaffold WIT from an existing
proto RPC.

The generator picks its input format from the file extension, so the same
`generate <lang>` command accepts either WIT (`.wit`) or JSON Schema
(`.json`/`.yaml`/`.yml`) inputs.

> [!IMPORTANT]
> Everything on this page — the `dotnet` target, the `--support-file`,
> `--descriptors`, `--format`, and `--native-api` flags, and the `add-rpc` and
> `debug-wit-dir` subcommands — is
> gated behind the `advanced` Cargo feature, which is off by default. Build or
> run with `--features advanced` (as every command below does), or produce a
> binary that includes it with `cargo build --release --features advanced`.
> The `cargo build-examples` alias instead runs
> the repository-local `xtask` binary; they are not `nexgen` subcommands.

## Contents

- [Examples](#examples)
- [WIT-Direct Generation](#wit-direct-generation)
- [WIT Directives](#wit-directives)
- [Proto Backing](#proto-backing)
- [Validation](#validation)

## Examples

Each example starts with authored WIT under `advanced/samples/inputs/`.
Checked-in generated output lives under `advanced/samples/python/wit/<example_name>/`
and `advanced/samples/typescript/wit/<example-name>/`; Go output lives under
`advanced/samples/go/<example-name>/` and .NET output lives under
`advanced/samples/dotnet/wit/<example-name>/`. Language-specific tests live under
each language's `tests/` directory where present. See
[`samples/README.md`](samples/README.md) for links to each example's WIT,
generated code, and tests.

- [`user-service`](samples/inputs/user-service.wit): a small WIT-direct API showing the basic shape of an operation returning a resource and a resource method that calls another operation.
- [`type-showcase`](samples/inputs/type-showcase.wit): a WIT-direct API focused on type coverage, including records, enums, flags, variants, results, maps, tuples, resources, resource methods, and no-result operations.
- [`start-workflow`](samples/inputs/start-workflow.wit): a proto-backed Temporal workflow-start API that returns a generated resource handle with follow-up operations such as cancel, restart, and get-result.
- [`workflow-service`](samples/inputs/workflow-service.wit): a proto-backed `SignalWithStartWorkflowExecution` example showing flattened APIs, function arguments, sourced fields, support converters, and output transforms.
- [`type-roundtrip`](samples/inputs/type-roundtrip.wit): a proto-backed type roundtrip example for focused native/proto conversion coverage, including retry policies, activity options, durations, task queues, and priority.
- [`notification-service`](samples/inputs/notification-service.wit): a proto-backed notification completion payload with generic source-context and output values, currently generated for Python.

Rebuild the checked-in example outputs:

```bash
cargo build-examples
```

Rebuild one language or one example only:

```bash
cargo build-examples --format wit --lang python
cargo build-examples --format wit --lang dotnet
cargo build-examples --format wit user-service
cargo build-examples --format wit --lang typescript user-service
```

Run the same validations as the CI pipeline:

```bash
cargo validate
```

Write the prepared WIT workspace the loader actually parses:

```bash
cargo run --features advanced -- debug-wit-dir \
  --input advanced/samples/inputs/user-service.wit \
  --output /tmp/user-service-wit
```

## WIT-Direct Generation

Start with a WIT file that describes the API surface directly:

```wit
package temporal:user-service@1.0.0;

world system {
  export user-service;
}

/// @nexus.endpoint "__user_service"
interface user-service {
  resource user {
    constructor(user-id: string, email: string);

    update-email: func(email: string) -> user-result;
  }

  record get-user-request {
    user-id: string,
  }

  type user-result = own<user>;

  record update-email-request {
    user-id: string,
    email: string,
  }

  get-user: func(request: get-user-request) -> user-result;
  update-email: func(request: update-email-request) -> user-result;
}
```

Generate Python:

```bash
cargo run --features advanced -- python \
  advanced/samples/inputs/user-service.wit \
  --output /tmp/user_service
```

Generate TypeScript:

(`ts` is an alias for `typescript`.)

```bash
cargo run --features advanced -- typescript \
  advanced/samples/inputs/user-service.wit \
  --output /tmp/user-service
```

Generate Go:

```bash
cargo run --features advanced -- go \
  advanced/samples/inputs/user-service.wit \
  --output /tmp/userservice
```

Generate .NET:

```bash
cargo run --features advanced -- dotnet \
  advanced/samples/inputs/user-service.wit \
  --output /tmp/user-service-dotnet
```

Inputs are positional; pass more than one path (a file or a directory) to link
additional WIT into the parser workspace. Generation produces definitions by
default. Add `--native-api` to also generate native API bindings. Add `--format`
to run a formatter after generation:

- Python: `ruff format`
- TypeScript: `prettier --write`
- Go: `gofmt -w`
- .NET: no formatter is run

The `user-service` example is intentionally small and WIT-native. The `type-showcase` example demonstrates broader WIT type coverage: records, enums, flags, variants, results, maps, tuples, resources, resource methods, and an operation with no return value without proto annotations.

## WIT Directives

The WIT file defines the public surface. `@nexus` directives carry the parts WIT does not express directly:

- service endpoint names
- service wire names
- support file paths
- language-native service namespaces/packages
- language-native override types
- flattened API-only field types
- sourced field expressions
- function conversion metadata
- generic model parameters declared with `@nexus.type-parameter` aliases
- output transforms
- explicit resource method operation bindings
- experimental service, operation, and record warnings
- `@nexus.delay-load-temporalio-workflow` on Python services that must not import `temporalio.workflow` until an operation executes

Resource methods bind to operations only when the method and operation have the same generated operation name. When they intentionally differ, mark the method with `@nexus.operation`, for example `/// @nexus.operation "cancel-workflow"` on `cancel: func(...)` to bind it to `cancel-workflow: func(...)`.

Input WIT files can set generated service namespaces/packages with
`@nexus.namespace`, such as `dotnet="Temporalio.Workflows"` or
`go="go.temporal.io/sdk/workflow"`. For Go, the import path's final segment is
used as the package name and the full path is used to remove self-imports.

Input WIT files can add support code with `@nexus.support`. Python support fragments are copied into the generated private `_support` package, TypeScript support fragments are emitted as `support.ts` next to the generated `index.ts`, and .NET support fragments are copied under `Support/`.

Support code can also be supplied outside WIT with repeatable `--support-file`
arguments on the language subcommand. Explicit support files apply to the
selected language, are appended after WIT-declared support, and use the same generated
layout as `@nexus.support` fragments. .NET support files infer their support
namespace from the C# `namespace` declaration in the file:

```bash
cargo run --features advanced -- python \
  advanced/samples/inputs/user-service.wit \
  --support-file /path/to/custom_support.py \
  --output /tmp/user_service
```

## Proto Backing

Proto backing is opt-in per WIT type. Use it when an operation should accept or return generated API models while converting to or from protobuf messages at the Nexus boundary.

Proto-backed WIT uses:

- `@nexus.proto` on a WIT type to identify the protobuf message or enum it represents
- `@nexus.proto-field` when the WIT field name differs from the proto field name
- `--descriptors` on `generate` so the generator can validate fields and derive proto conversion code

Example:

```wit
package temporal:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "__temporal_system"
/// @nexus.service-name "temporal.api.workflowservice.v1.WorkflowService"
interface workflow-service {
  use nexus:temporal-types/model@1.0.0.{signal-function, task-queue, workflow-function};

  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
  record signal-with-start-workflow-request {
    /// @nexus.proto-field "workflow_type"
    workflow: workflow-function,
    workflow-id: string,
    task-queue: task-queue,
    /// @nexus.proto-field "signal_name"
    signal: signal-function,
    /// @nexus.source "workflow_namespace()"
    namespace: option<string>,
  }

  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionResponse"
  record signal-with-start-workflow-response {
    run-id: option<string>,
  }

  /// @nexus.output-transform
  ///   python-type="workflow.ExternalWorkflowHandle[typing.Any]"
  ///   python="workflow.get_external_workflow_handle(request.workflow_id, run_id=result.run_id)"
  ///   typescript-type="workflow.ExternalWorkflowHandle"
  ///   typescript="workflow.getExternalWorkflowHandle(request.workflowId, result.runId ?? undefined)"
  /// @nexus.operation name="SignalWithStartWorkflowExecution"
  signal-with-start-workflow: func(
    request: signal-with-start-workflow-request,
  ) -> signal-with-start-workflow-response;
}
```

Generate a proto-backed example:

```bash
cargo run --features advanced -- python \
  advanced/samples/inputs/workflow-service.wit \
  advanced/samples/inputs/deps \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --native-api \
  --output /tmp/workflow_service
```

`--descriptors` can be passed more than once when a proto-backed API depends on multiple descriptor files. Duplicate files or duplicate symbols are rejected.

The examples include a reusable Temporal semantic/common type WIT input:

- `nexus:temporal-types/model@1.0.0`

Pass it as an additional positional input when generating an API that uses
`nexus:temporal-types/model@1.0.0`. For `generate`, the first positional input is
the API generation root and later inputs are linked into the parser workspace.
For `add-rpc`, pass any WIT inputs needed to resolve shared types with `--input`;
when extending an existing WIT file, put that file first. A linked input can be a
single WIT file, a WIT package directory, or a directory containing WIT package
directories, so `advanced/samples/inputs/deps` links every package under it.

`add-message` follows the same input convention. It emits the selected message
and all reachable proto message/enum types without adding an operation. The
operation-free interface still emits and re-exports its owned model
declarations; linked dependency declarations remain tree-shaken unless an owned
model references them. When extending an existing WIT file, its world must
export exactly one interface.

### Protobuf oneofs

Both scaffold commands render protobuf `oneof` declarations as WIT variants.
Every message remains a record, with an `option<variant>` field for each
oneof—even when the message contains only one oneof. Scaffolding stays optional
because protobuf descriptors permit a oneof to be unset. Authors can remove
`option<>` to assert that exactly one case is required by their model. Existing
WIT mappings must use the same grouped-record shape.

Python native-API generation converts both authored shapes bidirectionally.
For `option<variant>`, an unset protobuf oneof decodes to `None`, and encoding
`None` leaves the oneof unset. A required `variant` has no `None` annotation or
constructor default; decoding an unset oneof or encoding a runtime `None`
raises `ValueError("missing required field Model.field")`. Each selected case
uses the normal WIT variant tuple form `(tag, payload)`, and unknown authored
tags continue to raise `ValueError`.

Other language backends report an explicit unsupported-conversion error when a
reachable oneof model requires protobuf conversion.

Python also preserves concrete runtime type arguments while decoding nested
proto-backed generic records. A type parameter represented by Temporal's
single-value `Payload` carrier is passed to the payload converter as its type
hint; `Payloads` continues to decode as a sequence. The
`proto-generic-python` sample exercises this Python-specific implementation without
making the cross-language `generic-models` sample depend on protobuf support.

Generate WIT for a proto RPC from a descriptor set:

```bash
cargo run --features advanced -- add-rpc \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --rpc SignalWithStartExecution \
  --input advanced/samples/inputs/deps
```

Generate WIT for a standalone proto message tree:

```bash
cargo run --features advanced -- add-message \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --message WorkflowExecutionInfo \
  --input advanced/samples/inputs/deps
```

Write the standalone WIT scaffold to a file instead of stdout:

```bash
cargo run --features advanced -- add-rpc \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --rpc temporal.api.workflowservice.v1.WorkflowService.SignalWithStartWorkflowExecution \
  --input advanced/samples/inputs/deps \
  --output /tmp/add-rpc.wit
```

Extend an existing WIT file with a new RPC:

```bash
cargo run --features advanced -- add-rpc \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --rpc SignalWorkflowExecution \
  --input advanced/samples/inputs/workflow-service.wit \
  --input advanced/samples/inputs/deps
```

Rewrite the existing WIT file in place by pointing `--output` at the same path:

```bash
cargo run --features advanced -- add-rpc \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --rpc SignalWorkflowExecution \
  --input advanced/samples/inputs/workflow-service.wit \
  --input advanced/samples/inputs/deps \
  --output advanced/samples/inputs/workflow-service.wit
```

## Validation

Validate the Python examples:

```bash
cargo build-examples --format wit --lang python
cd advanced/samples/python
uv run pytest
uv run basedpyright
```

Validate the TypeScript examples:

```bash
cargo build-examples --format wit --lang typescript
cd advanced/samples/typescript
npm install
npm run test
npm run typecheck
```

`cargo test` validates the checked-in example outputs as-is. Use `cargo build-examples` when you want to refresh them.
