Resource methods in Go, TypeScript, and .NET use their own identity fields instead of evaluating `@nexus.source` for those request fields.
Go APIs populate sourced fields for non-protobuf requests.
Definitions-only generation omits Go workflow APIs and supports Python operations returning protobuf-backed resources.
