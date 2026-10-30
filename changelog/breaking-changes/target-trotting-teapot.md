`--native-api` is removed. Use `--targets workflow` for the same output. Without `--targets`, nexgen generates only models and definitions.
The library replaces `GenerationMode` and `generate_source_with_mode` with `GenerationTargets` and `generate_source_with_targets`. The function `generate_source` generates only models and definitions, as the CLI does without `--targets`.
Sources apply to all API targets unless restricted with `targets="..."`. Shared models no longer evaluate source defaults. Resources and sourced fields require at least one API target.
