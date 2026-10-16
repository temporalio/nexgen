# `nullability` (cross-cutting design note)

Not a JSON Schema keyword. Captures the generator's per-language convention for
how **optionality** ("key may be absent") and **nullability** ("value may be
`null`") are encoded in emitted code. Per **P8** these are two different
concerns and we keep them distinct.

## Concept matrix

| Concern      | JSON Schema source                                                                                                                                           | Wire shape                          | Type-level question              |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------- | -------------------------------- |
| **Optional** | property name *not* in `required` array of the enclosing object                                                                                              | key may be absent                   | "How do we represent absent?"    |
| **Nullable** | the `oneOf:[{type:"T"},{type:"null"}]` pattern (see "Nullability convention" below); the array form `["T","null"]` is rejected per [[type]] support decision | value present, equal to JSON `null` | "How do we represent JSON null?" |

This doc owns the optionality conventions per language and the nullability
convention (the recognized `oneOf:[{type:T},{type:null}]` pattern, defined under
"Nullability convention" below).

## Optionality conventions per language

### Java

Primitive type for required fields; boxed type for optional. Emitted as a POJO
(Java 8 floor — **not** a record; see PRINCIPLES Java §1), so the fields below
are private with generated getters/constructor. The package is `@NullMarked`
(JSpecify; non-null by default), so optional reference fields carry `@Nullable`
and required ones need no annotation (see PRINCIPLES Java §3):

```java
// package-info.java: @NullMarked applies to the generated package.
public final class User {
    private final long      id;        // required: integer
    private final @Nullable Long   nickname;  // optional: integer — null if absent
    private final String    name;      // required: string (non-null by default; validator enforces)
    private final @Nullable String email;     // optional: string
    // generated constructor, getters, equals/hashCode/toString
}
```

| `type` token | required                         | optional            |
| ------------ | -------------------------------- | ------------------- |
| `"integer"`  | `long`                           | `@Nullable Long`    |
| `"number"`   | `double`                         | `@Nullable Double`  |
| `"boolean"`  | `boolean`                        | `@Nullable Boolean` |
| `"string"`   | `String` *(non-null validator)*  | `@Nullable String`  |
| `"object"`   | `T` *(non-null validator)*       | `@Nullable T`       |
| `"array"`    | `List<T>` *(non-null validator)* | `@Nullable List<T>` |

Required-field validators must still check absence and `null` explicitly for
reference types (`String`, `List<T>`, object types) — the type system can't
carry that constraint by itself. The `@Nullable`/ `@NullMarked` annotations are
**complementary**: they track in-memory post-construction nullness (required →
non-null; optional → may be null when the key is absent) and propagate it into
the consumer's static null-analysis. Because generated source imports them,
`org.jspecify:jspecify:1.0.0` must be on the compile classpath (normally
`compileOnly` in Gradle or `provided` in Maven). JSpecify's annotations have
runtime retention but enforce nothing at runtime; consumers that reflect over
them must provide JSpecify themselves. They do **not** encode the wire-level
nullable distinction — every optional reference field is `@Nullable` regardless
of whether it is optional-non-nullable or optional+nullable.

### C#

`Nullable<T>` for optional value types, a `T?` nullable reference annotation for
optional reference types, and the `required` modifier for required members
(PRINCIPLES C# §2):

```csharp
public sealed record User
{
    public required long Id { get; init; }        // required: integer
    public long? Nickname { get; init; }          // optional: integer — null if absent
    public required string Name { get; init; }    // required: string (Validate rejects null)
    public string? Email { get; init; }           // optional: string
}
```

| `type` token | required                                           | optional            |
| ------------ | -------------------------------------------------- | ------------------- |
| `"integer"`  | `required long`                                    | `long?`             |
| `"number"`   | `required double`                                  | `double?`           |
| `"boolean"`  | `required bool`                                    | `bool?`             |
| `"string"`   | `required string` *(non-null validator)*           | `string?`           |
| `"object"`   | `required T` *(non-null validator)*                | `T?`                |
| `"array"`    | `required IReadOnlyList<T>` *(non-null validator)* | `IReadOnlyList<T>?` |

Nullable reference annotations are compile-time only, so `Validate` still
rejects a required reference left `null` before writing. As in Java, the
annotations track in-memory nullness, not the wire's nullable distinction:
optional and optional+nullable members share one `null` state.

### Go

Pointer to primitive for optional fields; bare type for required.

```go
type User struct {
    ID       int64   `json:"id"`              // required
    Nickname *int64  `json:"nickname,..."`    // optional — nil if absent
    Name     string  `json:"name"`            // required
    Email    *string `json:"email,..."`       // optional
}
```

| `type` token | required  | optional                     |
| ------------ | --------- | ---------------------------- |
| `"integer"`  | `int64`   | `*int64`                     |
| `"number"`   | `float64` | `*float64`                   |
| `"boolean"`  | `bool`    | `*bool`                      |
| `"string"`   | `string`  | `*string`                    |
| `"object"`   | `T`       | `*T`                         |
| `"array"`    | `[]T`     | `[]T` *(nil-slice = absent)* |

Pointer-from-literal ergonomics: Go 1.26 extended the `new` builtin to take an
expression — `new(expr)` allocates a variable of the inferred type, initializes
it to `expr`'s value, and returns `*T`. The release notes explicitly call out
optional-pointer JSON fields as the motivating use case.

```go
// Pre-1.26 (verbose):
tmp := int64(42)
u := User{Nickname: &tmp}

// 1.26+ (a concise consumer call-site convention):
u := User{Nickname: new(int64(42))}
u := User{Nickname: new(req.Count)}        // arbitrary expressions work
u := User{Nickname: new(yearsSince(t))}    // including function calls
```

The generator emits no Go model constructors or builders; this is consumer
call-site guidance only. Go 1.26+ is **not** a hard requirement — on older
toolchains the equivalent verbose form (`tmp := 42; u.Nickname = &tmp`) compiles
correctly. The generated *user-facing* API surface is identical either way; only
the consumer's call-site idiom differs.

`[]T` for optional arrays: a `nil` slice and an empty slice are distinguishable
in Go (`s == nil` vs `len(s) == 0`), so a pointer wrapper is unnecessary; the
runtime check is `if user.Tags != nil`.

### TypeScript

Optional fields use the `?` modifier; the field type stays the bare type.
Absence is `undefined`, not `null`.

```ts
interface User {
  id: number;          // required
  nickname?: number;   // optional — undefined if absent
  name: string;        // required
  email?: string;      // optional
}
```

| `type` token | required | optional                  |
| ------------ | -------- | ------------------------- |
| any          | `T`      | `T` with `?` on the field |

Validator emits `if (raw.x === undefined) ...` for required-presence checks. We
never use `T | null` for the optional-only case — `?`/ `undefined` is the
absence channel; `null` is a value reserved for the nullability convention
(`x?: T | null` is the optional+nullable form).

### Python

Optional or nullable model properties use `T | None`; required non-nullable
properties carry bare `T`. Every public constructor argument is keyword-only
(see PRINCIPLES Python §1):

```python
from __future__ import annotations

import dataclasses


@dataclasses.dataclass(slots=True, kw_only=True)
class User:
    id: int                      # required
    nickname: int | None = None  # optional — None if absent
    name: str                    # required
    email: str | None = None     # optional
```

| `type` token | required | optional                           |
| ------------ | -------- | ---------------------------------- |
| any          | `T`      | `T | None` (with `= None` default) |

Absence is `None`. The dataclass itself neither coerces nor checks anything: the
only path from wire to field is the model's transfer-type converter (PRINCIPLES
Python §3), whose parse adapter classifies the type token outright and so admits
no lax coercion (`"1"`→`1`, `1`→`True`) that would violate P10/P7. At the value
level absence and explicit `null` both read as `None`, and a dataclass field has
nowhere to record which of the two the wire carried, so optional+nullable
collapses to the same tier as Go and Java (see "Round-trip behavior" and
"Serialize-side behavior" below).

## Nullability convention

The only accepted source-level expression for "this field's value may be `null`"
is the JSON Schema 2020-12 canonical idiom:

```json
{ "oneOf": [{"type": "<T>"}, {"type": "null"}] }
```

Order is insensitive — `[{"type":"null"}, {"type":"<T>"}]` is equivalent. The
non-null branch is a full subschema and may carry any sibling keyword recognized
for that `type`:

```json
{ "oneOf": [
    {"type": "string", "format": "email", "minLength": 5},
    {"type": "null"}
]}
```

This two-branch form is the **degenerate type-token case** of the general
`oneOf` rule (the `null` token is the selector); [[oneOf]] owns the general
treatment, and this doc owns this specific shape.

### Which node owns a keyword — the wrapper or the branch

The wrapper is a `oneOf`, so [[oneOf]]'s sibling rule governs every keyword
written on it; this doc settles the one case [[oneOf]] delegates here,
[[default]].
**Constraints belong on the non-null branch; `default` belongs on the wrapper.**

| Keyword                                                                                                                     | Node it is authored on             | On the other node                                                             |
| --------------------------------------------------------------------------------------------------------------------------- | ---------------------------------- | ----------------------------------------------------------------------------- |
| `type`, [[format]], [[contentEncoding]], [[const]]/[[enum]], [[pattern]], and every string, numeric, array and object bound | the **non-null branch**            | wrapper → **load reject**, fix-it naming the branch ([[oneOf]])               |
| [[default]]                                                                                                                 | the **wrapper**                    | branch → **load reject**, fix-it naming the wrapper                           |
| annotations ([[title]], [[description]], [[comment]], [[examples]], [[deprecated]]) and `x-<lang>-name`                     | the wrapper or the non-null branch | — (the `null` branch takes no siblings at all — see Pattern acceptance rules) |

A wrapper `default` lowers against the **resolved non-null branch's** type, not
against the JSON kind of the default literal — the wrapper carries no scalar
`type` of its own ([[default]] owns the lowering rule). The loader validates a
wrapper default against that branch's `type`, `enum`, `format` and
`contentEncoding`.

The wrapper does **not** shield a nullable member from a check that reads the
**property node's** keyword map — for a nullable member that node is the
wrapper, not the branch. So [[default]]'s "`default` on a **required** member →
reject" applies to a nullable member exactly as to a plain one, and moving the
`default` onto a branch is a reject rather than a way around it.

### The wrapper does not change what the branch requires

The wrapper is a presence/nullness marker, not a new schema: the non-null branch
lowers exactly as it would unwrapped.

- Every package-level artifact the branch's constraints need is still emitted —
  in Go the compiled-regex statics a [[pattern]], [[format]] or
  [[contentEncoding]] assertion needs, whether the assertion sits on the branch
  itself or on the elements or [[contains]] matcher of a branch that is an
  array. Dropping them because the member is nullable leaves the package
  referring to an undeclared identifier.
- Where the collapse removes the pointer, no emitted traversal may dereference
  the member. A nullable array is `[]T` with no wrapper (above), including as a
  typed [[additionalProperties]] value, so its element loop ranges over the
  value and not over `*value`.

### Pattern acceptance rules

This doc recognizes a `oneOf` as the nullability pattern iff:
- exactly 2 branches;
- exactly one branch is the literal `{"type": "null"}` with no sibling keywords
  on the null branch;
- the other branch declares a recognized [[type]] (must not itself be `"null"` —
  `{type:"null"}` paired with `{type:"null"}` is a tautology and rejected).

Any other `oneOf` shape is [[oneOf]]'s domain — supported there when its
branches are separable by a decidable selector (pairwise-disjoint JSON type
kinds), and **rejected** when they are not, since without a selector no
deserializer can route the wire value. [[oneOf]] names the individual shapes it
defers; a missing selector is not one of them. A `null` branch among 3+ kinds is
a **nullable union** — supported by [[oneOf]], reusing this doc's per-language
nullable encoding over the union type (the `null` branch marks the field
nullable; the non-null branches form the sum type).

### Required + nullable is supported

A field listed in the enclosing object's `required` array whose schema matches
the nullable `oneOf` pattern is **accepted**. It encodes "must be present, value
may be `null`" — `{}` is rejected (absent), `{"x": null}` and `{"x": T}` are
both accepted. The construct is fully decidable: required+nullable accepts
`{"x":null}` and `{"x":T}`; optional+non-nullable accepts `{}` — disjoint edge
cases, with none of the other three states expressing the `{null, T}` space. Per
**P8** optional and nullable are orthogonal, so all four combinations are legal.

### Per-language emitted type (nullable states)

Two nullable states exist: **optional+nullable** (absent / `null` / T) and
**required+nullable** (`null` / T; absent rejected). They share the same emitted
*type* in every language; only the presence check differs, and TypeScript/Python
also differ at the declaration level (TS's `?` modifier, Python's `= None`
default).

**Optional + nullable** (absent OK, `null` OK, T OK):

| `type` token | Java                | Go                           | TypeScript            | Python                      | C#                  |
| ------------ | ------------------- | ---------------------------- | --------------------- | --------------------------- | ------------------- |
| `"integer"`  | `@Nullable Long`    | `*int64`                     | `x?: number \| null`  | `x: int \| None = None`     | `long?`             |
| `"number"`   | `@Nullable Double`  | `*float64`                   | `x?: number \| null`  | `x: float \| None = None`   | `double?`           |
| `"boolean"`  | `@Nullable Boolean` | `*bool`                      | `x?: boolean \| null` | `x: bool \| None = None`    | `bool?`             |
| `"string"`   | `@Nullable String`  | `*string`                    | `x?: string \| null`  | `x: str \| None = None`     | `string?`           |
| `"object"`   | `@Nullable T`       | `*T`                         | `x?: T \| null`       | `x: T \| None = None`       | `T?`                |
| `"array"`    | `@Nullable List<T>` | `[]T` (nil = absent or null) | `x?: T[] \| null`     | `x: list[T] \| None = None` | `IReadOnlyList<T>?` |

**Required + nullable** (`null` OK, T OK, absent rejected) — same type, presence
enforced by the validator; TS drops the `?`, Python drops the `= None` default
(a dataclass field with no default must be supplied at construction):

| `type` token | Java                | Go                 | TypeScript           | Python               | C#                           |
| ------------ | ------------------- | ------------------ | -------------------- | -------------------- | ---------------------------- |
| `"integer"`  | `@Nullable Long`    | `*int64`           | `x: number \| null`  | `x: int \| None`     | `required long?`             |
| `"number"`   | `@Nullable Double`  | `*float64`         | `x: number \| null`  | `x: float \| None`   | `required double?`           |
| `"boolean"`  | `@Nullable Boolean` | `*bool`            | `x: boolean \| null` | `x: bool \| None`    | `required bool?`             |
| `"string"`   | `@Nullable String`  | `*string`          | `x: string \| null`  | `x: str \| None`     | `required string?`           |
| `"object"`   | `@Nullable T`       | `*T`               | `x: T \| null`       | `x: T \| None`       | `required T?`                |
| `"array"`    | `@Nullable List<T>` | `[]T` (nil = null) | `x: T[] \| null`     | `x: list[T] \| None` | `required IReadOnlyList<T>?` |

(Java is `@Nullable` across every nullable column — the annotation tracks
in-memory nullness, not the wire distinction; see the optionality section above
and PRINCIPLES Java §3. In Java/Go, required+nullable and optional+nullable
share both type *and* annotation; the presence check is the only difference,
exactly as required-non-nullable reference types already rely on a validator the
type can't express.)

### Round-trip behavior

- **Required + nullable round-trips losslessly in all five languages.** Presence
  is guaranteed, so an in-memory `null`/`nil`/`None` unambiguously means "the
  wire sent `null`"; the serializer always emits the key (never omits it), and
  `null` ⟷ `null`. There is no absent state to confuse it with.
- **Optional + nullable round-trips faithfully in TypeScript; collapses in Go,
  Java, Python and C#.** TS keeps `undefined` (absent) vs `null` distinct in
  memory, so its serializer re-emits a wire `null` as `null` and omits a
  wire-absent key. Go (`*T` `nil`), Java (`null`), Python (`None`) and C#
  (`null`) genuinely cannot distinguish the two in memory, so they emit a single
  canonical form — the key is **omitted** (the conservative choice; emitting
  `null` would fabricate a value the client may never have sent). A client that
  sent explicit `null` on an optional+nullable field reads it back as absent
  **in Go/Java/Python/C#**.

**Collapse note (Go / Java / Python / C#):** the in-memory representations of
"absent" and "JSON null" are the same (`nil`, `null`, `None`, `null`), and —
unlike TS's `undefined` — there is no side channel recording which the wire
carried, so post-validation user code can't recover it. This matches **P8**'s
framing — optional and nullable are distinct *schema* concerns; runtime collapse
is acceptable when the language can't represent the difference. Making any of
the four faithful would require a presence-tracking channel — a `Null[T]`
wrapper or shadow bit in Go/Java/C#, an `UNSET` sentinel widening every optional
field to `T | None | UnsetType` or a hidden per-instance presence set in Python;
each is rejected as ergonomic overhead (P2) that the conservative omit avoids,
and the sentinel additionally forces every consumer to test against a generated
marker instead of plain `None`.

TypeScript enforces *and* preserves the distinction; Go, Java, Python and C#
enforce it at the boundary but collapse it in memory.

**What the collapse is licensed for, and where that runs out.** P1 excepts the
collapse from *value-level round-trip fidelity*, and only for Go, Java, Python
and C# — TypeScript is not in the exception because it does not collapse. P1
excepts nothing from **validation semantics**. The strict subset therefore
rejects presence-sensitive combinations that cannot survive the collapse: a
positive [[minProperties]] floor beside any optional nullable declared member,
and a non-vacuous [[dependentRequired]] edge whose trigger or dependent is
optional nullable. This keeps the plain ergonomic representation above without
admitting a schema whose accepted set changes by target or direction.

### Diagnostics

Wire form → required generator output:

| Source form                                                      | Action                                                                                                                                                            |
| ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `"type": ["T", "null"]` (array form)                             | Reject. Diagnostic suggests `oneOf: [{type:"T"}, {type:"null"}]`.                                                                                                 |
| `{type:"T", "nullable": true}` (OAS 3.0)                         | Reject. Diagnostic suggests `oneOf: [{type:"T"}, {type:"null"}]`.                                                                                                 |
| `oneOf` with `{type:"null"}` branch where field is in `required` | **Accept** — required+nullable (must be present, may be `null`).                                                                                                  |
| `oneOf` of 3+ branches with `{type:"null"}` among them           | **Accept** — a nullable union ([[oneOf]]): the `null` branch marks the field nullable, the non-null branches (which must be pairwise-disjoint) form the sum type. |
| A shape or constraint keyword on the **wrapper**                 | Reject. Fix-it names the non-null branch as the node that owns it.                                                                                                |
| `default` on the **wrapper**                                     | **Accept** — lowered against the resolved non-null branch's type.                                                                                                 |
| `default` on a **branch**                                        | Reject. Fix-it names the wrapper as the node that owns it.                                                                                                        |

## Validator implications

Four schema states × five languages → twenty cells. The two axes are orthogonal:
**presence** (required = reject absent; optional = accept absent) and
**null acceptance** (non-nullable = reject `null`; nullable = accept `null`).

| State                                                                       | Java                                                                                      | Go                                                                           | TS                                                                         | Python                                                                                                                   | C#                                                                                                                             |
| --------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- | -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| **Required, non-nullable** — must be present, must be T                     | type is `long`/`String`/etc.; collecting deserializer rejects an absent or null tree node | type is `int64`/`string`/etc.; raw-map lookup distinguishes absent from null | type is `x: T`; emit `raw.x === undefined \|\| raw.x === null` reject      | type is `x: T` with no default; converter rejects an absent key **and** a `null` token with `required`                   | type is `long`/`string`/etc. with `required`; `Converter.Parse` rejects an absent or null member                               |
| **Optional, non-nullable** — absent OK, T OK, explicit `null` rejected      | collecting deserializer's three-way tree-node branch (see strategy below)                 | raw-map `*json.RawMessage` lookup plus `isNull`                              | `raw.x === null` rejected; `=== undefined` OK                              | type is `x: T \| None = None`; converter branch over the raw dict rejects a key present with `None` (see strategy below) | `Converter.Parse`'s three-way branch over the member lookup and `ValueKind == Null`                                            |
| **Optional + nullable** — absent OK, `null` OK, T OK                        | type is `@Nullable Long`/`String`/etc.; no extra check beyond type binding                | type is `*int64`/`*string`/etc.; no extra check beyond type binding          | type is `x?: T \| null`; both `undefined` and `null` accepted              | type is `x: T \| None = None`; both absent and `null` accepted, no extra check                                           | type is `long?`/`string?`/etc.; absent and `null` collapse to `null`, re-emitted absent                                        |
| **Required + nullable** — must be present, `null` OK, T OK, absent rejected | collecting deserializer accepts a null node and rejects an absent one                     | raw-map lookup rejects absence and accepts a null token                      | type is `x: T \| null`; emit `raw.x === undefined` reject; `null` accepted | type is `x: T \| None` with **no** default; converter rejects an absent key, accepts the `null` token as `None`          | `required long?`/`string?`/etc.; `Converter.Parse` accepts a null member and rejects an absent one; `WriteValue` writes `null` |

## Serialize-side behavior

Per **P12** the encode adapter chooses,
*per field from the optional/nullable/required declaration*, whether an empty
in-memory value (`undefined`/`nil`/`None`/`null`) is omitted or emitted as
`null`. The decision is **static** (baked into the generated serializer), never
a function of the value alone: a blanket "omit all nulls" would drop a
required+nullable `null` (violating `required`), and a blanket "emit all nulls"
would fabricate an invalid `null` for optional-non-nullable.

| required | nullable     | empty-value serialize action                                                                                                                                                                                                                                                                                          |
| -------- | ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| optional | non-nullable | **omit** the key (emitting `null` is invalid; `Validate` also rejects an explicit in-memory `null` where the language can hold one)                                                                                                                                                                                   |
| optional | nullable     | **omit** (conservative) in Go/Java/Python/C#; **faithful** in TS — omit if `undefined`, emit `null` if explicitly `null`. Presence-sensitive combinations that would observe the collapse reject at load — see the Collapse note                                                                                      |
| required | non-nullable | cannot be empty — always emit the value. Where the language can hold an empty reference (Java `null`, Python `None`, TS `undefined`/`null`) `Validate` **rejects** it rather than omitting the key; a Go `nil` slice on a required array is written as `[]`, since Go has no non-nil empty-slice type (see [[items]]) |
| required | nullable     | **emit `key: null`** — omitting violates `required`                                                                                                                                                                                                                                                                   |

Per-language mechanism (all are *encode-adapter* concerns; the shared `Validate`
runs first and is identical to the deserialize side):

- **Go** — the generated `MarshalJSON` builds a `map[string]json.RawMessage` key
  by key. Optional pointers are omitted when nil; required+nullable pointers
  always write their key and encode nil as `null`. Struct tags remain useful to
  readers and reflection-based tools but are not consulted by the generated
  codec.
- **TypeScript** — `toTransferType` omits `undefined`, emits `null`; the
  three-state gives faithful optional+nullable for free.
- **Python** — the model's `to_transfer_type` builds the outgoing dict key by
  key (PRINCIPLES Python §3): an optional field is written only when its
  attribute is not `None`; a required+nullable field is always written, as
  `None` when the attribute is `None` (which encodes to JSON `null`);
  required-non-nullable, `const` and defaulted fields are always written.
  Optional+nullable takes the conservative omit — a `None` attribute cannot say
  whether the wire carried `null`.
- **Java** — the per-POJO nested `Serializer` (PRINCIPLES Java §6) writes the
  object field by field, honoring `@JsonInclude` semantics **in code** rather
  than by annotation: an optional field is written only when non-`null`
  (`NON_NULL` behavior), a required+nullable field is always written and emits
  `writeNullField` when `null` (`ALWAYS` behavior), and optional+nullable
  collapses to the conservative omit. The POJO itself carries no `@JsonInclude`
  — the mirror of the collecting deserializer, keeping both directions in one
  hand-emitted place.

## Strict enforcement of optional-non-nullable

**Decision:** explicit `"key": null` is rejected when the schema is
optional-non-nullable (the JSON-Schema-default case — a `{type: "T"}` field not
listed in `required`, where T is not the nullability pattern). This honors the
spec: `null` is not a valid value of any non-`null` type, so a bare
`{type: "string"}` doesn't admit `null`.

### Java

The per-POJO collecting deserializer (PRINCIPLES Java §5) decides the three-way
per field over the parsed tree node, mirroring Go:

1. node absent (`root.get(name) == null`) → key absent (required → push
   `Violation`; optional → leave field null/unset)
2. node `isNull()` → explicit `null` (optional-non-nullable → push `Violation`;
   nullable → accept as `null`)
3. otherwise → call the type-specific strict-parse helper (e.g.
   `SpecNumbers.specLong(node, path, errs)`), which pushes its own `Violation`
   on a spec violation

There is **no** `…StrictDeserializer` / `getNullValue` override and no per-field
`@JsonDeserialize`: the null/absence decision lives in the collecting
deserializer's branch, so the old two-subclass split collapses into the same
three-way Go uses in `UnmarshalJSON`.

### C#

The model's `Converter.Parse` decides the three-way per field over the wire
members (PRINCIPLES C# §4), mirroring Go and Java:

1. member absent (`!members.TryGetValue(name, out var node)`) → key absent
   (required → record `Violation`; optional → leave the property `null`)
2. `node.ValueKind == JsonValueKind.Null` → explicit `null`
   (optional-non-nullable → record `Violation`; nullable → accept as `null`)
3. otherwise → the type-specific strict parse (e.g.
   `SpecJson.SpecLong(node, path, violations)`), which records its own
   `Violation` on a spec violation

### Go

The generated `UnmarshalJSON` first reads a `map[string]json.RawMessage` and
uses a per-key accessor returning `*json.RawMessage`. Per field:

1. `raw == nil` → key absent (required → emit error; optional → leave field
   zero)
2. `isNull(*raw)` → explicit `null` (optional-non-nullable → emit error;
   nullable → accept)
3. otherwise → delegate to the type-specific runtime helper (e.g.,
   `parseSpecInteger`)

### TypeScript

Natural three-way via `=== undefined` vs `=== null`:

```typescript
if (raw.x === null) {
    violations.push({ path: "x", reason: "explicit null not allowed" });
} else if (raw.x !== undefined) {
    // validate value
}
```

No runtime helper needed.

### Python

The model's `from_transfer_type` (PRINCIPLES Python §3) decides the three-way
per field over the raw decoded dict, structurally identical to Go's and Java's:

1. key absent (`name not in raw`) → key absent (required → push
   `Violation(path, "required")`; optional → leave the field at `None`)
2. key present and `raw[name] is None` → explicit `null` (optional-non-nullable
   → push `Violation(path, "explicit null not allowed")`; nullable → accept as
   `None`)
3. otherwise → call the type-specific violation-collecting parse helper (e.g.
   `_parse_spec_integer(raw[name], path, violations)`), which pushes its own
   `Violation` and returns `None` on a spec violation

```python
if "nickname" in raw:                        # optional, non-nullable
    nickname_value_raw = raw["nickname"]
    if nickname_value_raw is None:
        violations.append(
            Violation(path="nickname", reason="explicit null not allowed")
        )
    else:
        nickname = _parse_spec_integer(nickname_value_raw, "nickname", violations)
```

The absent-vs-`None` distinction is available here because the converter
inspects the **raw dict**, before any field is assigned — which is exactly why
this check can only live in the parse adapter (**P12** layer 1). Once the value
has landed on the dataclass, `None` means only "empty", and the wire information
the check needs is gone.

## See also

- [[type]] — emitted bare type per `type` token; this doc wraps that.
- [[required]] — owns *which* fields are optional (the JSON Schema side of the
  decision).
- [[oneOf]] — owns the general `oneOf` treatment (type-token-separable unions);
  this doc owns the degenerate two-branch `oneOf:[{T},{null}]` nullability shape
  (defined under "Nullability convention" above).
- [[PRINCIPLES.md]] — **P8** (optional ≠ nullable), **P9** (distinguish absent
  from zero value), **P2** (ergonomics).
