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

The suite requires `advanced` and `test-support`, both enabled by
`--all-features`. The opt-in `test-support` feature derives `ErrorKind` using
Strum; ordinary builds, including those with only `advanced`, do not include
Strum or the derived enum.

## Adding a case

Create `<category>/<case>/` containing `input.wit` and `expected.toml`.
Categories group the compiler rule, not merely the input format or target:

- `wit-parser/`: WIT syntax and directive parsing.
- `proto/`: protobuf binding compatibility and defaults.
- `type-parameters/`: shared generic-type restrictions.
- `resources/`: resource methods and operation bindings.
- `name-collisions/`: conflicts caused by emitted names, including
  target-specific naming transformations.
- `system-nexus/`: cases requiring system-Nexus mode, grouped further by
  feature, such as `serialization-context/`.

Discovery recurses through categories but stops at each `expected.toml`, so a
case's linked packages are not mistaken for additional cases. Empty categories
and case directories without expectations fail discovery.

Keep the input small and self-contained, with only the declarations needed to
reproduce the failure. For example:

```toml
descriptors = "temporal"

[error]
kind = "invalid-proto-enum-binding"
contains = ["authored case `severity-low` has number 0"]
```

`kind` names a compiler `Error` variant in kebab-case. The fieldless `ErrorKind`
enum is derived directly from `Error`, so every compiler error variant is
available without a separate test mapping. It identifies the error variant
without requiring the fixture to provide its payload fields.

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
- `system_nexus` defaults to `false`. Set it to `true` for cases requiring
  system-Nexus generation. Category names are organizational only and never
  enable compiler settings implicitly.
- `targets` defaults to the workflow target. Set a list such as
  `["workflow", "client"]` for a case about another generation target.
- `descriptors` defaults to no descriptors. Set `"temporal"` to use the
  repository's checked-in `advanced/samples/descriptors/temporal_api.bin`,
  independent of the case directory or working directory. For custom descriptor
  sets, use a list such as `["custom.bin"]`; paths are relative to the case
  directory. An empty list also selects no descriptors. Unknown preset names are
  errors.

Use representative compiler fixtures for equivalent spelling or syntax
variations. Keep exhaustive lexical checks, such as Go's disallowed helper
identifiers and keywords, in focused helper-level unit tests.

Compilation uses native API mode, disables formatting, and supplies a
target-appropriate support-package name. Each target gets its own temporary
output directory. The proof of concept does not offer arbitrary CLI arguments or
exact stderr snapshots; add configuration only when a concrete case needs it.
