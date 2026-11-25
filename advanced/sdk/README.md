# Workflow-service SDK integration

`workflow-service.wit` is the authoritative, SDK-native workflow-service
fixture. Do not make it independently executable as a sample. Validate its
generated API and behavior inside the actual SDKs:

```sh
cargo validate sdk-go --verbose
cargo validate sdk-python --verbose
cargo validate sdk-typescript --verbose
cargo validate sdk-dotnet --verbose
```

These checks build the current nexgen source with `advanced`, including
uncommitted changes. They stage this WIT unchanged, together with the shared
Temporal type package from `../samples/inputs/deps/`. The local notification WIT
is also staged because SDK generation scripts emit both services together.

The Rust validation task in `xtask/src/validate/sdk.rs` owns orchestration. Its
unit tests run with the regular Rust suite; there is no separate Python
validation runner.

## Review snapshots

Checked-in output under `generated/<language>/workflow-service/` makes changes
to the generated API visible in nexgen diffs. Regenerate all four SDK snapshots
with:

```sh
cargo build-examples --format wit workflow-service
```

Use `--lang` to select one language. The regular `cargo build-examples` command
also refreshes these snapshots, and sample validation checks their freshness.
They use SDK-native namespaces and imports and are not standalone projects: they
are neither compiled nor executed here, and they are never copied into SDK
checkouts. The SDK checks above independently regenerate and test the bindings
against real SDK internals.

## Ownership

Support changes referenced by the WIT belong alongside it. Files under
`<language>/` use SDK-relative paths and replace the corresponding authored SDK
files verbatim. Each of Go, Python, TypeScript, and .NET carries its SDK-native
support sources here so changes to nexgen, WIT, and support implementations can
be validated together. Runtime internals remain supplied by the SDK. Do not add
mocks, standalone facades, or sample-only import adapters.

The SDKs are pinned Git submodules under `repos/`, declared in the root
`.gitmodules`. Validation initializes the selected submodule if needed, then
uses a detached Git worktree of its current commit. It does not reset or modify
the dependency checkout, and does not copy uncommitted SDK edits into the test
run. Commit SDK runtime experiments on an SDK branch before testing them.

Python pins the dedicated `nexgen/streams-storage-integration` SDK branch at
`6a08da463440121053bc51e1585fd0f083a680fb`. This combines SDK mainline
`37a262bd722746b9d4f7ae44790a1221ee9ae28f`,
[streams PR #2129](https://github.com/temporalio/sdk-python/pull/2129) at
`09dc86b479d9e7098e9251b825dcdc72cb946388`, and
[storage-routing PR #2147](https://github.com/temporalio/sdk-python/pull/2147)
at `4347459b21bc940d37db7299065d40cc518a1d07`. It also supplies the explicit
reuse policy in the storage tests' request constructors and preserves stream
workflow exports during regeneration. The runner does not merge anything or
configure a Git identity. Move the submodule to mainline once both features
land.

.NET pins `03c2b173383c7280e04079f21436270a90715646`, the published tip of the
SDK PR stack:

1. [Interceptor PR #1048](https://github.com/temporalio/sdk-dotnet/pull/1048).
2. [Storage-routing PR #1056](https://github.com/temporalio/sdk-dotnet/pull/1056),
   based on #1048.
3. [Optional notification context PR #1060](https://github.com/temporalio/sdk-dotnet/pull/1060),
   based on #1056.

Keep dependent SDK changes in separate stacked PRs and pin the published stack
tip. Complete the batch after the nexgen release and API → Core updates, as
described in
[the contributor guide](../../CONTRIBUTING.md#coordinating-sdk-changes).

Go pins
[storage-routing PR #2914](https://github.com/temporalio/sdk-go/pull/2914) at
`d49666c1c08d58fbfb93c0e4a5a74f247b1b64a4`. TypeScript pins
[storage-routing PR #2720](https://github.com/temporalio/sdk-typescript/pull/2720)
at `74c4b8a0f518d238a194a80f84df0b3145afac9b`. Both suites include
destination-routing and replay scenarios.

To update an SDK baseline, fetch and check out the desired commit in its
submodule, run the corresponding validation stage, and stage the submodule
pointer change in nexgen. The Git diff records the exact dependency update.
There are no runtime branch-selection environment variables.

## Functional coverage

| SDK        | Native suite                                          | Required behavior                                                                                                                |
| ---------- | ----------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| Go         | `TestSystemNexusSuite` in the separate `test` module  | Signal-with-start, options, conversion failures, interceptors, tracing                                                           |
| Python     | System Nexus and native stream suites                 | Protobuf round trips, converters/codecs, destination storage routing, stream client/activity/workflow operations, stores, replay |
| TypeScript | `test-integration-system-nexus.cloud-unavailable`     | Generated public API, interceptor routing, target context, external storage                                                      |
| .NET       | System Nexus and concurrent destination-storage tests | Signal-with-start, target serialization context, codecs, replay, interceptor routing, destination storage routing                |

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

Isolated worktrees, generated sources, server logs, and structured reports are
retained under `target/sdk-validation/<language>-*/`. Per-stage logs live under
`target/validate-logs/`. Use the owning SDK submodule's `git worktree` commands
to inspect and remove old runs after debugging. The runner never resets a
developer checkout or writes to Go's module cache.

The harness translates legacy SDK generation scripts' `--native-api` spelling to
`--targets workflow` and gives standalone Rust bridge manifests an explicit
workspace boundary for nesting inside nexgen. These are build invocation
adjustments, not changes to the fixture's semantics or SDK runtime.
