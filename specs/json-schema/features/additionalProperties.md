# `additionalProperties`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.3.2.3 "Keywords
for Applying Subschemas to Objects → additionalProperties".

Controls instance members **not** matched by [[properties]] or
[[patternProperties]]. This is where the generator's **open-vs-closed struct**
decision lands.

## Spec summary

Verbatim (2020-12 core, Applicator):

> The value of "additionalProperties" MUST be a valid JSON Schema.

> The behavior of this keyword depends on the presence and annotation results of
> "properties" and "patternProperties" within the same schema object. Validation
> with "additionalProperties" applies only to the child values of instance names
> that do not appear in the annotation results of either "properties" or
> "patternProperties".

> For all such properties, validation succeeds if the child instance validates
> against the "additionalProperties" schema.

> The annotation result of this keyword is the set of instance property names
> validated by this keyword's subschema. This annotation affects the behavior of
> "unevaluatedProperties" in the Unevaluated vocabulary.

> Omitting this keyword has the same assertion behavior as an empty schema.

Distilled:
- Applies only to members **not** matched by [[properties]] /
  [[patternProperties]].
- The value is a full subschema — `false` (nothing additional allowed),
  `true`/`{}` (anything allowed), or a typed schema (additional members must
  match it).
- **Omitting it = empty schema = everything additional allowed** → the spec
  default is **open**.

## Support decision

**Support:** partial.

The binding decision:

> **Typed structs are OPEN by default.** Per the JSON Schema spec default
> (omitted `additionalProperties` = empty schema = allow anything) and **P13**
> (forward compatibility: accept and preserve unknown fields), a
> `{type:object, properties:{...}}` with no `additionalProperties` emits an
> **untyped catch-all** that preserves and round-trips unmatched members. Closed
> behavior requires an explicit `additionalProperties: false`.

Accepted forms and their meaning:

| Form                                 | With `properties` (struct)                          | Without `properties` (map)                             |
| ------------------------------------ | --------------------------------------------------- | ------------------------------------------------------ |
| **omitted**                          | open struct + untyped catch-all (the default)       | **rejected** by [[type]] (`type:object` with no shape) |
| **`false`**                          | closed struct (extras rejected)                     | closed empty object (any member rejected)              |
| **`true`**                           | open struct + untyped catch-all                     | open opaque map                                        |
| **`{type:T}`** (supported subschema) | **supported** — typed catch-all (`T`-valued extras) | typed map (`T`-valued)                                 |
| **`{}`** (empty-schema spelling)     | **rejected** per **P7**                             | **rejected** per **P7**                                |

Rationale (citing [[PRINCIPLES.md]]):
- **P13 (forward compat)**: open-by-default preserves unknown members so a
  producer adding a field never breaks an older consumer. Verified the
  preserve+round-trip behavior per language (see Validator mapping).
- **P7 (strict schema)**: `additionalProperties: {}` is the empty-schema
  spelling of `true` — ambiguous, so **rejected**; diagnostic says "use `true`
  for an open object." (Matches PRINCIPLES P7's `additionalProperties: {}` →
  reject.)
- **Typed `additionalProperties` is supported in every position**, including
  *alongside* `properties`. The key that makes it lower coherently is the
  **named-field representation** below: the typed catch-all is a dedicated
  `additionalProperties` member, never an inline index signature, so the TS
  index-signature conformance problem (`[k:string]: T` forcing every declared
  property to be a subtype of `T`) never arises.
- The extras schema must be a **supported subschema** (`{type:T}` with a
  recognized shape). `additionalProperties:{type:object}` with no shape is
  rejected per **P7.1**, same as anywhere else.
