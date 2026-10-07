# `propertyNames`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.3.2.4
"Keywords for Applying Subschemas to Objects → propertyNames".

Constrains every **member name** of an object against a subschema (the
name, always a string, is the instance under test). Partially supported:
map-shaped objects only.

## Spec summary

Verbatim (2020-12 core, Applicator):

> The value of "propertyNames" MUST be a valid JSON Schema.

> If the instance is an object, this keyword validates if every property
> name in the instance validates against the provided schema. Note the
> property name that the schema is testing will always be a string.

> Omitting this keyword has the same behavior as an empty schema.

Distilled:
- The subschema validates **keys**, not values; the instance it sees is
  always a string.
- In practice the subschema is a string schema using [[pattern]],
  [[minLength]], [[maxLength]], [[enum]], or [[format]].

## Support decision

**Support:** partial — accepted **only** on a map-shaped object (an
object with [[additionalProperties]] and **no** [[properties]]);
**rejected** when [[properties]] is present. The test is keyword
**presence**, not member count: an explicit `properties: {}` is present, so it
rejects too ([[properties]] states the same rule from its side).

Rationale (citing [[PRINCIPLES.md]]):
- **P10 (enforced)**: on a map, key constraints lower to a clean runtime
  loop over the keys — checked at the boundary, aggregated per **P11**.
- **P7 / P7.1 (reject ambiguity)**: alongside [[properties]] the
  declared member names are static and known at generation time;
  layering a name constraint over them is ambiguous (does it gate the
  declared names, the extras, or both?) and adds little. Reject and ask
  the author to encode key shape on the map form instead.
- The propertyNames subschema must itself be a **supported string
  subschema** — implicitly/explicitly `type:"string"` with only
  string-applicable assertions. Anything else (e.g. `type:"integer"`,
  which can never match a string key) → reject per **P7.1**.

Loader behavior:
- `propertyNames` value not a valid schema → reject.
- Subschema not a string schema (or carrying non-string assertions) →
  reject; diagnostic explains keys are always strings.
- `propertyNames` present **with** [[properties]] — an empty
  `properties: {}` included — → reject; diagnostic
  points at the map form. A future relaxation could validate declared names
  at generation time and apply the runtime check only to extras — deferred
  pending demand.
- `propertyNames` with no [[additionalProperties]] (so no map, no
  properties) → already rejected by [[type]] (`type:object` needs a
  shape); `propertyNames` alone is not a shape.
- Empty / `true` subschema → reject per **P7.1** (no constraint; just
  drop the keyword).

## Type mapping

None of its own. The host object's type comes from
[[additionalProperties]] — all four languages wrap the map in a named
catch-all member (`AdditionalProperties map[string]T` /
`Map<String,T> additionalProperties` / `additionalProperties:
Record<string,T>` / `additional_properties: dict[str, V]`).
`propertyNames` only adds a key validator over those keys.

## Validator mapping

