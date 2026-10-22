# Contributing

Run `cargo validate` before pushing; it validates Rust, Markdown formatting, and
all five language sample projects. Run `cargo fmt-markdown` to rewrap Markdown
you have edited. Follow the generated sample guidance in [AGENTS.md](AGENTS.md).

## Local setup

Install [mise](https://mise.jdx.dev/getting-started.html), then run:

```sh
mise trust
mise install
```

Activate mise in your shell, or prefix repository commands with `mise exec --`.
`mise.toml` specifies the required global toolchains and Markdown formatter. CI
installs these before validation; local developers must install them explicitly.
Sample generation and validation still install project dependencies through npm,
uv, and the native build tools as needed.

## Running tests

`cargo test --all-features` runs the Rust-only suite, including parsing,
planning, CLI behavior, and generated-source assertions for every language. It
does not require Python, Node, Go, Java, or .NET tooling.

Use `cargo validate <language>` to validate one language: `rust`, `python`,
`typescript`, `go`, `java`, or `dotnet`. Each non-Rust stage checks generated
sample freshness, runs the sample projects' native checks, and exercises
generated-code regressions using the applicable compilers, formatters, and
runtimes. Go, Java, Python, and TypeScript validation also check JSON Schema
conformance cases, format and pattern corpora, and generated-output compilation
probes.

Toolchain tests are integration-test targets in the `nexgen-validation-tests`
package under `validation-tests/`. For focused debugging after dependencies are
installed, use Cargo's test selection:

```sh
cargo test -p nexgen-validation-tests --test python
cargo test -p nexgen-validation-tests --test python union_serializer -- --nocapture
```

The workspace's default members are `nexgen` and `xtask`.
`cargo test --workspace` explicitly includes the toolchain tests and requires
all language toolchains.

Use `cargo validate conformance` to compare JSON Schema behavior across Go,
Java, Python, and TypeScript. It requires all four toolchains and checks
acceptance, validation errors, and serialized values against expected results
and across backends. It also checks that declared divergences remain necessary.
These checks use unformatted generator output so formatters cannot hide syntax
defects. `cargo validate` runs all language stages followed by this
cross-language check.

Validation writes per-stage logs to `target/validate-logs/`. Use `--verbose` to
also see command output in the terminal.

Public-facing changes need a [changelog fragment](changelog/README.md).

## Preparing a release

From a clean worktree, run:

```sh
python3 scripts/prepare_release.py 0.3.0
```

The script creates or resumes `chore/release-0.3.0`, updates the crate and
lockfile versions, verifies the lockfile, assembles a dated release section, and
consumes fragments. It regenerates samples, commits the release changes, pushes,
and opens or reuses the release PR. Empty releases are allowed.
`--date YYYY-MM-DD` overrides the release date. Resuming a prepared release
preserves its changelog and any later fragments. Release PRs use the
`skip-changelog` label because they consume existing entries.

After merging, run the **Create Release** workflow from main. It uses the shared
tool to extract the completed changelog section for the draft GitHub release,
attaches binaries, and publishes the crate.

GitHub Actions checks out a pinned SDK Rust revision to build only the shared
changelog CLI for fragment validation and release-note extraction. Local release
preparation needs no SDK Rust checkout; Nexgen has no Core submodule. Keep the
tool pins in the changelog and Create Release workflows consistent.
