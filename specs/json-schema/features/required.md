# `required`

Source: JSON Schema 2020-12, Validation vocabulary, §6.5.3 "Validation Keywords
for Objects → required".

Declares which object members **must be present**. The keyword that owns the
optional-vs-mandatory half of field shape; pairs with [[nullability]] (which
owns present-vs-null) and [[properties]] (which owns the member types).

## Spec summary

Verbatim (2020-12 validation, §6.5.3):

> The value of this keyword MUST be an array. Elements of this array, if any,
> MUST be strings, and MUST be unique.

> An object instance is valid against this keyword if every item in the array is
> the name of a property in the instance.

> Omitting this keyword has the same behavior as an empty array.

Distilled:
- `required` asserts **presence only** — it says nothing about the member's type
  (that is [[properties]]) or whether the value may be `null` (that is
  [[nullability]]).
- A name in `required` need not appear in [[properties]] (spec-legal); we reject
  that combination (see below).

## Support decision

**Support:** yes — core keyword.

Each listed name becomes a **mandatory** member: in languages with a type-level
presence channel it is emitted unwrapped (non-pointer / non-`?` /
non-`Optional`); everywhere the (de)serializer enforces presence per **P10** and
aggregates per **P11**.

Rationale (citing [[PRINCIPLES.md]]):
- **P10 (enforced, not advisory)**: presence is checked at the boundary, not
  just documented.
- **P8 (optional ≠ nullable)**: presence and null-acceptance are orthogonal, so
  `required` composes freely with the [[nullability]] `oneOf` pattern. A
  required name whose schema is nullable is **required+nullable** — must be
  present, value may be `null` — and is **supported** (presence-check on,
  null-rejection off). See [[nullability]].
- **P9 (distinguish absent from zero)**: Go reads the raw-object map and Java
  reads the exact tree node to detect absence; a present zero value is not
  "absent."

Loader behavior:
- `required` not an array → reject. An explicit `null`, and the bare YAML key
  `required:` that spells it, are **not** absence: reading them as absence turns
  every member of the object optional and silently widens the wire contract, so
  they take the same reject as any other non-array value.
- Any element not a string → reject.
- Duplicate elements → reject (spec says MUST be unique; a dup is a schema bug).
- A required name **not** present in [[properties]] → reject per **P7.1**: a
  mandatory member with no declared shape is undecidable. Diagnostic names the
  missing property. (This is the binding form of the rule sketched in
  [[properties]].)
- A required name whose schema matches the [[nullability]] pattern →
  **accepted** as required+nullable (see [[nullability]]).
- Empty `required: []` → accepted (vacuous no-op; equals omission).

## Type mapping

`required` does not introduce a type; for a **non-nullable** member it
**removes** the optional wrapper that [[nullability]] would otherwise apply. For
a **nullable** member (the `oneOf` null pattern) it removes nothing — the type
stays the nullable form and `required` only adds the presence check (this is the
required+nullable state; see [[nullability]]). The table shows the required vs
optional emitted form for the non-nullable case (mirrors [[nullability]]):

| `type` token | Required (this keyword)                                                                                            | Optional (name absent from `required`)                                                          |
| ------------ | ------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------- |
| `"integer"`  | Go `int64` · TS `x: number` · Py `int` · Java `long`                                                               | Go `*int64` · TS `x?: number` · Py `int \| None = None` · Java `@Nullable Long`                 |
| `"string"`   | Go `string` *(non-null validator)* · TS `x: string` · Py `str` · Java `String` *(non-null; `@NullMarked` default)* | Go `*string` · TS `x?: string` · Py `str \| None = None` · Java `@Nullable String`              |
| `"object"`   | Go `T` *(non-null)* · TS `x: T` · Py `T` · Java `T` *(non-null; `@NullMarked` default)*                            | Go `*T` · TS `x?: T` · Py `T \| None = None` · Java `@Nullable T`                               |
| `"array"`    | Go `[]T` *(non-null)* · TS `x: T[]` · Py `list[T]` · Java `List<T>` *(non-null; `@NullMarked` default)*            | Go `[]T` *(nil=absent)* · TS `x?: T[]` · Py `list[T] \| None = None` · Java `@Nullable List<T>` |

Go value and slice types, and Java's advisory `@NullMarked` signal, can't carry
"must be present" in the type system; TypeScript's `?` and Python's defaultless
field do carry it. Every target still leans on the validator, since no static
channel survives a cast, a mutation, an untyped caller or an unchecked runtime
value. In Java the emitted package is `@NullMarked` (JSpecify), so a required
reference type is non-null by default (no annotation) and an optional one is
`@Nullable` — restoring at the type level the in-memory nullness signal that
`long`-vs-`Long` gives scalars, complementary to the non-null validator. See
[[nullability]] and PRINCIPLES Java §3.

## Validator mapping

