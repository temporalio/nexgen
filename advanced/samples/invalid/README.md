# Compiler failure fixtures

These inputs must fail compilation for the reason in their `expected.toml`. They
live separately from `../inputs/`, whose examples must generate successfully.
They do not produce checked-in output or run target-language tools.

Run them with:

```sh
cargo test --all-features --test compiler_failures
```

The normal Rust stage of `cargo validate` also runs this suite. The integration
test reports all failing case/target pairs, including the actual error.

## Adding a case

Create a directory containing `input.wit` and `expected.toml`. Keep the input
small and self-contained, with only the declarations needed to reproduce the
failure. For example:

```toml
descriptors = "temporal"

[error]
kind = "invalid-proto-enum-binding"
contains = ["authored case `severity-low` has number 0"]
```

`kind` selects a typed Rust compiler error through a test-only adapter in
`tests/compiler_failures.rs`. This is not a public diagnostic-code API. Add a
new adapter variant when migrating a failure that uses a different error.

`contains` must have at least one nonempty fragment. Every fragment must occur
in the rendered diagnostic. Assert details that establish the intended failure,
not the entire message or machine-specific paths. An unexpected success, a
different error kind, or any missing fragment fails the test.

Unknown configuration fields and kinds are errors. Missing inputs or descriptor
paths are fixture errors, not expected compiler failures.

## Optional configuration

Fields before `[error]` configure compilation:

```toml
languages = ["python"]
inputs = ["input.wit", "deps"]
descriptors = "temporal"

[error]
kind = "invalid-type-override-field"
contains = ["does not match proto field type `bytes`"]
```

- `languages` defaults to Python, TypeScript, Go, and .NET, the supported native
  WIT targets. Set it for a target-specific failure. Java can be selected
  explicitly for tests of its unsupported WIT features. Empty or duplicate
  target lists are rejected.
- `inputs` defaults to `["input.wit"]`. Use an explicit list for multiple inputs
  or linked packages; paths are relative to the case directory.
- `descriptors` defaults to no descriptors. Set `"temporal"` to use the
  repository's checked-in `advanced/samples/descriptors/temporal_api.bin`,
  independent of the case directory or working directory. For custom descriptor
  sets, use a list such as `["custom.bin"]`; paths are relative to the case
  directory. An empty list also selects no descriptors. Unknown preset names are
  errors.

Compilation uses native API mode, disables formatting, and supplies a
target-appropriate support-package name. Each target gets its own temporary
output directory. The proof of concept does not offer arbitrary CLI arguments or
exact stderr snapshots; add configuration only when a concrete case needs it.
