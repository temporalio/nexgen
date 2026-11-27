# SDK integration validation

`workflow-service.wit` and `stream-service.wit` use SDK internals. Validate their
bindings in the SDKs. Workflow service tests run in all four SDKs. Stream runtime
tests run in Python. TypeScript and .NET have Stream review snapshots.

These stages are separate from `cargo validate`. Install the language toolchain,
`protoc`, a C/C++ compiler and CMake first.

```sh
cargo validate sdk-go --verbose
cargo validate sdk-python --verbose
cargo validate sdk-typescript --verbose
cargo validate sdk-dotnet --verbose
```

Each stage builds the current nexgen source and tests generated bindings in an
isolated SDK worktree. It includes uncommitted nexgen changes and the committed
SDK `HEAD`. Uncommitted SDK changes are excluded.

## Review snapshots

The files under `generated/` expose API changes for review. SDK validation
regenerates bindings inside the SDK instead of copying these snapshots.

```sh
cargo build-examples --format wit workflow-service stream-service
cargo build-examples --format wit --lang python stream-service
```

`examples.toml` selects languages, targets and descriptors. Python's Stream
`both/` child includes Workflow and client APIs. TypeScript and .NET use the
`workflow/` child. Normal language validation checks that snapshots match the
inputs.

## Support files and baselines

The SDK owns each support implementation. Copies under `<language>/` replace
the corresponding SDK files during validation. Keep each copy identical to its
SDK implementation and include support changes in the associated SDK PR.

Submodules under `repos/` pin the SDK baselines used in CI.

| SDK | Dependencies |
| --- | --- |
| Go | [#2914](https://github.com/temporalio/sdk-go/pull/2914) for destination storage routing |
| Python | [#2128](https://github.com/temporalio/sdk-python/pull/2128) and [#2129](https://github.com/temporalio/sdk-python/pull/2129) for Streams, plus [#2147](https://github.com/temporalio/sdk-python/pull/2147) for storage routing |
| TypeScript | [#2720](https://github.com/temporalio/sdk-typescript/pull/2720) for storage routing |
| .NET | [#1060](https://github.com/temporalio/sdk-dotnet/pull/1060), [#1056](https://github.com/temporalio/sdk-dotnet/pull/1056) and [#1048](https://github.com/temporalio/sdk-dotnet/pull/1048) for storage routing and notification context |

Use mainline after it contains these dependencies and the matching released
nexgen API. See [SDK change coordination](../../CONTRIBUTING.md#coordinating-sdk-changes).

A fresh submodule uses its recorded commit. An initialized submodule uses its
checked-out `HEAD`. To test another baseline, check out that SDK commit and run
its validation stage. Commit the submodule pointer to share that baseline.

## Runtime checks

| SDK | Native suites |
| --- | --- |
| Go | `TestSystemNexusSuite` with `-dev-server` |
| Python | System Nexus, Stream client, Activity, Workflow, test Store and helper suites |
| TypeScript | `test-integration-system-nexus.cloud-unavailable.ts` |
| .NET | Notification, completion callback, System Nexus and concurrent destination storage tests |

The suites exercise conversions, interceptors, storage routing and replay.
Python also runs `basedpyright` over generated bindings, support, the Workflow
runtime and the selected tests. SDK CI checks unrelated SDK code.

Validation starts its own local server and ignores external-server overrides.
It returns the native runner's exit status. A build without runtime tests is not
a successful SDK validation.

Logs are in `target/validate-logs/`. Isolated SDK worktrees remain under
`target/sdk-validation/`, with native reports in `.nexgen-test-results/`.

```sh
git -C advanced/sdk/repos/python worktree list
git -C advanced/sdk/repos/python worktree remove <path-to-old-run>
```
