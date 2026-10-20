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
    selectedValidated --> selectedForBinding
    operationBound --> operationBoundForPlanning
    reachable --> reachableForEmission
```

## Passes

- `AuthoredValidationPass` validates authored API intent and source-format
  constraints.
- `LanguageSelectionPass` selects the target-language values from authored
  language maps.
- `SelectedValidationPass` validates the selected generic surface after
  selection.
- `ResourceResolutionPass` resolves resource-method and resource-return facts
  from the API spec and descriptors.
- `OperationBindingPass` attaches resolved resource facts to their owning
  operations and resources.
- `OperationLoweringPass` turns wire-backed resource returns into explicit
  result records.
- `TypePlanningPass` materializes target-ready type metadata, including resolved
  enum defaults on record fields. Defaults identify a member of a declared enum
  or a descriptor-backed enum, following aliases; language backends render that
  member using the API type's naming and conversion conventions rather than
  resolving the authored case again.
- `ReachabilityPass` removes declarations outside the generated surface.
- `EmittedNameResolutionPass` resolves final emitted JSON model identifiers. Its
  name manifest spans the whole tree, not one leaf: a `$ref` across input files
  names a model whose `x-<lang>-name` override is declared in the other file, so
  the consuming module can only resolve it from the tree-wide manifest.

## Java emission

Native Java operation rendering honors the selected operations-class name.
Explicit names produce an inheritable generated base for SDK facades, allowing
new operations to become available without editing the facade. Package placement
is controlled by the Java package option; request builders carry API options.

The Java backend renders native declarations and external protobuf bindings from
`PlannedFamily`, alongside its existing JSON Schema renderer. Java-specific type
mapping, descriptor-based protobuf class naming, and runtime helpers belong to
`src/generator/java.rs`. JSON Schema orchestration and rendering live in
`src/generator/json_schema/java.rs`; the formatter is a child module under
`src/generator/java/`. No additional compiler pass is required.

The Java renderer receives the existing descriptor index directly for protobuf
accessor names, collection shapes, and numeric widths. It emits typed protobuf
getters/setters and ordinary Java collection conversions. Runtime requirements
are tracked locally during Java rendering; only referenced helpers and value
types are emitted. Converter scoping belongs to SDK internals; generated
transfer converters obtain the active converter through the configured support
class. This adds no planning metadata or compiler pass.

System Nexus generation also emits `system-nexus-roots.txt`. Its entries come
from the protobuf transfer types of operation inputs and outputs, including
payload-free responses. SDK build-time visitor generators consume this manifest
without maintaining a separate list of operation message names.

## Java formatting

Java formatting runs after emission when requested. The Java backend owns the
pinned google-java-format invocation and tool cache. It receives the generated
file manifest so formatting leaves neighboring handwritten files untouched.
Sample regeneration uses the same generation-time formatting path.
