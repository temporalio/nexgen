# WIT Author's Guide to nexgen

This guide explains what the code generator produces from your WIT definitions,
with examples in Python and TypeScript. It covers every type mapping, the
resource name-binding mechanism (including edge cases), and a complete glossary
of `@nexus` directives.

## Contents

- [Quick Start](#quick-start)
- [Integrating a Support Package](#integrating-a-support-package)
- [Type Mappings](#type-mappings)
  - [Records](#records)
  - [Enums](#enums)
  - [Flags](#flags)
  - [Variants](#variants)
  - [Results](#results)
  - [Tuples](#tuples)
  - [Option Types](#option-types)
  - [Lists and Maps](#lists-and-maps)
- [Resources](#resources)
  - [Resource Declaration](#resource-declaration)
  - [Name-Binding: How Resource Methods Find Operations](#name-binding-how-resource-methods-find-operations)
  - [Resource Return Binding](#resource-return-binding)
  - [Closing a Resource](#closing-a-resource)
  - [Variadic Items](#variadic-items)
  - [Edge Cases and Error Conditions](#edge-cases-and-error-conditions)
- [Proto-Backed Models](#proto-backed-models)
  - [Transfer-Type Conversion](#transfer-type-conversion)
  - [Sourced Fields](#sourced-fields)
  - [Omitted Fields](#omitted-fields)
  - [Flattened Records](#flattened-records)
  - [Output Transforms](#output-transforms)
- [Function References](#function-references)
- [Generation Targets](#generation-targets)
- [Directive Glossary](#directive-glossary)

---

## Quick Start

A WIT file is the source of truth for a Nexus service's public API. The
generator reads it and produces language-specific client code: data models,
service definitions, and convenience wrappers that let callers invoke Nexus
operations from Temporal workflows.

Minimal example:

```wit
package temporal:user-service@1.0.0;

world system {
  export user-service;
}

/// @nexus.endpoint "user-service"
interface user-service {
  record get-user-request {
    user-id: string,
  }

  record user-response {
    user-id: string,
    email: string,
  }

  get-user: func(request: get-user-request) -> user-response;
}
```

This produces a data model for the request and response, a service definition,
and a convenience wrapper function that lets callers write:

**Python:**

```python
user = await get_user(user_id="abc")
```

**TypeScript:**

```typescript
const user = await getUser({ userId: "abc" });
```

An exported interface with no operations or resources acts as a model-only
module. Its declared types remain public, but the generator does not emit an
empty service definition or client. The interface still names the generated
output where a target would otherwise use the service: its `@nexus.namespace`
directive selects the .NET namespace or Go import path, and .NET otherwise uses
`Nexgen.<InterfaceName>`, such as `Nexgen.NotificationService` for
`interface notification-service`.

.NET `@nexus.namespace` and `@nexus.operations-class` overrides are reserved for
SDK integration and require `--system-nexus`. Ordinary .NET WIT output uses the
derived namespace and the `Operations` class. Go namespace overrides and the C#
JSON Schema `--namespace` option are unaffected.

---

## Integrating a Support Package

For **WIT** generation in Go, Python, TypeScript, and .NET, pass one
`--support-package` value on the language subcommand, even if that particular
input does not use support helpers. It identifies code
**you maintain in your project**; nexgen neither reads it nor copies it into the
output. Java does not consume user support, and JSON Schema generation does not
require this option (its generated JSON runtime helpers are unrelated).

The package must be importable when the generated code is compiled or run.
Nexgen does not verify that it exists or that its exports have the required
signatures: use the language's compiler or type checker to check the
integration. The WIT directives name some helpers explicitly (for example
`python-from` on `@nexus.type` and `typescript-converter` on `@nexus.function`);
for other protobuf conversions nexgen derives names from the proto message.
Check the calls in generated code for the exact exports and signatures your
support package needs. Keep authored support code outside directories that you
replace when regenerating output.

Each `@nexus.source` value is a call to a helper in the support package. Write
the helper path and its arguments, without the package prefix. Nexgen imports
support and adds the prefix to the helper call. It does not add or change the
arguments. For example, write `workflow_namespace()` in Python,
`workflowNamespace()` in TypeScript, `WorkflowNamespace(ctx)` in Go, or
`TemporalWorkflowContext.WorkflowNamespace()` in .NET. Generated code calls
`_support.workflow_namespace()`, `support.workflowNamespace()`,
`support.WorkflowNamespace(ctx)`, or
`Nexgen.Support.TemporalWorkflowContext.WorkflowNamespace()` for those examples.
`@nexus.output-transform` remains a separate authored expression.

### Go

Normally, create a separate Go package in a directory outside the generated
output, in a module that the generated code can import. Pass its full Go import
path, not the package name or a `.go` file path. For the sample
[`go.mod`](advanced/samples/go/go.mod) and
[`support/`](advanced/samples/go/support/), that value is
`go.temporal.io/sdk/advanced/samples/go/support`; generated code imports it as
`support`. For example:

```sh
cargo run --features advanced -- go advanced/samples/inputs/proto-backed-showcase.wit \
  advanced/samples/inputs/deps \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --support-package go.temporal.io/sdk/advanced/samples/go/support \
  --targets workflow --output advanced/samples/go/protobackedshowcase
```

Export converter functions from the support package. For
`google.protobuf.Duration`, nexgen derives `support.DurationToProto(ctx, value)`
and `support.DurationFromProto(ctx, value)` when WIT does not specify names.
When WIT sets `go-to` or `go-from` on `@nexus.type`, nexgen uses each given name
exactly. Imported converters must be exported. Their parameters and
`(value, error)` results must match the generated call. See
[the sample converters](advanced/samples/go/support/temporal_model_converters.go).
Do not import the generated package from support. That import creates a cycle.
The sample converters import `go.temporal.io/sdk/internal`. Only packages under
the SDK module tree can import it. If your module is elsewhere, use dependencies
that your module can access. Do not copy the sample file without changing that
import.

For `@nexus.source`, the sample writes `WorkflowNamespace(ctx)`. Generated code
calls `support.WorkflowNamespace(ctx)`. The support package implements it with
`workflow.GetInfo(ctx).Namespace`. The generated Go package imports support for
the sourced field. Verify with `go test ./...` from the module root.

Support can also share the generated package. Set `--support-package` and
`@nexus.namespace go="..."` to the same import path; nexgen omits the
self-import, so support helpers need not be exported. Authored files alongside
generated files are preserved.

With `--system-nexus`, Go bindings use SDK `internal` APIs and prefix
package-level declarations with `SystemNexus_` (exported) or `systemnexus_`
(private). To generate into the SDK's `internal` package, set
`@nexus.namespace go="go.temporal.io/sdk/internal"`.

Workflow System Nexus generation emits an operation-specific outbound interface,
forwarding implementations, and terminal implementations. Public functions apply
authored defaults and resolve function references before calling the SDK's typed
interceptor chain or, in standalone packages, the generated terminal directly.
Interceptor methods receive the public options and positional arguments, with
resolved function names in place of function references, and return a workflow
`Future`. The terminal constructs the request from the intercepted arguments,
evaluates sourced fields, schedules the Nexus operation, and adapts the result.
It does not reapply defaults or re-enter the typed interceptor chain. No
service-name dispatcher is needed.

Ordinary Go public functions construct requests and inline execution and result
adaptation. Neither mode generates private operation helpers.

Go resource methods construct requests from their bound resource fields and
method arguments, then inline the same execution and result adaptation. They
preserve resource-owned values even when the corresponding public function
sources those fields from the workflow context. Resource methods use the
low-level Nexus client path, including generic Nexus interceptors, rather than
the free function's typed interceptor hook in System Nexus mode. The existing
`start-workflow`, `user-service`, and `defaults` samples exercise resource
execution, resource results, namespace precedence, defaults, optional values,
and failures.

For generation into `go.temporal.io/sdk/internal`, nexgen emits methods directly
on `WorkflowOutboundInterceptorBase` and `workflowEnvironmentInterceptor`. The
base method forwards to `Next`; the terminal method constructs the request,
calls `wc.ExecuteNexusOperation` directly, and adapts the result. There are no
extra adapter functions or handwritten methods. The SDK embeds the generated
interface once; its existing `Next` field remains the only forwarding state.

Standalone packages emit free functions named
`SystemNexus_WorkflowOutboundInterceptorBase_<Operation>` and
`SystemNexus_WorkflowEnvironmentInterceptor_<Operation>`, together with the
private interceptor interface. The forwarding function takes the next typed
interceptor explicitly; the terminal function takes an `ExecuteNexusOperation`
function explicitly. They use the same request and result renderers as the SDK
methods, without generated test-only base, terminal, or runtime types.

Standalone public functions call the generated terminal with an inline callback
that delegates execution to the input's Nexus client. No execution helper is
required in the support package. Tests can instead supply the next interceptor
or execution function directly. There is no context installer or scheduling
fallback, and facades do not re-export interceptor helpers. The Go
`workflow-service` and `multi-operation-service` samples exercise typed calls,
forwarding, short-circuiting, and terminal scheduling. SDK tests cover
public-function entry into the SDK's interceptor chain.

The terminal uses default Nexus settings. With
[`@nexus.context-propagation`](#nexuscontext-propagation), SDK generation
initializes workflow headers before interception and injects configured context
propagators afterward. Without that directive, it does neither. SDK interceptor
generation requires concrete types because Go methods cannot declare type
parameters.

Go System Nexus operation requests use default-punning for optional scalar-like
fields, matching the options surface: zero durations and numbers, false, empty
strings, and zero enum or flag values represent omission. Encoding omits those
values, and decoding maps absent fields to their zero value. Requests cannot
distinguish an explicitly supplied zero from an omitted field. Optional structs
and nested record fields retain their existing presence-bearing representation.
Authored defaults apply when preparing public function calls or resource-method
requests, not in interceptors or encoding/decoding.

### Python

Place an importable Python package in your project and pass its module name, not
a filename. The sample's
[`temporal_support/`](advanced/samples/python/temporal_support/) is importable
as `temporal_support` from the sample project root. Its `__init__.py` exposes
the converter functions needed by generated code; merely putting them in a
submodule is not enough if they are not re-exported. Generated code imports
`temporal_support as _support` where needed and calls functions such as
`_support.retry_policy_to_proto(value)` or `_support.payloads_to_proto(values)`.

```sh
cargo run --features advanced -- python advanced/samples/inputs/proto-backed-showcase.wit \
  advanced/samples/inputs/deps \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --support-package temporal_support \
  --targets workflow --output advanced/samples/python/wit/proto_backed_showcase
```

Your runtime must also be able to `import temporal_support` (for example,
install the package or make its parent directory available on Python's import
path). The sample tests set `pythonpath = ["."]` in
[`pyproject.toml`](advanced/samples/python/pyproject.toml); from the sample
directory, `uv run python -c 'import temporal_support'` checks resolution.
Default converter names use proto-message snake case, such as
`duration_from_proto` / `duration_to_proto`; user-authored names in WIT must
also be exported. Keep support independent of generated models to avoid circular
imports.

For `@nexus.source`, the sample writes `workflow_namespace()`. Generated code
calls `_support.workflow_namespace()`. Check the generated project with
`ruff check`, a Python type checker, and your tests.

### TypeScript

Export helpers from a TypeScript module and pass either an installed package
specifier (for example `@my-org/nexgen-support`) or a path
**relative to the output directory**, not the directory in which you run the
CLI. For the sample output root
`advanced/samples/typescript/wit/proto-backed-showcase`, the sample module is
`../../support/temporal_model_converters`; generated `models.ts` imports it as
`support`. Nexgen adjusts that relative specifier for nested files such as
`operations/<operation>.ts`.

```sh
cargo run --features advanced -- typescript advanced/samples/inputs/proto-backed-showcase.wit \
  advanced/samples/inputs/deps \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --support-package ../../support/temporal_model_converters \
  --targets workflow --output advanced/samples/typescript/wit/proto-backed-showcase
```

Export the exact WIT-named or derived converter functions, for example
`durationFromProto` / `durationToProto`. Generic payload and function
conversions can also call `valueToPayload`, `payloadToValue`,
`payloadsFromProto`, `payloadsToProto`, and `functionInputTypes`. See
[`support/temporal_model_converters.ts`](advanced/samples/typescript/support/temporal_model_converters.ts)
for their contracts. For `@nexus.source`, write `workflowNamespace()`. Generated
code calls `support.workflowNamespace()`. Include both the support module and
generated output in your project. Run the project's TypeScript type checker (the
sample uses `npm run typecheck`).

Validation helpers such as `requiredField` are generated privately in each
module that needs them. They are not exported and do not belong in your support
module.

### .NET

Pass the C# **namespace**, not a source path or assembly name:
`--support-package Nexgen.Support` for the sample. Put hand-written `.cs` files
such as
[`TemporalSupport.cs`](advanced/samples/dotnet/TemporalSupport/TemporalSupport.cs)
and
[`WorkflowServiceSupport.cs`](advanced/samples/dotnet/TemporalSupport/WorkflowServiceSupport.cs)
outside regenerated output and compile them into the **same project/assembly**
as the generated files. This is why the sample's `internal` helpers work; simply
referencing a separate support DLL would not give the generated code access to
those members. The support namespace need not equal the generated models'
namespace. If your `.csproj` disables default compile items or excludes
directories, explicitly include the authored sources (see
[`Nexgen.DotNetMultiOperationService.csproj`](advanced/samples/dotnet/Nexgen.DotNetMultiOperationService.csproj)).

```sh
cargo run --features advanced -- dotnet advanced/samples/inputs/proto-backed-showcase.wit \
  advanced/samples/inputs/deps \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --support-package Nexgen.Support \
  --targets workflow --output advanced/samples/dotnet/wit/proto-backed-showcase
```

Generated calls qualify helper names with `Nexgen.Support`; where extension
methods such as `ToProto()` need it, generated modules also use the support
namespace. Provide `ProtoExtensions.FromPayload`, `FromPayloads`, `ToPayload`,
and `ToPayloads` for the generic payload carriers, along with any converters and
serialization-context helpers named in your WIT. For `@nexus.source`, write
`TemporalWorkflowContext.WorkflowNamespace()`. Generated code adds the support
namespace to the call. Verify that the project compiles with `dotnet build`.

---

## Type Mappings

### Records

WIT records become dataclasses (Python) or interfaces (TypeScript).

```wit
record postal-address {
  street: string,
  city: string,
  country: string,
}
```

**Python:**

```python
@dataclasses.dataclass(slots=True)
class PostalAddress:
    street: str
    city: str
    country: str
```

**TypeScript:**

```typescript
export interface PostalAddress {
  street: string;
  city: string;
  country: string;
}
```

TypeScript always emits a companion `const` alongside the interface. For
WIT-direct records it is empty; for proto-backed records it gains `fromProto()`
and `toProto()` methods.

### Enums

WIT enums become integer-valued enumerations.

Enums with `@nexus.proto` must declare every protobuf case in descriptor order,
using the protobuf member name in kebab-case (for example, `severity-low` for
`SEVERITY_LOW`). Protobuf numbers must match WIT's zero-based declaration
indices. Generation fails for missing or reordered cases and for gaps in
protobuf numbering; omitting individual enum cases is not supported.

```wit
enum user-status {
  active,
  suspended,
  deleted,
}
```

**Python:**

```python
class UserStatus(enum.IntEnum):
    Active = 0
    Suspended = 1
    Deleted = 2
```

**TypeScript:**

```typescript
export enum UserStatus {
  Active = 0,
  Suspended = 1,
  Deleted = 2,
}
```

### Flags

WIT flags become bit-shifted integer constants.

```wit
flags user-capability {
  read-profile,
  update-email,
  deactivate,
}
```

**Python:**

```python
UserCapability: typing.TypeAlias = int
UserCapabilityReadProfile = 1 << 0
UserCapabilityUpdateEmail = 1 << 1
UserCapabilityDeactivate = 1 << 2
```

**TypeScript:**

```typescript
export enum UserCapability {
  ReadProfile = 2 ** 0,
  UpdateEmail = 2 ** 1,
  Deactivate = 2 ** 2,
}
```

### Variants

Python represents WIT variants as tagged tuples. A variant backed by a protobuf
`oneof` uses case dataclasses, as described under [`@nexus.proto`](#nexusproto).
TypeScript represents variants as tagged object unions.

```wit
variant notification-target {
  email(string),
  sms(string),
  none,
}
```

**Python:**

```python
NotificationTarget = (
    tuple[typing.Literal["email"], str]
    | tuple[typing.Literal["sms"], str]
    | tuple[typing.Literal["none"]]
)
```

**TypeScript:**

```typescript
export type NotificationTarget =
  | { tag: "email"; value: string }
  | { tag: "sms"; value: string }
  | { tag: "none" };
```

TypeScript cases with a payload carry a `value` field. Cases without a payload
do not.

### Results

WIT `result<T, E>` types use the same tagged-union pattern as variants, with
tags `"ok"` and `"err"`.

```wit
record user-profile {
  sync-state: result<string, string>,
}
```

**Python:**

```python
sync_state: tuple[typing.Literal["ok"], str] | tuple[typing.Literal["err"], str]
```

**TypeScript:**

```typescript
syncState: { tag: "ok"; value: string } | { tag: "err"; value: string };
```

### Tuples

WIT tuples map to native tuple types.

```wit
record postal-address {
  coordinates: option<tuple<f64, f64>>,
}
```

**Python:**

```python
coordinates: tuple[float, float] | None = None
```

**TypeScript:**

```typescript
coordinates?: [number, number];
```

### Option Types

`option<T>` fields become optional with a `None`/`undefined` default.

In proto-backed records, API optionality is independent of protobuf presence.
Non-optional booleans and numbers accept implicit protobuf defaults, including
`false` and zero, even when the field is omitted from the wire.

**Python:**

```python
# Required field: no default
user_id: str
# Optional field: defaults to None
reason: str | None = None
```

**TypeScript:**

```typescript
// Required field: no ?
userId: string;
// Optional field: ? suffix
reason?: string;
```

### Lists and Maps

Lists and maps without an `option<>` wrapper are required and have no implicit
empty-collection default. Optional collections default to `None` in Python or
may be omitted in TypeScript; absence is not replaced with an empty collection.

```wit
record user-profile {
  tags: list<string>,
  metadata: map<string, string>,
  aliases: option<list<string>>,
}
```

**Python:**

```python
tags: list[str]
metadata: dict[str, str]
aliases: list[str] | None = None
```

**TypeScript:**

```typescript
tags: string[];
metadata: Record<string, string>;
aliases?: string[];
```

The WIT `option<>` wrapper controls authored collection optionality, including
in proto-backed records. Protobuf repeated and map fields have no presence
information, so decoding their wire values produces collections even when empty;
their API fields do not gain implicit constructor defaults.

In a record with `@nexus.proto`, write a proto `bytes` field as `list<u8>`.
Write a `bytes` member of a oneof the same way, as the case payload of a variant
with `@nexus.proto`. A resource method parameter of `list<u8>` that fills such a
field is `bytes` too. nexgen generates Python `bytes`, TypeScript `Uint8Array`,
Go `[]byte`, or .NET `byte[]`. The .NET converters translate between `byte[]`
and protobuf `ByteString` at the wire boundary. In other records, `list<u8>` is
a list of integers, like any other `list<T>`.

---

## Resources

Resources are the most complex feature of the code generator. They produce
classes whose constructor fields represent the resource's identity, and whose
methods delegate to Nexus operations.

A resource that uses a type parameter is generic. See
[Generic resources](#generic-resources).

### Resource Declaration

```wit
resource user {
  constructor(user-id: string, email: string);

  update-email: func(email: string) -> user-result;
}

type user-result = own<user>;

record update-email-request {
  user-id: string,
  email: string,
}

update-email: func(request: update-email-request) -> user-result;
```

**Python:**

```python
class User:
    user_id: str
    email: str

    def __init__(self, user_id: str, email: str) -> None:
        self.user_id = user_id
        self.email = email

    async def update_email(self, email: str) -> User:
        request = UpdateEmailRequest(user_id=self.user_id, email=email)
        return await _update_email(request)
```

**TypeScript:**

```typescript
export class User {
  public constructor(
    public readonly userId: string,
    public readonly email: string,
  ) {}

  public async updateEmail(email: string): Promise<User> {
    return await updateEmail({ userId: this.userId, email: email });
  }
}
```

Key observations:

- The resource's **constructor fields** become the class's identity fields
  (`user_id`, `email`).
- Resource methods fill in identity fields from `self` (`self.user_id`) and
  accept only the remaining parameters.
- A Python resource with methods is a handle, not data. It compares and prints
  its identity fields, but the payload converter can't encode or decode it. Pass
  the identity fields, and build the handle again from them. A resource with no
  methods stays a dataclass.
- A standalone convenience wrapper is also generated for each resource method,
  taking all fields as explicit arguments.
- Resource operations additionally produce three layers:
  1. `User.update_email(self, email)` -- instance method, uses `self.user_id`
  2. `_update_email(request)` -- internal standalone, takes request object
  3. `update_email(*, user_id, email)` -- public standalone convenience

### Name-Binding: How Resource Methods Find Operations

This is the non-obvious part. The generator does not require you to explicitly
say "method X calls operation Y." Instead, it uses **field-name matching** to
automatically bind resource methods to operations.

For each resource method, the generator builds a **name environment** from:

- The resource's **constructor field names** (e.g., `user-id`, `email`)
- The method's **parameter names** (e.g., `email`)

It then tries every operation in the service as a candidate. For each candidate
operation, it checks whether every **required** field in the operation's input
record can be satisfied by a name in the environment.

**Example walkthrough:**

```wit
resource user {
  constructor(user-id: string, email: string);
  update-email: func(email: string) -> user-result;
}

record update-email-request { user-id: string, email: string, }
update-email: func(request: update-email-request) -> user-result;
```

The environment for the `update-email` method is:

```
user-id  =>  ResourceField("user-id")   (from constructor)
email    =>  MethodParam("email")       (from method signature)
```

Notice that the method's `email` parameter shadows the `email` parameter we got
from the constructor!

The generator tries the `update-email` operation, which has input
`update-email-request` with required fields `user-id` and `email`. Both names
are found in the environment, so the binding succeeds.

The generated code then uses `self.user_id` for the resource field and the
`email` parameter for the method parameter:

```python
request = UpdateEmailRequest(user_id=self.user_id, email=email)
```

**Nested struct matching:**

When an operation's input contains a field whose type is itself a record, the
generator recursively tries to match the nested record's fields against the same
environment.

```wit
resource started-workflow {
  constructor(namespace: string, workflow-id: string, run-id: option<string>);
  cancel: func(reason: option<string>);
}

record workflow-execution { workflow-id: string, run-id: option<string>, }

record cancel-workflow-request {
  namespace: string,
  workflow-execution: workflow-execution,
  reason: option<string>,
}
```

The environment for `cancel` is:

```
namespace    =>  ResourceField
workflow-id  =>  ResourceField
run-id       =>  ResourceField
reason       =>  MethodParam
```

The `cancel-workflow-request` has three fields:

- `namespace` -- directly matched to ResourceField
- `workflow-execution` -- not directly in the environment, but it is a record
  type, so the generator recurses into it and finds `workflow-id` and `run-id`,
  both of which match
- `reason` -- matched to MethodParam

Result: the binding succeeds, and the generated code constructs the nested
struct:

```python
request = CancelWorkflowRequest(
    workflow_execution=WorkflowExecution(
        workflow_id=self.workflow_id, run_id=self.run_id,
    ),
    reason=reason,
)
```

**Return type filtering:**

After field matching, the generator also filters candidates by return type. If
the method returns `own<user>`, only operations that also return `own<user>` (or
the equivalent proto type) are considered. A method that returns a record binds
to an operation that returns that record. An output transform on the operation
changes the method's result too.

**Disambiguation by name:**

If multiple operations match a method's field requirements and return type, the
generator tries to disambiguate by comparing the method name (converted to
UpperCamelCase) with the operation names. If exactly one operation name matches,
it wins. Otherwise, the generator raises an error.

### Resource Return Binding

When an operation returns a resource (via `own<resource-name>`), the generator
must determine where each resource constructor field's value comes from. It
searches **by name** in this order:

1. **The operation's input (request)** -- for fields like `workflow-id` that the
   caller provides
2. **The operation's output (response)** -- for fields like `run-id` that come
   back from the server

If both the request and response contain a field with the same name, the request
takes priority.

The type of message that comes back from the server is specified using the
`@nexus.proto` directive:

```wit
/// @nexus.proto "temporal.api.workflowservice.v1.StartWorkflowExecutionResponse"
type start-workflow-result = own<started-workflow>;
```

The generator proceeds to generate:

```python
# workflow_id comes from the request, run_id comes from the response
return StartedWorkflow(
    namespace=request_proto.namespace,
    workflow_id=request.workflow_id,
    run_id=result.run_id or None,
)
```

If a resource constructor field name cannot be found in either, the generator
raises `InvalidResource`.

**A result record that holds the resource.** An operation can also return a
proto-backed record with one `own<T>` field and other fields. The resource binds
from the record's message as above, and the other fields come from the fields of
the same name. The record is the API model only, and it declares only the fields
that its caller sees. Its message travels as a wire record that takes the proto
message's name. Python, TypeScript, .NET and Go generate it. In Python, the
record lives in its resource's module.

```wit
/// @nexus.proto "acme.jobs.v1.CreateJobResponse"
record create-job-result {
  job: own<job>,
  created: option<bool>,
}
```

```python
result = await handle  # CreateJobResponse
return CreateJobResult(job=Job(job_id=result.job_id), created=result.created)
```

### Closing a Resource

`@nexus.close` marks the method that ends a resource. In Python, the end of an
`async with` block calls it with no arguments, so its parameters must be
optional.

```wit
/// @nexus.operation "close-stream"
/// @nexus.close
close: func();
```

```python
async with client.create_stream(id="turns", body_type=Turn) as stream:
    await stream.publish(records=[AppendRecord(body=Turn(role="user"))])
```

A call that returns the resource returns a `ResourceCall`, such as
`ResourceCall[Stream[Turn]]`. Await it for the resource, or enter it with
`async with`. Only Python closes a resource this way.

### Variadic Items

`@nexus.varargs "<parameter>"` makes a resource method take the items of that
list parameter as positional arguments. Only Python generates it, and Go refuses
it.

```wit
resource %stream {
  /// @nexus.operation "append-records"
  /// @nexus.varargs "records"
  publish: func(records: list<append-record>) -> append-records-response;
}
```

```python
async def publish(self, *records: BodyT) -> int: ...
```

### Edge Cases and Error Conditions

#### Same field name in constructor and method parameter

When a resource constructor field and a method parameter have the same name, the
**method parameter takes precedence**. This is the expected pattern for "update"
operations:

```wit
resource user {
  constructor(user-id: string, email: string);
  update-email: func(email: string) -> user-result;
}
```

Here `email` appears in both the constructor and the method. The method
parameter wins, so the generated code passes the new email value (from the
argument), not the old one (from `self`). The `user-id` field still comes from
`self.user_id`.

#### No matching operation found

If no operation's input fields can be satisfied by the method's environment, the
method becomes a **stub** that raises an error at runtime:

```python
async def get_result(self) -> ...:
    raise NotImplementedError("started-workflow.get_result is not yet implemented")
```

```typescript
public async getResult(): Promise<common.Payload[]> {
    throw new Error("started-workflow.getResult is not yet implemented");
}
```

A warning is emitted during generation.

#### Multiple operations match ambiguously

If two or more operations satisfy a method's field requirements and return type,
and disambiguation by name fails, the generator raises `InvalidResourceMethod`
with a list of the ambiguous matches. To fix this, rename either the method or
one of the operations so the preferred name match succeeds, or adjust the
operation's input record so fewer candidates match.

#### Required field in operation input has no match

If an operation's input record has a required field whose name does not appear
in the resource constructor or method parameters, that operation will silently
be skipped as a candidate. So watch out for typos!

#### Resource field cannot be bound from input or output

When an operation returns a resource but one of the resource's constructor
fields cannot be found (by name) in either the operation's input or output
records, the generator raises `InvalidResource`:

```
could not bind resource field `namespace` from operation input or output
```

#### Same operation bound by methods on two different resources

Each operation can only be "owned" by one resource. If methods on different
resources both try to bind the same operation, the generator raises an error.

#### Duplicate resource type names

If two resources in different services would produce the same generated type
name, the generator raises an error.

---

## Proto-Backed Models

When a WIT record is annotated with `@nexus.proto`, the generator produces code
that converts between the public model and its protobuf transfer type. Generated
callers continue to accept and return the public model; conversion is performed
by the target SDK or generated operation helpers.

An enum annotated with `@nexus.proto` likewise remains a generated public enum
unless a type replacement selects an external model. Fields use the authored
enum's name and members, even when its name differs from the protobuf enum's.
Converters bridge the model and protobuf enum by number. A case with
[`@nexus.fallback`](#nexusfallback) takes a protobuf number that the enum
doesn't know.

Records cannot mix JSON and protobuf backing within one value. A plain record
cannot contain a protobuf-backed record, and a protobuf-backed record cannot
contain a plain record. This rule applies to every target, including model-only
modules, and follows aliases, containers, and variant payloads. Give nested
records consistent backing, or use separate operation inputs. Independent JSON
and protobuf operations may still share a service. External types with authored
converters retain their own conversion contracts.

Constructor fields of a resource returned as plain JSON cannot contain
protobuf-backed records either. Use JSON-backed constructor values or return the
resource through a protobuf-backed result.

### Transfer-Type Conversion

```wit
/// @nexus.proto "temporal.api.activity.v1.ActivityOptions"
record activity-options {
  task-queue: option<task-queue>,
  retry-policy: retry-policy,
}
```

**Python:**

```python
@dataclasses.dataclass(slots=True, kw_only=True)
class ActivityOptions:
    task_queue: str | None = None
    retry_policy: temporalio.common.RetryPolicy


class _ActivityOptionsTransferTypeConverter(
    temporalio.converter.TransferTypeConverter[ActivityOptions, ProtoActivityOptions]
):
    def from_transfer_type(self, value, type_hint) -> ActivityOptions:
        return ActivityOptions(
            task_queue=task_queue_from_proto(value.task_queue)
                if value.HasField("task_queue") else None,
            retry_policy=retry_policy_from_proto(value.retry_policy),
        )

    def to_transfer_type(self, value: ActivityOptions) -> ProtoActivityOptions:
        message = ProtoActivityOptions()
        if value.task_queue is not None:
            message.task_queue.CopyFrom(task_queue_to_proto(value.task_queue))
        message.retry_policy.CopyFrom(retry_policy_to_proto(value.retry_policy))
        return message


_ = temporalio.converter.transfer_type_convertible(
    _ActivityOptionsTransferTypeConverter
)(ActivityOptions)
```

**.NET:**
```csharp
[TemporalTransferTypeConverter(typeof(ActivityOptions.TransferTypeConverter))]
public class ActivityOptions
{
    public ActivityOptions(Temporalio.Common.RetryPolicy retryPolicy)
    {
        RetryPolicy = retryPolicy;
    }

    public string? TaskQueue { get; init; }
    public Temporalio.Common.RetryPolicy RetryPolicy { get; }

    internal static ActivityOptions FromTransferType(
        Temporalio.Api.Activity.V1.ActivityOptions wire) { /* ... */ }

    internal Temporalio.Api.Activity.V1.ActivityOptions ToTransferType()
        { /* ... */ }

    public sealed class TransferTypeConverter : ITemporalTransferTypeConverter
    {
        public Type TransferType =>
            typeof(Temporalio.Api.Activity.V1.ActivityOptions);

        public object? ToTransferType(object? value) => value is null
            ? null
            : ((ActivityOptions)value).ToTransferType();

        public object? FromTransferType(object? transferType) =>
            transferType is null
                ? null
                : ActivityOptions.FromTransferType(
                    (Temporalio.Api.Activity.V1.ActivityOptions)transferType);
    }
}
```

**TypeScript:**

```typescript
export interface ActivityOptions {
    taskQueue?: string;
    retryPolicy: common.RetryPolicy;
}

function activityOptionsFromProto(
    proto: temporal.api.activity.v1.IActivityOptions,
): ActivityOptions {
    return {
        taskQueue: proto.taskQueue == null ? undefined
            : taskQueueFromProto(proto.taskQueue),
        retryPolicy: requiredField(
            retryPolicyFromProto(requiredField(proto.retryPolicy, ...)), ...
        ),
    };
}

function activityOptionsToProto(
    model: ActivityOptions,
): temporal.api.activity.v1.IActivityOptions {
    return {
        taskQueue: model.taskQueue == null ? undefined
            : taskQueueToProto(model.taskQueue),
        retryPolicy: retryPolicyToProto(model.retryPolicy),
    };
}
```

Required fields are validated in `from_proto` -- missing required proto fields
raise a `ValueError` (Python) or throw an `Error` (TypeScript). .NET output
requires `Temporalio` 1.18.0 or newer, or 1.19.0 or newer for generic
proto-backed models.

Generated TypeScript message-conversion helpers take non-null inputs and return
non-null outputs. Callers guard optional fields before conversion and validate
required wire fields before passing them to a helper. Required results from
authored support converters are still validated; those converters retain their
own nullable contracts.

### Sourced Fields

Fields annotated with `@nexus.source` are supplied by a helper in each
applicable API target, rather than exposed as convenience arguments. Sources
apply to all targets unless restricted by `targets="workflow"`,
`targets="client"`, or a comma-separated list. Other targets expose the field as
an ordinary argument. With no API target, a sourced field is an ordinary model
field. Each value is a helper call relative to the project's support package,
supplied with `--support-package`. Include the call and its arguments in WIT.
Nexgen adds the package prefix and imports support for the generated field.

```wit
record start-workflow-request {
  workflow-id: string,
  /// @nexus.source targets="workflow" python="workflow_namespace()" typescript="workflowNamespace()" go="WorkflowNamespace(ctx)" dotnet="TemporalWorkflowContext.WorkflowNamespace()"
  namespace: string,
}
```

The workflow convenience API does not require a `namespace` argument. Request
construction evaluates the helper. In a resource method, a resource field or a
method parameter of the same name takes precedence over a source, and the source
fills only the fields that the method has no value for. Every language follows
this rule. Shared models and their converters do not evaluate sources, so direct
model construction must supply required fields:

```python
request = StartWorkflowRequest(
    workflow_id=workflow_id,
    namespace=_support.workflow_namespace(),
)
```

In Python, TypeScript and .NET, an optional method parameter (`option<T>`) takes
precedence only when the caller passes a value. Otherwise the source fills the
field. The source is not evaluated when the caller passes a value. Other
languages pass an optional parameter as it is.

```python
producer=producer if producer is not None else _support.default_producer()
```

Nexgen prefixes the helper call, but it does not add arguments. The configured
package supplies the helper. The `@nexus.output-transform` directive is
different: its value remains a raw expression.

### Omitted Fields

Fields annotated with `@nexus.omit` are excluded entirely from the generated
API. They exist in the proto message but are not relevant to the API consumer.
Use the `placeholder` type for omitted fields.

```wit
/// @nexus.omit
identity: placeholder,
/// @nexus.omit
request-id: placeholder,
```

### API-omitted fields

Fields annotated with `@nexus.api-omit` remain on generated models and are
included in proto conversion, but do not appear in generated convenience
operation APIs. Use this when generated code must carry a wire field without
making it a public operation parameter.

```wit
/// @nexus.api-omit
headers: option<header>,
```

`targets="workflow"` or `targets="client"` omits the field from those targets
only. The other targets take it as an argument or from their `@nexus.source`.

```wit
/// @nexus.api-omit targets="workflow"
last: option<s32>,
```

The field must be an `option<T>` or a `list<T>`, since a call leaves it unset.
An omitted list is sent empty. `@nexus.api-omit` cannot be combined with
`@nexus.omit`, `@nexus.default`, or a `@nexus.source` for the same target.

### Flattened Records

A record annotated with `@nexus.flatten-in-api` has its fields "promoted" into
the parent record's convenience wrapper instead of being passed as a nested
object.

```wit
/// @nexus.flatten-in-api
record user-metadata {
  /// @nexus.flattened-type python="str"
  static-summary: option<payload>,
  /// @nexus.flattened-type python="str"
  static-details: option<payload>,
}
```

When a parent record has `user-metadata: option<user-metadata>`, the convenience
wrapper exposes `static_summary` and `static_details` as top-level parameters
instead of requiring a nested `UserMetadata(...)` object:

```python
async def signal_with_start_workflow(
    ...,
    static_summary: str | None = None,
    static_details: str | None = None,
) -> ...:
    user_metadata = (
        None if static_summary is None and static_details is None
        else UserMetadata(static_summary=static_summary, static_details=static_details)
    )
```

In the snippet above, notice that the entire `user-metadata` field is `None` if
all its constituent fields are `None`. Python, TypeScript, .NET, and Go all omit
the optional nested record when none of its fields are supplied. Go uses nil
pointers internally for wire presence, but its public options use
default-punning: empty strings mean omitted.

#### Flattening one field

On a request field, `@nexus.flatten-in-api` makes that call take the fields of
the field's record. The models keep the record.

```wit
record create-stream-request {
  /// @nexus.flatten-in-api
  stream-ref: stream-ref,
}
```

```python
stream = await client.create_stream(id="s1", store="redis")
```

A resource method can't bind an operation whose request flattens a field.

A list of a flattened record with one field takes that field's values, and the
call wraps each value in the record. The record must be proto-backed. Go refuses
such a list.

```wit
/// @nexus.flatten-in-api
record append-record {
  body: body-t,
}
```

```python
# Sends records=[AppendRecord(body="a"), AppendRecord(body="b")].
await append_records(ref=ref, records=["a", "b"])
```

### Output Transforms

An operation annotated with `@nexus.output-transform` transforms the raw
operation result into a different type before returning it. Its language
expressions are emitted verbatim, not interpreted as names of support functions
to call. Unlike `@nexus.source`, this directive can call an SDK function
directly. Moving that call into a Go support package could require the support
package to import generated request or result types. That import would create a
package cycle.

```wit
/// @nexus.output-transform
///   python-type="workflow.ExternalWorkflowHandle[typing.Any]"
///   python="workflow.get_external_workflow_handle(request.id, run_id=result.run_id)"
///   typescript-type="ExternalWorkflowHandle"
///   typescript-type-import="../../../../workflow"
///   typescript="workflow.getExternalWorkflowHandle(request.id, result.runId ?? undefined)"
///   typescript-import="@temporalio/workflow"
///   go-type="example.com/handles:handles.WorkflowHandle"
///   go="handles.FromSignalWithStart(request, &result)"
signal-with-start-workflow: func(
  request: signal-with-start-workflow-request,
) -> signal-with-start-workflow-response;
```

The generated wrapper returns the transformed type. The transform expression has
access to `request` (the input) and `result` (the raw response). Go transform
expressions must evaluate to `(T, error)`, where `T` is the `go-type` value:

Go transforms run inside the terminal workflow interceptor, so `request`
contains any changes made by outbound interceptors. References to the `workflow`
package in the transform use the same import alias as generated SDK references.

For TypeScript, `typescript-type-import` adds a type-only import for the type
named by `typescript-type`; `typescript-import` adds the value import used by
the transform expression. They are separate because a transform can need a type
from one module and a runtime value from another.

```python
result = await handle
return workflow.get_external_workflow_handle(request.id, run_id=result.run_id)
```

```typescript
const result = await handle.result();
return workflow.getExternalWorkflowHandle(
  request.id,
  result.runId ?? undefined,
);
```

```go
return handles.FromSignalWithStart(request, &result)
```

---

## Function References

Nexus and Temporal often need to identify a function by name on the wire, while
SDK users want to pass a typed Python callable. WIT does not have a first-class
"callable reference" field type, so `@nexus.function` marks a WIT type as a
semantic function reference. The directive tells the generator which WIT
function describes the callable's arguments and result, whether a raw name is
also accepted, and where any serialized arguments live in proto-backed records.

Use this when the generated API should feel like an SDK call:

```python
await start_workflow(MyWorkflow.run, "user-123", id="sync-user", task_queue="workers")
```

instead of forcing callers to manually build wire-shaped data:

```python
await start_workflow(workflow="MyWorkflow", args=["user-123"], ...)
```

### Basic usage

To use `@nexus.function`, define a WIT function that has the signature you want:

```wit
my-function-call: func(name: string, enabled: bool) -> string;
```

Then define a type alias that points to the function you defined:

```wit
/// @nexus.function signature="my-function-call"
type my-exe = placeholder;
```

Now any field of type `my-exe` is treated as a function reference with the
specified signature.

```wit
record execute-function-request {
  function: my-exe,
}
```

Generated Python:

```python
@dataclasses.dataclass(slots=True)
class ExecuteFunctionRequest:
    function: collections.abc.Callable[[str, bool], str]
    name: str
    enabled: bool
```

### `alternate-type`

`alternate-type` allows the caller to pass other types as the function argument,
besides callables.

```wit
function-call: func(name: string, enabled: bool) -> string;

/// @nexus.function
///   signature="function-call"
///   alternate-type="string"
type named-function = placeholder;
```

Now in the generated Python, `function` can be a string:

```python
@dataclasses.dataclass(slots=True)
class ExecuteNamedFunctionRequest:
    function: str | collections.abc.Callable[[str, bool], str]
    name: str
    enabled: bool
```

### `@nexus.function-args`

`@nexus.function-args varargs=true` changes the last parameter into a variable
argument list.

```wit
/// @nexus.function-args varargs=true
varargs-function-call: func(args: list<string>) -> string;

/// @nexus.function signature="varargs-function-call"
type varargs-function = placeholder;
```

When the signature has multiple parameters, use `param` to specify which one is
variadic:

```wit
/// @nexus.function-args
///   varargs=true
///   param="args"
///   typescript-drop-prefix=true
workflow-call: func(pfx: callable-prefix, args: list<string>) -> workflow-result;
```

`typescript-drop-prefix=true` is TypeScript-specific. It tells TypeScript
generation to omit prefix parameters from callable argument inference, which is
useful for Temporal method forms that include an implicit receiver/context
parameter. Python keeps the prefix in the callable annotation where needed.

### `args-field`

For proto-backed APIs, the WIT record often has a wire field like `input` or
`signal-input` that stores a payload list. `args-field` maps the ergonomic
Python arguments back to that field.

```wit
type payloads = list<string>;

/// @nexus.function-args varargs=true
workflow-call: func(args: payloads) -> string;

/// @nexus.function
///   signature="workflow-call"
///   args-field="input"
type workflow-function = placeholder;
```

Generated Python request model and proto conversion:

```python
@dataclasses.dataclass(slots=True, kw_only=True)
class StartWorkflowRequest:
    workflow: collections.abc.Callable[..., str]
    args: list[typing.Any] | None = None

    def to_transfer_type(self, value: "StartWorkflowRequest"):
        if value.args is not None:
            message.input.CopyFrom(payloads_to_proto(value.args))
        return message
```

### `result-type-parameter`

`result-type-parameter` names a generated Python type variable for the callable
result. The motivation is typed handles: if the caller starts `MyWorkflow.run`
and that workflow returns `OrderSummary`, the returned handle should be typed as
`ExternalWorkflowHandle[OrderSummary]`.

```wit
type payloads = list<string>;

/// @nexus.type python="collections.abc.Awaitable[WorkflowResult]"
type workflow-result = placeholder;

/// @nexus.function-args varargs=true
workflow-call: async func(args: payloads) -> workflow-result;

/// @nexus.function
///   signature="workflow-call"
///   result-type-parameter="WorkflowResult"
type workflow-function = placeholder;
```

Generated Python overloads can preserve the callable result type in the handle:

```python
WorkflowResult = typing.TypeVar("WorkflowResult")

@typing.overload
async def start_workflow(
    *,
    workflow: collections.abc.Callable[..., collections.abc.Awaitable[WorkflowResult]],
    args: list[typing.Any],
) -> ExternalWorkflowHandle[WorkflowResult]: ...
```

Without a typed callable, generated Python has no result type to infer.

### `primary`

When an operation has multiple `@nexus.function` parameters, use `primary=true`
to indicate which function is "primary".

```wit
type payloads = list<string>;
type task-queue = string;
type workflow-type = string;

/// @nexus.type python="collections.abc.Awaitable[WorkflowResult]"
type workflow-result = placeholder;

/// @nexus.type python="None"
type signal-result = placeholder;

/// @nexus.function-args varargs=true
workflow-call: async func(args: payloads) -> workflow-result;

/// @nexus.function-args varargs=true
signal-call: func(signal-args: payloads) -> signal-result;

/// @nexus.function
///   primary=true
///   signature="workflow-call"
///   result-type-parameter="WorkflowResult"
type workflow-function = placeholder;

/// @nexus.function
///   signature="signal-call"
type signal-function = placeholder;

record signal-with-start-workflow-request {
  workflow: workflow-function,
  signal: signal-function,
}
```

Generated Python dataclass:

```python
@dataclasses.dataclass(slots=True, kw_only=True)
class SignalWithStartWorkflowRequest:
    workflow: collections.abc.Callable[..., collections.abc.Awaitable[typing.Any]]
    args: list[typing.Any] | None = None
    signal: collections.abc.Callable[..., None]
    signal_args: list[typing.Any] | None = None
```

Generated Python operation overload:

```python
WorkflowResult = typing.TypeVar("WorkflowResult")
WorkflowArgs = typing_extensions.TypeVarTuple("WorkflowArgs")

@typing.overload
async def signal_with_start_workflow(
    workflow: collections.abc.Callable[
        [typing_extensions.Unpack[WorkflowArgs]],
        collections.abc.Awaitable[WorkflowResult],
    ],
    *positional_args: typing_extensions.Unpack[WorkflowArgs],
    signal: collections.abc.Callable[..., None | collections.abc.Awaitable[None]],
    signal_args: list[typing.Any] | None = ...,
) -> ExternalWorkflowHandle[WorkflowResult]: ...
```

Here `workflow` is primary, so workflow arguments become positional operation
arguments and the workflow result type flows into the returned handle. `signal`
is still a typed function reference, but its arguments stay in `signal_args`.

### `converter`, `python-converter`, and `typescript-converter`

Converters tell proto-backed generation how to turn the callable reference into
the wire value. Signals are the common example: callers can pass a string signal
name or a Python signal method, while the proto field is just `signal_name`.

```wit
/// @nexus.function
///   signature="signal-call"
///   args-field="signal-input"
///   alternate-type="string"
///   python-converter="signal_function_to_proto"
///   typescript-converter="signalFunctionToProto"
type signal-function = placeholder;
```

Generated Python imports the user-owned support module and calls the converter
during transfer-type conversion:

```python
import temporal_support as _support

@dataclasses.dataclass(slots=True, kw_only=True)
class SignalWithStartWorkflowRequest:
    signal: str | collections.abc.Callable[..., None | collections.abc.Awaitable[None]]
    signal_args: list[typing.Any] | None = None

    def to_transfer_type(self, value: "SignalWithStartWorkflowRequest"):
        message.signal_name = _support.signal_function_to_proto(value.signal)
        if value.signal_args is not None:
            message.signal_input.CopyFrom(_support.payloads_to_proto(value.signal_args))
        return message
```

Use `converter="<func>"` when the same helper name applies to every generated
language, or use language-prefixed keys such as `python-converter` and
`typescript-converter` when helper names differ.

### Direct Field Result Overrides

Most authored APIs should use the type-alias `signature` form because it keeps
arguments and result types in WIT. A direct record-field annotation is available
for the narrower case where the field itself describes a callable and the result
type is already language-specific.

```wit
type function-args = list<string>;

record invoke-request {
  /// @nexus.function
  ///   args-field="args"
  ///   python-result="typing.Any"
  target: placeholder,
  args: function-args,
}
```

Generated Python:

```python
@dataclasses.dataclass(slots=True)
class InvokeRequest:
    target: collections.abc.Callable[..., typing.Any]
    args: list[typing.Any] | None = None
```

Use `result` when one result annotation works for every language, or
language-prefixed keys such as `python-result` and `typescript-result` when it
does not. Do not combine these result overrides with `signature` on a type
alias.

### Option Summary

| Option                           | Motivation                                                                     | Generated effect                                                                                            |
| -------------------------------- | ------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------- |
| `signature`                      | Keep callable shape in a WIT function instead of a placeholder alias.          | Derives callable argument and result annotations. Required for type-alias `@nexus.function`.                |
| `alternate-type`                 | Accept raw names as well as typed callables.                                   | Generates union annotations and overloads such as `str \| Callable[...]`.                                   |
| `args-field`                     | Map ergonomic function args to a wire field such as `input` or `signal-input`. | Stores normalized args in that request/proto field. Defaults to the signature's args name for type aliases. |
| `result-type-parameter`          | Preserve the callable's result type in returned handles.                       | Emits a type variable such as `WorkflowResult` and uses it in callable overload returns.                    |
| `primary`                        | Identify the main callable when a request has multiple function references.    | Enables positional primary args and result type propagation. Defaults to `false`.                           |
| `converter`                      | Reuse the same conversion helper name in all languages.                        | Calls the helper when converting the function field to proto.                                               |
| `<language>-converter`           | Use language-specific conversion helper names.                                 | Python uses `python-converter`; TypeScript uses `typescript-converter`.                                     |
| `result` / `<language>-result`   | Direct field-level form when there is no type alias `signature`.               | Supplies the callable result annotation directly. Prefer `signature` for authored aliases.                  |
| `go-value-type` / `go-args-type` | Override the Go reference and argument representation together.                | A string reference passes through as its name; the argument override is one payload, even for slice types.  |
| `go-name-extractor`              | Resolve callable names with a support helper.                                  | Calls the helper with `(ctx, value)` instead of runtime function-name extraction.                           |

Go argument overrides require a variadic signature with one list-backed argument
field. For example, `go-value-type="string" go-args-type="any"` exposes a name
and one opaque payload, packed into the wire argument list. Without an override,
Go groups each reference with its signature-derived arguments, preserves the
other groups' authored order, and places primary variadic arguments last. Only a
final argument list uses `...T`; other lists use `[]T`. An explicit primary
descriptor replaces signature-derived typing.

`@nexus.function-args` options:

| Option                   | Motivation                                                             | Generated effect                                           |
| ------------------------ | ---------------------------------------------------------------------- | ---------------------------------------------------------- |
| `varargs`                | Treat a final list-shaped signature parameter as `*args`.              | Generates positional overloads plus list-form `args`.      |
| `param`                  | Disambiguate which final parameter is the varargs list.                | Required when the signature has more than one parameter.   |
| `typescript-drop-prefix` | Remove implicit receiver/context parameters from TypeScript inference. | TypeScript omits the prefix from inferred argument tuples. |

---

## Generation Targets

`--targets` sets the code that calls the generated API. Repeat the flag, or
separate the targets with commas.

- Without `--targets`, generation emits models and service definitions only.
  Sourced fields are ordinary model fields. Python emits no resource classes
  without an API target; contracts that depend on resources also belong to the
  selected API. Other languages represent resources as data classes.
- `workflow` selects the API for Workflow code.
- `client` writes the same API for code outside a Workflow, in a `client`
  package inside the output directory. Only Python System Nexus client
  generation is supported, so this target requires `--system-nexus`. It can be
  selected independently or alongside `workflow`. nexgen refuses it for a
  service with no `@nexus.endpoint`.

Go operation convenience wrappers take `ctx`, an options struct, and positional
arguments for required input fields, in that order. Optional fields and fields
with an applicable `@nexus.default` stay in the options struct. Fields supplied
by `@nexus.source`, omitted fields, and API-omitted fields are not arguments.
The options struct is required even when it is empty:

```go
future := userservice.UpdateEmail(ctx, userservice.UpdateEmailOptions{}, "user-123", "alice@example.com")
```

Python keeps shared models and service contracts in the root package. Client
APIs live in `client/`. Workflow-only generation keeps its API in the root
package; when both targets are selected, the workflow API lives in `workflow/`.
Each API owns its operation wrappers and resource classes. Neither API imports
the other. The `client` package exposes a `<Service>ClientExtension` for each
service. Client handles are constructed with an explicit service-bound Nexus
client, and shared identity DTOs decode resource results without loading
workflow resource classes.

```text
stream_service/
  __init__.py, models.py, services.py # shared models and wire contracts
  workflow/
    __init__.py, operations/, _resources/
  client/
    __init__.py, _extensions.py, _resources/
```

Service definitions always live in root `services.py`, including when no API
target is selected. Both APIs use that same wire contract. Plain resource
results use identity DTOs in root `_resources/_resource_data.py`; callable
resource handles remain part of their respective APIs.

A client class inherits one or more extensions and provides the SDK's
`create_nexus_client` method. Inheriting `temporalio.client.Client` supplies
that method directly, with no adapter. Generated calls pass the service type to
the factory and typed operation descriptors to the SDK, which infers the request
and result types from the shared service contract.

```python
import temporalio.client
from stream_service import StreamRef
from stream_service.client import StreamServiceClientExtension


class Client(temporalio.client.Client, StreamServiceClientExtension):
    pass


client = await Client.connect("localhost:7233")
stream = await client.create_stream(stream_ref=StreamRef(id="s1"))
await stream.publish(records=records)
```

Each call starts the operation with a new random operation ID.

The extension exposes standalone operations and resource factories. Operations
bound to resource methods are exposed only through their resource handles.

Each Python call takes the keyword options `rpc_metadata` and `rpc_timeout`,
which default to `None`, and passes them to the `start_operation` of its Nexus
client. The methods of a client resource take them too. nexgen refuses a request
field or a method parameter with one of these names.

Each standalone operation also has a private method that takes the request, such
as `_stream_service_create_stream`. Its name has the service name. Two services
of one package can't have methods of the same name, private or public, and
nexgen refuses them.

A client resource takes a service-bound Nexus client as its first constructor
argument. Its methods call that client's `start_operation` directly, preserving
the service and endpoint binding. An operation that returns a resource gives the
handle the same Nexus client used to start the operation. API operations can
only return resources declared by their own service, including resources held in
result records.

```python
job = Job(nexus_client, job_id="job")
await job.restart()
```

The client is runtime state, not a resource identity field. When an operation
returns resource data, the generated API decodes the identity DTO and then
builds the handle. The Workflow API does the same for a resource with methods.

To rebuild a handle from identity fields that you stored, for example in a
Workflow result, call the factory of the resource on the extension. Its name is
`get_` and the snake-case name of the resource, and it takes the identity
fields. It creates a Nexus client for the resource's own service with
`create_nexus_client`.

```python
job = client.get_job(job_id=stored_job_id)
await job.restart()
```

nexgen refuses a factory name that is also the name of a client method of a
service in the package.

In Python, the Workflow package has the same `get_<resource>` factory for each
resource with a method. nexgen refuses one with the name of an operation
function of the package. The factory returns the handle from its ref, with no
call.

```python
stream = get_stream(ref=StreamRef(id="chat-1:turns", store="redis"), body_type=Turn)
```

Sources work in the client target as well. Restrict workflow-only helpers with
`targets="workflow"`. If a required resource-method request field has neither an
applicable source nor a resource field or method parameter supplying it,
generation rejects that API target.

---

## Directive Glossary

All directives are written in WIT doc comments (`///`) and prefixed with
`@nexus.`. Multi-line directives continue on subsequent `///` lines with
indented key-value pairs.

### Valid annotation placements

The frontend rejects unknown `@nexus.` names, unsupported placements and
argument names, duplicate arguments, and duplicate singleton directives.
Validation covers unused declarations and linked packages too, before target
selection. An annotation for another target language is still valid; it need not
affect the currently selected target. Ordinary prose and other comment
namespaces are not Nexus annotations.

The following placement index is checked against the compiler's annotation
catalog. A valid placement does not bypass semantic requirements described
below, such as protobuf bindings, function signatures, and conflicting options.

<!-- annotation-placements:start -->

| Annotation                              | Valid placements                                                                              |
| --------------------------------------- | --------------------------------------------------------------------------------------------- |
| `@nexus.endpoint`                       | interface                                                                                     |
| `@nexus.namespace`                      | interface                                                                                     |
| `@nexus.facade`                         | interface                                                                                     |
| `@nexus.operations-class`               | interface                                                                                     |
| `@nexus.service-name`                   | interface                                                                                     |
| `@nexus.delay-load-temporalio-workflow` | interface                                                                                     |
| `@nexus.proto`                          | record, enum, flags, variant, type alias, resource                                            |
| `@nexus.type-parameter`                 | type alias                                                                                    |
| `@nexus.generic`                        | record                                                                                        |
| `@nexus.proto-field`                    | record field                                                                                  |
| `@nexus.name`                           | record field                                                                                  |
| `@nexus.type`                           | record field, type alias, resource                                                            |
| `@nexus.omit`                           | record field                                                                                  |
| `@nexus.api-omit`                       | record field, resource method, freestanding function                                          |
| `@nexus.source`                         | record field                                                                                  |
| `@nexus.default`                        | record field                                                                                  |
| `@nexus.fallback`                       | enum case                                                                                     |
| `@nexus.experimental`                   | interface, record, variant, freestanding function                                             |
| `@nexus.doc`                            | record, record field, variant, variant case, resource, resource method, freestanding function |
| `@nexus.operation`                      | resource method, freestanding function                                                        |
| `@nexus.poll`                           | freestanding function                                                                         |
| `@nexus.serialization-context`          | freestanding function                                                                         |
| `@nexus.context-propagation`            | freestanding function                                                                         |
| `@nexus.output-transform`               | freestanding function                                                                         |
| `@nexus.flatten-in-api`                 | record, record field                                                                          |
| `@nexus.flattened-type`                 | record field                                                                                  |
| `@nexus.function`                       | type alias, record field                                                                      |
| `@nexus.function-args`                  | freestanding function                                                                         |
| `@nexus.add-rpc-compatible-with`        | type alias                                                                                    |
| `@nexus.close`                          | resource method                                                                               |
| `@nexus.varargs`                        | resource method                                                                               |

<!-- annotation-placements:end -->

Packages, worlds, world import/export statements, constructors, flags members,
and enum cases other than `fallback` have no Nexus annotations. Interface and
type-alias prose can be written as ordinary documentation comments, but
`@nexus.doc` there has no generated documentation consumer and is rejected.

`add-rpc-compatible-with` may be repeated to declare multiple compatibility
targets. `source` and `default` may be repeated with disjoint `targets` sets;
all other annotations are singletons. For language-specific arguments, the
recognized prefixes are `dotnet`, `go`, `java`, `python`, `ruby`, and
`typescript`. Prefixes are accepted only for options that the annotation
actually consumes, not arbitrarily on every directive. For example, `name` takes
language names, `proto` takes `<lang>-type` and `<lang>-import`, and `type` on
an external binding also takes `<lang>-from` and `<lang>-to`. Field `type`
annotations do not take conversion helpers. `doc` return overrides are only
valid on freestanding functions, and `function` signatures belong on type
aliases, not record fields.

### Language and System Nexus restrictions

Catalog entries can restrict the languages in which an annotation is valid and
require System Nexus mode. These checks run before lowering, including on unused
declarations and linked packages. Scaffolding commands check the annotation
vocabulary and placement without choosing an output language or generation mode.

An unqualified annotation requests behavior for the current output language. For
example, `@nexus.operations-class "CustomOperations"` is rejected when
generating Go or Python instead of silently doing nothing. A positional default
still applies to every language when combined with an explicit override.

An annotation with only language-qualified values is scoped to those languages.
`@nexus.operations-class dotnet="CustomOperations"` is therefore valid while
generating Go, even without System Nexus mode: the annotation is inactive for
that output. Generating .NET makes it active and requires `--system-nexus`.
Unsupported language-qualified arguments, such as
`@nexus.operations-class python="CustomOperations"`, are rejected regardless of
the selected output language.

The catalog currently declares these restrictions:

<!-- annotation-availability:start -->

| Annotation                              | Language restriction | System Nexus            |
| --------------------------------------- | -------------------- | ----------------------- |
| `@nexus.namespace`                      | dotnet, go           | not required by catalog |
| `@nexus.facade`                         | go                   | required                |
| `@nexus.operations-class`               | dotnet               | required                |
| `@nexus.delay-load-temporalio-workflow` | none                 | required                |
| `@nexus.poll`                           | python               | required                |
| `@nexus.context-propagation`            | go                   | required                |

<!-- annotation-availability:end -->

This index is checked against the catalog. "None" means the catalog imposes no
language restriction, not that every backend emits the feature. In particular,
delayed workflow loading remains a Python-specific effect of an unqualified
marker shared by multi-language WIT files. More detailed backend restrictions
remain in semantic validation, including .NET's System Nexus requirement for
namespace overrides.

### @nexus.endpoint

**Placement:** Interface doc comment **Syntax:**
`@nexus.endpoint "<endpoint-name>"`

Names the Nexus endpoint for the service. This is the value used when
registering or connecting to the endpoint at runtime.

```wit
/// @nexus.endpoint "user-service"
interface user-service { ... }
```

The service name (used in generated code) is derived from the interface name
converted to PascalCase: `user-service` becomes `UserService`.

---

### @nexus.namespace

**Placement:** Interface doc comment **Syntax:**
`@nexus.namespace "<namespace>"` or language-specific overrides such as
`@nexus.namespace dotnet="Example.Models" go="example.com/project/models"`

Selects the generated namespace or package path for targets that support an
override. A positional string supplies the default; a language-specific value
overrides it for that target. For Go, the value is an import path; for .NET, it
is a namespace. The Go import path's final segment determines the package name,
independently of the output directory.

.NET namespace overrides are reserved for models-only WIT exports.
Operation-bearing .NET WIT services use the module-derived namespace.

---

### @nexus.facade

**Placement:** Interface doc comment **Syntax:**
`@nexus.facade go="<import-path>"`

Generates a Go facade of type aliases, constants, and forwarding functions.
Requires `--system-nexus`, `@nexus.namespace` for Go, and `--targets workflow`.
Use `go=` in multi-language WIT files so the annotation is inactive for other
languages:

```wit
/// @nexus.namespace go="go.temporal.io/sdk/internal"
/// @nexus.facade go="go.temporal.io/sdk/workflow"
interface workflow-service { ... }
```

Nexgen emits `<name>.go` and `<name>_facade.go` side by side under `--output`.
Place them into their respective packages before compiling. Each import path's
final segment determines its package name. Facade names omit the
implementation's `SystemNexus_` prefix.

Facades use the implementation package's name as its import qualifier, such as
`internal`. Generator-provided SDK types use their local names when the facade
is in a package that already re-exports them. For example, a facade targeting
`go.temporal.io/sdk/workflow` uses `Context` and `Future` rather than
`internal.Context` and `internal.Future`. This lookup uses the full import path;
an unrelated package named `workflow` does not receive those substitutions. The
built-in catalog does not infer re-exports for other authored type mappings.

---

### @nexus.operations-class

**Placement:** Interface doc comment **Syntax:**
`@nexus.operations-class "<class-name>"` or a language-specific override such as
`@nexus.operations-class dotnet="ModelOperations"`

Selects the .NET operations class name in System Nexus mode. A positional string
applies to the currently selected language and therefore fails for non-.NET
output. Use `dotnet=` in WIT files shared with other output languages.

---

### @nexus.service-name

**Placement:** Interface doc comment **Syntax:**
`@nexus.service-name "<WireServiceName>"` or
`@nexus.service-name name="<WireServiceName>"`

Overrides the service's wire name without changing its generated code name or
Nexus endpoint. Without this directive, the wire name is the interface name
converted to PascalCase. The override must not be empty.

---

### @nexus.proto

**Placement:** Type alias, record, variant, enum, or flags **Syntax:**
`@nexus.proto "<fully.qualified.proto.MessageName>"`

For variants, use `@nexus.proto "<fully.qualified.proto.MessageName>.<oneof>"`.

Maps a WIT type to a protobuf message or enum. The generator emits
target-specific transfer-type conversion code and validates field mappings
against the proto descriptor. Requires `--descriptors` on the CLI.

Native WIT declarations (`record`, `variant`, `enum`, and `flags`) own this
protobuf source mapping directly. A non-native alias or resource-backed type
uses an external type binding instead. `@nexus.type` is reserved for those
non-native bindings and is rejected when combined with `@nexus.proto` on a
native declaration.

```wit
/// @nexus.proto "temporal.api.activity.v1.ActivityOptions"
record activity-options { ... }
```

Protobuf `oneof` groups are authored as a single record field whose name matches
the oneof and whose type is a WIT variant. The variant's directive identifies
that same message and oneof. Each variant case must match a protobuf member name
(with kebab-case WIT naming) and carry the corresponding member value. Use
`option<variant>` when the protobuf oneof may be unset, or a required `variant`
when an unset value should be rejected during conversion.

```wit
/// @nexus.proto "example.v1.Response.result"
variant outcome {
  success(list<u8>),
  failure(string),
}

/// @nexus.proto "example.v1.Response"
record response {
  result: option<outcome>,
}
```

A case may name a proto-bound record. Its payload is that record's model, which
converts with the record's own converter.

Python represents a protobuf oneof-backed variant as a union of slotted case
dataclasses. Each case contains its payload in a `value` field. For example, the
variant above generates `OutcomeSuccess(value)` and `OutcomeFailure(value)`. The
containing record's protobuf conversion constructs and matches those case
classes.

.NET uses the variant's nested case records, such as `Outcome.Success(Value)`.
The containing record's transfer-type conversion switches over the protobuf
`<Oneof>Case` property when decoding and pattern-matches the case records when
encoding. Decoding an unset required oneof throws `InvalidOperationException`.
Encoding a required variant property that holds `null` (for example after
`null!`) also throws `InvalidOperationException`. An unset `option<variant>`
decodes to `null`, and encoding `null` leaves the oneof unset.

TypeScript uses tagged object unions, such as
`{ tag: "success", value: output }`. The containing record's transfer converter
switches on the SDK's oneof discriminator and writes the discriminator together
with the selected member. A missing required discriminator throws an `Error`; an
absent optional discriminator decodes as `undefined`. Plain wire objects must
supply the discriminator when a case is selected; member presence is not used to
infer it. Unknown discriminator values and missing selected member values throw
an `Error`, as does encoding an invalid variant tag. Model-only modules export
their model types and transfer converters from `index.ts` without emitting an
empty service.

Generated TypeScript oneof switches use `satisfies never` in their default
branches. Adding an unhandled case to the SDK discriminator or the generated
variant is therefore a compile-time error. Runtime errors still protect against
invalid values passed from JavaScript.

Other targets reject a reachable model containing a oneof they cannot convert.
Unreachable declarations and omitted oneofs remain valid.

---

### @nexus.type-parameter

**Placement:** Type alias **Syntax:** `@nexus.type-parameter` (no arguments)

Declares an opaque model type parameter. Records and variants infer their
ordered generic parameters from public fields and case payloads, including
parameters reached through lists, maps, tuples, results, nested records, and
nested variants. Reusing the same alias correlates the parameter through nested
models and operation inputs and outputs.

```wit
/// @nexus.type-parameter
type context-t = placeholder;

record request {
  context: context-t,
  previous: list<context-t>,
}
```

This generates `Request[ContextT]`-style models in Python, TypeScript, and Go.
.NET follows the C# convention and puts the `T` first, so it generates
`Request<TContext>`. A trailing `T` in the alias name moves to the front, and
.NET adds a `T` prefix to an alias name with no trailing `T`. .NET rejects two
parameters of one declaration that map to the same C# name. In a model with
workflow-function fields, the names `TWorkflow` and `TResult` are also reserved.
A language-specific field-level `@nexus.type` override replaces that field
occurrence and therefore removes it from generic inference for that target.

Type parameters are not currently supported in proto-backed records except in
Python, .NET, and TypeScript when a field or oneof member maps to Temporal's
protobuf `Payload` or `Payloads` carrier. When decoding a parameterized Python
model, concrete type arguments propagate through nested proto-backed records and
become type hints for single-value `Payload` and `Payloads` fields. An
unparameterized model decodes those fields as `typing.Any`. A generic `Payloads`
carrier with an empty envelope decodes as `None`; more than one payload raises
`ValueError` with the field name and payload count. Required carrier fields
still require a wire envelope.

.NET generic proto-backed records declare an open generic transfer-type
converter, such as `typeof(Request<,>.TransferTypeConverter)`, which the SDK
closes with the model's type arguments. Carrier fields decode with
`ProtoExtensions.FromPayload<T>` (or the first value of
`ProtoExtensions.FromPayloads<T>` for `Payloads`) from the support file, so
values decode as the concrete type argument. Like the SDK, a `Payloads` carrier
with no payload decodes as the default value of the type argument, for example
for a `NoValue` result. If a `Payloads` carrier holds more than one payload,
decoding throws `InvalidOperationException` with the field name and the payload
count. This requires `Temporalio` 1.19.0 or newer.

TypeScript retains model type parameters on the default transfer-converter
methods. Generic proto-backed models also export a
`create<Model>TransferTypeConverter` factory that binds optional runtime
`TypeInfo` for each explicit type parameter. Configuration keys are the
parameter names in lower camel case, such as `outputT` and `sourceContextT`. For
example, a notification handler can preserve a successful output as a `RawValue`
while decoding source context normally:

```ts
import { RawValue, rawValueTypeInfo } from "@temporalio/common";
import { createOnCompleteRequestTransferTypeConverter } from "./generated";

const inputType = {
  transferTypeConverter: createOnCompleteRequestTransferTypeConverter<
    RawValue,
    { workflowId: string }
  >({ outputT: rawValueTypeInfo }),
};
```

Type parameters are also unsupported in map keys and in function-signature
metadata.

Python represents generic variants as tagged tuples and protobuf oneof-backed
generic variants as unions of generic case dataclasses. TypeScript uses tagged
object unions, Go uses sealed interfaces and case structs, and .NET uses nested
records. References to generic variants are closed automatically wherever they
occur.

#### Generic resources

A resource that uses a type parameter in its fields, its methods or the
operations that they bind is generic in it. Records, protos and a returned
generic resource count too, so the resource needs no annotation. It takes its
type parameters in the order of their aliases.

```wit
/// @nexus.type-parameter
type body-t = placeholder;

/// @nexus.proto "temporal.api.stream.v1.AppendRecord"
record append-record {
  body: body-t,
}

resource %stream {
  /// @nexus.operation "append-records"
  publish: func(records: list<append-record>) -> append-records-response;
}
```

The handle keeps a runtime type for each type parameter and decodes responses
with it. Python takes `Stream[Turn](...)` with a `StreamRef[Turn]`, or a
keyword, as in `create_stream(stream_ref=ref, body_type=Turn)` or
`client.get_stream(ref=ref, body_type=Turn)`. A create call with no `body_type`
decodes as `typing.Any`, but its static return type is `Stream[object]`; a
concrete `Stream[Turn]` requires a typed ref or `body_type=Turn`. Generic Python
resource handles do not support subclasses; constructing one raises `TypeError`.
TypeScript takes `bodyType` after the fields, and .NET takes the type argument,
as in `CreateStreamAsync<Turn>(options)`.

In Python, a ref explicitly declared generic with `@nexus.generic "body-t"` can
carry the payload type without storing a body. Its phantom parameter defaults to
`object`, and the ref keeps the same wire message, equality and repr. The type
argument rides in memory, in `__orig_class__`. `StreamRef[Turn]`, the ref
returned by a factory with `body_type=Turn`, and a decode with the type hint
`StreamRef[Turn]`, such as a Workflow input's, each set it. `get_stream(ref)`
infers the handle's type from the ref, so a reader that gets a ref keeps its
payload type. An untyped ref produces `Stream[object]`, whose bodies decode as
ordinary Python values. The `ref` attribute is a plain field: direct
construction preserves the supplied ref rather than attaching the handle's type
to it.

For callable Python resources, a constructor record field that explicitly
declares any resource type parameters must declare all of them, and only one
such field is supported. Partial declarations and multiple sources are rejected
during Python backend planning rather than silently disabling inference.
Resources without a declared source retain the explicit runtime-type keyword
API.

```python
stream = client.get_stream(ref)  # Stream[Turn] for a StreamRef[Turn]
stream = client.get_stream(StreamRef(id="s1"), body_type=Turn)  # Choose a decoder
```

An explicit `body_type` must agree with an already-typed ref; it cannot
reinterpret `StreamRef[Turn]` as `StreamRef[str]`. Requests and results
containing a generic ref inherit its parameters too, so
`CreateStreamRequest[Turn]` contains `StreamRef[Turn]`. Decoding a request or
result with a concrete type hint preserves the nested ref's type argument.

Public operation functions preserve this relationship too:
`read_records(ref=ref)` infers and decodes `Turn` bodies from a
`StreamRef[Turn]`, while an unknown ref yields `ReadRecordsResponse[object]`. An
explicit `body_type=Turn` can select the decoder for an unknown ref, but cannot
contradict a typed ref. Write operations such as `append_records` require the
ref and the records to have the same payload type.

Go and Java refuse a generic resource. A record field can't hold one yet, and an
operation with no `@nexus.proto` message can't return one yet.

---

### @nexus.generic

**Placement:** Record **Syntax:** `@nexus.generic "type-parameter-alias"`

Explicitly declares a record's dependency on an alias marked
`@nexus.type-parameter`, even when no field uses that parameter. Repeat the
annotation to declare multiple parameters. The alias must be in scope in the
record's interface, including aliases imported with WIT `use`. Declaring the
same resolved parameter twice is an error.

```wit
/// @nexus.type-parameter
type body-t = placeholder;

/// @nexus.generic "body-t"
record stream-ref {
  id: string,
}

record read-request {
  ref: stream-ref,
}
```

Both `stream-ref` and `read-request` are generic in `body-t`. Explicit
parameters come first in annotation order, followed by parameters inferred from
fields in first-use order; a parameter used by both is included only once.
Dependencies propagate through containing records, variants, operations, and
resources using the same rules as field-inferred parameters.

A parameter unused by the record's fields is a phantom parameter: it affects the
generated type, but adds no field to the model or wire representation. Python
defaults phantom parameters to `object` and retains explicit runtime arguments
for decoding. No record becomes generic merely because a generic resource has a
constructor field of that record type.

---

### @nexus.delay-load-temporalio-workflow

**Placement:** Interface **Syntax:** `@nexus.delay-load-temporalio-workflow`

Delays Python workflow imports until an operation is called. This annotation is
supported only in System Nexus mode (`--system-nexus`), where it avoids eagerly
importing the workflow module from SDK-generated services. It is rejected
outside that mode, whether or not the interface declares an endpoint.

---

### @nexus.proto-field

**Placement:** Record field (within a `@nexus.proto` record) **Syntax:**
`@nexus.proto-field "<proto_field_name>"`

Maps a WIT field name to a differently-named proto field. Without this, the WIT
name converted to `snake_case` is assumed to match.

```wit
record start-workflow-request {
  /// @nexus.proto-field "workflow_type"
  workflow: workflow-function,
}
```

Here the WIT field `workflow` maps to proto field `workflow_type`.

---

### @nexus.name

**Placement:** Record field **Syntax:**
`@nexus.name go="<name>" python="<name>" typescript="<name>" dotnet="<name>"`

Overrides the generated field name for individual target languages. Languages
without an override continue to use the WIT field name.

```wit
record signal-with-start-workflow-request {
  /// @nexus.name go="WorkflowExecutionTimeout"
  execution-timeout: option<duration>,
}
```

This emits `WorkflowExecutionTimeout` in Go while retaining the normal
`execution-timeout` mapping in other languages.

---

### @nexus.type

**Placement:** Type alias or record field **Syntax:**
`@nexus.type python="<type>" typescript="<type>" go="<type>"`

Substitutes the WIT type with a language-native type in generated code. Each
language key is optional; omit a language to skip override for that target.

```wit
/// @nexus.type python="temporalio.common.RetryPolicy" typescript="common.RetryPolicy"
type retry-policy = placeholder;
```

The `placeholder` WIT type is a string alias used when the actual type is
entirely replaced by language-specific overrides.

Go imports required by an override are collected for both type aliases and
record fields. For example, this field generates `time.Duration` and imports
`time` without needing a separate type alias:

```wit
record timing {
  /// @nexus.type go="time.Duration"
  elapsed: u64,
}
```

---

### @nexus.omit

**Placement:** Record field (within a `@nexus.proto` record) **Syntax:**
`@nexus.omit` (no arguments)

Excludes a proto field from the generated API. Use this for infrastructure
fields (identity, request-id, headers) that are not relevant to API consumers.
Omitted fields should use the `placeholder` type.

```wit
/// @nexus.omit
identity: placeholder,
```

Cannot be combined with `@nexus.source`, `@nexus.type`, `@nexus.function`,
`@nexus.default`, or `@nexus.flattened-type`.

---

### @nexus.api-omit

**Placement:** Record field, resource method, or freestanding function
**Syntax:** `@nexus.api-omit [targets="workflow,client"]`

On a record field, keeps the field on generated models and in proto conversion,
but excludes it from generated convenience operation APIs. `targets` limits the
omission to those targets. Unlike `@nexus.omit`, this preserves the field in the
generated model. See [API-omitted fields](#api-omitted-fields) for an example.

On a freestanding function (an operation), the bare `@nexus.api-omit` suppresses
the operation's public top-level function and its export. Only the workflow
target has a top-level operation function, so on a function `@nexus.api-omit` is
effectively workflow-only, and `targets` (if given) must include `workflow`. The
operation stays registered, stays in the generated service, and stays bindable
by resource methods. The private per-operation helper that resource methods call
remains, so the operation is still reached through its resource.

On a field, it cannot be combined with `@nexus.omit`, `@nexus.default`, or a
`@nexus.source` for the same target.

On a resource method, `targets="workflow"` or `targets="client"` leaves the
method out of the resource class of that target, with the helpers that only it
used. The operation stays in the service and in the other target. Without its
`@nexus.close` method, the resource of that target is not a context manager.
nexgen refuses a method omission without one target, since no target would keep
the method. Remove such a method instead.

```wit
/// @nexus.operation "append-records"
/// @nexus.api-omit targets="workflow"
append: func(records: list<append-record>) -> append-records-response;
```

---

### @nexus.source

**Placement:** Record field **Syntax:**
`@nexus.source [targets="workflow,client"] python="<helper-call>" typescript="<helper-call>" go="<helper-call>" dotnet="<helper-call>"`

Provides a field value through a helper in the package specified by
`--support-package`. The field is not an argument in operation convenience APIs.
Write a helper call with its arguments, but without the package prefix. Nexgen
adds the prefix and imports support when it generates the sourced field. Other
expressions, including property access after a call, are not valid here.

```wit
/// @nexus.source python="workflow_namespace()" typescript="workflowNamespace()" go="WorkflowNamespace(ctx)" dotnet="TemporalWorkflowContext.WorkflowNamespace()"
namespace: string,
```

The sample's workflow API calls `_support.workflow_namespace()` in Python or
`support.WorkflowNamespace(ctx)` in Go while constructing a request. Restrict
this helper to the workflow target with `targets="workflow"`. Without a target
list, a source applies to all targets. Models and wire conversion never evaluate
the source.

Repeat the directive to use different helpers for different targets:

```wit
/// @nexus.source targets="workflow" python="workflow_namespace()"
/// @nexus.source targets="client" python="client_namespace()"
namespace: string,
```

Repeated source directives must have disjoint target sets. A directive without
`targets` covers all targets, so it overlaps any other source directive on the
same field. Overlaps are errors, not declaration-order overrides.

Cannot be combined with `@nexus.default` for the same language.

---

### @nexus.default

**Placement:** Record field **Syntax:**
`@nexus.default [targets="workflow,client"] python="<expression>" typescript="<expression>" go="<expression>" dotnet="<expression>"`

Makes an operation argument omittable and calculates its value when the
generated operation prepares its outbound request. The field must have a
non-option type: `@nexus.default` is rejected on `option<T>`, including aliases
to option types. Without a target list, a default applies to all targets.
Restrict a default that needs a Workflow context, such as
`_support.workflow_task_queue()`, with `targets="workflow"`.

Repeat the directive with disjoint target sets to choose different defaults:

```wit
/// @nexus.default targets="workflow" python="'workflow-label'"
/// @nexus.default targets="client" python="'client-label'"
label: string,
```

Repeated default directives follow the same disjoint-target rule as sources:
omitting `targets` covers all targets, and overlapping sets are errors.

```wit
/// @nexus.default
///   python="temporalio.common.WorkflowIDReusePolicy.ALLOW_DUPLICATE"
///   typescript="common.WorkflowIdReusePolicy.ALLOW_DUPLICATE"
///   go="enums.WORKFLOW_ID_REUSE_POLICY_ALLOW_DUPLICATE"
///   dotnet="Temporalio.Api.Enums.V1.WorkflowIdReusePolicy.AllowDuplicate"
id-reuse-policy: workflow-id-reuse-policy,
```

The operation evaluates an expression only when its argument is missing and only
once per invocation. Support helpers must explicitly use `_support` in Python,
`support` in Go and TypeScript, or a class in the configured .NET support
namespace. Go expressions can use `ctx`.

Python uses `None` as the omission marker; TypeScript uses `undefined`; and .NET
uses null on the options surface. Go uses default-punning: empty strings, false,
zero, and nil represent an omitted argument.

Defaults apply only to exposed operation arguments, including flattened
arguments. Supplied nested records must be complete and pass through unchanged;
defaults are not applied to their fields or to records inside lists or maps.
Constructors and encoding/decoding do not evaluate defaults.

A default cannot be combined with `@nexus.source` for the same language.

---

### @nexus.fallback

**Placement:** Case of an enum with `@nexus.proto` **Syntax:** `@nexus.fallback`

A protobuf enum is open, so a newer peer can send a number that the generated
enum doesn't know. When a protobuf value converts to the model, such a number
becomes the marked case. An enum has at most one fallback case, and the
directive takes no arguments.

```wit
/// @nexus.proto "temporal.api.enums.v1.StreamStatus"
enum stream-status {
  /// @nexus.fallback
  stream-status-unspecified,
  stream-status-open,
  stream-status-closed,
  stream-status-expired,
}
```

Python, TypeScript, and .NET decode an unknown number as the case. Without the
directive, Python raises `ValueError`, and TypeScript and .NET keep the number.
Go and Java ignore the directive. nexgen refuses it on an enum without
`@nexus.proto`, and anywhere other than an enum case.

---

### @nexus.experimental

**Placement:** Interface, record, variant, or operation (function) **Syntax:**
`@nexus.experimental`

Adds an experimental API warning to the generated declaration's documentation.
The directive takes no arguments. Mark declarations individually; marking an
interface does not implicitly mark its types. For variants, the warning belongs
to the variant declaration, not its individual cases, in Python, TypeScript, and
Go. In .NET, the warning appears on both the variant's base record and each
nested case record.

```wit
/// @nexus.experimental
variant outcome {
  success(string),
  failure(string),
}
```

---

### @nexus.doc

**Placement:** Record, record field, variant, variant case, operation
(function), resource, or resource method **Syntax:**
`@nexus.doc "<text>" [returns="<text>"]` Per-language overrides:
`python="<text>"` `typescript="<text>"` `go="<text>"` `dotnet="<text>"`
`java="<text>"` Return docs: `python-returns="<text>"`
`typescript-returns="<text>"` `go-returns="<text>"` `dotnet-returns="<text>"`
`java-returns="<text>"`

Adds documentation to generated code. The default text applies to all languages;
per-language keys override for specific targets. The `returns` key generates
return-value documentation. Generated documentation preserves paragraph breaks,
wraps prose to 88 columns, and escapes comment terminators (plus HTML-sensitive
Javadoc text) so authored content cannot break the generated source.

```wit
/// @nexus.doc
///   "Signal a workflow, starting it first if needed."
///   returns="A workflow handle to the started workflow."
signal-with-start-workflow: func(...) -> ...;
```

Python emits the docs of a resource and its methods as docstrings. The other
targets don't emit them yet.

Variant documentation is emitted by Python, TypeScript, Go, and .NET; Java does
not currently generate WIT variants. Python emits an attribute docstring after
the variant type alias, comments beside tagged-tuple cases, and class docstrings
on protobuf oneof-backed cases. TypeScript emits JSDoc on the union and its
branches; Go emits godoc on the interface and case structs; .NET emits XML
documentation on the base record and nested case records.

```wit
/// @nexus.doc "Where to send a notification."
variant notification-target {
  /// @nexus.doc "Send to an email address." python="Send to the given email address."
  email(string),
  /// @nexus.doc "Do not send a notification."
  none,
}
```

---

### @nexus.operation

**Placement:** Operation (function) **Syntax:**
`@nexus.operation name="<WireOperationName>"`

Overrides the Nexus wire operation name. Without this, the WIT function name
converted to UpperCamelCase is used (e.g., `signal-with-start-workflow` becomes
`SignalWithStartWorkflow`).

```wit
/// @nexus.operation name="SignalWithStartWorkflowExecution"
signal-with-start-workflow: func(...) -> ...;
```

---

### @nexus.context-propagation

**Placement:** Operation (function) **Syntax:**
`@nexus.context-propagation go="request.headers"`

Enables outbound workflow context propagation for a Go System Nexus operation
generated into `go.temporal.io/sdk/internal`. It requires `--targets workflow`
and a protobuf-backed request record. The Go backend ignores this directive when
generating into any other package because header injection calls the SDK's
private helpers. Operations bound to resource methods are not supported because
those methods bypass the typed interceptor chain.

The value is a checked WIT field path, not a Go expression. Its root must be the
operation's input parameter name, and it must select a direct field of that
record. Input record aliases are supported; nested paths are not. Nexgen
resolves the WIT field identity before applying `@nexus.proto-field` and
`@nexus.name` mappings, so the path uses authored WIT names.

The selected field must use `@nexus.api-omit`. Its Go binding must accept
`map[string]*commonpb.Payload`; the Go compiler checks that binding. For
example, using the Temporal `header` binding:

```wit
/// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest"
record signal-with-start-workflow-request {
  /// @nexus.api-omit
  /// @nexus.proto-field "header"
  headers: option<header>,
}

/// @nexus.context-propagation go="request.headers"
signal-with-start-workflow: func(
  request: signal-with-start-workflow-request,
) -> signal-with-start-workflow-response;
```

The public function creates a fresh header map before the typed interceptor
chain. Interceptors read and modify it with `interceptor.WorkflowHeader(ctx)`.
The terminal calls the configured propagators' `InjectFromWorkflow` methods with
the forwarded context, then assigns the map to the selected request field before
encoding or scheduling. Injection errors resolve the returned workflow `Future`
with an error and do not schedule an operation. Interceptor-supplied headers are
preserved unless a propagator overwrites the same key.

These are Temporal payload headers inside the operation request, not Nexus
transport headers. The directive does not enable inbound extraction or response
propagation. Use `go=` in multi-language inputs to keep it inactive for other
backends.

---

### @nexus.close

**Placement:** Resource method **Syntax:** `@nexus.close` (no arguments)

Marks the method that ends the resource. Its parameters must be optional. See
[Closing a Resource](#closing-a-resource).

---

### @nexus.varargs

**Placement:** Resource method **Syntax:** `@nexus.varargs "<parameter>"`

The method takes the items of that list parameter as positional arguments. See
[Variadic Items](#variadic-items).

---

### @nexus.serialization-context

**Placement:** Operation (function) **Syntax:**
`@nexus.serialization-context python="<support-helper>"`

Supplies a support helper that returns the serialization context to use when
encoding operation inputs. Use this when the generated operation request
contains user payloads that should be serialized for a context other than the
Nexus operation itself, such as signal-with-start payloads that will be received
by the target workflow.

The generated operation registry stores the helper alongside the operation
definition. SDKs use that registry entry to call the helper with the operation
request and construct the target serialization context before converting user
payloads.

The helper is invoked with the actual generated operation request model. In
Python examples below, the request is annotated as `typing.Any` because support
files are shared inputs that do not import the generated model class from each
output package. The runtime value is still the generated request model, and the
helper must return a `temporalio.converter.SerializationContext`.

```wit
/// @nexus.operation name="SignalWithStartWorkflowExecution"
/// @nexus.serialization-context python="signal_with_start_workflow_serialization_context"
signal-with-start-workflow: func(
  request: signal-with-start-workflow-request,
) -> signal-with-start-workflow-response;
```

```python
import typing

import temporalio.converter


def signal_with_start_workflow_serialization_context(
    request: typing.Any,
) -> temporalio.converter.WorkflowSerializationContext:
    # At runtime, request is the actual generated operation request model.
    return temporalio.converter.WorkflowSerializationContext(
        namespace=request.namespace,
        workflow_id=request.id,
    )
```

The referenced helper must be available from the package specified by
`--support-package`.

---

### @nexus.output-transform

**Placement:** Operation (function) **Syntax:**

```
@nexus.output-transform
  python-type="<type>" python="<expr>"
  typescript-type="<type>" typescript-type-import="<type-module>"
  typescript="<expr>" typescript-import="<value-module>"
  go-type="<type>" go="<expr-returning-T-error>"
```

Transforms the raw operation result to a different return type. The expression
is emitted as written and has access to `request` and `result` variables.
Helpers used by the expression must be available to the generated code. Go
expressions must return `(T, error)`.

```wit
/// @nexus.output-transform
///   python-type="workflow.ExternalWorkflowHandle[typing.Any]"
///   python="workflow.get_external_workflow_handle(request.id, run_id=result.run_id)"
///   typescript-type="ExternalWorkflowHandle"
///   typescript-type-import="../../../../workflow"
///   typescript="workflow.getExternalWorkflowHandle(request.id, result.runId ?? undefined)"
///   typescript-import="@temporalio/workflow"
///   go-type="example.com/handles:handles.WorkflowHandle"
///   go="handles.FromSignalWithStart(request, &result)"
```

Both the type and expression must be provided together for each language. The
TypeScript import fields are optional, but required when the generated return
type or transform expression refers to an otherwise-unavailable module.

---

### @nexus.poll

**Placement:** Operation (function) **Syntax:**
`@nexus.poll python="<support-helper>" targets="client"`, with optional
`items="<field>"` and `python-resume="<support-helper>"`

Generates an `iter_<name>` async iterator on the client extension, and on each
client resource method that binds the operation. The iterator sends the request,
yields the response, and calls `<support-helper>(request, response)` for the
next request. The helper is a sync function in the `--support-package`. It
returns `None` to stop.

```wit
/// @nexus.operation name="ReadRecords"
/// @nexus.poll python="next_read_request" targets="client"
read-records: func(request: read-records-request) -> read-records-response;
```

```python
def next_read_request(request, response):
    if response.done:
        return None
    # A read that doesn't wait stops at the newest Record.
    if not response.records and not request.wait:
        return None
    return dataclasses.replace(request, from_offset=response.next_offset)
```

With `items="<field>"`, nexgen also generates `iter_<name>_items`. It takes the
parameters of `iter_<name>`, reads its pages, and yields the items of the list
field `<field>` of each response. It returns a `PollIterator`, whose
`next_request` is the request to resume with:

- After an item, the `python-resume` helper gives it. The helper takes the
  page's request and the item, and returns the request that resumes after the
  item.
- Without `python-resume`, it reads the item's page again, so a resumed read can
  yield an item twice.
- After a page's last item, it is the page helper's request. It is `None` after
  the last page.

```wit
/// @nexus.poll python="next_read_request" items="records"
///   python-resume="resume_read_request" targets="client"
read-records: func(request: read-records-request) -> read-records-response;
```

```python
def resume_read_request(request, item):
    return dataclasses.replace(request, from_offset=item.offset + 1)
```

To resume, pass the fields of a saved `next_request` to `iter_<name>_items`. A
resource method's iterator takes only the method's parameters, such as
`from_offset` of the Stream's `iter_read_items`.

The stream sample's full helpers are in
[`temporal_support/poll.py`](advanced/samples/python/temporal_support/poll.py).

- `iter_<name>` yields pages, and `iter_<name>_items` yields the items in them.
- All pages use one Nexus client. Each page is a new operation with a new random
  operation ID.
- Only the Python client target generates the iterator, so `targets` must be
  `client`.
- nexgen refuses the directive on a resource method, on an operation that it
  can't poll, and when an iterator clashes with another member.
- nexgen refuses an `items` field that the response doesn't have, a field with
  `@nexus.omit`, or a field that isn't a list in the model, such as a proto
  `bytes` field. It also refuses `python-resume` without `items`, and `items` in
  a package with a model or resource called `PollIterator`.
- Use `asyncio.timeout` to limit a whole iteration. After an error, start a new
  iteration at the last page's `next_offset`.

---

### @nexus.flatten-in-api

**Placement:** Record type (with `@nexus.proto`), or a request field of such a
type **Syntax:** `@nexus.flatten-in-api` (no arguments)

Flattens the record's fields into the parent record's generated API surface
instead of creating a nested model. On a field, it flattens only that call. See
[Flattening one field](#flattening-one-field).

```wit
/// @nexus.proto "temporal.api.sdk.v1.UserMetadata"
/// @nexus.flatten-in-api
record user-metadata {
  static-summary: option<payload>,
  static-details: option<payload>,
}
```

In Go, `@nexus.flatten-in-api` promotes the record's fields into the generated
operation options struct. For a parent request with
`user-metadata: option<user-metadata>` and string flattened-type overrides, the
options struct exposes `StaticSummary string` and `StaticDetails string`:

```go
opts := SomeOperationOptions{}
opts.StaticSummary = "Nightly sync"
```

As described in [Flattened Records](#flattened-records), the generated helper
leaves the nested metadata nil when both strings are empty. Each nonempty option
populates its corresponding payload; an empty option remains absent from the
nested protobuf message. This follows Go's default-punning convention, so the
public options cannot distinguish an omitted string from an explicitly empty
string.

The containing record is an unexported implementation type in Go unless another
public signature exposes it, such as an operation result or a non-flattened
input field. Types referenced by the promoted fields remain public as needed.

---

### @nexus.flattened-type

**Placement:** Field within a `@nexus.flatten-in-api` record **Syntax:**
`@nexus.flattened-type python="<type>" typescript="<type>"`

Overrides the type of a field when it is flattened into the parent API. Useful
when the non-flattened type (e.g., `Payload`) should be simplified (e.g., to
`str`) in the flattened context.

Go ignores `@nexus.flattened-type`; embedded fields keep the generated model's
normal Go field types.

```wit
/// @nexus.flattened-type python="str"
static-summary: option<payload>,
```

---

### @nexus.function

**Placement:** Type alias or record field **Syntax:**

```
@nexus.function
  signature="<wit-function-name>"
  [alternate-type="<wit-type>"]
  [args-field="<request-or-proto-field>"]
  [result-type-parameter="<PythonTypeVarName>"]
  [primary=<bool>]
  [converter="<func>"]
  [<language>-converter="<func>"]
```

Marks a type alias as a callable function reference. See
[Function References](#function-references) for motivation, WIT samples, and
generated Python samples.

---

### @nexus.function-args

**Placement:** Function used as a `@nexus.function signature` **Syntax:**

```
@nexus.function-args
  varargs=true
  [param="<final-parameter-name>"]
  [typescript-drop-prefix=<bool>]
```

Marks the final signature parameter as a variable argument list. See
[Function References](#function-references) for examples.

```wit
/// @nexus.function-args
///   varargs=true
///   param="args"
workflow-call: async func(callable-prefix: callable-prefix, args: payloads) -> workflow-result;
```

---

### @nexus.add-rpc-compatible-with

**Placement:** Type alias (in a linked/dependency WIT package) **Syntax:**
`@nexus.add-rpc-compatible-with "<type-name>"`

Declares that a type is assignment-compatible with another type for the
`add-rpc` and `add-message` scaffolding commands. This allows the scaffolder to
substitute compatible types when generating WIT from proto RPCs.

```wit
/// @nexus.add-rpc-compatible-with "workflow-type"
type workflow-function = placeholder;
```

This directive has no effect on code generation -- it only affects the scaffold
commands.

### Scaffolding existing proto-backed models

The advanced `add-message` and `add-rpc` commands share record reconciliation.
Selected messages, or the RPC request and response, supply the roots. Both
commands check reachable mapped child records, add missing owned declarations
and fields, and reject stale or incompatible fields. Reused oneof variants are
checked for missing or extra cases and incompatible payloads.

Explicit `@nexus.omit` fields and compatible substitutions remain intentional
boundaries. Opaque proto type mappings are preserved rather than expanded into
records, and type parameters backed by the supported Payload/Payloads carriers
are preserved. Linked records are validated but not edited; missing fields there
must be fixed in the dependency package. Reconciliation completes before the
output is written, and rerunning it on complete WIT leaves the model unchanged.
