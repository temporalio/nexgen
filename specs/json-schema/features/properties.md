# `properties`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.3.2.1 "Keywords
for Applying Subschemas to Objects → properties".

Maps named object members to subschemas. The primary keyword for producing typed
structs: each entry becomes a field whose type is the mapping of that member's
subschema. `properties` is the structural backbone of every generated model.

## Spec summary

Verbatim (2020-12 core, Applicator):

> The value of "properties" MUST be an object. Each value of this object MUST be
> a valid JSON Schema.

> Validation succeeds if, for each name that appears in both the instance and as
> a name within this keyword's value, the child instance for that name
> successfully validates against the corresponding schema.

> The annotation result of this keyword is the set of instance property names
> matched by this keyword. This annotation affects the behavior of
> "additionalProperties" (in this vocabulary) and "unevaluatedProperties" in the
> Unevaluated vocabulary.

> Omitting this keyword has the same assertion behavior as an empty object.

Distilled:
- `properties` itself only asserts on members **that are present**; it says
  nothing about whether a member must be present (that is [[required]]) nor
  about unmatched members (that is [[additionalProperties]]).
- Its matched-name set is an *annotation* consumed by [[additionalProperties]]
  to decide which members count as "additional." ([[unevaluatedProperties]], the
  standard's other consumer, is rejected at load per **P6**, so it never reads
  the annotation here.)
- An explicit empty `properties: {}` declares a zero-member struct; omitting
  `properties` declares a free-form object owned by [[additionalProperties]].
  Their *assertion* behavior coincides, as the quoted rule says, and both emit
  the same catch-all-only aggregate. What differs is that `properties` is
  **present**: presence is what selects the [[additionalProperties]] column and
  what [[propertyNames]] rejects beside, and an empty map is present.

## Support decision

**Support:** yes — core keyword.

