# Compiler output test follow-ups

Follow-up inventory after PR #316. Keep these three groups separate from the
larger, mostly untouched JSON Schema source-assertion inventory.

- [x] Group 1: replace the three synthetic callable tests with sample coverage.
- [x] Group 2: remove obsolete embedding/copy checks; retain CLI contracts.
- [ ] Group 3: review small backend renderer tests.

## 1. Go callable tests in native validation — implemented locally

Removed all three tests and their unused fixture/compiler helpers. The existing
`function-execution` sample now covers both secondary-group orders, inferred
string-or-callable groups, primary-last variadics, and trailing secondary
variadics with fixed or explicitly described primaries.

`TestSecondaryCallableGroupsAndTrailingVarargs` invokes the generated APIs and
checks the delivered names and argument groups. A Python runtime regression also
covers the overload separator bug discovered by the fixed-primary scenario.

### Original investigation

Location: `validation-tests/tests/go.rs`.

These tests generate synthetic WIT fixtures, inspect source, and call
`compile_go_descriptor_fixture`. That helper runs `go test ./...`, but creates
no Go test functions or callers of the generated wrappers: this is compilation
coverage, not runtime argument-packing coverage.

- `go_explicit_descriptors_control_signal_api_and_packing_independent_of_wire_name`:
  checks a 2x2 matrix of argument-field names and explicit argument types.
  Existing `workflow-service` covers a string descriptor with a single `any`
  argument; `function-execution` covers explicit scalar/slice descriptor
  arguments, nonstandard argument-field names, and packing a slice as one
  application value. Their native tests exercise the resulting requests. The
  exact string-descriptor/slice-argument combination is not in a sample, but the
  asserted rules are covered collectively. Recommend removing this duplicate
  matrix without reproducing every combination.
- `go_primary_variadic_group_is_last_and_secondary_groups_keep_authored_order`:
  existing `function-execution` covers primary-last ordering, multiple secondary
  groups, and packing. Its secondary groups use explicit descriptors, whereas
  this fixture covers inferred string-or-callable secondary groups, reflection
  fallback, and swapping their authored order. Preserve that remaining scenario
  through a small extension of the existing sample, rather than source checks.
- `go_final_secondary_signature_args_are_variadic_without_primary_signature_varargs`:
  neither current sample covers a trailing secondary variadic group when the
  primary group has fixed arguments or an explicit nonvariadic descriptor. Those
  two branches remain unique coverage. Exercise the generated calls in the
  existing sample before removing the synthetic fixtures.

Relevant native sample checks:

- `advanced/samples/go/tests/model_and_callable_shapes_test.go`:
  `TestCompositeDescriptorsAndArgumentPacking`.
- `advanced/samples/go/tests/workflowservice_test.go`: signal argument delivery
  and registry/context integration.
- `advanced/samples/go/tests/functionexecution_test.go`: callable forms, name
  collisions, and output transforms.

Do not change `workflow-service` to add artificial scenarios.

## 2. Retained WIT and CLI output checks

- Ordinary-mode serialization-context conversion in `tests/generate_go.rs`.
  Deliberately retained; its TODO says to remove it with full Go
  signal-with-start support.
- Go self-import CLI test: removed. SDK embedding integration coverage belongs
  in the Go SDK itself.
- Go WIT service named `support`: removed without replacement; the copied
  support-file mechanism that motivated this regression test no longer exists.
- CLI support path/namespace propagation, default generation mode, formatter
  invocation, overwrite behavior, and version headers. These are intentional CLI
  contracts; do not remove them merely because they inspect output.

## 3. Small backend renderer tests

Reviewed after creating stacked PR #317; no group 3 tests changed yet.

- The Go System Nexus factory self-import test constructs SDK-embedding
  destinations. Remove for the same ownership reason as the CLI embedding test.
- The TypeScript `Long` import rendering is covered by the compiled
  `stream-service` sample.
- Python and TypeScript native/SDK enum defaults are already represented in
  `proto-backed-showcase` and `workflow-service`. TypeScript's structured
  namespace-import requirement assertions are distinct from expression strings.
- Go's private getter exclusion is not represented by current authored samples.
  Prefer testing the owning field-selection rule rather than occurrence counts
  and method-body substrings.
- Source/helper qualification tests combine covered basic calls with extra
  cases: argument-bearing calls, already-qualified names, and inline
  expressions. Those extra branches are not all present in WIT samples. Do not
  mechanically replace their small helper contracts with a new compiler
  abstraction.

- Go getter occurrence counts/private-field exclusion and qualified factory/type
  expressions in `src/generator/go.rs`.
- Python source-provider/default expressions and enum-default rendering in
  `src/generator/python.rs`.
- TypeScript requirement-import rendering, helper qualification, and enum
  defaults in `src/generator/typescript.rs`.
- .NET sourced-helper qualification in `src/generator/dotnet.rs`.

Distinguish a renderer's small output contract from a compiler decision that can
be tested through existing structured data. Do not mechanically replace every
string equality or create new abstractions solely to accommodate tests.
