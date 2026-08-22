# WIT Author's Guide to nexgen

This guide explains what the code generator produces from your WIT definitions,
with examples in Python and TypeScript. It covers every type mapping, the
resource name-binding mechanism (including edge cases), and a complete glossary
of `@nexus` directives.

## Contents

- [Quick Start](#quick-start)
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
  - [Edge Cases and Error Conditions](#edge-cases-and-error-conditions)
- [Proto-Backed Models](#proto-backed-models)
  - [Transfer-Type Conversion](#transfer-type-conversion)
  - [Sourced Fields](#sourced-fields)
  - [Omitted Fields](#omitted-fields)
  - [Flattened Records](#flattened-records)
  - [Output Transforms](#output-transforms)
- [Function References](#function-references)
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

WIT variants become discriminated unions using a `tag` field.

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

Cases with a payload carry a `value` field; cases without a payload do not.

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

Lists and maps that are optional default to empty collections, not `None`.

```wit
record user-profile {
  tags: list<string>,               // optional in this context
  metadata: map<string, string>,    // optional in this context
}
```

**Python:**

```python
tags: list[str] | None = dataclasses.field(default_factory=list)
metadata: dict[str, str] | None = dataclasses.field(default_factory=dict)
```

**TypeScript:**

```typescript
tags?: string[];
metadata?: Record<string, string>;
```

Whether a field is required or optional is determined by the WIT `option<>`
wrapper (for WIT-direct fields) or proto field presence semantics (for
proto-backed fields).

---

## Resources

Resources are the most complex feature of the code generator. They produce
classes whose constructor fields represent the resource's identity, and whose
methods delegate to Nexus operations.

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
@dataclasses.dataclass
class User:
    user_id: str
    email: str

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

It then tries every operation in the service as a candidate. For each
candidate operation, it checks whether every **required** field in the
operation's input record can be satisfied by a name in the environment.

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
generator recursively tries to match the nested record's fields against the
same environment.

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

After field matching, the generator also filters candidates by return type.
If the method returns `own<user>`, only operations that also return
`own<user>` (or the equivalent proto type) are considered.

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

### Edge Cases and Error Conditions

#### Same field name in constructor and method parameter

When a resource constructor field and a method parameter have the same name,
the **method parameter takes precedence**. This is the expected pattern for
"update" operations:

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

If no operation's input fields can be satisfied by the method's environment,
the method becomes a **stub** that raises an error at runtime:

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
and disambiguation by name fails, the generator raises
`InvalidResourceMethod` with a list of the ambiguous matches. To fix this,
rename either the method or one of the operations so the preferred name
match succeeds, or adjust the operation's input record so fewer candidates
match.

#### Required field in operation input has no match

If an operation's input record has a required field whose name does not appear
in the resource constructor or method parameters, that operation will silently be
skipped as a candidate. So watch out for typos!

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

When a WIT record is annotated with `@nexus.proto`, the generator produces
code that converts between the public model and its protobuf transfer type.
Generated callers continue to accept and return the public model; conversion is
performed by the target SDK or generated operation helpers.

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
class _ActivityOptionsTransferTypeConverter(
    temporalio.converter.TransferTypeConverter["ActivityOptions", ProtoActivityOptions]
):
    def from_transfer_type(self, value, type_hint) -> "ActivityOptions":
        return ActivityOptions(
            task_queue=task_queue_from_proto(value.task_queue)
                if value.HasField("task_queue") else None,
            retry_policy=retry_policy_from_proto(value.retry_policy),
        )

    def to_transfer_type(self, value: "ActivityOptions") -> ProtoActivityOptions:
        message = ProtoActivityOptions()
        if value.task_queue is not None:
            message.task_queue.CopyFrom(task_queue_to_proto(value.task_queue))
        message.retry_policy.CopyFrom(retry_policy_to_proto(value.retry_policy))
        return message

@temporalio.converter.transfer_type_convertible(_ActivityOptionsTransferTypeConverter)
@dataclasses.dataclass(slots=True, kw_only=True)
class ActivityOptions:
    task_queue: str | None = None
    retry_policy: temporalio.common.RetryPolicy
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

export const ActivityOptions = {
    fromProto(proto: temporal.api.activity.v1.IActivityOptions | null | undefined) {
        if (proto == null) return undefined;
        return {
            taskQueue: proto.taskQueue == null ? undefined
                : taskQueueFromProto(proto.taskQueue),
            retryPolicy: requiredField(
                retryPolicyFromProto(requiredField(proto.retryPolicy, ...)), ...
            ),
        };
    },
    toProto(model: ActivityOptions | null | undefined) {
        if (model == null) return undefined;
        return {
            taskQueue: model.taskQueue == null ? undefined
                : taskQueueToProto(model.taskQueue),
            retryPolicy: retryPolicyToProto(model.retryPolicy),
        };
    },
};
```

Required fields are validated in `from_proto` -- missing required proto fields
raise a `ValueError` (Python) or throw an `Error` (TypeScript). .NET output
requires `Temporalio` 1.18.0 or newer.

### Sourced Fields

Fields annotated with `@nexus.source` are not exposed in the user-facing API.
Instead, transfer-type conversion calls a support function to obtain the value.

```wit
record start-workflow-request {
  workflow-id: string,
  /// @nexus.source python="workflow_namespace" typescript="workflowNamespace"
  namespace: string,
}
```

The `namespace` field does not appear as a constructor parameter. During
transfer-type conversion:

```python
message.namespace = workflow_namespace()   # auto-injected
```

The support function must be defined in the support file referenced by
`@nexus.support`.

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

`@nexus.api-omit` cannot be combined with `@nexus.omit` or `@nexus.source`.

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

When a parent record has `user-metadata: option<user-metadata>`, the
convenience wrapper exposes `static_summary` and `static_details` as top-level
parameters instead of requiring a nested `UserMetadata(...)` object:

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
all its constituent fields are `None`. This is the behavior in Python and Typescript,
but not in Go; in Go we implement flattening with value embedding, so `user-metadata`
is never nil.

### Output Transforms

An operation annotated with `@nexus.output-transform` transforms the raw
operation result into a different type before returning it.

```wit
/// @nexus.output-transform
///   python-type="workflow.ExternalWorkflowHandle[typing.Any]"
///   python="workflow.get_external_workflow_handle(request.id, run_id=result.run_id)"
///   typescript-type="workflow.ExternalWorkflowHandle"
///   typescript="workflow.getExternalWorkflowHandle(request.id, result.runId ?? undefined)"
///   go-type="example.com/handles:handles.WorkflowHandle"
///   go="handles.FromSignalWithStart(request, &result)"
signal-with-start-workflow: func(
  request: signal-with-start-workflow-request,
) -> signal-with-start-workflow-response;
```

The generated wrapper returns the transformed type. The transform expression
has access to `request` (the input) and `result` (the raw response). Go
transform expressions must evaluate to `(T, error)`, where `T` is the
`go-type` value:

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

Now any field of type `my-exe` is treated as a function reference with the specified signature.

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

`alternate-type` allows the caller to pass other types as the
function argument, besides callables.

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

`@nexus.function-args varargs=true` changes the last parameter into
a variable argument list.

```wit
/// @nexus.function-args varargs=true
varargs-function-call: func(args: list<string>) -> string;

/// @nexus.function signature="varargs-function-call"
type varargs-function = placeholder;
```

When the signature has multiple parameters, use `param` to
specify which one is variadic:

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

For proto-backed APIs, the WIT record often
has a wire field like `input` or `signal-input` that stores a payload list.
`args-field` maps the ergonomic Python arguments back to that field.

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

When an operation has multiple `@nexus.function` parameters, use `primary=true` to
indicate which function is "primary".

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
the wire value. Signals are the common example: callers
can pass a string signal name or a Python signal method, while the proto field
is just `signal_name`.

```wit
/// @nexus.function
///   signature="signal-call"
///   args-field="signal-input"
///   alternate-type="string"
///   python-converter="signal_function_to_proto"
///   typescript-converter="signalFunctionToProto"
type signal-function = placeholder;
```

Generated Python imports and calls the converter during transfer-type
conversion:

```python
from ._support import signal_function_to_proto

@dataclasses.dataclass(slots=True, kw_only=True)
class SignalWithStartWorkflowRequest:
    signal: str | collections.abc.Callable[..., None | collections.abc.Awaitable[None]]
    signal_args: list[typing.Any] | None = None

    def to_transfer_type(self, value: "SignalWithStartWorkflowRequest"):
        message.signal_name = signal_function_to_proto(value.signal)
        if value.signal_args is not None:
            message.signal_input.CopyFrom(payloads_to_proto(value.signal_args))
        return message
```

Use `converter="<func>"` when the same helper name applies to every generated
language, or use language-prefixed keys such as `python-converter` and
`typescript-converter` when helper names differ.

### Direct Field Result Overrides

Most authored APIs should use the type-alias `signature` form because it keeps
arguments and result types in WIT. A direct record-field annotation is available
for the narrower case where the field itself describes a callable and the
result type is already language-specific.

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

| Option                         | Motivation                                                                     | Generated effect                                                                                            |
| ------------------------------ | ------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------- |
| `signature`                    | Keep callable shape in a WIT function instead of a placeholder alias.          | Derives callable argument and result annotations. Required for type-alias `@nexus.function`.                |
| `alternate-type`               | Accept raw names as well as typed callables.                                   | Generates union annotations and overloads such as `str \| Callable[...]`.                                   |
| `args-field`                   | Map ergonomic function args to a wire field such as `input` or `signal-input`. | Stores normalized args in that request/proto field. Defaults to the signature's args name for type aliases. |
| `result-type-parameter`        | Preserve the callable's result type in returned handles.                       | Emits a type variable such as `WorkflowResult` and uses it in callable overload returns.                    |
| `primary`                      | Identify the main callable when a request has multiple function references.    | Enables positional primary args and result type propagation. Defaults to `false`.                           |
| `converter`                    | Reuse the same conversion helper name in all languages.                        | Calls the helper when converting the function field to proto.                                               |
| `<language>-converter`         | Use language-specific conversion helper names.                                 | Python uses `python-converter`; TypeScript uses `typescript-converter`.                                     |
| `result` / `<language>-result` | Direct field-level form when there is no type alias `signature`.               | Supplies the callable result annotation directly. Prefer `signature` for authored aliases.                  |

`@nexus.function-args` options:

| Option                   | Motivation                                                             | Generated effect                                           |
| ------------------------ | ---------------------------------------------------------------------- | ---------------------------------------------------------- |
| `varargs`                | Treat a final list-shaped signature parameter as `*args`.              | Generates positional overloads plus list-form `args`.      |
| `param`                  | Disambiguate which final parameter is the varargs list.                | Required when the signature has more than one parameter.   |
| `typescript-drop-prefix` | Remove implicit receiver/context parameters from TypeScript inference. | TypeScript omits the prefix from inferred argument tuples. |

---

## Directive Glossary

All directives are written in WIT doc comments (`///`) and prefixed with
`@nexus.`. Multi-line directives continue on subsequent `///` lines with
indented key-value pairs.

### @nexus.endpoint

**Placement:** Interface doc comment
**Syntax:** `@nexus.endpoint "<endpoint-name>"`

Names the Nexus endpoint for the service. This is the value used when
registering or connecting to the endpoint at runtime.

```wit
/// @nexus.endpoint "user-service"
interface user-service { ... }
```

The service name (used in generated code) is derived from the interface name
converted to PascalCase: `user-service` becomes `UserService`.

---

### @nexus.support

**Placement:** Package doc comment
**Syntax:** `@nexus.support python="<path>" typescript="<path>"`

Includes external support code in the generated output. Paths are resolved
relative to the WIT file. Python support files are copied into a private
`_support/` package; TypeScript support files become `support.ts` next to the
generated `index.ts`.

```wit
/// @nexus.support
///   python="python/model_overrides.py"
///   typescript="typescript/model_overrides.ts"
package nexus:temporal-types@1.0.0;
```

---

### @nexus.proto

**Placement:** Type alias, record, or enum
**Syntax:** `@nexus.proto "<fully.qualified.proto.MessageName>"`

Maps a WIT type to a protobuf message or enum. The generator emits
target-specific transfer-type conversion code and validates field mappings
against the proto descriptor. Requires `--descriptors` on the CLI.

```wit
/// @nexus.proto "temporal.api.activity.v1.ActivityOptions"
record activity-options { ... }
```

Protobuf `oneof` groups are authored as a single record field whose name
matches the oneof and whose type is a WIT variant. Each variant case must match
a protobuf member name (with kebab-case WIT naming) and carry the corresponding
member value. Use `option<variant>` when the protobuf oneof may be unset, or a
required `variant` when an unset value should be rejected during conversion.

```wit
variant outcome {
  success(list<u8>),
  failure(string),
}

/// @nexus.proto "example.v1.Response"
record response {
  result: option<outcome>,
}
```

Python performs bidirectional oneof conversion using tagged tuples such as
`("success", value)`. Other targets reject a reachable model containing a
oneof they cannot convert; unreachable declarations and omitted oneofs remain valid.

---

### @nexus.type-parameter

**Placement:** Type alias
**Syntax:** `@nexus.type-parameter` (no arguments)

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

This generates `Request[ContextT]`-style models in Python, TypeScript, Go, and
.NET. A language-specific field-level `@nexus.type` override replaces that
field occurrence and therefore removes it from generic inference for that
target.

Type parameters are not currently supported in proto-backed records except in
Python when a field or oneof member maps to Temporal's protobuf `Payload` or
`Payloads` carrier. When decoding a parameterized Python model, concrete type
arguments propagate through nested proto-backed records and become type hints
for single-value `Payload` fields. An unparameterized model decodes those fields
as `typing.Any`. `Payloads` fields continue to decode as untyped sequences.

Type parameters are also unsupported in resources, map keys, function-signature
metadata, or resource-bound generic operations.

.NET also rejects every generic proto-backed record: the current SDK transfer
type converter registration cannot instantiate an open generic converter. The
generator reports this explicitly rather than emitting a model that cannot be
serialized through the SDK.

Generic variants retain each target's normal tagged representation: tagged
tuples in Python, tagged object unions in TypeScript, sealed interfaces and
case structs in Go, and nested records in .NET. References to generic variants
are closed automatically wherever they occur.

---

### @nexus.proto-field

**Placement:** Record field (within a `@nexus.proto` record)
**Syntax:** `@nexus.proto-field "<proto_field_name>"`

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

**Placement:** Record field
**Syntax:** `@nexus.name go="<name>" python="<name>" typescript="<name>" dotnet="<name>"`

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

**Placement:** Type alias, enum, or record field
**Syntax:** `@nexus.type python="<type>" typescript="<type>" go="<type>"`

Substitutes the WIT type with a language-native type in generated code. Each
language key is optional; omit a language to skip override for that target.

```wit
/// @nexus.type python="temporalio.common.RetryPolicy" typescript="common.RetryPolicy"
type retry-policy = placeholder;
```

The `placeholder` WIT type is a string alias used when the actual type is
entirely replaced by language-specific overrides.

---

### @nexus.omit

**Placement:** Record field (within a `@nexus.proto` record)
**Syntax:** `@nexus.omit` (no arguments)

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

### @nexus.source

**Placement:** Record field (within a `@nexus.proto` record)
**Syntax:** `@nexus.source python="<func>" typescript="<func>" go="<Func>"`

Populates a field by calling a support function instead of exposing it as an API
parameter. The function must be defined in the support file.

```wit
/// @nexus.source python="workflow_namespace" typescript="workflowNamespace"
namespace: string,
```

During transfer-type conversion, this becomes
`message.namespace = workflow_namespace()`.

Cannot be combined with `@nexus.default`.

---

### @nexus.default

**Placement:** Record field (whose type is an enum)
**Syntax:** `@nexus.default "<enum-case-name>"`

Sets a default value for an enum field, making it optional in the generated API.

```wit
/// @nexus.default "allow-duplicate"
id-reuse-policy: workflow-id-reuse-policy,
```

**Python:**

```python
id_reuse_policy: temporalio.common.WorkflowIDReusePolicy = (
    temporalio.common.WorkflowIDReusePolicy.ALLOW_DUPLICATE
)
```

Cannot be combined with `@nexus.source`.

---

### @nexus.doc

**Placement:** Record field or operation (function)
**Syntax:** `@nexus.doc "<text>" [returns="<text>"]`
Per-language overrides: `python="<text>"` `typescript="<text>"` `go="<text>"`
Return docs: `python-returns="<text>"` `typescript-returns="<text>"` `go-returns="<text>"`

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

---

### @nexus.operation

**Placement:** Operation (function)
**Syntax:** `@nexus.operation name="<WireOperationName>"`

Overrides the Nexus wire operation name. Without this, the WIT function name
converted to UpperCamelCase is used (e.g., `signal-with-start-workflow` becomes
`SignalWithStartWorkflow`).

```wit
/// @nexus.operation name="SignalWithStartWorkflowExecution"
signal-with-start-workflow: func(...) -> ...;
```

---

### @nexus.serialization-context

**Placement:** Operation (function)
**Syntax:** `@nexus.serialization-context python="<support-helper>"`

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

The referenced helper must be provided through `@nexus.support`.

---

### @nexus.output-transform

**Placement:** Operation (function)
**Syntax:**

```
@nexus.output-transform
  python-type="<type>" python="<expr>"
  typescript-type="<type>" typescript="<expr>"
  go-type="<type>" go="<expr-returning-T-error>"
```

Transforms the raw operation result to a different return type. The expression
has access to `request` and `result` variables. Go expressions must return
`(T, error)`.

```wit
/// @nexus.output-transform
///   python-type="workflow.ExternalWorkflowHandle[typing.Any]"
///   python="workflow.get_external_workflow_handle(request.id, run_id=result.run_id)"
///   typescript-type="workflow.ExternalWorkflowHandle"
///   typescript="workflow.getExternalWorkflowHandle(request.id, result.runId ?? undefined)"
///   go-type="example.com/handles:handles.WorkflowHandle"
///   go="handles.FromSignalWithStart(request, &result)"
```

Both the type and expression must be provided together for each language.

---

### @nexus.flatten-in-api

**Placement:** Record type (with `@nexus.proto`)
**Syntax:** `@nexus.flatten-in-api` (no arguments)

Flattens the record's fields into the parent record's generated API surface
instead of creating a nested model.

```wit
/// @nexus.proto "temporal.api.sdk.v1.UserMetadata"
/// @nexus.flatten-in-api
record user-metadata {
  static-summary: option<payload>,
  static-details: option<payload>,
}
```

Only supported on record types.

In Go, `@nexus.flatten-in-api` is rendered as value embedding in the generated
operation options struct. For a parent request with
`user-metadata: option<user-metadata>`, the options struct embeds
`UserMetadata`, so callers can set fields through the embedded value:

```go
opts := SomeOperationOptions{}
opts.StaticSummary = "Nightly sync"
```

This differs from the nil-collapse behavior shown in
[Flattened Records](#flattened-records): because the Go options struct contains
an embedded zero-value record, generated Go cannot distinguish "omitted" from
"present with zero-value fields".

---

### @nexus.flattened-type

**Placement:** Field within a `@nexus.flatten-in-api` record
**Syntax:** `@nexus.flattened-type python="<type>" typescript="<type>"`

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

**Placement:** Type alias or record field
**Syntax:**

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

**Placement:** Function used as a `@nexus.function signature`
**Syntax:**

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

**Placement:** Type alias (in a linked/dependency WIT package)
**Syntax:** `@nexus.add-rpc-compatible-with "<type-name>"`

Declares that a type is assignment-compatible with another type for the
`add-rpc` and `add-message` scaffolding commands. This allows the scaffolder to substitute
compatible types when generating WIT from proto RPCs.

```wit
/// @nexus.add-rpc-compatible-with "workflow-type"
type workflow-function = placeholder;
```

This directive has no effect on code generation -- it only affects the scaffold
commands.
