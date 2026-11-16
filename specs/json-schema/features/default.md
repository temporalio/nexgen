# `default`

Source: JSON Schema 2020-12, Validation vocabulary, §9.2 "A Vocabulary for Basic
Meta-Data Annotations → default".

Supplies a fallback value for an absent member. In the spec it is a pure
**annotation** — it never affects validation pass/fail. We give it the
**off-the-wire, materialized-on-read** operational semantics: set-ness tracked,
omit-unset on serialize (no deep-equals), materialized **on read** via a
generated `<Field>OrDefault()` accessor in Go, a generated
`get<Field>OrDefault()` accessor in Java, a generated property in Python, and a
generated `DEFAULT_<FIELD>` constant in TypeScript.

## Spec summary

Verbatim (2020-12 validation, §9.2):

> There are no restrictions placed on the value of this keyword. When multiple
> occurrences of this keyword are applicable to a single sub-instance,
> implementations SHOULD remove duplicates.

> This keyword can be used to supply a default JSON value associated with a
> particular schema. It is RECOMMENDED that a default value be valid against the
> associated schema.

Distilled:
- An **annotation**, not an assertion: per spec it
  **never changes whether an instance validates**. A value that violates the
  schema is invalid whether or not a `default` is present.
- Supplies the value to use when the member is **absent**.
- The default *should* be valid against the schema (we strengthen this to a
  load-time **MUST** — see Support decision).
- Only meaningful on an **optional** member (a required member is never absent,
  so its default never applies).
- **v1 scope:** scalar values only (`string`/`number`/`integer`/ `boolean`);
  object/array/`null` defaults are rejected for now — a provisional limit (see
  Support decision).

## Support decision

**Support:** yes — with **off-the-wire, materialized-on-read** semantics, *not*
the literal "populate on deserialize" reading.

The defining choices (citing [[PRINCIPLES.md]]):
- **Default off-the-wire, materialized on read**: the default is **never**
  written into the field on deserialize and **never** emitted on serialize. The
  generator tracks field *set-ness*; serialize omits any unset field with
  **no value comparison** (never a deep-equals against the default). The default
  is surfaced lazily *on read*. All five languages omit an unset defaulted key,
  so all five preserve wire byte-identity (**P1**) — the wire beats ergonomics
  (**P2**), which is why no target bakes the default into the field itself.
- **P9 (absent ≠ zero / set)**: tracking set-ness (not value) is what preserves
  the absent-vs-explicitly-set distinction. Explicitly setting a field to a
  value *equal to* the default marks it set and **pins it on the wire** — a
  deep-equals strip would erase that signal, so we don't do one.
- **P12 (serialize-side)**: omit-unset lives in the encode adapter; it is the
  serialize mirror of the parse adapter's "wire-absence → use default on read."
  The shared `Validate` is unaffected — `default` contributes **no** constraint
  predicate (it is not an assertion).
- **Scalar values only, for now (P6/P7.1).** v1 supports `default` only when its
  value is a **scalar primitive** — `string`, `number`, `integer`, or `boolean`.
  An **object** or **array** default is rejected at load time, and a `null`
  default is rejected as degenerate (see Loader behavior). The blocker is purely
  the composite case: a literal object/array default would have to be
  materialized into a constructed language value (a populated
  struct/`record`/dataclass, or a typed slice/`List`) on read and woven into the
  per-field omit-unset machinery — a meaningfully harder problem than emitting a
  scalar literal in `<Field>OrDefault()` / a `DEFAULT_X` constant / a getter
  fallback. **This scope limit is provisional and expected to relax** once
  composite-value materialization is specified; it mirrors how [[const]] also
  defers composite values in v1.

Because `default` is an annotation, it has
**no runtime validation check of its own**. At load time its shape is checked
here, while each supported constraint keyword validates the literal against its
own rule:

