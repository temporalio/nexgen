# Compiler architecture

`nexgen` turns WIT or JSON Schema input into language-specific source files.
The long-term design keeps one structural API-spec graph throughout the compiler:
`ApiSpec<F>`. A type family `F` changes the metadata attached to otherwise
equivalent services, operations, fields, and type references as passes enrich
the graph.

## Flow

The compiler has an explicit spine. Parsing creates authored IR; selection
collapses language-specific values once; the remaining passes enrich the same
structural `ApiSpecTree` family. Each pass returns a complete IR for its
successor; no pass receives planner state saved by an earlier pass.

Parsers mark public roots on the corresponding neutral type-declaration entries.
The WIT frontend marks types owned by operation-free exported interfaces, while
the JSON Schema frontend marks the root model, `$defs`, and inline operation
models owned by each source. The metadata stays with each declaration through
family transformations and supplies reachability roots; later passes do not
inspect the input format to reconstruct them.

Native WIT declarations own their external protobuf source metadata. External
type bindings are reserved for non-native WIT overrides and JSON Schema models;
the protobuf planning helpers resolve source metadata from either form without
making later passes depend on the input format.

Each IR box in the diagram names the `TypeFamily` parameter of the
`ApiSpecTree<F>` produced at that point. A family may appear more than once
where the same tree crosses a logical phase boundary or a pass refines its
contents without changing its family.

```mermaid
flowchart LR
    subgraph parse["Input & Parsing"]
        direction TB
        input[WIT or JSON Schema input]
        authored[AuthoredFamily]

        input -->|Detect format and parse WIT| authored
        input -->|Detect format and parse JSON Schema| authored
    end

    subgraph select["Validation & Selection"]
        direction TB
        authoredForBinding[AuthoredFamily]
        validated[AuthoredFamily<br/>validated]
        selected[SelectedFamily]

        authoredForBinding -->|AuthoredValidationPass| validated
        validated -->|LanguageSelectionPass| selected
    end

    subgraph bind["Resource & Operation Binding"]
        direction TB
        selectedForBinding[SelectedFamily]
        resourceBound[ResourceBoundFamily]
        operationBound[OperationBoundFamily]

        selectedForBinding -->|ResourceResolutionPass| resourceBound
        resourceBound -->|OperationBindingPass| operationBound
    end

    subgraph plan["Lowering & Type Planning"]
        direction TB
        operationBoundForPlanning[OperationBoundFamily]
        operationLowered[OperationLoweredFamily]
        planned[PlannedFamily]
        reachable[PlannedFamily<br/>reachable]

        operationBoundForPlanning -->|OperationLoweringPass| operationLowered
        operationLowered -->|TypePlanningPass| planned
        planned -->|ReachabilityPass| reachable
    end

    subgraph emit["Emission"]
        direction TB
        reachableForEmission[PlannedFamily<br/>reachable]
        generatorReady[PlannedFamily<br/>generator-ready]
        output[Generated files]

        reachableForEmission -->|EmittedNameResolutionPass| generatorReady
        generatorReady -->|Language backend| output
    end

    authored --> authoredForBinding
    selected --> selectedForBinding
    operationBound --> operationBoundForPlanning
    reachable --> reachableForEmission
```

## Passes

- `AuthoredValidationPass` validates authored API intent and source-format
  constraints.
- `LanguageSelectionPass` selects the target-language values from authored
  language maps.
- `ResourceResolutionPass` resolves resource-method and resource-return facts
  from the API spec and descriptors.
- `OperationBindingPass` attaches resolved resource facts to their owning
  operations and resources.
- `OperationLoweringPass` turns wire-backed resource returns into explicit
  result records.
- `TypePlanningPass` materializes target-ready type metadata.
- `ReachabilityPass` removes declarations outside the generated surface.
- `EmittedNameResolutionPass` resolves final emitted JSON model identifiers. Its
  name manifest spans the whole tree, not one leaf: a `$ref` across input files
  names a model whose `x-<lang>-name` override is declared in the other file, so
  consuming module can only resolve it from the tree-wide manifest.

## Go operation serialization contexts

Serialization-context helper selection remains operation metadata through the
existing pipeline. The Go backend owns helper-reference validation, registry
emission, helper imports, and conversion-context rendering. No new compiler pass
or per-model policy validation is required. Operations that share a model, alias,
or protobuf identity can select different helpers or omit them. The backend adds
no request methods or provider interface assertions and permits externally owned
proto-backed request types. Selected helpers for non-proto or JSON requests
remain unsupported.

