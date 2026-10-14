`cargo test --all-features` no longer requires language-specific toolchains. `cargo validate <language>` now runs that language's generated-code compilation and runtime regressions alongside its sample checks.
`cargo validate conformance` checks JSON Schema behavior across Go, Java, Python, and TypeScript, and is included in full validation.