Loader behavior:
- `default` on a **required** member → **reject**. A required member is always
  present, so the default is dead metadata; its presence signals author
  confusion (P7.1). Diagnostic: make the member optional, or drop the `default`.
  For a nullable member the [[nullability]] wrapper **is** the member's keyword
  node, so this reject — and every other rule here that inspects the member's
  own keywords — applies to it unchanged.
- `default` value **not valid against the member's own schema** → **reject**.
  The spec only *RECOMMENDS* validity; we enforce it (P7.1): a default that can
  never satisfy the field (`{type:"integer", minimum:5, default:0}`,
  `{type:"string", default:42}`) is a schema bug, not a runtime concern.
  Diagnostic names the violated constraint. Constraint keywords including
  `pattern`, `minLength`/`maxLength`, numeric bounds, `multipleOf`, `format`,
  `contentEncoding`, and `enum` validate `default` literals in their own loader
  validators. This is the same cross-cutting obligation as [[const]] and
  [[enum]]. Kind compatibility is **directional**: an integral literal satisfies
  a `number` member, and a **fractional** literal never satisfies an `integer`
  one — P1 names `1.5` on an `integer` as a value every target must refuse, so
  `{type:"integer", default:1.5}` is a load reject and not a value the emitters
  are free to round.
- `default` **and** [[const]] both present → reject (see [[const]]): `const`
  already fixes the value; opposite serialize behavior (always-emit vs
  omit-unset).
- `default` whose value is an **object or array** → **reject** (for now).
  Diagnostic reads "object/array defaults are not yet supported," not
  "forbidden" — the limit is provisional (see Support decision). The member's
  *type* may still be an object/array; it just cannot carry a `default`.
- `default` on a **sum type** — a [[oneOf]] with two or more non-`null` branches
  → **reject**: it names no branch, so nothing defines which branch supplies it,
  and no target can lower it. Diagnostic: move the `default` onto the branch
  that should supply it, or drop it. The reject is a property of the
  **resolved** union, so it fires identically whether the union is written
  inline or reached through a `$ref` ([[ref]]).
