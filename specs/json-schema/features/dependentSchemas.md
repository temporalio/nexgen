# `dependentSchemas`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.2.2.4 "Keywords
for Applying Subschemas Conditionally → dependentSchemas".

Conditional *subschema* application: when a trigger member is present, the
**whole object** must additionally validate against an associated subschema.
**Not supported** — rejected at load time per **P6** (categorical exclusion, not
a deferral).

## Spec summary

Verbatim (2020-12 core, Applicator):

> This keyword specifies subschemas that are evaluated if the instance is an
> object and contains a certain property.

> This keyword's value MUST be an object. Each value in the object MUST be a
> valid JSON Schema.

> If the object key is a property in the instance, the entire instance must
> validate against the subschema. Its use is dependent on the presence of the
> property.

> Omitting this keyword has the same behavior as an empty object.

Distilled:
- `{"a": <subschema>}` means: **if property `a` is present, the whole object
  must also validate against `<subschema>`.** If `a` is absent, no constraint.
- It applies a *subschema*, not a name list — it is exactly
  `if {required:["a"]} then <subschema>`, i.e. runtime **conditional shape**.
- Contrast [[dependentRequired]], which names dependent members only and never
  applies a subschema.

## Support decision

**Support:** no — **rejected at load time (P6).**

`dependentSchemas` branches the object's shape on a runtime condition, which has
no coherent lowering across the five targets:

- **P6 (strict subset), conditional shape.** The set of members and their types
  depends on whether a trigger key is present at runtime. Modeling that
  faithfully needs conditional-shape machinery — the same reason
  [[if-then-else]], [[anyOf]], and [[not]] are rejected. (Contrast [[allOf]], an
  *unconditional* intersection that flattens to one schema at load, and
  [[oneOf]], a *closed* sum type with a decidable selector — both supported
  because neither has a runtime-conditional shape, which this keyword does.) Go
  and Java have no way to express "this object has these extra required
  fields/constraints *only when* key `a` was supplied" as a single static type;
  the only lowerings are to forfeit typing or to synthesize conditional variants
  that do not round-trip identically across languages (**P1**).
- **P7 / P7.1 (reject ambiguity loudly).** A schema whose shape forks on
  instance content is exactly the kind of construct rejected at generator time
  rather than approximated.

[[dependentRequired]] is the supported subset of conditional object logic: it
tests only *name presence* (never branching on subschema validation), so it
lowers to a flat cross-field boundary check. `dependentSchemas` crosses the line
[[dependentRequired]] stays behind.

Loader behavior:
- Any `dependentSchemas` present → reject with a located diagnostic.
- An authored empty `dependentSchemas: {}` is rejected too, as a dead keyword;
  it constrains nothing, so the actionable remedy is to remove it rather than
  either alternative below.
- The diagnostic offers the coherent alternatives:
  1. **[[dependentRequired]]** — when the dependent subschema only makes other
     members *required* (`{"a": {required:["b"]}}` → `{"a":["b"]}`).
  2. **Unconditional [[properties]] + [[required]]** — when the fields are
     really always part of the shape.

## Type mapping

None — rejected before any type is emitted.

## Validator mapping

None — rejected at load time, so there is no (de)serialize boundary and no
serialize-side behavior (**P12**).

## Property-testing matrix

### Rejected at load time (negative) — the whole surface

| Reason                                  | Example                                                                                                      |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Conditional shape (P6)                  | `{type:object, properties:{a:{…}}, dependentSchemas:{a:{properties:{b:{type:string}}}}}`                     |
| Conditional required via subschema (P6) | `{type:object, properties:{a:{…},b:{…}}, dependentSchemas:{a:{required:["b"]}}}` (use [[dependentRequired]]) |

There are no accepted or runtime fixtures: the keyword never reaches code
generation.

## Interactions

- **[[dependentRequired]]**: the supported subset — name-presence dependency
  only, no subschema application. Its spec already records `dependentSchemas` as
  rejected. A `dependentSchemas` value that is purely `{required:[…]}` should be
  rewritten to [[dependentRequired]].
- **[[required]]** / **[[properties]]**: the unconditional counterparts — the
  coherent lowering when the dependent fields are always part of the shape.
- **[[unevaluatedProperties]]**: the other rejected conditional/
  annotation-driven object keyword.

## Ecosystem variance

| Source dialect            | Action                                                                                                                                                  |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12       | Reject (P6).                                                                                                                                            |
| OpenAPI 3.1               | `dependentSchemas` present (3.1 adopts 2020-12) → reject.                                                                                               |
| OpenAPI 3.0 / Swagger 2.0 | No `dependentSchemas` keyword — nothing to reject.                                                                                                      |
| draft-4..7                | The merged `dependencies` keyword is rejected in both forms. Rewrite an array form as [[dependentRequired]]; the schema form has no supported lowering. |

## See also

- [[dependentRequired]] — the supported name-presence dependency and the
  migration target for a legacy `dependencies` array form.
- [[required]], [[properties]] — the unconditional lowering for dependent fields
  that are always part of the shape.
- [[unevaluatedProperties]] — another rejected annotation-driven object keyword.
- [[if-then-else]] — the general conditional applicator this keyword is a
  special case of (`if {required} then <subschema>`); same P6 rejection.
- [[anyOf]] — the rejected inclusive-or applicator in the same P6 family.