Per **P10**/**P11**. The "Required, non-nullable" row of [[nullability]]'s
matrix is authoritative; summarized here:

| Language   | Presence enforcement                                                                                                                                                                                                                                                                                                                                                                                               |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Go         | raw-object map lookup returning `*json.RawMessage`; `nil` → `Violation{Path:name, Reason:"required"}`, collected into one `PayloadValidationError` application failure. An explicit `null` on a non-nullable member is a distinct reason (`explicit null not allowed`).                                                                                                                                            |
| TypeScript | the member's **own**-key presence test over `raw` (an inherited `Object.prototype` member is not a wire value); absent or `null` → push `Violation{path:name, reason:"required"}`, throw one `PayloadValidationError` application failure.                                                                                                                                                                         |
| Python     | in `from_transfer_type` (**PRINCIPLES Python §3**): an absent or `null` key for a required non-nullable member → `Violation(path=name, reason="required")`, appended and the field left unset, so its siblings are still checked; collected into the single `PayloadValidationError` application failure.                                                                                                          |
| Java       | in the per-POJO collecting deserializer (PRINCIPLES Java §5): a missing tree node for a required member → `Violation{path:name, reason:"required"}`, and an explicit `null` on a non-nullable member the distinct `explicit null not allowed`, decided by a per-field branch over the node's `isNull()` (Java §4) rather than a per-field binder. Collected into one `PayloadValidationError` application failure. |

Required + explicit `null`: for a required **non-nullable** member, rejected
(may not be `null`) — same machinery as the optional-non-nullable null rejection
in [[nullability]]. For a required **nullable** member, `null` is accepted (only
absence is rejected).

**Serialize side (P12).** The presence check runs again before emit, off the
in-memory value: a required member that is empty in memory (Go `nil` pointer ·
TS `undefined` · Python `None` · Java `null` reference) is a
`PayloadValidationError` application failure, so Go `MarshalJSON`, TypeScript
`toTransferType`, Python `to_transfer_type`, and Java's generated `Serializer`
fail rather than emitting a malformed object. A required member is therefore
**never omitted** on serialize — required-non-nullable always emits its value;
required+nullable emits the value or `null`, never absent (see the
[[nullability]] serialize table). This mirrors the deserialize wire-absence
check; only the absence *signal* differs (in-memory empty vs a missing
raw-object/tree entry on the wire).

## Property-testing matrix

### Accepted (positive)

| Shape               | Example                                                                                               |
| ------------------- | ----------------------------------------------------------------------------------------------------- |
| Some required       | `{type:object, properties:{id:{type:integer}, name:{type:string}}, required:["id"]}`                  |
| All required        | `required:["id","name"]`                                                                              |
| Empty (no-op)       | `required:[]`                                                                                         |
| Required + nullable | `properties:{x:{oneOf:[{type:string},{type:null}]}}, required:["x"]` — must be present, may be `null` |

### Rejected at load time (negative)

| Reason                          | Example                                         |
| ------------------------------- | ----------------------------------------------- |
| Not an array                    | `required:"id"`, `required:{}`, `required:null` |
| Non-string element              | `required:[1]`, `required:[true]`               |
| Duplicate element               | `required:["id","id"]`                          |
| Name not in `properties` (P7.1) | `properties:{id:{…}}, required:["name"]`        |

### Runtime fixtures (validator)

- Required member present + valid → OK.
- Required member absent → one `PayloadValidationError` application failure at
  `path:name`. The `path` names the member; the reason text is per-target
  (**P11**) and is given in the validator-mapping table above.
- Required **non-nullable** member present as `null` → rejected.
- Required **nullable** member present as `null` → OK; absent → still one
  `PayloadValidationError` application failure at `path:name`.
- Multiple required members absent → all reported in one shot (P11).
- Optional member absent → no error (contrast control).

## Interactions

- **[[properties]]**: orthogonal — `properties` types the member, `required`
  makes it mandatory. Required name must exist in `properties` (else reject).
- **[[nullability]]**: orthogonal (P8). `required` controls presence; the
  `oneOf` null pattern controls null-acceptance. All four combinations are
  legal, including required+nullable. Optional is the default (name absent from
  `required`).
- **[[additionalProperties]]**: independent — a closed struct still honors
  `required`; closing only forbids *unknown* members.
- **[[dependentRequired]]**: conditional requiredness layered on top — a member
  required only when another is present. Members named there stay optional at
  the type level (the requirement is runtime-only).
- **[[default]]**: a `default` on a required member never applies — the member
  is always on the wire — and is a load reject, owned by [[default]]. The reject
  reads the member's own keyword node, and for a nullable member that node is
  the [[nullability]] wrapper rather than the non-null branch, so `required` + a
  wrapper `default` rejects exactly as `required` + a plain `default` does, and
  a `default` moved onto a branch is a reject too rather than a way past this
  rule.
- **[[minProperties]] / [[maxProperties]]**: separate count assertions;
  `required` pins *which* members, the count keywords pin *how many*, and they
  own the reconciliation between the two (a cap below the required count is a
  load reject, specified there).

## Ecosystem variance

| Source dialect        | Action                                                                                                 |
| --------------------- | ------------------------------------------------------------------------------------------------------ |
| JSON Schema 2020-12   | Native.                                                                                                |
| OpenAPI 3.1           | Aligns with 2020-12. Native.                                                                           |
| OpenAPI 3.0           | `required` identical apart from the empty array, which 2020-12 permits and Wright-00 does not. Native. |
| Swagger 2.0 / draft-4 | `required` identical (draft-4 onward is the array form), again apart from the empty array. Native.     |

draft-03's boolean `required` (on the property schema itself) is obsolete; no
current toolchain emits it.

## See also

- [[nullability]] — present-vs-null; owns the optional/nullable wrapping
  `required` toggles.
- [[properties]] — member types; required names must be declared here.
- [[dependentRequired]] — conditional presence requirements.
- [[minProperties]], [[maxProperties]] — member-count assertions.
