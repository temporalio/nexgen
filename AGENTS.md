# nexgen contributor guidance

## Validation

- Before pushing, run the full validation suite from the repository root:

  ```sh
  cargo validate
  ```

- Focused checks are appropriate while iterating, but do not push without the
  full command passing. It covers Rust formatting and tests plus the checked-in
  Python, TypeScript, Go, Java, and .NET sample projects.

  ```sh
  cargo validate python
  ```

- When running Rust tests directly, pass the feature flag to test the "advanced" features:

  ```sh
  cargo test --all-features
  ```

## Stacked pull requests

When asked to create a stack of pull requests, use the
[`gh-stack`](https://github.com/github/gh-stack) GitHub CLI extension rather
than creating and linking the PRs manually.

- If `gh stack` is unavailable, install it with `gh extension install
github/gh-stack`.
- Initialize the stack from its trunk branch, normally with `gh stack init
--base main <bottom-branch>`, then add each successive layer from the current
  top branch with `gh stack add <branch>`.
- Commit each layer independently. Use `gh stack push` to publish every branch
  and `gh stack submit` to create the PRs with the correct parent branches.
- Use `gh stack view` to inspect the stack. When the trunk or lower layers
  change, use `gh stack sync` (or `gh stack rebase` when resolving conflicts)
  before resubmitting updates.

## Generated samples

- Never edit generated sample output directly. Change the authored input,
  generator, or shared model as appropriate, then regenerate the affected
  outputs with the generator.
- JSON Schema sample outputs: `cargo build-examples --format json-schema [--lang <language>]`.
- Advanced WIT sample outputs: `cargo build-examples --format wit [--lang <language>]
[<example>]`.
- Review regenerated diffs to ensure they are the intended consequence of the
  source change.

## Architecture and ownership

- Before changing JSON Schema behavior, read
  [`specs/json-schema/PRINCIPLES.md`](specs/json-schema/PRINCIPLES.md). Before
  changing compiler passes, read [`architecture.md`](architecture.md). Before
  changing WIT authoring or WIT-facing generated behavior, read
  [`GUIDE.md`](GUIDE.md).
- `src/parser/wit.rs` is the only place that should understand WIT-specific
  syntax and semantics. Keep WIT details out of the shared model and later
  pipeline stages.
- Everything after parsing—including `spec`, planning, validation, and the
  shared generator orchestration—must be input-format agnostic. Parsers lower
  their input into the shared API model; later stages must not branch on whether
  it originated as WIT or JSON Schema.
- Keep JSON and JSON Schema concerns within the relevant JSON Schema parser,
  planning, generator, and `src/json_schema/` components. Do not introduce JSON
  knowledge into generic or unrelated modules.
- Put target-language policy and rendering decisions in that language's files
  or modules (for example `src/generator/python.rs` and
  `src/generator/json_schema/python.rs`). Shared code may represent language
  neutral concepts, but must not encode one target's conventions.

### Planning

When planning a change, map each change onto one or more existing architectural component
such as parsing or specific passes. If the change requires adding a pass then explicitly
identify it as a new addition to the architecture. Each change should have a
justification for why it belongs where it is planned in the architecture.

## Documentation and public changes

- Update the architecture documentation when a pipeline boundary, ownership
  rule, or processing flow changes. JSON Schema architecture lives under
  `specs/json-schema/`, especially `pipeline.md` and `PRINCIPLES.md`.
- Update `GUIDE.md` when WIT authoring or generated WIT-facing behavior changes.
- Update `CHANGELOG.md` only for public-facing changes, such as CLI flags,
  generated APIs, or externally visible behavior. Use the `Unreleased` section.

## Compatibility

This repository is experimental. Do not infer compatibility guarantees for the
CLI, generated output, or internal APIs; document intentional public-facing
changes as above.