- The extras schema may be a [[oneOf]] **sum type**. Like an array element it is
  named after its position at load — `<EnclosingType>Value` — moved into
  `$defs`, and the extras schema rewritten to a `$ref`, so the member type is an
  ordinary named union in every target ([[oneOf]] §"Unions in element
  positions"). Go and Java decode each member through the union's dispatcher,
  keyed violations included; a whole-map decode cannot allocate a sealed
  interface.

### Catch-all representation: always a named field

The catch-all is emitted as a **dedicated named member in all four languages** —
never an inline open map — *even with no `properties`*. A pure map
(`{type:object, additionalProperties:{type:T}}`) emits the same named aggregate
it would carry with properties, holding the map in the catch-all member:

- **Go** — struct with `AdditionalProperties map[string]T`, **not** a bare
  `map[string]T`.
- **Java** — class with `Map<String,T> additionalProperties`, **not** a
  top-level `Map<String,T>`.
- **Python** — a dataclass with an
  `additional_properties: dict[str, V] = dataclasses.field(default_factory=dict)`
  member, **not** a `dict[str,T]` alias.
- **TypeScript** — an `interface` with an
  `additionalProperties: Record<string,T>` member, **not** an inline index
  signature or a bare `Record<string,T>` alias.

This buys two things:

1. **Shape stability** (**P2**/**P13**): adding `properties` later only
   *adds fields/attributes* to the same type — it never changes kind ("map
   alias" → "struct/model"), so downstream call sites keep compiling. The Python
   instability this avoids: a `dict[str,T]` alias that becomes a dataclass
   breaks `m["k"]` with `TypeError: not subscriptable`.
2. **A clean separation of declared keys from extra keys.** Declared members are
   renamed to canonical language identifiers (the identifier case-mapping in
   [[properties]]); extra keys are arbitrary and must be preserved **verbatim**.
   Keeping extras in their own `additionalProperties` member (rather than
   mingled with declared members via a TS index signature or a flat map) keeps
   the two namespaces unambiguous — the canonicalizer touches declared members
   only, extras pass through untouched.

Shape stability is also why an object written **inline** in a value position — a
property, an array element, a map member — is
*named after that position and hoisted into `$defs`* rather than lowered to a
bare map at the use site (see [[properties]] §"Naming an inline object shape").
It is the same aggregate either way, so
`{type:object, additionalProperties:true}` written on a property and a `$ref` at
an authored definition of that shape emit identical code, and adding
`properties` to the inline form later still only adds fields. The one position
where a free-form object stays inline is a [[oneOf]] branch, where it is the
union's object *kind* rather than a position of its own.

For TS specifically, the named member also sidesteps the index-signature
conformance limit: a *typed* index signature (`[k: string]: T`) is illegal
alongside heterogeneous declared props (TS2411 — a declared `id: number` is not
assignable to a `string` index type, verified `/tmp/ts_flatten.ts`), whereas a
named `additionalProperties: Record<string,T>` member has no such constraint and
stays fully typed.

The wire form is unchanged in all cases — extras are always top-level JSON
members; the in-memory catch-all is bridged by the generated (de)serializer (Go
custom `(Un)MarshalJSON`, Java the per-POJO collecting
`@JsonDeserialize`/`@JsonSerialize` — which routes undeclared tree keys into the
catch-all map and spreads them back on write, **not**
`@JsonAnySetter`/`@JsonAnyGetter` (a class-level custom (de)serializer bypasses
those), TS hand-emitted ser/deser that lifts top-level extras into
`additionalProperties` and spreads them back out, Python the generated transfer
type converter doing the same for `additional_properties`).

## Type mapping

All four languages emit the named catch-all member when extras are allowed (see
representation note above). The catch-all element type is `T` for `{type:T}`,
else the raw type (`json.RawMessage` / `unknown` / `Any` / `JsonNode`) so
untyped extras survive a round-trip without the generator guessing their shape
(**P13**).

| Case                                           | Go                                                                   | TypeScript                                                              | Python                                                                                                                          | Java                                                                                                               |
| ---------------------------------------------- | -------------------------------------------------------------------- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| Open struct, untyped extras (default / `true`) | struct + `AdditionalProperties map[string]json.RawMessage`           | `interface` + `additionalProperties: Record<string, unknown>`           | dataclass + `additional_properties: dict[str, typing.Any]`, populated/emitted by the converter (Python §3)                      | POJO + `Map<String,JsonNode>`, populated/emitted by the collecting (de)serializer (Java §5)                        |
| **Typed extras + `properties` (`{type:T}`)**   | struct + `AdditionalProperties map[string]T`                         | `interface` + `additionalProperties: Record<string, T>`                 | dataclass + `additional_properties: dict[str, T]`, populated/emitted by the converter (Python §3) with per-extra `T` validation | POJO + `Map<String,T>`, populated/emitted by the collecting (de)serializer (Java §5) with per-extra `T` validation |
| Closed struct (`false`)                        | no catch-all field; unknown → error                                  | exact `interface`, no `additionalProperties`; unknown → error           | no catch-all member; the converter flags each undeclared key as a `Violation`                                                   | no catch-all field; the collecting deserializer (Java §5) flags each undeclared tree key as a `Violation`          |
| Open opaque map (`true`, no props)             | struct + `AdditionalProperties map[string]json.RawMessage` (wrapper) | `interface` + `additionalProperties: Record<string, unknown>` (wrapper) | dataclass + `additional_properties: dict[str, typing.Any]` (wrapper)                                                            | class + `Map<String,JsonNode> additionalProperties` (wrapper)                                                      |
| Typed map (`{type:T}`, no props)               | struct + `AdditionalProperties map[string]T` (wrapper)               | `interface` + `additionalProperties: Record<string, T>` (wrapper)       | dataclass + `additional_properties: dict[str, T]` (wrapper) + per-extra `T` validation                                          | class + `Map<String,T> additionalProperties` (wrapper)                                                             |
| Closed empty object (`false`, no props)        | empty `struct{}`; any member → error                                 | empty `interface`; any member → error                                   | empty dataclass; the converter flags any key as a `Violation`                                                                   | empty POJO; the collecting deserializer (Java §5) flags any tree key as a `Violation`                              |

The TS `additionalProperties` member and the Python `additional_properties`
field are always present when extras are allowed (empty when none were received
— Python via `dataclasses.field(default_factory=dict)`), so the surface is
uniform whether or not a given instance carried extras.

A declared [[properties]] member literally named `additionalProperties` collides
with the generated catch-all member in **all four languages** — **Go**
(`AdditionalProperties`), **Java** (`additionalProperties`), **TS**
(`additionalProperties`), **Python** (`additional_properties`) → reject at load
time with a diagnostic.

### Why `json.RawMessage`, not `any`, for Go untyped extras

The Go untyped element type is `json.RawMessage` (raw JSON), **not** `any`
(`map[string]interface{}`). This is load-bearing for **P13** — `any` corrupts
the very data the catch-all exists to preserve, because `encoding/json` decodes
**every JSON number to `float64`**. Empirically, the same opaque object
round-trips as:

```
any : {"big":9007199254740992,"keep":{"a":1,"b":2},"price":1,"sci":100}
raw : {"big":9007199254740993,"keep":{"b":2,"a":1},"price":1.0,"sci":1e2}
```

With `any`:
- **silent precision loss** — `9007199254740993` → `…992` (the same `>2^53`
  float64 hazard as the [[type]] integer cap, but on data we never modeled, so
  we can't even detect it);
- number reformatting and object-member reordering are observable text changes
  but preserve JSON identity under **P1**.

`json.RawMessage` preserves the decoded JSON value, including number precision
and the authored number lexeme. `encoding/json` may still compact whitespace,
HTML-escape characters, and reorder the outer map. It also makes on-demand typed
decode clean (`json.Unmarshal(extra["foo"], &T)`) and reuses the same
`*json.RawMessage` shadow machinery the custom `UnmarshalJSON` already uses for
declared fields — `any` would introduce a second, lossy representation. The only
cost is that a value must be `Unmarshal`'d before use, which is acceptable for a
preserve-and-pass- through role.

Python `Any` and Java `JsonNode` don't share the `float64` hazard: the `json`
module decodes an integer literal to an arbitrary-precision `int`, and the
generated exact-tree reader hands the untyped member a `JsonNode` holding the
exact token — Jackson's own tree builder folds every floating token into a
`double`, which is why that reader exists — so both re-emit `9007199254740993`
unchanged and neither needs an equivalent workaround.

### TypeScript untyped extras: a bounded exception to P13.2

TS `unknown` **does** share the hazard, and — unlike Go — cannot be worked
around. **PRINCIPLES TypeScript §4** places the byte boundary *outside* the
converter: the Temporal converter owns `JSON.parse`/`JSON.stringify` and hands
`fromTransferType` an already-parsed value. By the time any generated code runs,
`9007199254740993` is already the `number` `9007199254740992` — there is no
surviving representation to preserve and no interception point. The only fixes
would be for the converter to own the parse step (a `JSON.parse` reviver, or
parsing from the raw text itself), which moves the boundary §4 defines and
breaks the composable transfer-value contract that lets a parent's
`toTransferType` embed its children's results.

So
**P13.2's "preserved verbatim" holds for TypeScript with one stated exception**:
an undeclared numeric value outside IEEE-754 double range round-trips to the
nearest double. Object keys, strings, booleans, `null`, nested structure and
every number representable as a double are preserved. Object-member **order** is
neither guaranteed nor part of JSON identity under **P1**. An author whose
payload carries integers past 2^53 must not rely on the catch-all to ferry them;
note that *declaring* the field does not rescue the value either — the
cross-language integer cap is `±(2^53−1)` and a literal past it is a validation
reject, not a silent round ([[type]]). Carrying such a value across a TypeScript
target requires modeling it as a `string`.

## Validator mapping

Per **P10**/**P11**: extras are handled at the boundary and any closed-mode
violation aggregates. Every violation below sits at the offending extra's path,
**rendered per P11.2** — an extra key is arbitrary text, so the spelling belongs
to that clause and is not restated per target here.

| Language   | Open, untyped (preserve)                                                                                                                                                      | Open, typed `{type:T}`                                                                                                                        | Closed (reject extras)                                                                                                                                                                                                              |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | `UnmarshalJSON` routes unmatched keys into `AdditionalProperties`; `MarshalJSON` re-emits them                                                                                | same routing, but each value goes through `T`'s runtime helper; failures → a `Violation` at the extra's path                                  | `UnmarshalJSON` emits a `Violation` at the extra's path with `Reason:"unknown field"` per unmatched key, collected into one `PayloadValidationError` application failure                                                            |
| TypeScript | deser lifts non-declared keys into the `additionalProperties` Record; reser spreads them back to top-level                                                                    | same, but each value validated as `T` before going into `additionalProperties` (member stays fully typed `Record<string,T>`)                  | check parsed keys against the known set; push one `Violation` per extra, throw one `PayloadValidationError` application failure                                                                                                     |
| Python     | `from_transfer_type` lifts non-declared keys into the `additional_properties` dict verbatim; `to_transfer_type` spreads them back to top-level                                | same, but each value is validated and materialized as `T` before going into `additional_properties` (member stays fully typed `dict[str, T]`) | check parsed keys against the declared set; append one `Violation` with `reason="unknown field"` per extra, raise one `PayloadValidationError` application failure                                                                  |
| Java       | the per-POJO collecting deserializer (Java §5) routes parsed-tree keys not in the declared set into the `additionalProperties` map; the matching serializer spreads them back | same routing, but each extra value is validated as `T` (bad keys → a `Violation` at the extra's path)                                         | the collecting deserializer pushes one `Violation` with `"unknown field"` per undeclared tree key into the single `PayloadValidationError` application failure — no fail-fast `ignoreUnknown=false`/`UnrecognizedPropertyException` |

### Per-member `T` validation

"Validated as `T`" is literal and symmetric: a member is held to **everything**
its declared type declares — the spec-strict integer parse and the integer cap,
numeric bounds and `multipleOf`, string length / `pattern` / `format`, a
materialized temporal or `contentEncoding` construct, array `minItems` /
`maxItems` / `uniqueItems` / `contains`, a `const`/`enum` value set, and a
referenced model's or union's own validation — with the
**member's key as the violation path** (`labels.env`, `entries.a.street`), and
in **both directions** (**P12**): a catch-all mutated to an invalid value fails
serialization rather than reaching the wire.

Per language, the mechanism is the one that position already uses:

- **Go / TypeScript / Java** run the same check emitters a *property* of that
  type runs, over the decoded member inside the member loop — one set of
  predicates, two call sites.
- **Python** validates and **materializes** each member inside the converter's
  member loop, calling the same `_parse_*` / `_check_*` helpers (or the
  referenced model's own converter) a *property* of that type calls, then
  re-encoding each member through the matching `_format_*` / serialize path on
  the way out. So `additional_properties` holds the *declared* member type — an
  `Inner` instance, an `int` parsed from `1.0`, a `datetime`, `bytes` — rather
  than the raw wire value.
- A **closed member value set** (`const`/`enum`) is a validator-only closedness
  in Go and Java: a member has no field to hang a defined type or value class
  off, so the admissible set is checked against wire literals instead. The
  accepted value set is identical in all four languages (TypeScript and Python
  additionally close the *type*, as `Record<string, "a" | "b">` /
  `Literal["a","b"]`).

A member may be **nullable** — `additionalProperties` is the [[nullability]]
`oneOf` — in which case an explicit `null` is *kept as a null member* rather
than dropped from the map or rejected: Go `map[string]*T`, Java
`Map<String, @Nullable T>`, TypeScript `Record<string, T | null>`, Python
`T | None`. A present member still carries its own constraints.

Wrapping the extras schema in the [[nullability]] wrapper does not change what
the schema requires of the emitter: every package-level compiled-regex static
the member's own assertions need is still emitted, and a nullable array is still
`[]T` — including as a typed `additionalProperties` value — so the element
traversal ranges over the value, never over `*value`.

### Serialize-side (P12)

The catch-all is re-emitted by spreading its members back to top-level JSON (Go
`MarshalJSON` / TS reserializer / Java the per-POJO collecting serializer, Java
§5 / Python `to_transfer_type`). Symmetry per mode:

- **Open, untyped** — extras pass through **verbatim**; Go's `json.RawMessage`
  element type carries the decoded JSON value out again with its number
  precision and lexeme intact (no `float64` degradation — the same hazard as on
  decode, see "Why `json.RawMessage`, not `any`" above).
- **Open, typed `{type:T}`** — each extra value is re-validated through `T`'s
  shared checks before emit, so a catch-all mutated to an invalid value fails
  serialization rather than emitting bad data.
- **Closed (`false`)** — no catch-all exists in memory, so there is nothing
  extra to emit; the closed shape is preserved by construction.

Declared members serialize under their original wire names ([[properties]]
case-mapping is reversed on the way out); extras keep their verbatim keys — the
named-catch-all split keeps the two namespaces unambiguous in both directions.

**The lift and the spread are key-preserving.** Every key the wire object owns
and `properties` does not match appears in the catch-all, and every catch-all
key appears back on the wire — an extra key is arbitrary data, not an
identifier. **P13.2(b)**'s verbatim rule carries exactly one exception, the
numeric one stated above for TypeScript; that exception is about a member's
*value* and reaches no key, so no key may be dropped or renamed under it. In
TypeScript the accumulator is therefore written and read by a mechanism the
prototype chain cannot intercept: a key named `__proto__` is an own member of
the parsed object and must stay one through the copy, rather than reassigning
the accumulator's prototype and vanishing. Dropping a key is not only a fidelity
loss — the key set is what [[minProperties]] / [[maxProperties]] count at each
boundary, so a non-key-preserving copy also makes the two counts disagree, and
validation semantics are never excepted (**P1**).

**Violation paths for extra keys.** An extra key is the one path segment this
subset lets a *payload* choose, and it may contain the characters the path
grammar itself uses. It is therefore rendered per **P11.2** rather than spliced
in raw; the grammar, its escaping and the current cross-target gap all belong to
that clause.

## Property-testing matrix

### Accepted (positive)

| Shape                                                                    | Example                                                                                               |
| ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------- |
| Open struct (default)                                                    | `{type:object, properties:{id:{type:integer}}}`                                                       |
| Closed struct                                                            | `{type:object, properties:{id:{type:integer}}, additionalProperties:false}`                           |
| Open struct (explicit)                                                   | `…, additionalProperties:true`                                                                        |
| Open opaque map                                                          | `{type:object, additionalProperties:true}`                                                            |
| Typed map                                                                | `{type:object, additionalProperties:{type:string}}`                                                   |
| **Typed extras + `properties`**                                          | `{type:object, properties:{id:{type:integer}}, additionalProperties:{type:string}}`                   |
| Closed empty object                                                      | `{type:object, additionalProperties:false}`                                                           |
| Any of the above written **inline** on a property / element / map member | named after its position and hoisted ([[properties]]); emits identically to the `$defs` + `$ref` form |

### Rejected at load time (negative)

| Reason                             | Example                                                                                                                                      |
| ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Empty-schema spelling (P7)         | `additionalProperties: {}` (use `true`)                                                                                                      |
| Non-schema value                   | `additionalProperties: "yes"`, `…: 1`                                                                                                        |
| Out-of-subset extras schema (P7.1) | `additionalProperties:{type:object}` with no shape                                                                                           |
| Catch-all name collision           | `{properties:{additionalProperties:{type:string}}, additionalProperties:{type:integer}}` (declared member collides with the generated field) |

### Runtime fixtures (validator)

- Open struct + extra key → preserved, present on re-serialize.
- Open struct or map + a hostile extra key (`""`, `"0"`, `"a-b"`, `"toString"`,
  `"constructor"`, `"__proto__"` — object- and string-valued) → every key
  preserved and re-emitted, in all four targets; the same key set is what the
  count keywords see at both boundaries.
- Closed struct + extra key → one `Violation` per extra, aggregated with
  declared-field errors into a single `PayloadValidationError` application
  failure.
- Typed map / typed extras + value of wrong type → rejected with `path = key`;
  multiple bad extras reported in one shot (P11).
- Typed map + a member violating a *constraint* of its type (a string under
  `minLength`, an integer off its `multipleOf`, an array under `minItems`, a
  value outside a `const`/`enum` set) → rejected with `path = key`, on both
  deserialize and serialize.
- Nullable member (`additionalProperties: {oneOf:[{T},{"type":"null"}]}`) + an
  explicit `null` → accepted and preserved as a null member; a present member
  still validates against `T`.
- Typed extras + good value → validated and round-trips.
- Open opaque map round-trips arbitrary nested JSON unchanged.
- Pure map (all four languages) decodes into the wrapper's catch-all
  (`AdditionalProperties` member / `additionalProperties` Record /
  `additional_properties` dict), not a bare map/dict.

## Interactions

- **[[properties]] / [[patternProperties]]**: `additionalProperties` only sees
  members **not** in their matched-name annotations (spec §10.3.2.3).
  [[patternProperties]] is **temporarily unsupported** (rejected at load time in
  v1), so in our subset only [[properties]] matches are excluded — every other
  member is "additional."
- **[[unevaluatedProperties]]**: strictly more powerful (sees the transitive
  evaluated set across applicators). We **reject** `unevaluatedProperties` per
  **P6** (its annotation-dependent semantics don't lower);
  `additionalProperties` is the supported subset.
- **[[type]]**: `additionalProperties: true|false` is one of the three explicit
  resolutions [[type]] requires for `{type:object}` with no `properties`.
- **[[minProperties]] / [[maxProperties]]**: count constraints apply to the full
  member set including preserved extras, and to the same key set at both
  boundaries. Closing the object caps that set at the declared count; the count
  keywords own the resulting satisfiability check.
- **[[required]]**: a closed struct still permits required members; it only
  forbids *unknown* ones.
- **[[oneOf]]**: `additionalProperties: true` with no `properties` — the
  free-form object — is the one object shape a `oneOf` branch can carry
  **inline** without a type name: it declares nothing to emit, so TypeScript and
  Python express it structurally (`Record<string, unknown>` / `dict[str, Any]`)
  and Go and Java wrap it as the union's `<Union>Object` variant, in the
  map-shaped form from the table above. Any *structured* inline branch is named
  and emitted as an ordinary model instead.

## Ecosystem variance

| Source dialect        | Action                                                                     |
| --------------------- | -------------------------------------------------------------------------- |
| JSON Schema 2020-12   | Native; open default honored.                                              |
| OpenAPI 3.1           | Aligns with 2020-12. Native.                                               |
| OpenAPI 3.0           | `additionalProperties` identical (bool or schema); `{}` → reject as above. |
| Swagger 2.0 / draft-4 | `additionalProperties` identical; same `{}` rejection.                     |

## See also

- [[properties]] — declares the matched members this keyword excludes.
- [[oneOf]] — the free-form object as an inline union branch: structural in
  TS/Python, wrapped in the map-shaped form above in Go/Java.
- [[patternProperties]] — temporarily unsupported; typed-map alternative.
- [[unevaluatedProperties]] — rejected per **P6**; this is the subset.
- [[type]] — requires an explicit open/closed choice for bare objects.
- [[minProperties]], [[maxProperties]], [[propertyNames]] — other object-level
  assertions.
