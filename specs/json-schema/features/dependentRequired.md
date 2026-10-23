# `dependentRequired`

Source: JSON Schema 2020-12, Validation vocabulary, §6.5.4 "Validation Keywords
for Objects → dependentRequired".

Conditional presence: when a trigger member is present, a set of other members
becomes required. Pure runtime cross-field assertion — no type impact.

## Spec summary

Verbatim (2020-12 validation, §6.5.4):

> The value of this keyword MUST be an object. Properties in this object, if
> any, MUST be arrays. Elements in each array, if any, MUST be strings, and MUST
> be unique.

> This keyword specifies properties that are required if a specific other
> property is present. Their requirement is dependent on the presence of the
> other property.

> Validation succeeds if, for each name that appears in both the instance and as
> a name within this keyword's value, every item in the corresponding array is
> also the name of a property in the instance.

> Omitting this keyword has the same behavior as an empty object.

Distilled:
- `{"a": ["b","c"]}` means: if `a` is present, `b` and `c` must also be present.
  If `a` is absent, no constraint.
- Names dependencies only — it never applies subschemas (contrast
  [[dependentSchemas]], which does and is rejected per **P6**).

## Support decision

**Support:** yes — runtime assertion.

Lowers to a boundary cross-field check in every language. It does **not** change
emitted types: every member involved (trigger and dependents) stays **optional**
at the type level, because the requirement is conditional — a member that were
unconditionally required would go in [[required]] instead.

Rationale (citing [[PRINCIPLES.md]]):
- **P10 (enforced)**: the conditional requirement is checked at the boundary,
  aggregated per **P11**.
- This is the one *conditional* object keyword that lowers cleanly: unlike
  [[dependentSchemas]] / `if`-`then`-`else` (rejected per **P6**), it only tests
  name presence, never branches on subschema validation, so no language needs
  sum-type or conditional-shape machinery.

Loader behavior:
- The keyword requires `type: "object"`; a missing or different type rejects at
  load time under **P7.1**, on the diagnostic shared with the other four
  object-constraint keywords.
- Value not an object → reject.
- Any value not an array of unique strings → reject.
- A trigger name or any dependent name not declared in [[properties]] → reject
  per **P7.1** (presence check on an undeclared member is undecidable).
  Diagnostic names the offender.
- A dependent name that is **also** in [[required]] → reject as redundant (it is
  unconditionally required already; the dependency is vacuous). Diagnostic
  suggests removing it from `dependentRequired`.
- A trigger name in [[required]] → **reject**: if the trigger is always present,
  its dependents are always required, so they belong in [[required]] directly.
  Keeps one canonical spelling.
- Empty object / empty arrays → accepted (vacuous). A vacuous entry asserts
  nothing, so it emits **no** runtime check — not an empty one. An emitter whose
  trigger header is written before its (empty) body has produced output the
  target may not even parse.
- A non-vacuous edge whose trigger or dependent is optional+nullable → reject.
  Go, Java and Python collapse explicit `null` to absence while TypeScript does
  not, so the presence predicate would otherwise differ after round-trip. Make
  the named member non-nullable or remove the dependency. Bare `$ref` aliases
  are followed when classifying nullability.

## Type mapping

None. All involved members keep their optional emitted form (see [[required]] /
[[nullability]]); the constraint is validator-only.

## Validator mapping

