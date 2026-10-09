# Changelog fragments

For each PR with public-facing changes, add a Markdown file under one or more
of `added/`, `stabilized/`, `changed/`, `deprecated/`, `breaking-changes/`,
`fixed/`, or `security/`. Use `stabilized` when a feature is no longer experimental.

Choose a fun, whimsical lowercase kebab-case filename, for example
`fixed/moonwalking-marshmallow.md`. Write concise entries, ideally one or two
sentences. Each nonempty line is a separate item; omit leading `-` characters
and headings. Multiple items from one PR can share a file. Use separate files
for unrelated PRs. Inline Markdown, including links, is supported.

CI validates fragments with the shared tool from SDK Rust. `CHANGELOG.md`
contains completed releases only. Local release preparation adds bullets,
groups entries by category (including `:boom: Breaking Changes`), and deletes
consumed fragments.

Internal changes that need no entry can use the `skip-changelog` PR label.
See [contributing instructions](../CONTRIBUTING.md) for the release process.
