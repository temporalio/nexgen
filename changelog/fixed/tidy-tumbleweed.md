Go output with no `--targets` no longer has the Workflow operation code, such as the `workflow.NewNexusClient` calls and the `*Options` wrappers. The file no longer imports the packages that only this code used.
Go operations whose request is not a proto message now fill the `@nexus.source` fields.
