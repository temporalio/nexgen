# `patternProperties`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.3.2.2 "Keywords
for Applying Subschemas to Objects → patternProperties".

Applies subschemas to members whose **names match a regular expression**.
**Temporarily unsupported** — rejected at load time in v1, but not a categorical
P6 exclusion: a narrow single-pattern form is plausibly lowerable and is
deferred to post-v1.

## Spec summary

Verbatim (2020-12 core, Applicator):

> The value of "patternProperties" MUST be an object. Each property name of this
> object SHOULD be a valid regular expression, according to the ECMA-262 regular
> expression dialect. Each property value of this object MUST be a valid JSON
> Schema.

> Validation succeeds if, for each instance name that matches any regular
> expressions that appear as a property name in this keyword's value, the child
> instance for that name successfully validates against each schema that
> corresponds to a matching regular expression.

> The annotation result of this keyword is the set of instance property names
> matched by this keyword. This annotation affects the behavior of
> "additionalProperties" (in this vocabulary) and "unevaluatedProperties" (in
> the Unevaluated vocabulary).

> Omitting this keyword has the same assertion behavior as an empty object.

Distilled:
- Regex-keyed subschemas: members whose name matches a pattern must validate
  against the corresponding subschema.
- A single name may match multiple patterns (must satisfy **all**).
- Contributes to the matched-name annotation that [[additionalProperties]]
  consumes.

## Support decision

**Support:** no — **temporarily unsupported.** Rejected at load time in v1, but
explicitly *deferred*, not categorically excluded: the general form carries real
lowering hazards (below), yet a narrow single-pattern form is plausibly
representable and is tracked for a future release.

Why deferred rather than landed in v1 (citing [[PRINCIPLES.md]]):
- **P6 (strict subset)**: regex-keyed members produce a **dynamically keyed**
  member set that cannot lower to named, statically-typed fields in
  Go/TS/Java/Python. It is a map construct wearing object clothes.
- **P7 / P7.1 (reject ambiguity loudly)**: overlapping patterns mean a single
  member may be governed by several subschemas at once (validate against *all*
  matching) — the emitted value type would be an intersection with no coherent
  cross-language representation.
- **Regex-dialect divergence** compounds it: the spec mandates ECMA-262, but
  Go's `regexp` (RE2) rejects lookahead/backreferences that ECMA-262 and
  `java.util.regex` accept. A pattern accepted by the input could be
  unrepresentable in one target's runtime. The value-level [[pattern]] keyword
  at least confines this hazard to one auditable gate; here it multiplies across
  the *key space*.

These hazards apply to the general form only. A single-pattern, no-`properties`,
RE2-safe schema is the candidate carve-out — it is why this keyword is
"temporarily unsupported" rather than a hard P6 reject like
[[dependentSchemas]]. Landing that carve-out first requires a sound
cross-runtime regex gate for pattern keys.

Loader behavior (v1):
- Any `patternProperties` present → reject with a located diagnostic.
- An authored empty `patternProperties: {}` is also rejected as a dead keyword;
  it constrains nothing, so the actionable remedy is to remove it.
- The diagnostic must read as "not yet supported," not "forbidden," and offer
  the two coherent alternatives available today:
  1. A **typed map** — `{type:object, additionalProperties:{type:T}}` — when the
     intent is "arbitrary keys, homogeneous values" (see
     [[additionalProperties]]).
  2. Enumerated **[[properties]]** — when the key set is actually finite and
     known.

## Type mapping

None — rejected before any type is emitted.

## Validator mapping

None — rejected at load time, so there is no (de)serialize boundary and no
serialize-side behavior (**P12**). If the single-pattern typed-map carve-out
later lands, its key+value checks would join the shared `Validate` and run in
both directions exactly like [[propertyNames]]'s key check.

## Property-testing matrix

### Rejected at load time (negative) — the whole surface

| Reason                   | Example                                                                                     |
| ------------------------ | ------------------------------------------------------------------------------------------- |
| Any pattern map (P6)     | `{type:object, patternProperties:{"^x-":{type:string}}}`                                    |
| With `properties`        | `{type:object, properties:{id:{type:integer}}, patternProperties:{"^meta_":{type:string}}}` |
| Overlapping patterns     | `{type:object, patternProperties:{"^a":{…}, "a$":{…}}}`                                     |
| RE2-incompatible pattern | `{type:object, patternProperties:{"(?=x)":{type:string}}}`                                  |

All rows currently receive the same keyword-level rejection. Pattern syntax and
overlap are not inspected until a supported carve-out exists.

There are no accepted or runtime fixtures in v1: the keyword does not yet reach
code generation. The single-pattern carve-out, should it land, would add
accepted + runtime rows.

## Interactions

- **[[additionalProperties]]**: per spec, `patternProperties` contributes to the
  matched-name annotation `additionalProperties` consumes. Since we reject
  `patternProperties` at load time in v1, that contribution never arises in
  practice — `additionalProperties` only ever excludes [[properties]] matches in
  our subset.
- **[[unevaluatedProperties]]**: rejected per P6 (a hard exclusion, unlike this
  keyword's temporary status); both belong to the annotation-dependent object
  machinery we exclude.
- **[[propertyNames]]**: the supported escape hatch for *constraining key shape*
  without assigning per-pattern value schemas — partial support, map-shaped
  objects only.
- **[[pattern]]**: the value-level regex keyword; it confines dialect handling
  to one gate, whose portability limitations are documented there.

## Ecosystem variance

| Source dialect        | Action                                                                                                                                                                                                 |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| JSON Schema 2020-12   | Reject (deferred — see Support decision).                                                                                                                                                              |
| OpenAPI 3.1           | `patternProperties` present (3.1 adopts 2020-12) → reject.                                                                                                                                             |
| OpenAPI 3.0           | No `patternProperties` keyword — nothing to reject; users already lean on `additionalProperties`.                                                                                                      |
| Swagger 2.0 / draft-4 | `patternProperties` present → reject — though a *declared* older dialect rejects on the `$schema` pin first ([[input-files]]), so only a document with no `$schema` reaches this keyword's diagnostic. |

## See also

- [[additionalProperties]] — typed-map alternative.
- [[propertyNames]] — constrain key shape (partial support).
- [[properties]] — enumerate a known key set.
- [[pattern]] — value-level regex with dialect caveats.
- [[unevaluatedProperties]] — hard-rejected (P6), unlike this keyword's
  temporary status.