Per **P10**/**P11**. For each trigger present in the instance, verify each
dependent is also present.

| Language   | Strategy                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | `UnmarshalJSON` applies the cross-field predicate to the raw wire-key set: for each present trigger, each dependent must also be present; a missing one → `Violation{Path:dependent, Reason: fmt.Sprintf("property %q is required when %q is present", dependent, trigger)}`, collected into one `PayloadValidationError` application failure. `Validate` applies the same predicate to the key set `MarshalJSON` will write. |
| TypeScript | the same predicate over the wire object's own keys inbound and the outbound key set on the way out; a missing dependent → push ``Violation{path, reason: `property "${dependent}" is required when "${trigger}" is present`}``, throw one `PayloadValidationError` application failure.                                                                                                                                       |
| Python     | `from_transfer_type` reads the raw wire dict: for each present trigger, append `Violation(path=dependent, reason=f'property "{dependent}" is required when "{trigger}" is present')` per absent dependent, into the single generated `PayloadValidationError` application failure. The dependency map is a module-level private constant, alongside `_<MODEL>_DECLARED`.                                                      |
| Java       | in the per-POJO collecting deserializer (PRINCIPLES Java §5): over the parsed tree's present-key set, for each present trigger push a `Violation{path:dependent, "property \"" + dependent + "\" is required when \"" + trigger + "\" is present"}` per missing dependent into the single `PayloadValidationError` application failure.                                                                                       |
| C#         | `Converter.Parse` checks the present wire keys and records `Violation{path:dependent, "property \"<dependent>\" is required when \"<trigger>\" is present"}` per missing dependent; `Validate` makes the same check over the members it will write. Both join the single `PayloadValidationError` application failure.                                                                                                        |

### Serialize-side (P12)

The cross-field check runs again before emit, over the **to-be-emitted** member
set (present = will be written, after default omission). If a trigger will be
emitted but a dependent is unset/omitted, serialize fails with the same
`Violation{path:dependent, reason}` in the payload-validation application
failure — symmetric with deserialize, where "present" means "on the wire." A
dependent satisfied only by an omitted default does **not** count as present.
The member set is the whole wire object, extras included: on an open object a
trigger or dependent carried in the [[additionalProperties]] catch-all is
present, in both directions — a target that consults only declared fields will
emit wire its own parser then rejects.

## Property-testing matrix

### Accepted (positive)

| Shape               | Example                                                                              |
| ------------------- | ------------------------------------------------------------------------------------ |
| Single dependency   | `{type:object, properties:{a:{…},b:{…}}, dependentRequired:{"a":["b"]}}`             |
| Multiple dependents | `dependentRequired:{"a":["b","c"]}`                                                  |
| Empty (no-op)       | `dependentRequired:{}`, and a per-trigger empty list `{"a":[]}` — both emit no check |

### Rejected at load time (negative)

| Reason                              | Example                                                                                         |
| ----------------------------------- | ----------------------------------------------------------------------------------------------- |
| Not object / not arrays             | `dependentRequired:[]`, `{"a":"b"}`                                                             |
| Non-string / non-unique dependents  | `{"a":[1]}`, `{"a":["b","b"]}`                                                                  |
| Undeclared trigger/dependent (P7.1) | trigger or dependent absent from [[properties]]                                                 |
| Dependent already in `required`     | `required:["b"], dependentRequired:{"a":["b"]}`                                                 |
| Trigger in `required`               | `required:["a"], dependentRequired:{"a":["b"]}`                                                 |
| Optional nullable trigger/dependent | `properties:{a:{oneOf:[{type:string},{type:null}]},b:{type:string}}, dependentRequired:{a:[b]}` |

### Runtime fixtures (validator)

- Trigger absent → no constraint (dependents may be absent).
- Trigger present + all dependents present → OK.
- Trigger present + a dependent absent → one `Violation{path:dependent, reason}`
  in the payload-validation application failure.
- Multiple triggers each missing dependents → all reported in one shot (P11).

## Interactions

- **[[required]]**: unconditional counterpart. A name can't be in both
  (`required` wins; the dependency would be vacuous → load error).
- **[[properties]]**: every trigger and dependent must be declared.
- **[[dependentSchemas]]**: the subschema-applying sibling — **rejected** per
  **P6** (conditional shape doesn't lower). `dependentRequired` is the supported
  subset of conditional object logic.
- **[[nullability]]**: independent at the type level — dependency is about
  presence, not null-ness. At load, however, a non-vacuous edge may not name an
  optional nullable trigger or dependent: the Go/Java/Python presence collapse
  would otherwise make the predicate differ after round-trip. Required nullable
  members remain legal generally, but the existing canonicalization rules make a
  required trigger or dependent redundant here.
- **[[minProperties]] / [[maxProperties]]**: a trigger forces its whole
  dependent set to be present at once, so a cap below that closure is
  uninhabitable — reconciled by the count keywords, specified in
  [[maxProperties]].

## Ecosystem variance

| Source dialect            | Action                                                                                                                                                                                                                                                                                                                                                                                                                          |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12       | Native.                                                                                                                                                                                                                                                                                                                                                                                                                         |
| OpenAPI 3.1               | Aligns with 2020-12. Native.                                                                                                                                                                                                                                                                                                                                                                                                    |
| OpenAPI 3.0 / Swagger 2.0 | No `dependentRequired`; draft-4..7 used `dependencies`. The legacy keyword is outside this loader's 2020-12 vocabulary and **rejects at load with a rewrite fix-it** for the array form (`dependencies: {a: [b]}` → `dependentRequired: {a: [b]}`), matching [[exclusiveMaximum]]'s treatment of its own superseded draft form. The schema form has no supported lowering and rejects as [[dependentSchemas]] does, per **P6**. |
| draft-4..7                | `dependencies` is rejected as a legacy keyword, **with the rewrite fix-it above** for the array form; its schema form has no supported lowering (see [[dependentSchemas]]). A bare "unknown schema keyword" diagnostic is **not** conformant here — the fix-it is the point.                                                                                                                                                    |

## See also

- [[required]] — unconditional presence.
- [[dependentSchemas]] — conditional *subschema* application (rejected, P6).
- [[properties]] — declares the members named here.
