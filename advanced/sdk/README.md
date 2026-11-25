# SDK integration fixtures and validation

`workflow-service.wit` and `stream-service.wit` are SDK-native fixtures. Do not
make them independently executable as samples. Validate their generated APIs and
behavior inside the actual SDKs. Workflow-service tests run in all four SDKs;
stream runtime tests run only in Python. .NET and TypeScript also have stream
review snapshots.

Run these opt-in stages from the nexgen repository root. They are not included
in bare `cargo validate` and download SDK dependencies. Install the relevant
language toolchain, `protoc`, a C/C++ compiler, and CMake first:

```sh
cargo validate sdk-go --verbose
cargo validate sdk-python --verbose
cargo validate sdk-typescript --verbose
cargo validate sdk-dotnet --verbose
```

Each stage builds the current nexgen source with `advanced`, then generates and
tests bindings in an isolated SDK worktree. Uncommitted nexgen changes,
including WIT and support files, are included; uncommitted SDK changes are not.
The staged inputs are the two WIT fixtures here,
`advanced/samples/inputs/deps/nexus-temporal-types/`, and
`advanced/samples/inputs/notification-service.wit`. Notification is included
because SDK generators process it in the same invocation.

`xtask/src/validate/sdk.rs` implements these stages, with unit tests in the
normal Rust suite. Logs are always written under `target/validate-logs/`;
`--verbose` also displays command output.

## Review snapshots

Checked-in output under `generated/<language>/<fixture>/[<child>/]` makes
generated API changes visible in nexgen diffs. `examples.toml` uses the same
configuration format as the standalone samples: it declares each fixture's
languages, targets, System Nexus bindings, descriptors, and optional child
configurations. Descriptor names resolve under `../samples/descriptors/`.

Regenerate the workflow-service and stream-service snapshots with:

```sh
cargo build-examples --format wit workflow-service stream-service
```

Use `--lang` for languages enabled in `examples.toml`. For example:

```sh
cargo build-examples --format wit --lang python stream-service
cargo build-examples --format wit --lang go workflow-service
```

The regular `cargo build-examples` command also refreshes these snapshots.
Normal language validation regenerates them and fails on Git differences. They
are review artifacts, not standalone projects: they are neither compiled nor
executed here, and are never copied into SDK checkouts. The SDK stages
independently regenerate and test bindings against real SDK internals.

The stream snapshot has a `both/` child for Python's workflow and client APIs
and a `workflow/` child for .NET and TypeScript. Adding a fixture or changing
its language coverage requires updating `examples.toml`, not the Rust builder.

## Ownership

The SDK owns its support implementations. Copies under `<language>/` let a
nexgen change update WIT and support together. They use SDK-relative paths and
replace SDK files verbatim: for example,
`python/temporalio/nexus/system/_support.py` replaces
`temporalio/nexus/system/_support.py` in the isolated Python checkout.

Keep each copy identical to its corresponding SDK implementation, and include
support changes in the associated SDK PR. Runtime internals come from the SDK,
not mocks, standalone facades, or sample-only import adapters.

## SDK baselines

The SDK submodules under `repos/` record the exact commits used in CI. These
temporary baselines provide capabilities not yet available together on SDK
mainline:

| SDK        | Upstream dependency                                                                                                                                                                           | Required capabilities                                                                      |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| Go         | [#2914](https://github.com/temporalio/sdk-go/pull/2914)                                                                                                                                       | Destination storage routing                                                                |
| Python     | [#2163](https://github.com/temporalio/sdk-python/pull/2163), based on `nexgen/streams-storage-integration`                                                                                    | Stream models, client and workflow generation, stream runtime, destination storage routing |
| TypeScript | [#2720](https://github.com/temporalio/sdk-typescript/pull/2720)                                                                                                                               | Destination storage routing                                                                |
| .NET       | [#1060](https://github.com/temporalio/sdk-dotnet/pull/1060), atop [#1056](https://github.com/temporalio/sdk-dotnet/pull/1056) and [#1048](https://github.com/temporalio/sdk-dotnet/pull/1048) | Optional notification context, destination storage routing, interceptor dispatch           |

Use mainline once it contains the required SDK changes and consumes the matching
released nexgen and API definitions. The
[contributor guide](../../CONTRIBUTING.md#coordinating-sdk-changes) describes
the SDK PR and release coordination process.

Fresh submodule initialization checks out the recorded Git pointer. For an
already initialized submodule, validation uses its checked-out committed `HEAD`
without modifying it. To test a different baseline, fetch and check out the
desired SDK commit, then run the corresponding validation stage. Uncommitted SDK
files are never included. Stage the submodule pointer change in nexgen to make
that revision the shared baseline.

## Functional coverage

| SDK        | Native suite                                                           | Required behavior                                                                                                                |
| ---------- | ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| Go         | `TestSystemNexusSuite` in the separate `test` module                   | Signal-with-start, options, conversion failures, interceptors, tracing, destination storage routing, replay                      |
| Python     | System Nexus and native stream suites                                  | Protobuf round trips, converters/codecs, destination storage routing, stream client/activity/workflow operations, stores, replay |
| TypeScript | `packages/test/src/test-integration-system-nexus.cloud-unavailable.ts` | Generated public API, interceptor routing, target context, destination storage routing, replay                                   |
| .NET       | System Nexus and concurrent destination-storage tests                  | Signal-with-start, target serialization context, codecs, replay, interceptor routing, destination storage routing                |

Go runs its canonical integration-test command with `-dev-server`; the other
suites start their own local server using their SDK's pinned download and
dynamic configuration. The runner removes external-server and skip overrides so
it does not use a developer's namespace. Every stage propagates the native test
runner's exit status. Reports and logs are retained for diagnostics, not parsed
for additional success criteria. Compilation alone is never a successful SDK
validation.

Python also runs basedpyright over the generated System Nexus package, its
support module, the consuming workflow runtime, and the selected SDK tests.
Whole-SDK linting of unrelated features remains the SDK's own CI responsibility.

Isolated worktrees, generated sources, and server logs are retained under
`target/sdk-validation/<language>-*/`. Native reports are in each worktree's
`.nexgen-test-results/`. To inspect and remove an old run, use:

```sh
git -C advanced/sdk/repos/python worktree list
git -C advanced/sdk/repos/python worktree remove <path-to-old-run>
```

The runner never resets a developer checkout or writes to Go's module cache. In
isolated SDK checkouts, it also adapts generation scripts' `--native-api`
argument to `--targets workflow` when present and isolates Rust bridge
workspaces from nexgen's Cargo workspace. WIT and support files are not
rewritten.