If a rendered system-endpoint operation selects a Go helper, the backend emits
the registry at the end of the generated API file under a `Registry` header.
An explicit system endpoint does not require the
`system_nexus` flag. Ordinary-only annotations do not create global policy entries.
SDK-owned scope removes the need for a generated `serialization_context.go`
runtime file. The registry includes rendered system-endpoint operations in that
package, not just annotated operations, and excludes ordinary-endpoint operations.
The map uses `internal.NexusOperationKey{Service, Operation}` wire names and
`internal.NexusOperationRegistryEntry` values directly; duplicate keys are
rejected. Each entry stores
`SerializationContext func(any) converter.SerializationContext`, with nil for
operations without a selected helper, and an optional `InputToTransfer` callback
for external native inputs without a transfer converter. Package `init` calls
`internal.RegisterNexusOperationRegistry`. The SDK retains the map and rejects
keys already registered by another package; entries must remain immutable once
registered, except in sequential tests with no concurrent lookup or execution.
Qualified helper imports are resolved from support imports or known model
imports and merged into the generated API file's imports.

Annotated entries inline `InputType: reflect.TypeFor[RequestType]()` and
a serialization-context callback that calls `helper(request.(RequestType))`.
Before invoking either callback, the SDK
checks input compatibility when `InputType` is non-nil. Nonmatching wire inputs
skip policy selection and external conversion; a nil `InputType` disables that
guard. The helper receives the native request value. This supports concrete
serialization-context return types and external request types without attaching
policy to either type.

The SDK consumes the registered metadata and owns conversion scope:

1. A generated wrapper assigns sourced fields and passes the native input to
   `ExecuteOperation`. Workflow interceptors can replace the context or input
   before selection.
2. The SDK looks up the final wire service and operation only for endpoint
   `__temporal_system` or legacy `temporal-system`. Other endpoints retain existing
   behavior even if the package emitted a registry without `system_nexus`.
3. The SDK invokes `SerializationContext` once with the post-interceptor native
   input when it matches the entry's optional `InputType`. A missing entry or
   nonmatching type preserves existing conversion. A nil callback or nil result
   preserves caller scope without skipping `InputToTransfer`. Otherwise, the SDK
   scopes the root data and failure converters for
   the target and installs the inner data converter in transfer callback scope.
   The outer envelope keeps its Nexus-scoped converter.
4. For external native inputs without a transfer converter, the SDK calls
   `InputToTransfer` in the selected inner scope before encoding the envelope,
   or in caller scope if no context was selected. Ordinary-endpoint wrappers
   preserve eager external-input conversion without registering a global policy.
   Generated transfer callbacks use the SDK-supplied context directly; there is
   no generated context key or model policy method.
5. The SDK future retains the inner scope for result transfer conversion, and the
   operation retains its selected failure converter. Eager output adapters,
   output transforms, and resource-return conversion recover the captured scope
   with `internal.NexusOperationPayloadContext(ctx, fut)`. Selection is not
   repeated at `Get`, even if its context differs. The helper recognizes an
   optional `NexusOperationPayloadContext() workflow.Context` carrier method.
   Interceptor future wrappers must explicitly forward that method for eager
   adapters; arbitrary wrappers do not forward scope automatically. The public
   `workflow.NexusOperationFuture` interface is unchanged. Without a carrier or
   with a nil carried context, the helper returns its supplied context.

Direct `NexusClient.ExecuteOperation` calls use the same path when the generated
package has been imported, registration has run, and the call supplies a system
endpoint, registered wire key, and expected native model. Raw callers supply
sourced fields themselves when using native models. Raw protobuf inputs remain
compatible: the generated `InputType` guard skips both callbacks for nonmatching
wire types instead of passing them to a native-model helper.

The internal registry and future-context APIs require a compatible SDK checkout
and restrict generated integration to SDK-hosted packages. This is not full
Python parity: Go's existing outer envelope remains Nexus-scoped, with no special
envelope bypass, system-envelope marking, or codec-visitor parity. The SDK selects
the target failure converter, but authored nested-failure helpers still construct
a default failure converter with the scoped data converter instead of using the
configured failure converter.
