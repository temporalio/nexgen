# Operation-scoped defaults

[`inputs/defaults.wit`](inputs/defaults.wit) exercises target-language
expressions that initialize operation inputs. Its native tests inspect the
request passed to the SDK, not generated source text.

## Timing and presence

Defaults run while preparing each outbound operation request, before the SDK
call. They run only for missing fields, once per invocation.

Python uses `None`, TypeScript uses null or undefined, and .NET uses nullable
options. Explicit null also requests the default. These languages preserve
supplied empty strings, false, numeric zero, and empty collections. Python
evaluates native parameter-default expressions when defining a function, so
generated operation parameters use `None` and evaluate authored expressions
inside the async body instead.

Go keeps its public default-punning policy. Empty strings, false, and numeric
zero select defaults on scalar options. Non-nil internal pointers preserve
presence, including pointers to zero. Only nil slices and maps select defaults;
non-nil empty collections remain empty. A defaulted required record uses a
pointer on the options surface and a value on the initialized request.

Model constructors, encoding, and inbound decoding never evaluate providers. The
same record is used for both input and output in this sample to exercise that
directional distinction. Reusing options for another call recalculates missing
context-dependent defaults without modifying those options.

## Input omission and model nullability

A defaulted operation argument must represent omission, but that does not
require the initialized model field to be nullable. For
`alias-label: aliased-string`, the operation accepts a nullable input, selects
`"alias"` when it is null, and constructs a model whose field is a required
string. It never treats an empty string as missing outside Go.

`@nexus.default` is rejected on `option<T>` fields, including aliases to option
types. The separate operation input surface expresses the precise contract: the
caller may omit a value that the initialized request must contain.

Python, TypeScript, and .NET expose `RecordWithDefaultsInput` for partial nested
values. Its `attempts` and `label` fields may be omitted; the initialized
`RecordWithDefaults` requires both. Python and TypeScript also accept complete
models directly. In .NET, `new RecordWithDefaultsInput(model)` copies a complete
model into the input surface without evaluating providers. Go uses the model
struct with its scalar zero-punning convention.

## Cases in the input

- `namespace` reads workflow context only if omitted.
- `attempts`, `enabled`, and `ratio` expose scalar presence and Go punning.
- `mode`, `permissions`, and `alias-label` exercise `my-enum`, `my-flags`, and
  `aliased-string`.
- `retry` stays absent when omitted. Its child defaults run only if present.
- `metadata` is flattened. No promoted arguments means no parent, despite its
  summary default; supplying details establishes a parent and defaults summary.
- `sender.submit` forwards to an operation with the same defaults.

Collections are initialization boundaries: records inside lists and maps are
passed through without recursively applying defaults. Direct record fields, such
as the optional `retry: option<record-with-defaults>`, are initialized when
present. The flattened `metadata` field uses `flattened-record-with-defaults`.

## Expression scope

Expressions follow ordinary target-language name resolution. Nexgen does not
prefix them, resolve enum cases, or infer arbitrary dependency imports.

Use a generated API type's emitted public name and enum/flag value names.
External or replaced API types use their target-language `@nexus.type` reference
and declared imports. Support calls explicitly use `_support` in Python,
`support` in Go and TypeScript, or the configured support namespace in .NET.
This sample uses the existing `TemporalWorkflowContext` support class in .NET.
Go expressions can reference the operation context as `ctx`.

The language compiler or checker diagnoses invalid expressions, undefined
symbols, and incompatible values. Nexgen checks annotation structure and
incompatible directives, not the expression language.

## .NET shape

The generated operation accepts nullable options, then constructs an initialized
request with required values. Its constructor arguments include expressions such
as:

```csharp
options?.Namespace ?? TemporalWorkflowContext.WorkflowNamespace()
options?.Attempts ?? 3u
```

`?.` reads the property only when options is not null. `??` evaluates its right
side only when the left side is null. `3u` is the unsigned integer literal 3.
Thus an explicit empty namespace or zero attempts remains unchanged in .NET.

Present nested input objects are converted to complete models; absent optional
parents remain null. No provider is placed in an options constructor, model
constructor, or property initializer.

## Migrating enum-case defaults

The former `@nexus.default "allow-duplicate"` in
[`workflow-service.wit`](inputs/workflow-service.wit) now spells its native API
member separately for each target, such as
`python="temporalio.common.WorkflowIDReusePolicy.ALLOW_DUPLICATE"` and
`dotnet="Temporalio.Api.Enums.V1.WorkflowIdReusePolicy.AllowDuplicate"`.

The former `@nexus.default "namespace-state-registered"` in
[`proto-backed-showcase.wit`](inputs/proto-backed-showcase.wit) instead names
the generated API enum member, such as
`python="NamespaceState.NamespaceStateRegistered"` and
`go="NamespaceStateNamespaceStateRegistered"`.

These defaults no longer make model fields nullable or substitute values during
conversion. Required API enums preserve the protobuf zero member when decoding
zero; only outbound operation omission selects the authored default.