- On a **nullable** member the `default` is authored on the [[nullability]]
  **wrapper** and never on a branch; a branch `default` is a **reject**
  ([[nullability]] owns the placement rule, with constraints on the non-null
  branch and each node rejecting the other's keyword). The wrapper's `default`
  is validated against the resolved non-null branch — its `type`, `enum`,
  `format` and `contentEncoding` — by pushing the literal down onto that branch
  and re-running the branch's own checks.
- `default` in a **non-member** position — an [[items]] element schema, an
  [[additionalProperties]] value schema, a [[contains]] matcher, a
  [[propertyNames]] key schema → **reject**. The whole feature is member-scoped:
  nothing outside a [[properties]] member has a set-ness channel, an accessor or
  a constant to materialize into. A `default` there is dead metadata signalling
  author confusion — the same P7.1 judgement as `default` on a required member,
  and not the inert-annotation exception, since an author who writes a `default`
  is asking for a value to be materialized. Diagnostic names the position and
  says the value is never materialized.
- `default: null` → **reject** as degenerate: on a non-nullable member it is
  invalid against the schema (caught by the validity check above); on a nullable
  member it is a no-op, since absence already surfaces as
  `nil`/`None`/`null`/`undefined` and `<Field>OrDefault()` returning `nil` adds
  nothing. Mirrors [[const]]'s `const: null` rejection.
- Multiple `default` occurrences applicable to one sub-instance (via merged
  schemas) → **last-wins**: identical values dedup per spec; when they differ,
  the value from the **last-merged** schema survives — a later [[allOf]] branch,
  or a `$ref` use-site sibling overriding the target (see [[allOf]]/[[ref]]). A
  differing default is a deterministic override, not a conflict; nothing is
  rejected.

## Type mapping

**None of its own.** `default` does not change the emitted type — the type comes
from [[type]] + [[nullability]], and `default` implies the member is
**optional**, so it takes the optional form (`*T` / `x?: T` / `T | None` /
boxed-or-`@Nullable`). The default value never appears in the field itself in
any target. What `default` *does* add is the **read-side surfacing mechanism**
and the generated default value itself, which differ per language.

The surfacing mechanism's own type is the member's **emitted element type** —
the one [[type]] + [[nullability]] already fixed, which for a nullable member is
the `type` declared on the **non-null branch** — and is **never** inferred from
the spelling of the default literal. So an integral literal on a `number` member
surfaces as that member's floating-point type, not an integer one; the literal
decides only its own rendering, never the accessor's, constant's or property's
signature.

| Language   | Set-ness signal (omit-unset)                      | Read-side surfacing of the default                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ---------- | ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Python     | private `_<field>: T | None`                      | **native property** — `@property def field(self) -> T` returns the private value when set and the scalar default otherwise. Its setter accepts `T`; `del model.field` invokes a property deleter that restores unset. Models with defaults receive a generated keyword-only constructor so `Model(field=...)` remains the public construction API.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Java       | `null` field, omitted by the generated serializer | **generated accessor** — the plain getter preserves the nullable set-ness signal; a separate `get<Field>OrDefault()` returns the field when set and the scalar default otherwise.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| C#         | `null` member (`T?`), omitted by `WriteValue`     | **generated accessor** — the property keeps the set-ness signal; a computed `<Member>OrDefault` returns the member when set and the scalar default otherwise.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| TypeScript | `undefined` (the `?` field)                       | **advisory** — interfaces have no methods (PRINCIPLES TS §2), so the generator emits `export const DEFAULT_X = "anon"`. Consumers use `value.x === undefined ? DEFAULT_X : value.x` when `null` must remain distinct; `??` is sufficient only for non-nullable fields.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Go         | `*T` `nil` (the field also carries `,omitempty`)  | **generated accessor** — a `func (m M) <Field>OrDefault() T` returns `*m.Field` when set and the default literal when `nil` (`func (u User) NicknameOrDefault() string { if u.Nickname != nil { return *u.Nickname }; return "anon" }`). The bare field stays `*T` (set-ness intact); the accessor is the materialize-on-read path. Emitted **only** for default-bearing fields. Modeled on proto3's `GetX()` — the same omit-default-on-wire + accessor-materializes-default pattern already familiar to Temporal users. Named `<Field>OrDefault` rather than `Get<Field>` to read as "the value, or its default" and to avoid implying a getter on every field. Alternative approaches considered: (a) advisory constant (`DEFAULT_X` + caller nil-checks) — pushes nil-checks to every call site; (b) populate on deserialize — destroys set-ness, forces deep-equals, breaks P9. |

### Naming and collisions (P15)

The read-side surfacing synthesizes **one new identifier in five targets** —
names absent from the schema, so they can collide:

| Target     | Synthesized identifier         | Scope                                                                                                  | Collision risk                                                                                                                                                                                                                                                                                                                                                         |
| ---------- | ------------------------------ | ------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | `<Field>OrDefault()` method    | struct method-set                                                                                      | a **declared** member whose name maps to `<Field>OrDefault` (Go forbids a field and method of the same name — a **hard compile error**); another `<Field>OrDefault` from a sibling field                                                                                                                                                                               |
| TypeScript | `DEFAULT_<FIELD>` const        | the **whole run** (P15 — the root barrel re-exports every module's top-level names into one namespace) | another `DEFAULT_<FIELD>` from a field that case-maps the same, **in any input file**, not only in the same one. [[const]] synthesizes no named *type* in TS (the type closes to an inline literal) but does emit a module-scope `<FIELD>_CONST` binding holding the wire value, which shares this scope — unexported, yet still a redeclaration error if it coincides |
| Python     | `_<field>` backing slot        | class/member                                                                                           | a declared member overridden to that private identifier; another backing slot after member mapping                                                                                                                                                                                                                                                                     |
| Java       | `get<Field>OrDefault()` method | POJO method namespace                                                                                  | the plain getter of a sibling member named `<field>OrDefault`; another default accessor after member mapping                                                                                                                                                                                                                                                           |
| C#         | `<Member>OrDefault` property   | the model's member scope                                                                               | a sibling member named `<member>OrDefault`; any other name the model synthesizes                                                                                                                                                                                                                                                                                       |

The TypeScript constant is always named **`DEFAULT_<FIELD>`**. Its spelling is a
function of the declaring member alone: adding an unrelated model with the same
default-bearing member must not rename an already-published constant to
`DEFAULT_<MODEL>_<FIELD>` (**P13**). A second claimant therefore
**rejects at load** under **P15**; `x-ts-name` on either declaring member moves
that member's constant and is the escape hatch. Python emits no `DEFAULT_*`
binding: its private backing slot is exactly the emitted member identifier
prefixed with `_`.

Per **P15** these participate in the single per-scope collision pass and
**reject at load** on any coincidence — never auto-mangled (a
`NicknameOrDefault2` would renumber under schema evolution, a P13 break). The
rename **escape hatch** is the [[properties]] case-mapping override
(`x-go-name`, …) on the *declaring* field — re-mapping it moves the synthesized
Go `<Field>OrDefault`, Java `get<Field>OrDefault`, `DEFAULT_<FIELD>`, and
`_<field>` names with it, because all are named off the **emitted** member
identifier rather than the JSON key (`retryCount` + `x-ts-name: attempts` →
`DEFAULT_ATTEMPTS`). The derivation has to work that way for the hatch to open
at all: two members that recase alike collide on `DEFAULT_<FIELD>`, and an
override that moved the members apart while leaving both constants on the
JSON-derived name would reject with a fix-it the author cannot act on — the only
remaining escape being a rename of the JSON property, i.e. a change to the wire
contract (P15, P7.1).

Java materializes-on-read through `get<Field>OrDefault()`; Go does so via the
generated `<Field>OrDefault()` accessor, and Python through a generated
property. TypeScript interfaces have no methods, so TypeScript alone leans on a
generated constant the consumer applies. In every language the
**bare field still carries set-ness** (`nil` / `undefined` / `None` / `null`);
the default is layered on read, never written back into the field, so
omit-on-serialize stays faithful.

## Validator mapping

`default` emits **no validator** — it is an annotation (§9.2), so it never
appears in the shared `Validate` and never causes a runtime pass/fail. Its
operational behavior is entirely in the **adapters**:

- **Parse adapter (deserialize-only):** when the member is absent on the wire,
  leave the set-ness signal "unset" (nil / `undefined` / `None` / `null`). Do
  **not** write the default into the field. Required-presence and constraint
  checks are unaffected (a client sending fewer keys is judged on the wire,
  before any default — this is why [[minProperties]]/[[maxProperties]] count
  *before* default population).
- **Encode adapter (serialize-only), P12:** omit any unset member —
  declaratively, via the per-language set-ness signal above — with
  **no deep-equals**. An explicitly-set member (even to the default value)
  emits.

### Serialize-side (P12)

The whole point of `default` lives here. The encode adapter omits unset members
so the wire stays minimal and the round-trip is faithful: a value that arrived
absent leaves absent, never echoed back as a materialized default. Mechanisms:

| Language   | Omit-unset mechanism                                                                                                                                                                            |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | the generated `MarshalJSON` builds its raw output map field by field, omitting a nil pointer and emitting a non-nil pointer even when it points to the zero value.                              |
| TypeScript | `toTransferType` skips keys whose value is `undefined` when building the transfer value (PRINCIPLES TS §4).                                                                                     |
| Python     | `to_transfer_type` uses the private backing slot as the emitted value and skips the key when that slot is `None`; constraint checks may read the public property under the same presence guard. |
| Java       | the class-level serializer writes the field only when its raw backing value is non-null; the plain getter preserves that raw state and `get<Field>OrDefault()` supplies the read default.       |
| C#         | `WriteValue` writes the member only when it is non-null; `<Member>OrDefault` supplies the read default.                                                                                         |

Three consequences that the count specs already encode:
- A default-filled key is **never on the wire**, so it does not count toward
  [[minProperties]]/[[maxProperties]] in either direction.
- A model that *reads* as fully populated in memory (defaults visible via the
  read-side mechanism) can legitimately serialize **fewer** keys than it appears
  to hold — by design.
- Explicitly setting a member to a value equal to its default
  **keeps it on the wire** (P9). This is the deliberate non-deep-equals
  behavior.

## Property-testing matrix

### Accepted (positive)

| Shape                                   | Example                                                                                                                                                                                                                                                                                                                                                     |
| --------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Optional string default                 | `{type:"string", default:"anon"}` (member optional)                                                                                                                                                                                                                                                                                                         |
| Optional integer default                | `{type:"integer", default:0}`                                                                                                                                                                                                                                                                                                                               |
| Optional boolean default                | `{type:"boolean", default:false}`                                                                                                                                                                                                                                                                                                                           |
| Optional + nullable with scalar default | `{oneOf:[{type:"string"},{type:"null"}], default:"x"}`                                                                                                                                                                                                                                                                                                      |
| Differing merged defaults (last-wins)   | `allOf:[{default:"a"},{default:"b"}]` → `"b"` (see [[allOf]]). The `$ref`-sibling half — `{$ref:"#/$defs/X", default:"local"}` overriding X's own `default` — is **unreachable today**: a `default` belongs to a scalar, a scalar `$defs` entry is a load reject ([[const]] *Naming and collisions*), and an object/array `default` is rejected on its own. |

### Rejected at load time (negative)

| Reason                                    | Example                                                                                                                                                                                                                                                                                                        |
| ----------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `default` on a required member            | `required:["x"]` with `x:{type:"string", default:"a"}`                                                                                                                                                                                                                                                         |
| Default `type`-mismatch (P7.1)            | `{type:"string", default:42}`                                                                                                                                                                                                                                                                                  |
| Fractional literal on an `integer` member | `{type:"integer", default:1.5}` (directional: `{type:"number", default:1}` is fine)                                                                                                                                                                                                                            |
| Default fails a *constraint*              | `{type:"integer", minimum:5, default:0}`                                                                                                                                                                                                                                                                       |
| **Object default (deferred)**             | `{type:"object", properties:{…}, default:{a:1}}`                                                                                                                                                                                                                                                               |
| **Array default (deferred)**              | `{type:"array", items:{type:"string"}, default:["a"]}`                                                                                                                                                                                                                                                         |
| `default: null` (degenerate)              | `{oneOf:[{type:"string"},{type:"null"}], default:null}`                                                                                                                                                                                                                                                        |
| `default` on a sum type (names no branch) | `{oneOf:[{type:"string"},{type:"integer"}], default:"x"}`, and the same union reached as `{$ref:"#/$defs/U", default:"x"}`                                                                                                                                                                                     |
| `default` on a nullability **branch**     | `{oneOf:[{type:"string", default:"x"},{type:"null"}]}` — put it on the wrapper                                                                                                                                                                                                                                 |
| `default` outside a `properties` member   | `{type:"array", items:{type:"string", default:"x"}}`; likewise under `additionalProperties`, `contains`, `propertyNames`                                                                                                                                                                                       |
| With `const`                              | `{type:"string", const:"v1", default:"v1"}`                                                                                                                                                                                                                                                                    |
| Synthesized-name collision (P15)          | a field `nickname` with a `default` **and** a sibling member mapping to `NicknameOrDefault` (Go field/method clash); two default-bearing members whose emitted identifiers case-map to the same stable `DEFAULT_<FIELD>` name (TS); a Python sibling explicitly renamed to `_nickname` (private-backing clash) |

### Runtime fixtures (validator / adapters)

- Member **absent** on the wire → field unset; re-serialize **omits** it (no
  echo). Read surfaces the default per the language mechanism.
- Member **present** with a non-default value → preserved, emitted.
- Member **explicitly set to the default value** → marked set, **emitted** (no
  deep-equals strip). Round-trips as present.
- A default-filled member does **not** count toward
  [[minProperties]]/[[maxProperties]] (cross-checked in those specs).
- `default` never causes a validation failure (annotation): an invalid *wire*
  value still fails its own constraint, default or not.

## Interactions

- **[[required]]**: mutually exclusive in practice — `default` on a required
  member is a load reject (the default can never apply).
- **[[const]]**: mutually exclusive (load reject). Opposite serialize behavior:
  `const` always-emits an auto-populated value; `default` omit-unsets. See
  [[const]].
- **[[oneOf]]**: a `default` is admitted on the [[nullability]] wrapper only. On
  a **sum type** (two or more non-`null` branches) it is a load reject, through
  the inline spelling and through a `$ref` to a named union alike, because the
  resolved shape is what decides.
- **[[nullability]]**: composable, with the `default` on the **wrapper** (a
  branch `default` rejects — see Loader behavior). For an optional+nullable
  member with a default, **absence** materializes the default on read while an
  **explicit `null`** pins `null` (faithful in TS via an explicit
  `=== undefined` read; `??` would collapse null; Go, Java and Python collapse
  absent-vs-`null` — see [[nullability]] round-trip tiers). The default applies
  to *absence*, never overriding an explicit `null`.
- **[[minProperties]] / [[maxProperties]]**: a default-filled key is never on
  the wire, so the count (taken before default population on the way in, over
  to-be-emitted keys on the way out) excludes it. Already documented in both
  specs.
- **[[type]]**: the default value must be valid for the declared type (enforced
  at load, P7.1). In v1 the default value must additionally be a **scalar**
  (`string`/`number`/`integer`/`boolean`) — a member typed `object`/`array` may
  not carry a `default` yet (see Support decision).
- **[[properties]]**: `default` lives on a member subschema; the per-member
  set-ness machinery is what [[properties]] emits.

## Ecosystem variance

| Source dialect        | Action                                                |
| --------------------- | ----------------------------------------------------- |
| JSON Schema 2020-12   | Native annotation (§9.2).                             |
| OpenAPI 3.1           | Adopts 2020-12 — `default` native, same semantics.    |
| OpenAPI 3.0           | `default` present; same annotation semantics. Native. |
| Swagger 2.0 / draft-4 | `default` present (draft-4+); native.                 |

`default` is universal across dialects; no rewrite is ever needed. The only
divergence is *our* strengthening of "RECOMMENDED valid" to a load-time MUST
(P7.1), which is stricter than every source dialect — a schema that ships an
out-of-range default is accepted upstream but rejected here, with a fix-it
diagnostic. The owning constraint validators enforce that MUST; see Loader
behavior.

## See also

- [[const]] — the opposite serialize behavior (always-emit); mutually exclusive
  with `default`.
- [[required]] — mutually exclusive with `default` (a required member is never
  absent).
- [[nullability]] — composes; the default sits on the wrapper, applies to
  absence, and never overrides explicit `null`.
- [[oneOf]] — a `default` on a sum type names no branch and rejects.
- [[minProperties]] / [[maxProperties]] — default-filled keys never count.
- [[type]] — the default value must be valid for the declared type.
- [[properties]] — hosts the member subschema and the set-ness machinery, and
  owns the case-mapping + collision/escape-hatch policy that governs the
  synthesized `<Field>OrDefault` / `get<Field>OrDefault` / `DEFAULT_<FIELD>` /
  `_<field>` names (P15).
