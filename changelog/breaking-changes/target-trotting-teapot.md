`--native-api` is removed. Use `--targets workflow` for the same output. Without `--targets`, nexgen generates only models and definitions.
The library replaces `GenerationMode` and `generate_source_with_mode` with `GenerationTargets` and `generate_source_with_targets`. The function `generate_source` generates only models and definitions, as the CLI does without `--targets`.
Sources apply to all API targets unless restricted with `targets="..."`.
Python models no longer default a `@nexus.source` field to its source. Code that builds a model directly must pass the sourced field.
With no `--targets`, a resource is a data class with its fields in every language, so a constructor that took no arguments now takes the fields.
Without the source default, `kw_only` of a Python model follows where its sourced field is. For example `StartWorkflowRequest` becomes `kw_only=True`, so positional construction breaks, and `CancelWorkflowRequest` is no longer `kw_only`. Decode of an absent sourced message field raises a missing-field error instead of calling the source helper.