Per **P10**/**P11**. Loop over the parsed object's keys; validate each key
string against the (string) constraint. The violation's `path` is the offending
key **rendered per P11.2** — a map key is arbitrary text, so it is not spliced
raw into the path grammar; the spelling belongs to that clause and is not
restated here.

| Language | Strategy |
|---|---|
| Go | Iterate the wire keys and run the key predicate (compiled `regexp` for [[pattern]], length checks); a failure → a `Violation` at the key's path with `Reason: "invalid property name " + quoteValue(key) + ": " + why` (`why` is the underlying assertion's reason, e.g. `must match pattern "^[a-z]+$"`), collected into one `PayloadValidationError` application failure. |
| TypeScript | the same predicate over the wire object's own keys; a failure → push a `Violation` at the key's path with ``reason: `invalid property name ${JSON.stringify(k)}: ${why}` ``, throw one `PayloadValidationError` application failure. |
| Python | both directions of the `_<Model>TransferTypeConverter` (**PRINCIPLES Python §3**) loop the map's keys and apply the same key check; a failure appends a `Violation` at the key's path with ``reason=f'invalid property name {_quote(key)}: {why}'`` per bad key into the single `PayloadValidationError` application failure. |
| Java | in the per-POJO collecting deserializer (PRINCIPLES Java §5), iterate the parsed tree's keys, apply the same key check, and push a `Violation` at the key's path with `"invalid property name " + Violation.quote(key) + ": " + why` per bad key into the single `PayloadValidationError` application failure. |

Every row states the same predicate and the same `invalid property name <key>: `
prefix on both paths, with `<key>` quoted as a JSON string literal (the
[[pattern]] quoting rule, so a key containing `"` or `\` reads identically in
every target). A [[pattern]] key failure's `why` is `must match pattern
<quoted pattern>` with no `, got` suffix, since the prefix already names the
key — `invalid property name "Bad": must match pattern "^[a-z]+$"`. Per
**P12.2** that identity is the requirement, and whether a target reaches it
through one exported validator or an inlined check is an emission choice.

Reuses whatever the string-assertion specs ([[pattern]], [[minLength]],
[[maxLength]], [[enum]], [[format]]) emit — `propertyNames` is just those
checks applied to keys instead of values, so it inherits their
dialect/strategy decisions (notably [[pattern]]'s regex-dialect caveat).

### Serialize-side (P12)

The key check runs again before
emit: every catch-all key about to be written is re-validated against the
constraint, and a key inserted in memory that violates it (e.g. a map key
not matching the `pattern`) fails serialization rather than emitting an
out-of-contract object. Extra keys serialize **verbatim** (no
case-mapping — see [[additionalProperties]]), so the in-memory key is
exactly the wire key the check applies to, in both directions.

## Property-testing matrix

### Accepted (positive)

| Shape | Example |
|---|---|
| Pattern keys on a map | `{type:object, additionalProperties:{type:integer}, propertyNames:{type:string, pattern:"^[a-z]+$"}}` |
| Length-bounded keys | `{type:object, additionalProperties:true, propertyNames:{type:string, maxLength:64}}` |

### Rejected at load time (negative)

| Reason | Example |
|---|---|
| With `properties` (P7) | `{type:object, properties:{id:{type:integer}}, propertyNames:{type:string, pattern:"…"}}`; and the empty-but-present `properties:{}` spelling |
| Non-string subschema | `propertyNames:{type:integer}` |
| Shapeless subschema | `propertyNames:{}`, `propertyNames:true` |
| No host map | `propertyNames` with neither `properties` nor `additionalProperties` (caught by [[type]]) |

### Runtime fixtures (validator)

- All keys satisfy the constraint → OK.
- One key violates (bad pattern / too long) → one `Violation` in the
  payload-validation application failure, at that key's path (**P11.2**).
- Multiple bad keys → all reported in one shot (P11).
- Empty object → vacuously OK.

## Interactions

- **[[additionalProperties]]**: the host. `propertyNames` constrains the
  map's keys; `additionalProperties` constrains its values.
- **[[properties]]**: mutually exclusive with `propertyNames` in our
  subset (reject if both present).
- **[[patternProperties]]**: temporarily unsupported (rejected at load
  time in v1); `propertyNames` is the supported way to constrain key
  shape without per-pattern value schemas.
- **[[pattern]] / [[minLength]] / [[maxLength]] / [[enum]] / [[format]]**:
  the string assertions reused against keys; inherit their decisions.
- **[[minProperties]] / [[maxProperties]]**: count constraints compose
  with key-shape constraints on the same map. Where the key constraint makes the
  key language **finite and enumerable** — an `enum`, a `maxLength: 0` — it caps
  how many members the object can ever have, and reconciling that against a
  count floor is owned by [[minProperties]].

## Ecosystem variance

| Source dialect | Action |
|---|---|
| JSON Schema 2020-12 | Partial (map-only) as above. |
| OpenAPI 3.1 | Aligns with 2020-12. Partial. |
| OpenAPI 3.0 | No `propertyNames` keyword — nothing to map. |
| Swagger 2.0 / draft-4 | `propertyNames` (draft-6+) → same partial handling. |

## See also

- [[additionalProperties]] — the map host; constrains values.
- [[patternProperties]] — temporarily unsupported; key-constraint alternative.
- [[pattern]], [[minLength]], [[maxLength]], [[enum]], [[format]] —
  string assertions reused on keys.
- [[type]] — requires the object to have a shape.
