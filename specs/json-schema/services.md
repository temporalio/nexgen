# `services` / `operations`

Source: **not** a JSON Schema 2020-12 keyword. A Nexus extension to the
input document — the cross-cutting design note for declaring **Nexus
services and their operations** (alongside the type-producing `$defs`),
and emitting idiomatic service bindings in all four targets. Like
[[nullability]] and [[generated-file-layout]], this is a design note, not
a keyword spec, so it adapts the `features/<keyword>.md` skeleton
rather than following it verbatim.

## Spec summary

- `services` is a top-level document keyword: a map of **service name →
  service definition**, a sibling of `$defs`. It is recognized **only**
  in a *Nexus document* — one whose root opts in with `nexusrpc:
  "1.0.0"`; without that marker the file is plain JSON Schema. The
  document-mode, envelope, and dialect rules live in [[input-files]].
- A service definition carries an optional wire name (`fqn`), an optional
  `description`, and a **non-empty** map of `operations` — a service with
  no operations is a **load reject** (P7.1).
- An operation definition carries an optional wire name (`fqn`), an
  optional `description`, and optional `input` / `output` subschemas.
- A service becomes one idiomatic *service binding* per language (a typed
  holder of operation references); an operation becomes one typed entry
  on it. Operations carry **no runtime validator of their own** — their
  `input`/`output` *types* validate through the existing type machinery
  (P12, [[ref]] delegation).
