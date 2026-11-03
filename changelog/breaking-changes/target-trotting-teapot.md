Replace `--native-api` with `--targets workflow`; omitting `--targets` generates only models and service definitions.
Rust generation configuration replaces `GenerationMode` with `Vec<GenerationTarget>`; use `generate_source_with_targets` instead of `generate_source_with_mode`.
Python keeps shared models and service definitions at the root; when both targets are selected, workflow APIs live in `workflow/` and client APIs in `client/`.
Python model constructors no longer evaluate `@nexus.source` expressions; callers must supply sourced fields explicitly.
Operations returning resources must return resources owned by their service, including inside result records.
