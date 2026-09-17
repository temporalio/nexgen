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

## WIT examples

### `function-execution`

- WIT: [`inputs/function-execution.wit`](inputs/function-execution.wit)
- .NET: [`dotnet/wit/function-execution/`](dotnet/wit/function-execution/)
- Go: [`go/functionexecution/`](go/functionexecution/)
- Python: [`python/wit/function_execution/`](python/wit/function_execution/)
- TypeScript: [`typescript/wit/function-execution/`](typescript/wit/function-execution/)

### `user-service`

- WIT: [`inputs/user-service.wit`](inputs/user-service.wit)
- .NET: [`dotnet/wit/user-service/`](dotnet/wit/user-service/)
- Go: [`go/userservice/`](go/userservice/), [`go/tests/userservice_test.go`](go/tests/userservice_test.go)
- Python: [`python/wit/user_service/`](python/wit/user_service/), [`python/tests/test_user_service.py`](python/tests/test_user_service.py)
- TypeScript: [`typescript/wit/user-service/`](typescript/wit/user-service/), [`typescript/tests/user-service.test.ts`](typescript/tests/user-service.test.ts)

### `type-showcase`

- WIT: [`inputs/type-showcase.wit`](inputs/type-showcase.wit)
- .NET: [`dotnet/wit/type-showcase/`](dotnet/wit/type-showcase/)
- Go: [`go/typeshowcase/`](go/typeshowcase/), [`go/tests/typeshowcase_test.go`](go/tests/typeshowcase_test.go)
- Python: [`python/wit/type_showcase/`](python/wit/type_showcase/), [`python/tests/test_type_showcase.py`](python/tests/test_type_showcase.py)
- TypeScript: [`typescript/wit/type-showcase/`](typescript/wit/type-showcase/), [`typescript/tests/type-showcase.test.ts`](typescript/tests/type-showcase.test.ts)

### `start-workflow`

- WIT: [`inputs/start-workflow.wit`](inputs/start-workflow.wit)
- .NET: [`dotnet/wit/start-workflow/`](dotnet/wit/start-workflow/)
- Go: [`go/startworkflow/`](go/startworkflow/)
- Python: [`python/wit/start_workflow/`](python/wit/start_workflow/), [`python/tests/test_start_workflow.py`](python/tests/test_start_workflow.py)
- TypeScript: [`typescript/wit/start-workflow/`](typescript/wit/start-workflow/), [`typescript/tests/start-workflow.test.ts`](typescript/tests/start-workflow.test.ts)

### `workflow-service`

- WIT: [`inputs/workflow-service.wit`](inputs/workflow-service.wit)
- .NET: [`dotnet/wit/workflow-service/`](dotnet/wit/workflow-service/)
- Go: [`go/workflowservice/`](go/workflowservice/)
- Python: [`python/wit/workflow_service/`](python/wit/workflow_service/), [`python/tests/test_workflow_service.py`](python/tests/test_workflow_service.py)
- TypeScript: [`typescript/wit/workflow-service/`](typescript/wit/workflow-service/), [`typescript/tests/workflow-service.test.ts`](typescript/tests/workflow-service.test.ts)

### `type-roundtrip`

- WIT: [`inputs/type-roundtrip.wit`](inputs/type-roundtrip.wit)
- .NET: [`dotnet/wit/type-roundtrip/`](dotnet/wit/type-roundtrip/)
- Go: [`go/typeroundtrip/`](go/typeroundtrip/), [`go/tests/typeroundtrip_test.go`](go/tests/typeroundtrip_test.go)
- Python: [`python/wit/type_roundtrip/`](python/wit/type_roundtrip/), [`python/tests/test_type_roundtrip.py`](python/tests/test_type_roundtrip.py)
- TypeScript: [`typescript/wit/type-roundtrip/`](typescript/wit/type-roundtrip/), [`typescript/tests/type-roundtrip.test.ts`](typescript/tests/type-roundtrip.test.ts)

### `notification-service`

- WIT: [`inputs/notification-service.wit`](inputs/notification-service.wit)
- Currently generated for Python: [`python/wit/notification_service/`](python/wit/notification_service/)

## Supporting files

- [`inputs/deps/`](inputs/deps/): reusable Temporal semantic/common type WIT inputs linked into proto-backed example generation.
- [`descriptors/temporal_api.bin`](descriptors/temporal_api.bin): Temporal API descriptor set used by proto-backed examples.
- [`wire/proto/`](wire/proto/): proto wire fixtures used by the proto-compatibility tests.
- Per-language notes: [`dotnet/README.md`](dotnet/README.md), [`go/README.md`](go/README.md), [`java/README.md`](java/README.md), [`python/README.md`](python/README.md), [`typescript/README.md`](typescript/README.md).

## Regenerating

From the repository root:

```sh
cargo build-examples                         # both input formats, all languages
cargo build-examples --format wit --lang go  # WIT for a single language
cargo build-examples --format json-schema    # JSON Schema outputs (definitions + api)
```

Each example's output directory is deleted before it is regenerated, so a file
whose definition was renamed or removed never lingers — and nothing
hand-written belongs in one. The per-language tests live in the sibling
`tests/` directories. (The `nexgen` CLI itself never deletes: it writes into an
existing `--output` directory and leaves everything already there alone.)