- This is the JSON-Schema-input analog of the WIT generator's `@nexus.*`
  directives; it is designed to emit the **same per-language shapes** so
  the emitters can be shared (see [Reuse](#reuse-existing-wit-emitters)).

## Support decision

**Support: yes** — services and operations are first-class.

Rationale (citing [[PRINCIPLES.md]]):
- **P1 (polyglot wire compatibility)**: an operation's wire name is the
  single contract every language must agree on. The four SDKs default an
  omitted name *differently* (Go requires it; Java defaults to the
  unqualified method name; Python to the class/attr name; TS requires
  it), so the generator **always emits the resolved wire name
  explicitly** rather than relying on any SDK default — identical bytes
  on the wire in every target.
- **P2 (idiomatic output)**: each target uses its native service idiom —
  Go a typed `struct` of `OperationReference`s, TS a `nexus.service(...)`
  object, Python an `@nexusrpc.service` class, Java a `@Service`
  interface.
- **P3 / P4 (minimal SDK-only deps)**: emitted bindings depend only on
  the per-language nexus-rpc SDK (`github.com/nexus-rpc/sdk-go/nexus`,
  `nexus-rpc`, `nexusrpc`, `io.nexusrpc`) — verified signatures below.
- **P5 (2020-12 base) + P13 (forward compat)**: `services` is recognized
  only in a Nexus document (root `nexusrpc: "1.0.0"`); the
  document-gating, version-marker, and dialect rules are owned by
  [[input-files]]. The version property is what lets the format evolve
  without breaking older consumers.
- **P7 / P7.1 (strict schema, reject loudly)**: service and operation
  names are constrained by regex and rejected at load, a service's
  `operations` map must be **non-empty**, and `input`/`output` must be an
  **object** type (inline or `$ref`). (The document-level rejects —
  missing/unknown `nexusrpc`, wrong `$schema`, a schema-shaped root in a
  Nexus document — live in [[input-files]].)
- **P14 / P15 ([[generated-file-layout]])**: bindings emit into the
  declaring file's module (Java the one-class-per-file exception); every
  synthesized identifier and I/O type name enters the collision pass for
  its scope — run-wide for Go, TypeScript and Python, the declaring package
  for Java.

## Document gating

`services` is recognized **only** in a *Nexus document* — a file whose
root opts in with `nexusrpc: "1.0.0"`. That marker, the document-envelope
rule (a Nexus document's root is an envelope, not a type — a schema-shaped
root keyword rejects), the `$schema` dialect rule, and the stray-`services`
guard all live in **[[input-files]]**. Without the marker the file is
pure JSON Schema and `services` is not a recognized keyword (a stray
`services` rejects with a fix-it). This spec covers only what a service
and its operations look like *inside* such a document.

## Input grammar

```yaml
nexusrpc: "1.0.0"               # REQUIRED — Nexus-document opt-in; defined by [[input-files]]
services:                       # map<service-name, service-def>
  ChatService:                  # service name — identifier key
    fqn: "example.v1.ChatService"   # optional wire name (arbitrary chars)
    description: A service for sending chat messages.
    deprecated: true             # optional annotation; defaults to false
    x-go-name: ChatApi          # optional per-language code identifier; one of
                                # x-{go,ts,py,java}-name; verbatim, never fqn
    operations:                 # map<operation-name, operation-def> — REQUIRED, non-empty
      pollMessages:             # operation name — identifier key
        fqn: "poll-messages"        # optional wire name (arbitrary chars)
        x-go-name: PollNewMessages  # optional per-language code identifier
        description: Poll for new messages.
        deprecated: true
        input:  { $ref: '#/$defs/PollMessagesInput' }
        output: { $ref: '#/$defs/PollMessagesOutput' }
      sendMessage:
        description: Send a message.
        input:                  # inline object → synthesized SendMessageInput
          type: object
          properties:
            message: { type: string }
        # output omitted → void / empty
```

### Names and the two-name model

| | Key (identifier) | `fqn` (wire name) |
|---|---|---|
| **Service** | `^[A-Z][a-zA-Z\d]+$` | optional; arbitrary chars; **defaults to the service name** |
| **Operation** | `^[a-z][a-zA-Z\d]+$` | optional; arbitrary chars; **defaults to the PascalCase canonical of the operation name** |

- The **key** drives the per-language code identifier (case-mapped — see
  [Identifiers](#identifiers--naming)). The **`fqn`** is the wire name,
  pinned verbatim on the wire and never altered (P1/P3) — exactly the
  `properties` JSON-name-pinning relationship, one level up.
- **An empty `fqn` (`fqn: ""`) is a load reject** on a service or an
  operation (P7.1). "Arbitrary characters" does not extend to *no*
  characters: an empty wire name is not addressable, and it is
  indistinguishable from the author meaning "use the default" — which is
  spelled by omitting the key. The diagnostic names the service or
  operation and says to remove the `fqn` to take the default, or give it a
  non-empty value.
- **`x-<lang>-name` (the code-identifier override).** A service or an
  operation may carry an optional per-language override — one of
  `x-go-name` / `x-ts-name` / `x-py-name` / `x-java-name` — that replaces
  the *emitted code identifier* for **that one target**, **verbatim** (no
  recasing, it is the Stage 4 escape hatch of the [[properties]]
  algorithm). It is the third naming axis and is **orthogonal to both key
  and `fqn`**: every other target still derives its identifier from the
  key, and the wire name (`fqn` / `@Operation(name=…)` / the emitted
  `name=`) is never affected. On an **operation** the override renames only
  the emitted method/field identifier — the synthesized
  `<Op>Input`/`<Op>Output` type names still derive from the operation
  key's PascalCase canonical, **not** the override (so a `getShowcase`
  with `x-go-name: FetchShowcase` emits a `FetchShowcase` method still
  typed on `GetShowcaseInput`). An override value that is not a legal,
  non-reserved identifier in its target → **load reject**.
- **Operation default wire name = PascalCase canonical**, not the literal
  key: `sendMessage` → `"SendMessage"`. This matches the existing WIT
  generator's output (`get_user` → `Operation(name="GetUser")`), and
  resolves the looser "defaults to the operation name" wording. (Service
  default is the service name as-is, already PascalCase by its regex.)
- A name violating its regex → **load reject** with a fix-it naming the
  service/operation and the required shape.
- Service and operation objects use exact allowlists. Unknown members reject,
  including the undocumented `endpoint` field and OpenAPI's `discriminator`;
  neither has Nexus envelope semantics in this format.
- The resolved wire name is **always emitted explicitly** (`name=…` /
  `@Operation(name=…)` / the Go struct field / the TS `name` key), even
  when it equals what the SDK would default to. This is P1 itself: the
  four SDKs default an omitted name *differently* (Go/TS require one;
  Java/Python derive it from the identifier), so leaning on any default
  would make the wire contract vary per target and shift silently if the
  identifier is later recased. The redundancy is cosmetic; the
  explicitness is the cross-language wire guarantee, so it is kept
  unconditionally.

### `input` / `output`

Each is optional, and when present **must be an object type** — a Nexus
payload is always a structured message:

- **`$ref` to `$defs`** → reuse that named type ([[ref]] type-name
  rules). The target **must resolve to an object type** — a `type: object`
  schema, or an [[allOf]] that merges to one (the merge runs at load). A
  `$ref` to a scalar/array/enum/const type, or to a [[oneOf]] **union**,
  → **load reject**: a union has no single extensible shape.
- **Inline schema** → must be an **object**: either `type: object` (with
  `properties` and/or `additionalProperties`) or an [[allOf]] that
  resolves to one (merged/flattened at load). It is **promoted to a
  synthesized named type** `<OperationPascal>Input` /
  `<OperationPascal>Output` (e.g. `SendMessageInput`), a normal top-level
  type in the declaring module that enters the package-wide namespace and
  the P15 collision pass. A non-object inline form (`type: string`,
  `type: array`, a `oneOf` union, …) or a shapeless `type: object` →
  **load reject** (P7.1).
- **Omitted** → void / empty (this is *no* I/O, distinct from a non-object
  I/O), encoded per target: Go `nexus.NoValue`, TS `void`, Python `None`.
  **Java distinguishes the two sides** (verified below): an absent
  *output* is a `void` return, but an absent *input* is a **method that
  takes no arguments** (`Out m()`) — Java has no payload-position void, so
  the empty side is expressed by the method's shape, not a `void`
  argument. Go/TS/Python carry a void marker symmetrically on both sides.

Object-only is a deliberate restriction for **forward compatibility
(P13)**, not a deferral. Only an object can be **extended** — a new
optional field can be added to a request or response later without
breaking existing peers (the additive-change guarantee P13/P13.2 rests
on). A primitive (`string`, `integer`, …) or an `array` payload has **no
place to add a field**: an operation that ships a bare `string` output
can never grow structure without a breaking wire/code change. Forcing
every I/O through an object means every operation is born evolvable. Wrap
a scalar in a single-field object (`{ value: <scalar> }`) — that wrapper
*can* gain fields; the bare scalar cannot.

### Scope: a typed contract, not a handler

The generated binding is deliberately a **typed contract** — operation
references (Go/TS/Python) or a service interface (Java) — and nothing
more. Handler-shape and runtime concerns the SDKs carry — **sync vs async
handlers, headers, links** — live at *implementation* time, are expressed
by the SDKs there, and are **intentionally out of scope** here: a contract
that named them would constrain the handler the generator does not emit.
This is a settled boundary, not a deferral.

A future **I/O cardinality or streaming** concept, if one is ever added,
would arrive in a later `nexusrpc` version. Since the version marker is an
exact-match pin with **no cross-version compatibility promise**
([[input-files]] version policy), such an addition is free to change
shape — a `"1.0.0"` generator simply rejects the newer document rather
than guessing — so this spec need not anticipate it now.

## Type mapping

Service key → per-language identifier; operation key → Go `PascalCase`
field, TS/Java `camelCase`, Python `snake_case` (the shared
[[properties]] 4-stage algorithm). `description` → the language's native
doc comment. `description` is optional at both levels ([[input-files]]);
when absent, Go still emits a name-led fallback comment on the service
binding, client, constructor, and each operation entry/method (every
exported Go declaration must carry one — PRINCIPLES.md, Go §1). The other
three languages simply emit no comment, as elsewhere ([[description]]).
`deprecated: true` is also annotation-only and lowers to each target's native
service/operation marker and documentation tag; it never changes the wire name,
signature, validation, or dispatch behavior. A non-boolean value rejects.

| Aspect | Go | TypeScript | Python | Java |
|---|---|---|---|---|
| Service binding | pkg-level `var <Name> = struct{…}{…}` | `export const <name> = nexus.service(fqn, {…})` | `@service(name=fqn)` class | `@Service(name=fqn)` interface |
| Operation entry | field `nexus.OperationReference[In, Out]` set via `nexus.NewOperationReference[In,Out](wire)` | `nexus.operation<In, Out>({ name: wire, inputType, outputType })` | attr `Operation[In, Out] = Operation(name=wire)` | method `Out m(In input)` + `@Operation(name=wire)` |
| Operation type info | — | `inputType`/`outputType` carry the I/O type's transfer type converter (below) | — | — |
| Service wire name | `ServiceName` struct field | first arg to `nexus.service` | `service(name=…)` | `@Service(name=…)` |
| Void output | `nexus.NoValue` | `void` | `None` | `void` return |
| Void input | `nexus.NoValue` | `void` | `None` | **no-arg method** `Out m()` |
| Deprecated | godoc `Deprecated:` paragraph | JSDoc `@deprecated` | PEP 702 `typing_extensions.deprecated(..., category=None)` | `@Deprecated` + Javadoc `@deprecated` |
| Import | `github.com/nexus-rpc/sdk-go/nexus` | `nexus-rpc` | `from nexusrpc import Operation, service` | `io.nexusrpc.{Service,Operation}` |
| File | declaring module's `<module>.go` | `services.ts` | `services.py` | **own `<Service>.java`** |

### Verified SDK signatures

- **Go** (`github.com/nexus-rpc/sdk-go/nexus`):
  `type OperationReference[I, O any] interface { Name() string; … }`,
  `func NewOperationReference[I, O any](name string) OperationReference[I, O]`,
  `type NoValue *struct{}`.
- **TypeScript** (`nexus-rpc` 0.0.3 or newer): `nexus.service(name, operations)`,
  `nexus.operation<In, Out>({ name, inputType?, outputType? })` where both
  type-info fields are `TypeInfo<T, unknown>` and
  `interface TypeInfo<T = unknown, D = T> { transferTypeConverter?:
  TransferTypeConverter<T, D> }`,
  `interface TransferTypeConverter<T, D = unknown> { fromTransferType(value:
  D): T; toTransferType(value: T): D }`. The converter contract is exported
  directly by `nexus-rpc` 0.0.3 or newer.
- **Python** (`nexusrpc`): `@service` /
  `@service(name=…)` (name defaults to the class name);
  `Operation[InputT, OutputT]` dataclass, `Operation(name=…)`. **No
  `NoValue`** — void is `None` / `type(None)`.
- **Java** (`io.nexusrpc`): `@Service` on an **interface**, `String
  name() default ""` (defaults to the unqualified interface name);
  `@Operation` on a **method**, `String name() default ""` (defaults to
  the unqualified method name); a method takes **zero or one** convertible
  param (zero ⇒ absent input) and returns `void` (absent output) or a
  convertible type, no `throws`.

### The Go `--native-api` client

In `--native-api` mode Go emits, beside the `var <Service>` binding, a
workflow-side client per service:

```go
type <Service>Client struct{ client workflow.NexusClient }

func New<Service>Client(endpoint string) *<Service>Client
func (c *<Service>Client) <Op>(ctx workflow.Context, request In) workflow.Future
```

`<Service>` is the resolved Go service identifier, so an `x-go-name` moves the
client with the binding, and `<Op>` is the operation's Go field identifier; a
void-input operation takes no `request` parameter. This is the only place
generated Go depends on `go.temporal.io/sdk/workflow`; definitions-only mode
emits none of it.

`<Service>Client` and `New<Service>Client` are synthesized from the service key,
so per **P15** they are in the collision pass like any other name: a
`$defs/AlphaClient` beside a service `Alpha` is a load reject, not a Go
redeclaration. The pass runs over the same namespace in both modes, so a
document does not load in one mode and emit uncompilable output in the other.

## Worked example

Input is the `ChatService` from the [Input grammar](#input-grammar)
above (`pollMessages` with `$ref`'d I/O; `sendMessage` with an
inline-object input and omitted output). The per-language output: fqn is
used verbatim on the wire, Python void is `None`, the Go literal uses
`key: value` form, and Java carries the return type and param name.

### Go (in the declaring module's file)

```go
import "github.com/nexus-rpc/sdk-go/nexus"

// ChatService - A service for sending chat messages.
var ChatService = struct {
	ServiceName string
	// PollMessages - Poll for new messages.
	PollMessages nexus.OperationReference[PollMessagesInput, PollMessagesOutput]
	// SendMessage - Send a message.
	SendMessage nexus.OperationReference[SendMessageInput, nexus.NoValue]
}{
	ServiceName:  "example.v1.ChatService",
	PollMessages: nexus.NewOperationReference[PollMessagesInput, PollMessagesOutput]("poll-messages"),
	SendMessage:  nexus.NewOperationReference[SendMessageInput, nexus.NoValue]("SendMessage"),
}
```

### TypeScript

```ts
import * as nexus from "nexus-rpc";

/**
 * A service for sending chat messages.
 */
export const chatService = nexus.service("example.v1.ChatService", {
  /**
   * Poll for new messages.
   */
  pollMessages: nexus.operation<PollMessagesInput, PollMessagesOutput>({
    name: "poll-messages",
    inputType: { transferTypeConverter: pollMessagesInputTransferTypeConverter },
    outputType: { transferTypeConverter: pollMessagesOutputTransferTypeConverter },
  }),
  /**
   * Send a message.
   */
  sendMessage: nexus.operation<SendMessageInput, void>({
    name: "SendMessage",
    inputType: { transferTypeConverter: sendMessageInputTransferTypeConverter },
  }),
});
```

### Python

```python
from nexusrpc import Operation, service


@service(name="example.v1.ChatService")
class ChatService:
    """
    A service for sending chat messages.
    """

    poll_messages: Operation[PollMessagesInput, PollMessagesOutput] = Operation(name="poll-messages")
    """
    Poll for new messages.
    """

    send_message: Operation[SendMessageInput, None] = Operation(name="SendMessage")
    """
    Send a message.
    """
```

### Java (`ChatService.java`)

```java
import io.nexusrpc.Operation;
import io.nexusrpc.Service;

/**
 * A service for sending chat messages.
 */
@Service(name = "example.v1.ChatService")
public interface ChatService {
    /**
     * Poll for new messages.
     */
    @Operation(name = "poll-messages")
    PollMessagesOutput pollMessages(PollMessagesInput input);

    /**
     * Send a message.
     */
    @Operation(name = "SendMessage")
    void sendMessage(SendMessageInput input);
}
```

All four agree byte-for-byte on the wire: service `"example.v1.ChatService"`,
operations `"poll-messages"` and `"SendMessage"` (P1).

`sendMessage` here has a present input and an absent output, so Java emits
a `void` *return*. The mirror case — an operation with an absent *input*
(e.g. a `ping` with output but no `input:`) — is the one place Java
diverges from the other targets: it emits a **no-arg method**, `PingOutput
ping()`, rather than a void argument. Go/TS/Python instead carry their void
marker (`nexus.NoValue` / `void` / `None`) symmetrically in the input
type position.

## Identifiers & naming

- Service and operation **keys** run through the shared [[properties]]
  4-stage algorithm — Stage 1 segmentation (`pollMessages` → `[poll,
  messages]`; `sendHTTPRequest` → `[send, http, request]`, acronyms
  folded), Stage 2 per-language recasing, Stage 3 per-target validity,
  Stage 4 `x-<lang>-name` override. The documented acronym-folding
  limitation and the `x-<lang>-name` escape hatch apply unchanged. Note
  the override axes are orthogonal: **`fqn`** overrides the *wire* name;
  **`x-<lang>-name`** overrides the *code identifier*.
- **Synthesized I/O type names** (`<OperationPascal>Input`/`Output`) are
  derived from the operation's PascalCase canonical and are ordinary
  named types — they share the package namespace and the single P15
  collision pass with declared `$defs` types, service identifiers, and
  each other; a coincidence (e.g. a `$defs/SendMessageInput` plus an
  inline `sendMessage.input`) → **load reject**, no mangling. The escape
  hatch is renaming the `$defs` type or switching the operation to a
  `$ref`.
- The **TS service const is `camelCase`** (`chatService`) — a value
  binding, recased like an operation/member. The shared WIT and JSON Schema
  emitter uses this form. Go `var`, Python `class`, and Java `interface` keep
  the PascalCase service identifier (the key), which their regex already
  guarantees.
- `description` → doc comment per target (Go `//`, TS/Java JSDoc/JavaDoc
  `/** */`, Python docstring), identical to type/member handling.

## File layout & namespacing

Per [[generated-file-layout]]:

- A service binding emits into the **same per-input module directory** as the
  models declared in that file: Go places both in `<module>.go`, while Python
  and TypeScript use sibling `services.py` / `services.ts` files beside
  `models.py` / `models.ts`. Synthesized `<Op>Input`/`<Op>Output` types are
  ordinary model types in that module.
- **Java** is the exception: each service is its **own `<Service>.java`**
  (one-public-class-per-file), exactly as each model is its own file.
- Aggregators re-export services: `index.ts` re-exports `./services` and
  `__init__.py` includes them in `__all__`. Go/Java rely on exported
  visibility (capitalized / `public`).
- Service identifiers and synthesized I/O type names live in the declaring
  module's identifier namespace and are checked **per emitted target**: the
  scope is run-wide for Go, TypeScript and Python (flat package or root barrel)
  and the declaring package for Java; see [[generated-file-layout]] and P15.
- **Operation identifiers are members, not module-level bindings** — a Go struct
  field, a TypeScript object key, a Python class attribute, a Java interface
  method — so they share one scope **per service** and are checked there. An
  operation `page` beside a `$defs/Page` in the same module does not collide.

> **A generated service and a generated model that resolve to the same
> identifier collide, and the generator fails the build (P7.1/P15).** A
> service binding occupies the *same* namespace as the `$defs` model types
> of its module — it is not a separate namespace. So a service
> `ChatService` and a `$defs/ChatService` model both claim the identifier
> `ChatService` in Go (a `var ChatService` against a `type ChatService`),
> Python (two `class ChatService`) and Java (two top-level `ChatService`
> types). **TypeScript is the exception**: it binds a service to a
> lower-camel `const`, so `chatService` and the model's `ChatService` are
> distinct identifiers and the pair generates cleanly. What a TypeScript
> service *can* collide with is another lower-camel module binding — most
> readily a model's `<model>TransferTypeConverter`, which a service named
> `ChatServiceTransferTypeConverter` would claim. This is a **load reject**
> with a fix-it, **never silently mangled** — exactly the
> synthesized-I/O-vs-`$defs` rule above, one level up. Resolve it by
> renaming the `$defs` model or applying `x-<lang>-name` to the service
> (the `fqn` wire name is unaffected — only the *code identifier*
> collides). The check runs per emitted target, so a pair that collides
> in one language may be fine in another and still rejects.

**Two services collide too.** The key grammar admits both `HTTPService` and
`HttpService`, which resolve to one identifier, so the check compares the
**authored keys** and not the identifiers they fold to: a second service landing
on an identifier another service in the same module already claims is a reject
naming both keys, never a re-insertion of "the same" service. Python is why this
cannot be left to the target — one module gets two `class HttpService:`
statements, the later definition silently replaces the earlier, and one whole
service binding is gone from the package with no diagnostic. This is the P15
case-mapping rule applied to service keys.

**A service file's imports occupy its namespace.** P15 makes an imported bare
name reserved against everything else in the scope; what this spec supplies is
*which* names each target's service file imports:

| Target | Imported bare names |
|---|---|
| Go | `nexus` |
| TypeScript | `nexus` |
| Python | `Operation`, `service` |
| Java | `Operation`, `Service` |

Two routes reach them, and both are load rejects. An `x-<lang>-name` on a
service or operation may name one directly. And a `$defs` model may be named
`Operation` or `Service` with no override anywhere, entering the service file
because an operation's `input`/`output` points at it — so the reservation covers
model and synthesized I/O names reached through operation I/O, not only service
identifiers.

## Validator / serializer (P12)

Services and operations emit **no runtime validator**. An operation is a
typed *reference*; validation happens on its `input`/`output` **types**,
which carry their own shared `Validate` (de)serializers ([[type]],
[[properties]], [[ref]] delegation). A bad payload surfaces as that type's
non-retryable Temporal application failure with type `PayloadValidationError`
and the aggregated violations as its first detail (P11). The SDK's Nexus
integration recognizes that reserved type; the generated service binding adds
no wrapper or behavior to the path. Void I/O (`nexus.NoValue` / TS
`void` / Python `None` / Java `void` return or no-arg method) has no value
to validate.

### TypeScript operation type info

TypeScript is the one target where the operation entry **names** its I/O
types' converters: each non-void side carries
`inputType`/`outputType` = `{ transferTypeConverter: <side>TransferTypeConverter }`,
the model's exported converter instance (PRINCIPLES TS §4). This is
metadata, not behavior — nexus-rpc carries it verbatim and interprets
nothing; Temporal SDK 1.23.0 or newer applies the conversion when transferring
the value. It exists because TS is the only target whose conversion is
*not* discoverable from the type: Go reaches it through
`MarshalJSON`/`UnmarshalJSON` on the model, Python through the model's
Pydantic hooks, Java through the POJO's class-level Jackson
(de)serializer — all attached to the type itself, so the SDK finds them
with nothing named at the operation. A TS model is a bare `interface` with
no runtime footprint (PRINCIPLES TS §2), so its converter is a separate
value and the operation is the only place that can point at it.

A **void** side carries neither field. There is no value to convert, so an
empty `TypeInfo` would assert a conversion that does not exist; absence is
the accurate encoding and matches the SDK's optional fields. Since a
declared `input`/`output` is always an object type (above), a non-void side
always has exactly one converter to name — there is no case where the
field would be present but empty.

The converter identifier is derived, not declared: it is the model's
resolved type identifier lower-camel-cased plus `TransferTypeConverter` —
the same identifier the type declaration uses, so an `x-ts-name` override
moves the type and its converter together. Because it is derived and
lower-camel-casing folds names the type namespace keeps apart (`HTTPError`
and `HttpError` both yield `httpErrorTransferTypeConverter`), the converter
identifier also enters TypeScript's identifier namespace for the
PRINCIPLES §15 collision pass — which the package barrel makes run-wide, so
the converter is checked against every module's, not just its own: a fold
rejects at load with a fix-it rather than emitting one `export const` twice. Converters declared in another
input file's module import as **values** from that module (beside the
type-only model import), following the same module resolution as any
cross-module reference ([[ref]], [[generated-file-layout]]).

## Reuse (existing WIT emitters)

This spec is intentionally shape-compatible with the WIT generator's
existing, tested output so the **future separate crate** can share the
emission code rather than duplicate it:

- Python `@service` + `Operation[...]` class attrs
  (`advanced/samples/python/wit/user_service/services.py`) and TS `nexus.service(...,
  { op: nexus.operation<…>({ name }) })`
  (`advanced/samples/typescript/wit/user-service/services.ts`) already match
  this spec's shapes through the shared emitters, including the camelCase TS
  service const.
- Only the **input model** differs (JSON Schema here vs WIT there); none
  of WIT's input-side concepts (directives, proto backing, resources)
  cross into this spec. The TS transfer type converter wiring (below) is
  JSON-Schema-only: a WIT-input operation carries the TS generics but no
  operation type info, since WIT models convert through proto helpers
  rather than a `TransferTypeConverter`.

## Property-testing matrix

### Accepted (positive)

| Case | Shape |
|---|---|
| Service + ops, mixed I/O | the `ChatService` worked example (in a Nexus document — gating per [[input-files]]) |
| Operation `$ref` I/O (object) | `input: {$ref: '#/$defs/X'}`, `X` is `type: object` → field typed `X` |
| Inline-object I/O promoted | `input: {type: object, properties: {…}}` → `<Op>Input` |
| Omitted output | → `NoValue` / `void` / `None` / Java `void` return |
| Omitted input | → `NoValue` / `void` / `None` / Java **no-arg method** |
| TS type info on a non-void side | `inputType`/`outputType` = `{ transferTypeConverter: … }` naming the I/O type's converter |
| TS type info on a void side | neither field emitted |
| TS type info across modules | I/O `$ref` into another module → converter imported as a value from that module |
| `fqn` overrides on service and op | wire name = `fqn` verbatim |
| Defaults applied | op without `fqn` → PascalCase wire; service without `fqn` → service name |
| Acronym op name | `sendHTTPRequest` → field `SendHttpRequest`, type `SendHttpRequestInput` (folded; `x-*-name` to refine) |
| `x-<lang>-name` on a service/op | code identifier overridden verbatim for that target; wire `fqn` independent |
| `x-<lang>-name` on an operation | method/field renamed; synthesized `<Op>Input`/`Output` names still from the op key |
| Operation identifier equal to a module type name | operation `page` + a `$defs/Page` in the same module — a member and a type, different scopes |

### Rejected at load time (negative)

| Reason | Example |
|---|---|
| Service name regex | `chatService` / `Chat_Service` / `9Service` |
| Service with no operations (P7.1) | `operations:` omitted, or an empty `operations: {}` |
| Operation name regex | `PollMessages` / `poll_messages` / `2fa` |
| Operation identifiers collide after target recasing | two operation keys that map to the same emitted method/field identifier |
| Duplicate operation wire name | two operations in one service whose `fqn` values are equal |
| Inline I/O without shape (P7.1) | `input: {}` / `input: {type: object}` (no `properties`/`additionalProperties`) |
| Non-object I/O (inline) | `input: {type: string}` / `input: {type: array, …}` |
| Non-object I/O (`$ref`) | `input: {$ref: '#/$defs/Y'}` where `Y` is not `type: object` |
| Synthesized name collides | inline `sendMessage.input` + a `$defs/SendMessageInput` |
| Service collides with a model | service `ChatService` + a `$defs/ChatService` model (same per-package identifier) |
| Two service keys resolve to one identifier | `HTTPService` + `HttpService` in one document (Python would emit one `class HttpService:` twice) |
| Identifier collides with a service file's SDK import | a `$defs/Operation` used as an operation's I/O; `x-py-name: Operation` on a service |
| Go client identifier collides | `$defs/AlphaClient` beside service `Alpha` |
| TS converter identifiers fold together | `$defs/HTTPError` (kept verbatim by `x-ts-name`) + `$defs/HttpError` → one `httpErrorTransferTypeConverter` |
| `$ref` I/O unresolvable / non-`$defs` | `input: {$ref: '#/properties/x'}` — per [[ref]] |
| Identifier invalid/reserved in an emitted lang (no override) | a service/op key mapping to a reserved word |
| `x-<lang>-name` value not a legal identifier | `x-go-name: "2fa"` / a reserved word on a service or op |

### Runtime fixtures

- A valid `input` payload validates and round-trips via the I/O type's
  own (de)serializer; an invalid one aggregates a `Violation` with a path
  into that type (P11) — the operation binding adds nothing.
- Void I/O carries no payload to validate in any language.

## Interactions

- **[[input-files]]** — owns the Nexus-document mode that enables
  `services`, the envelope/dialect/gating rules, and the document-level
  reject cases this spec defers to.
- **[[ref]]** — `input`/`output` `$ref` resolution and type-name
  derivation; inline I/O promotion follows the same anonymous-type
  synthesis. Synthesized I/O types join the [[ref]] reference graph.
- **[[properties]]** — the 4-stage identifier algorithm, `x-<lang>-name`
  override, and P15 collision policy that service/operation/I/O names all
  reuse; inline-object I/O is a `properties` schema.
- **[[generated-file-layout]]** — owns the module placement, the Java
  separate-file rule, and the aggregator re-exports referenced here.
- **[[type]]** / **[[nullability]]** — the I/O types' base-type and
  optional/nullable handling; nothing service-specific.

## Ecosystem variance

| Source dialect | Handling |
|---|---|
| JSON Schema 2020-12 (+ `services` extension) | native (this spec) |
| WIT (`@nexus.endpoint` / `@nexus.operation`) | the existing generator's input; produces the same emitter shapes — shared output, different front end |
| OpenAPI / proto service definitions | out of scope; not an input to this generator |

## See also

- [[input-files]] — document modes, the `nexusrpc` opt-in that enables
  `services`, the envelope rule, the `$schema` dialect rule, the
  stray-`services` guard.
- [[ref]] — `$ref`/inline I/O resolution, type-name derivation, the
  reference graph the synthesized I/O types join.
- [[properties]] — shared 4-stage identifier algorithm, `x-<lang>-name`,
  P15 collision policy.
- [[generated-file-layout]] — module placement, Java separate-file rule,
  aggregator re-exports.
- [[type]], [[nullability]] — I/O base-type and optional/nullable
  handling.
- [[PRINCIPLES.md]] — **P1** (polyglot wire), **P2** (idiomatic),
  **P3/P4** (SDK-only deps), **P7/P7.1** (strict/reject), **P14/P15**
  (layout + namespace).