`properties` is the canonical way to declare a typed struct and is fully
supported. Each member subschema is recursively a supported subschema (rejected
at load if it isn't, per [[type]] and **P7.1**).

Rationale (citing [[PRINCIPLES.md]]):
- **P2 (idiomatic output)**: members lower to idiomatic fields (struct fields /
  interface members / model attrs / POJO fields + accessors).
- **P7 / P7.1 (strict schema)**: each member must carry an explicit, supported
  `type` (or the [[nullability]] `oneOf` pattern); a member schema that is bare
  `{}` or otherwise out-of-subset is rejected with a located diagnostic.

Loader behavior:
- `properties` value not an object → reject.
- Any member value not a valid subschema → reject (recurse).
- A member schema that is empty `{}` / `true` / `false` → reject per **P7.1**
  (no shape). Diagnostic names the member and asks for an explicit `type`.
- Member name collisions after idiomatic case-mapping (see below) → reject (two
  JSON names would map to one field). Diagnostic names both.

`properties` may appear:
- with [[type]] `"object"` — the typed-struct case (this spec's focus);
- without `type` — **rejected** per [[type]] (missing `type`), even though
  presence of `properties` implies object. Require explicit `type: "object"`.

## Type mapping

A `{type:"object", properties:{...}}` schema emits one named aggregate per
language; each member becomes a field. The member's bare type comes from
[[type]]; optional/nullable wrapping from [[required]] + [[nullability]];
open/closed catch-all from [[additionalProperties]].

| Aspect            | Go                  | TypeScript                  | Python                                                                         | Java                                                    | C#                                                    |
| ----------------- | ------------------- | --------------------------- | ------------------------------------------------------------------------------ | ------------------------------------------------------- | ----------------------------------------------------- |
| Aggregate         | `struct`            | `interface` (**not class**) | `@dataclasses.dataclass(slots=True, kw_only=True)` (**not a validating base**) | POJO `class` (Java 8; **not records**)                  | `sealed record` (**not** attribute-bound)             |
| Member            | struct field        | interface member            | dataclass field; default-bearing property over private storage                 | private field + getter                                  | `required` or optional `{ get; init; }` property      |
| JSON-name binding | `json:"<name>"` tag | exact key (index access)    | the wire key, read and written by the converter                                | exact key in the collecting deserializer and serializer | exact key in the converter's `Parse` and `WriteValue` |

Field naming: JSON member names are mapped to each language's idiomatic
identifier and the **original JSON name is always pinned** — by a Go struct tag
and by the exact wire key the TypeScript, Python, and Java converters read and
write — so the wire contract is stable regardless of the emitted identifier
(**P2**, **P3**). A wire key is an arbitrary string and need not be a legal
identifier in any target, so the three converter-based targets reach it by
**key lookup** on the wire object — `raw["content-type"]`,
`out["content-type"]`, `node.get("content-type")` — never by member access
(`raw.content-type` is not a property read). Presence is decided over the wire
object's **own** members only: in TypeScript a member named after anything the
host object inherits (`toString`, `valueOf`, `constructor`, `__proto__`) must
not read as present on a payload that never carried it, so "not `undefined`" is
not a sufficient presence test there. The exact transform, collision policy, and
escape hatch are specified in
[Identifier case-mapping](#identifier-case-mapping) below.

## Identifier case-mapping

One shared algorithm drives every emitter so the five languages agree on
segmentation and recasing. A language-agnostic **segmentation** core produces a
canonical word list; thin per-language layers **recase** it; then each emitted
target **validates** the result. The original JSON name is always pinned on the
wire (above), so this transform is purely ergonomic — it never changes the
contract.

### Stage 1 — Segmentation (shared)

A JSON member name is split into a canonical, lowercased word list:

- Runs of explicit separators (`_`, `-`, space) are consumed as boundaries.
- A boundary is inserted at a lower/digit → upper transition (`fooBar` →
  `foo|Bar`).
- A boundary is inserted before the final capital of an uppercase run that
  precedes a lowercase (`HTTPServer` → `HTTP|Server`).
- Digits attach to the adjacent word; they never force a boundary (`oauth2` →
  `[oauth2]`).
- Every word is then lowercase-folded, yielding e.g. `[user, id]`,
  `[http, server]`, `[oauth2]`.

**Acronyms are folded as ordinary words** (no initialism set). This is the
single decision that makes "one algorithm for all five emitters" literally true;
its limitation is documented below.

### Stage 2 — Recasing (per language)

From the canonical word list:

| Language   | Identifier                                        | Example (`[user, id]`) |
| ---------- | ------------------------------------------------- | ---------------------- |
| Go         | exported `PascalCase`                             | `UserId`               |
| TypeScript | `camelCase`                                       | `userId`               |
| Java       | field `camelCase` + `get` + `PascalCase` accessor | `userId` / `getUserId` |
| C#         | property `PascalCase`                             | `UserId`               |
| Python     | `snake_case`                                      | `user_id`              |

### Stage 3 — Validity (per emitted target)

The recased identifier is **rejected at load** if it is empty, begins with a
digit, contains a character outside the shared ASCII identifier grammar (`A-Z`,
`a-z`, `0-9`, and `_` after an ASCII letter or `_`), or is a word the emitted
target reserves in any position where the identifier is emitted. The
deliberately conservative ASCII intersection avoids unexported Go fields and
normalization collisions in Python. Rejection is **per emitted target** only — a
name invalid in a language not being generated is not a concern and produces no
diagnostic.

The emitted-position qualifier is load-bearing. A TypeScript interface member
and property access may use a keyword, but the converter also stages each member
in a `let` binding, where a keyword is illegal. Java likewise uses method-local
bindings, and Python emits attribute/binding identifiers. Go's exported
`PascalCase` identifiers do not collide with its lowercase keywords.
Consequently Go is the only target where a keyword-named property needs no
override under the current emission strategy.

What a target "reserves" in an emitted position is **not** its keyword list. It
is every name already bound where the identifier lands, and for a
converter-based target that includes the converter's own parameter and locals,
and the bindings the language restricts without reserving (in TypeScript
`undefined`, `eval` and `arguments`; the members an emitted `interface` inherits
from `Object`). A member name that collides with one of those is a Stage-3
rejection with the `x-<lang>-name` remedy, exactly as a keyword is — never
emitted and left to fail downstream. Where a target's collision surface is an
artifact of *how* it stages members rather than of the language, prefixing the
emitter's own bindings out of the member namespace is the alternative to
rejecting; either is conformant, silently emitting a colliding binding is not.

### Stage 4 — Escape hatch (per-language override)

A property may carry a per-language override — `x-go-name` / `x-ts-name` /
`x-py-name` / `x-java-name`. The override is used **verbatim** (it skips Stages
1–2), must itself be a legal, non-reserved identifier in that language, and
participates in collision detection. It is the only way to admit a name Stage 3
rejects (e.g. a `class` member needs `x-ts-name` + `x-py-name` + `x-java-name`;
Go needs nothing).

### Collision policy

Within a single object's member set, if two distinct JSON names map to the same
identifier in an **emitted** language (after recasing / override), the schema is
**rejected** with a diagnostic naming both members — e.g. `user_id` + `userId` →
Go `UserId`. Like Stage 3, collisions are evaluated only for languages being
generated.

The check is not limited to declared members. The generator also synthesizes
identifiers from member/type names — [[const]]'s named type (Go defined type /
Java value class), the Go `<Field>OrDefault()` accessor, TS `DEFAULT_<FIELD>`
constant, and Python `_<field>` default backing slot ([[default]]), the [[enum]]
value class — and these enter the **same per-scope namespace** as the declared
names and each other (package/module scope for package-level types/consts; the
struct method-set for the Go accessor; the Python class scope for the backing
slot). The single collision pass runs over that full union and rejects on any
coincidence; the `x-*-name` override (Stage 4) on the declaring member is the
escape hatch for these, and re-mapping the member moves every name synthesized
*from the member* with it — the Go `<Field>OrDefault()` accessor, TS
`DEFAULT_<FIELD>` constant, Python `_<field>` backing slot ([[default]]), the Go
closed-value type and Java value class ([[const]]) are all named off the
**emitted** member identifier, not the JSON key, so the override reaches them. A
name synthesized from a **position** rather than a member does not move: an
inline object hoisted to `<Model><Property>` keeps the position's name (see
[Naming an inline object shape](#naming-an-inline-object-shape) below). A
[[const]]/[[enum]] **value constant** is synthesized from the *value*, not the
member — it shares the same namespace and collision pass but is re-mapped by its
own `x-<lang>-const-name` override, not `x-*-name`. The generator
**never auto-mangles** (P15) — a numeric suffix would be unstable under schema
evolution (P13).

### Synthesized type names

The Stage 1–4 algorithm maps *member* names; a synthesized **named type** (the
[[const]]/[[enum]] value class / defined type) is named separately — but off the
**emitted** member identifier, so a Stage 4 override moves it along with the
member (`kind` + `x-go-name: Category` → `ProbeCategory`, Java
`Probe.Category`). That is what makes the override a working escape hatch for a
collision on the synthesized type, and it keeps the two languages that
synthesize one from disagreeing about its name. A const or an enum synthesizes a
named type where the language lacks literal types (Go defined type, Java value
class), for every scalar kind; TS and Python close the type inline (a literal /
union of literals) and synthesize no named type. When the const/enum is a named
`$defs` definition, the synthesized type reuses the `$defs` name. When it is
**anonymous** (inline on a property), the synthesized type is
**nested inside its enclosing model** where the language supports it, so it
leaves the package/module namespace and cannot collide with a
coincidentally-named top-level type:

- **Java** — `public static final class Kind` nested in `UserEvent`, referenced
  `UserEvent.Kind`. Java is the only target that cannot inline a const/enum, so
  it is where nesting matters most.
- **Python** — a const/enum is an inline `typing.Literal[…]`, so there is no
  named type to nest and nothing synthesized in the module namespace; the fixed
  value is compared inline in the converter.
- **TypeScript** — a const/enum is an inline literal / union of literals. Enums
  synthesize no binding; const validation may synthesize a private module-scope
  binding, which participates in the run-wide namespace check.
- **Go** — has no nested types (a `type` decl inside a struct is a syntax
  error), so it falls back to flat package-level composition
  `<EnclosingType><Property>` (`UserEventKind`) and relies on the P15 collision
  backstop (load reject + `x-go-name`) for any residual coincidence.

This trades uniform cross-language shape for collision-minimization and
per-language idiom — the one place the project accepts shape divergence, because
flat-everywhere would needlessly inflict Go's limitation on Java, the language
that benefits most from nesting. P15 remains the backstop in every language:
nesting reduces, never eliminates, the collision surface.

### Naming an inline object shape

A member's schema may be an object written **inline** rather than `$ref`ed:

```yaml
Order:
  type: object
  properties:
    address:
      type: object
      properties:
        street: { type: string }
```

That object is a type in every target — a Go struct, a TS interface, a Python
dataclass, a Java class — so, exactly like an inline [[oneOf]] object branch,
the constraint on it is not its shape but its **name**. It is resolved the same
way: the shape is **named after the position it was written in, moved into
`$defs`, and the position rewritten to a `$ref`** at it. From there it *is* an
ordinary definition, so one object-model emitter per target covers it, and the
inline form emits **identical code to the `$defs` + `$ref` form** — the choice
between them is only where the shape reads best.

| Position                                              | Synthesized name            |
| ----------------------------------------------------- | --------------------------- |
| the member `address` of `Order`                       | `OrderAddress`              |
| `items` of the member `rows`                          | `OrderRowsItem`             |
| `items` of `items` (nested array)                     | `OrderRowsItemItem`         |
| typed `additionalProperties` of `Order`               | `OrderValue`                |
| a `oneOf` branch (the union owns the position's name) | `<Union>Object` ([[oneOf]]) |

Rules that follow from "the name belongs to the position":

- **The hoist runs to a fixpoint**, so an object nested inside a hoisted one is
  named against *its* name — `OrderAddress` → `OrderAddressGeo`. Nesting
  therefore needs no `$defs` boilerplate from the author at any depth.
- **Nullability does not rename anything.** A nullability `oneOf`
  (`oneOf: [{object}, {"type":"null"}]`, see [[nullability]]) emits no type of
  its own — every target expresses it on the value — so the object inside it
  takes the position's name, the same name it would take written plainly. A
  *sum type*, by contrast, occupies the position (it is the emitted union type),
  so its inline object branch derives `<Union>Object`.
- **Every object is named, including the free-form one**
  (`additionalProperties: true`): [[additionalProperties]] emits every object as
  a named aggregate holding its members in a catch-all field, so that adding
  `properties` to it later only *adds fields* rather than changing the emitted
  type's kind (**P13**). Its member-count and key-shape constraints ride along
  with it. The one place a free-form object stays inline is as a `oneOf` branch,
  where it is the union's object *kind* rather than a position of its own
  ([[oneOf]]).
- **`x-<lang>-name` on the member still names the member.** It is the Stage 4
  escape hatch for the *member* identifier, and the same keyword names a *type*
  in `$defs`, so it stays on the property and the hoisted type keeps its
  position-derived name. (For the same reason it is the one keyword legal beside
  a `$ref` and is not an implicit-`allOf` conjunct — see [[ref]].) To choose the
  type's name, author the shape in `$defs` and `$ref` it.
- **The shape's own `title`/`description` travel with it**, since they describe
  the object that is now a type; the member falls back to its synthesized doc
  line — again identical to the `$defs` + `$ref` form.
- **P15 is the backstop.** A synthesized name that collides with a declared
  `$defs` entry, with another synthesized name, or with the **file-root type**'s
  name ([[ref]] type-name derivation) is a load reject with a fix-it diagnostic
  naming the position the shape was written in, never auto-mangled.

### Documented limitation

Folding acronyms as ordinary words means Go yields `UserId` / `HttpServer`
rather than the `golint`-preferred `UserID` / `HTTPServer` (likewise Java
accessors `getUserId`). This is accepted for v1 to keep one shared algorithm;
the per-language override (Stage 4) is the escape hatch for any name where the
folded casing is unacceptable — e.g. `x-go-name: "UserID"` restores the
idiomatic initialism. A later known-initialisms pass could refine Go/Java casing
without touching the wire contract or this spec's other guarantees.

## Validator mapping

Per **P10** each present member is validated at the (de)serializer boundary; per
**P11** failures aggregate. `properties` contributes the per-member dispatch;
presence/absence is [[required]], extras are
[[additionalProperties]].

| Language   | Strategy                                                                                                                                                                                                                                                                                                                                                   |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | Custom `UnmarshalJSON` decodes into a shadow of `*json.RawMessage` per member, dispatches each present member through its type helper, collects `Violation{Path, Reason}` into a single `PayloadValidationError` application failure. `Path` is the JSON member name.                                                                                      |
| TypeScript | Hand-emitted per-member checks over the parsed object; push `Violation { path, reason }` into the list, throw one `PayloadValidationError` application failure.                                                                                                                                                                                            |
| Python     | hand-emitted per-member checks over the raw `dict` in the model's `_<Model>TransferTypeConverter` (**PRINCIPLES Python §3**); each appends a `Violation { path, reason }` (`path` = the JSON member name) to the list raised as one `PayloadValidationError` application failure. The TypeScript strategy, expressed through the SDK's transfer-type hook. |
| Java       | per-POJO collecting `@JsonDeserialize` (PRINCIPLES Java §5): a two-stage bind that reads the object into a `JsonNode` tree, then dispatches each present member through its spec-strict/constraint helper (see [[type]]), collecting `Violation{path,reason}` into one `PayloadValidationError` application failure. The Go parallel.                      |
| C#         | the model's nested `Converter : JsonConverter<T>` (PRINCIPLES C# §4): `Read` binds the whole object with `JsonElement.ParseValue`, then `Parse` dispatches each present member through its spec-strict/constraint path (see [[type]]), collecting `Violation{path,reason}` into one `PayloadValidationError` application failure.                          |

A member subschema validates recursively — nested objects become nested
aggregates, arrays use [[items]], etc.

### Serialize-side (P12)

`properties` is symmetric across directions: serialize recurses the shared
`Validate` into each present member (a nested aggregate's own
`MarshalJSON`/`toTransferType`/`to_transfer_type` validates it), and the
JSON-name binding (`json` tag / `@JsonProperty` / the converter's wire key)
re-emits each member under its **original wire name**, not the case-mapped
identifier — so the contract is stable in both directions. Member
omit-vs-emit-`null` is owned by [[required]] + [[nullability]]; the per-member
value checks are the same predicates the deserializer runs. `path` on a
serialize-side failure is the JSON member name, identical to deserialize.

## Property-testing matrix

### Accepted (positive)

| Shape                                                         | Example                                                                                                                                                                         |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Typed struct                                                  | `{type:object, properties:{id:{type:integer}, name:{type:string}}}`                                                                                                             |
| Nested object member                                          | member schema is itself `{type:object, properties:{...}}` — named `<Model><Member>` and materialized (above)                                                                    |
| Nested object member, nullable                                | `{oneOf:[{type:object, properties:{…}},{type:"null"}]}` — same name, nullable value                                                                                             |
| Object element / map member                                   | `{type:array, items:{type:object, properties:{…}}}` → `<Model><Member>Item`; typed `additionalProperties` → `<Model>Value`                                                      |
| Member with assertions                                        | `{name:{type:string, minLength:1}}`                                                                                                                                             |
| Member using nullability                                      | `{bio:{oneOf:[{type:string},{type:null}]}}`                                                                                                                                     |
| Direct self-reference, optional (recursive, see [[ref]])      | linked list; `next` omitted from `required` so the chain can terminate: `{value:{type:string}, next:{$ref:"#"}}` with `required:[value]`                                        |
| Self-reference via array items, **required** OK (see [[ref]]) | tree node; the empty array terminates, so `children` may be required: `{value:{type:string}, children:{type:array, items:{$ref:"#"}}}`                                          |
| Mutual / indirect self-reference (see [[ref]])                | two `$defs` types referencing each other (cycle through `$ref`)                                                                                                                 |
| Name needing recasing                                         | `{properties:{user_id:{type:string}}}` → Go `UserId`, TS `userId`, Python `user_id`                                                                                             |
| Wire key that is not an identifier                            | `{properties:{"content-type":{type:string}}}` → Go `ContentType` with `json:"content-type"`, TS `contentType` read and written as `raw["content-type"]` / `out["content-type"]` |
| Acronym folded                                                | `{properties:{userID:{…}, httpServer:{…}}}` → Go `UserId`, `HttpServer`                                                                                                         |
| Per-language override admits an otherwise-rejected name       | `{properties:{class:{type:string, x-ts-name:"klass", x-py-name:"klass", x-java-name:"klazz"}}}`                                                                                 |
| Keyword-named member needs no override in Go                  | `{properties:{class:{type:string}}}` generating Go only → `Class`                                                                                                               |

### Rejected at load time (negative)

| Reason                                                                 | Example                                                                                                                                                                                                                                                                                                    |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `properties` not object                                                | `{type:object, properties: []}`, `…: "x"`                                                                                                                                                                                                                                                                  |
| Member not a schema                                                    | `{properties:{a: 5}}`, `{properties:{a: "string"}}`                                                                                                                                                                                                                                                        |
| Shapeless member (P7.1)                                                | `{properties:{a: {}}}`, `{a: true}`, `{a: false}`                                                                                                                                                                                                                                                          |
| Member missing `type`                                                  | `{properties:{a: {minLength: 1}}}`                                                                                                                                                                                                                                                                         |
| `properties` without `type:object`                                     | `{properties:{...}}` (no `type`) — per [[type]]                                                                                                                                                                                                                                                            |
| Name collision after recasing (emitted lang)                           | `{properties:{user_id:{…}, userId:{…}}}` → one Go `UserId`                                                                                                                                                                                                                                                 |
| Invalid identifier in an emitted lang (no override)                    | `{properties:{class:{…}}}` when emitting TypeScript/Python/Java; `{properties:{"2fa":{…}}}` (leading digit); `{properties:{"":{…}}}` (empty)                                                                                                                                                               |
| Collides with a name the converter binds (emitted lang)                | `{properties:{violations:{…}}}`, `{properties:{undefined:{…}}}`, `{properties:{toString:{…}}}` when emitting TypeScript; the Java deserializer's locals likewise                                                                                                                                           |
| Override not a legal identifier                                        | `x-py-name:"2fa"` / `x-py-name:"class"` (reserved)                                                                                                                                                                                                                                                         |
| Override collides                                                      | two members whose `x-go-name` both resolve to `Foo`                                                                                                                                                                                                                                                        |
| Unsatisfiable direct self-reference (see [[ref]] satisfiability check) | direct `$ref:"#"` member that is **required and non-nullable** — the chain can never terminate, so no finite instance exists (a satisfiability constraint, not a nullability one). A terminating form is required: optional (absent ends it), required+nullable (`null` ends it), or a collection wrapper. |

### Runtime fixtures (validator)

- Present member valid / invalid against its subschema.
- Member absent → defers to [[required]] (no error here if optional).
- Extra member not in `properties` → defers to [[additionalProperties]]
  (preserved when open, rejected when closed).
- Nested member failure produces a dotted `path` (`address.zip`, `address.zip`
  reason).
- Self-referential member (see [[ref]]): a recursive instance of bounded depth
  validates and round-trips; failure at depth N produces a path threading the
  recursion (`next.next.value`). The emitted aggregate must reference itself
  without infinite expansion (Go needs a pointer/ slice for indirection;
  Java/TS/Python use plain recursive reference types).

## Interactions

- **[[additionalProperties]]**: consumes `properties`' matched-name annotation.
  Members listed in `properties` are never "additional." Typed structs are
  **open by default** (per spec + **P13**) — see [[additionalProperties]] for
  the binding decision; closed requires explicit `additionalProperties: false`.
- **[[required]]**: orthogonal — `properties` types the members, `required`
  decides which must be present. A name may appear in `required` without
  appearing in `properties` (spec-legal); we **reject** that as a schema bug
  (required name with no declared shape) per **P7.1**.
- **[[patternProperties]]**: per spec also contributes to the matched-name
  annotation, but it is **temporarily unsupported** (rejected at load time in
  v1), so [[properties]] is the only contributor in practice.
- **[[type]]**: `properties` is only meaningful under `type:"object"`; pairing
  with any other `type` is a generator-time error.
- **[[nullability]]**: a member whose schema is the recognized `oneOf` null
  pattern is nullable; it is optional+nullable when absent from `required` and
  required+nullable when listed (both supported — presence and null-acceptance
  are orthogonal per **P8**).
- **[[dependentRequired]] / [[minProperties]] / [[maxProperties]]**: layer
  additional object-level assertions over the same member set. [[propertyNames]]
  layers a key-shape assertion, but only on the map form — beside `properties`,
  including an empty `properties: {}`, it is rejected. [[dependentSchemas]] is
  unsupported and rejected at load time.
- **[[ref]]**: a member schema may be a `$ref`, including one that resolves back
  to the containing object (direct, via-array, or mutual recursion). This is the
  only way to express recursive types; the matrix rows above are specified in
  [[ref]], which defines resolution scope and how recursive aggregates emit
  without infinite expansion. Termination rule: a **direct** self-reference must
  have a terminating form — either **optional** (absent ends the chain) or
  **required+nullable** (`null` ends the chain). Only required+**non-nullable**
  direct recursion has no finite instance and is rejected. A self-reference
  wrapped in a **collection** (array `items` / map `additionalProperties`) may
  be **required** and non-nullable, since the empty collection terminates.
  Representation note: Go must use indirection for any recursive type (a struct
  can't contain itself by value — `*T` for direct, slices/maps are already
  references); that nil-able pointer coincides with the nullable/optional slot
  the satisfiability rule already forces for direct recursion. Java/TS/Python
  use reference semantics and would accept a required non-null direct recursive
  field at the type level, but it is still semantically unsatisfiable, so we
  reject it uniformly.

## Ecosystem variance

| Source dialect        | Action                                                                                          |
| --------------------- | ----------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12   | Native.                                                                                         |
| OpenAPI 3.1           | Aligns with 2020-12. Native.                                                                    |
| OpenAPI 3.0           | `properties` identical; `nullable: true` on a member → reject (rewrite to nullability `oneOf`). |
| Swagger 2.0 / draft-4 | `properties` identical; same nullable rewrite.                                                  |

## See also

- [[additionalProperties]] — open/closed default; catch-all for extras.
- [[required]] — which members must be present.
- [[patternProperties]], [[propertyNames]], [[minProperties]], [[maxProperties]]
  — other object-level keywords.
- [[type]] — gates `properties` to `type:"object"`; member base types.
- [[nullability]] — optional/nullable member wrapping.
