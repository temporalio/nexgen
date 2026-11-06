The Python `--system-nexus` typed hook of an operation with a generic response takes the caller's `output_type`, so the response decodes with its type arguments.
The .NET `--system-nexus` dispatch sends an operation with a generic request or response to the generic hook with the types of the call. Before, it refused each type argument other than `object`.
The Python and TypeScript `--system-nexus` interceptors type-check for an operation with a generic model.
