Go output with no `--targets` no longer has the Workflow operation code, such as the `workflow.NewNexusClient` calls and the `*Options` wrappers. The file no longer imports the packages that only this code used.
Go operations whose request is not a proto message now fill the `@nexus.source` fields.
Python generation with no `--targets` no longer fails on an operation that returns a proto-backed resource.
A resource method sends its own value for a request field that also has a `@nexus.source`, as Python did. Go, TypeScript and .NET called the source instead.
