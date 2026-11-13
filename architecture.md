# Compiler architecture

`nexgen` turns WIT or JSON Schema input into language-specific source files. The
long-term design keeps one structural API-spec graph throughout the compiler:
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
        selectedValidated[SelectedFamily<br/>validated]

        authoredForBinding -->|AuthoredValidationPass| validated
        validated -->|LanguageSelectionPass| selected
        selected -->|SelectedValidationPass| selectedValidated
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
        pollValidated[PlannedFamily<br/>polls validated]

        operationBoundForPlanning -->|OperationLoweringPass| operationLowered
        operationLowered -->|TypePlanningPass| planned
        planned -->|ReachabilityPass| reachable
        reachable -->|PollValidationPass| pollValidated
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
    selectedValidated --> selectedForBinding
    operationBound --> operationBoundForPlanning
    pollValidated --> reachableForEmission
```

## Compiler stages

### Parsing

The WIT and JSON Schema frontends construct authored IR. The WIT frontend
validates annotations after resolving the input packages. The catalog in
`src/parser/wit/annotations.rs` defines placements, argument names, cardinality,
language eligibility, and System Nexus requirements. Parsing uses that catalog
for common checks; a complete resolved tree walk checks placements, including
unused declarations and linked packages. WIT generation checks availability
against the selected language and mode before lowering. Unqualified annotations
apply to the selected output; language-qualified annotations can remain inactive
for other outputs. Scaffolding only checks vocabulary and placements, without
imposing a generation language or mode. Builders retain semantic checks and
interpretation.

### AuthoredValidationPass

Validates authored API intent and source-format constraints. Native
protobuf-backed enums must match the descriptor's full case list, order, and
numbers; reordered, omitted, and sparse cases are rejected before planning. It
also checks authored service configuration against the requested API targets.
Language-specific policies remain in language backends. After protobuf binding
checks, record-containment validation rejects mixed backing through aliases,
containers, and variants before resource binding or target-specific emission.
Independent operations may use different backing within the same service.
Validation also checks constructor fields of resources returned as plain JSON.

### LanguageSelectionPass

Selects the target-language values from authored language maps.

### SelectedValidationPass

Validates the selected generic model and operation surface after selection.

### ResourceResolutionPass

Resolves resource-method and resource-return facts from the API spec and
descriptors. After resolving method bindings, it infers each resource's type
parameters, including propagation through returned resources to a fixed point.
It validates generic-resource restrictions within this pass, before resolving
return-field bindings. Inferred parameters first appear in
`ResourceBoundFamily::ResourceData`, alongside the resolved resource facts;
authored and selected `ResourceSpec` nodes have no type-parameter field.
Inference also runs for models-only generation.

Operations must return resources owned by their service, including those held in
result records. Return bindings determine shared wire models regardless of API
targets. Method relationships are resolved for inference even in models-only
generation; callable method metadata is retained only when an API target is
selected.

### OperationBindingPass

Attaches resolved return facts to their owning operations and preserves each
resource's resolved data, including its inferred type parameters, through
operation lowering.

### OperationLoweringPass

Turns wire-backed resource returns into explicit result records.

### TypePlanningPass

Materializes target-ready type metadata. Protobuf messages and enums with native
model declarations resolve to ordinary record and enum references unless
replaced by an external type. The native declaration owns its protobuf source;
field wire bindings retain the separate protobuf type. Reachability and model
emission follow native model identity, while language-specific converters use
the wire metadata.

### ReachabilityPass

Removes declarations outside the generated surface.

### PollValidationPass

Refuses a client `@nexus.poll` on an operation that can't get an iterator, or
whose `items` field isn't a list in the model, the same way for every language.

### EmittedNameResolutionPass

Resolves final emitted JSON model identifiers. Its name manifest spans the whole
tree, not one leaf: a `$ref` across input files names a model whose
`x-<lang>-name` override is declared in the other file, so the consuming module
can only resolve it from the tree-wide manifest.

## Models and APIs

Data models and their wire converters are shared across API targets. Operation
wrappers and callable resource classes are target-specific because workflow and
client code use different SDK entry points.

`@nexus.api-omit` on an operation drops that operation's public top-level
function and its export for the named targets, while keeping the operation
registered, in the service, and bindable by resource methods. The private
per-operation helper that resource methods call stays, so the operation is
reached through its resource rather than a standalone function. The parser
records the omitted targets on `OperationSpec`, and each backend gates its own
public surface (the Python `__all__` entry and function, the TypeScript barrel
export, the .NET public options type and method, and the Go exported wrapper).

On a resource method, the parser records the omitted targets on
`ResourceMethodSpec`, and planning keeps them on `PlannedResourceMethod`.
`PlannedResource::methods_for` and `close_for` give each target its methods and
its close. Each backend renders a target's resource class from them, and the
shared request checks skip a method that the target omits.

Python emits shared models and service contracts at the package root, regardless
of API targets. Contracts describe wire data rather than callable resource
handles. Each API owns its operation wrappers and handles. The client API lives
in `client/`; the workflow API lives in `workflow/` when both targets are
selected, or at the root for workflow-only output. Neither API depends on the
other. Plain resource identity DTOs live in the root
`_resources/_resource_data.py` and have no runtime API behavior. Workflow
endpoint clients live in the workflow API's `_clients.py`, not in the shared
`services.py`.

Field policies separate model-level omission from a sparse map of
target-specific origins. `LanguageFieldPolicies` retains language alternatives
until selection; the selected `FieldPolicy` survives shared planning. Its
accessors resolve absent target entries to `FieldOrigin::Parameter`. An explicit
origin can instead omit the convenience argument, default a caller-overridable
argument, or source its value. The WIT parser lowers annotations into these
policies; later stages do not interpret annotation syntax. API preparation
selects its target's origin when building a request. Model constructors and
converters only handle data, preserving declared field optionality. This lets
workflow and client APIs share models without sharing runtime behavior.

Client resources keep their service-bound Nexus client. API code decodes
returned resource data and constructs handles with that client. Python client
APIs use the SDK's `NexusClient` and the shared typed operation descriptors.
