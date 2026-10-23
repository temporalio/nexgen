# `examples`

Source: JSON Schema 2020-12, Validation, §9.5 "A Vocabulary for Basic Meta-Data
Annotations → examples".

Carries an **array of sample instance values** illustrating a schema. In 2020-12
it is a **pure annotation** — it never affects validation.
**Accepted and ignored at load (inert)** — dropped with no effect on the emitted
type, validator, or wire, pending a future doc-comment rendering feature. Unlike
[[readOnly]] / [[writeOnly]] (whose directional intent makes them a modeling
problem that ignoring would silently violate), `examples` is fully inert:
dropping it yields correct code, missing only an optional doc rendering. So
rather than reject it — which would force authors to strip ubiquitous, harmless
metadata just to generate — the generator accepts and ignores it. This is a
deliberate, narrowly-scoped exception to the **P7.1** loud-reject stance, safe
precisely because the keyword touches neither the type nor the wire.

## Spec summary

Verbatim (2020-12 validation, §9.5):

> The value of this keyword MUST be an array. There are no restrictions placed
> on the values within the array.

> When multiple occurrences of this keyword are applicable to a single
> sub-instance, implementations MUST provide a flat array of all values rather
> than an array of arrays.

> This keyword can be used to provide sample JSON values associated with a
> particular schema, for the purpose of illustrating usage. It is RECOMMENDED
> that these values be valid against the associated schema.

Distilled:
- Value **MUST be an array**; its elements are arbitrary JSON values (any type,
  no restriction).
- An **annotation**, not an assertion: it
  **never changes whether an instance validates**.
- The values *should* be valid against the schema (a RECOMMENDATION we would
  strengthen to a load-time MUST when supported — see Open questions).
- Purely illustrative — sample values for documentation, the sibling of
  [[description]] / [[title]] / [[default]] in the basic-metadata vocabulary.

## Support decision

**Support:** not yet — **accepted and ignored at load (inert)**, pending the
deferred doc-comment feature.

- **It has a clear future home: the doc comment (P2).** As a pure annotation
  `examples` can only ever materialize in the generated **doc comment** — it
  never affects the emitted type, an identifier, or validation. That is the same
  landing site [[description]] and [[title]] use, via the shared doc-comment
  assembly machinery [[description]] owns. So this is a
  *deferral of a supportable feature*, categorically unlike the non-lowerable
  [[readOnly]] / [[writeOnly]] / [[contentSchema]] rejects.
- **Ignored, because it is inert (the P7.1 exception).** P7.1 rejects loudly
  what is *ambiguous* or would produce *silently-wrong* output; `examples` is
  neither — it has zero effect on the emitted type or the wire, so ignoring it
  yields fully correct code, lacking only an optional doc rendering. It is also
  **pervasive** in real-world and imported schemas (every OpenAPI example
  carries it), so rejecting each occurrence would force authors to strip
  harmless documentation just to generate. The safe, ergonomic choice is to
  accept and drop it. (Contrast [[readOnly]] / [[writeOnly]], also annotations
  but carrying a *directional intent* that ignoring would silently violate —
  hence those reject, this one does not.)
- **Not free like a string annotation, when supported.** `examples` is
  *structured data* (an array of arbitrary JSON values), so — unlike
  [[description]]'s verbatim prose — a rendering feature must
  **serialize each value to a stable literal**, place it in a per-language
  example slot, and enforce validity at load. Those are real design decisions
  (Open questions), so the feature is scoped out of the current subset; until it
  lands the keyword is ignored rather than half-implemented.

Loader behavior:
- Any `examples` present → **accepted and ignored** — dropped with no effect on
  the emitted type, validator, or wire, and **no diagnostic**.
- Its value shape (the spec's MUST-be-array) is **not enforced while ignored**:
  the keyword is dropped wholesale, so a malformed `examples` is inert rather
  than a load error. The array-MUST and per-value validity return when the
  doc-comment feature lands (Open questions).
- An `examples` value is **opaque instance data**, never schema. No pass may
  interpret anything inside it: a `$ref`-shaped object among the sample values
  is a sample value, not a reference edge, and must not enter the `$ref`
  closure, pull a file into the input set, raise the input root, or change one
  module, package or file name ([[ref]], [[generated-file-layout]]). This is
  what "inert" has to mean for a keyword whose value is arbitrary JSON, and it
  is the precondition for the P7.1 accept-and-ignore exception above: the
  keyword may not affect the emitted type, any identifier, or the wire.
- `examples` as a **sibling of `$ref`** leaves the reference intact: it is
  dropped **before** the loader decides whether the reference needs an implicit
  merge, so it can neither clone the target into a use-site type nor add a P15
  identifier ([[ref]], [[allOf]] state the same rule from the merge side).
- `examples` on a [[nullability]] `null` branch → **reject**: a `null` branch
  must be exactly `{type: "null"}` with no siblings, an invariant
  [[nullability]] owns and an inert annotation does not override. At a
  **document root** — definitions-only or Nexus envelope — `examples` is
  accepted and dropped, as [[description]] is there.

## Type mapping

None — ignored, so no type is emitted. When supported it will contribute **no**
type and **no** identifier (a doc-comment annotation only).

C# drops it like every target: the generated code is byte-identical to the same
schema without `examples`, and no sample value reaches the `/// <summary>`.

## Validator mapping

None — an annotation (§9.5); it will never appear in the shared `Validate` or
run in either adapter (**P12**). Its entire future effect is at generation time,
in the emitted doc comment.

## Property-testing matrix

### Accepted (ignored)

| Shape                                  | Handling                                                                                                                                                                                         |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `examples` array on any node           | `{type:"string", examples:["alice","bob"]}` → accepted, dropped, no output                                                                                                                       |
| Malformed `examples` value             | `{examples:"alice"}`, `{examples:{a:1}}` → also ignored (array-MUST not enforced while ignored)                                                                                                  |
| A sample value shaped like a reference | `{type:"object", examples:[{"$ref":"../other/victim.yaml"}]}` → the object is data; the input set, the input root and every emitted module name are byte-identical to the same schema without it |
| Sibling of a `$ref`                    | `{$ref:"#/$defs/User", examples:[{…}]}` → dropped, reference unchanged                                                                                                                           |
| At a document root                     | a definitions-only or Nexus envelope root carrying `examples` → dropped                                                                                                                          |

### Rejected at load time (negative)

| Reason                         | Example                                                                         |
| ------------------------------ | ------------------------------------------------------------------------------- |
| On a nullability `null` branch | `{oneOf:[{type:"string"},{type:"null", examples:["x"]}]}` (see [[nullability]]) |

There are no runtime fixtures: the keyword is inert at runtime — it produces no
output while ignored. What does need a barrier is the inertness itself: a
structural regression that generates otherwise-identical schemas with and
without `examples` — including a sample value containing a `$ref`-shaped object,
and an `examples` beside a `$ref` — and compares the whole emitted file map.

## Interactions

- **[[description]]**: owns the doc-comment assembly, wrapping, and escaping
  machinery `examples` will plug into (as a rendered tag/trailer) once
  supported.
- **[[title]]**: the other doc-comment annotation; `examples` would join the
  same summary → body → tags assembly.
- **[[default]]**: the spec permits a `default` to double as an example; and
  `default` is the precedent for the strengthen-RECOMMENDED-validity-to-a-MUST
  stance `examples` would adopt when supported.
- **[[const]] / [[enum]]**: supplied value sets whose members a supported
  `examples` would be validated against at load (same cross-cutting
  literal-validity obligation).
- **[[ref]]** / **[[generated-file-layout]]**: an `examples` value is data, so
  it contributes no closure edge, no input file, and no change to the input root
  — which is what keeps module, package and file names a function of the schema
  rather than of its documentation.

## Ecosystem variance

| Source dialect      | Action                                                                                                                                                                                                                 |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12 | Annotation (§9.5), an **array** → ignored (inert).                                                                                                                                                                     |
| OpenAPI 3.1         | Adopts 2020-12 `examples` (array) → ignored. Note OAS also has a *singular* `example` (a single value) that is **not** a JSON Schema keyword — it falls under the generator's unknown-keyword handling, not this rule. |
| OpenAPI 3.0         | Singular `example` only; no array `examples` at the schema level.                                                                                                                                                      |
| draft-07            | `examples` present since draft-06 (array) → ignored.                                                                                                                                                                   |

## Open questions

1. **Doc-comment rendering (the deferred design).** When supported, `examples`
   renders into the generated doc comment via [[description]]'s machinery, using
   each language's native slot where one exists — JSDoc `@example` (TS), an
   `Examples:` section in the attribute docstring for a field / in the class
   docstring for a type (Python), a rendered `Example:` line (Go godoc, Java
   Javadoc `{@code …}`), an `<example>` element beside the `<summary>` (C#).
   Each array value is serialized to a canonical JSON literal; multiple values →
   multiple tags/lines; merged occurrences flatten per the spec's flat-array
   rule.
2. **Validity as a load-time MUST.** Following [[default]], each example would
   be required to validate against the schema at load (P7.1), sharing the
   deferred constraint-validation obligation those specs carry. Note the
   **ignore → render+validate** transition is a behavior change (P13): schemas
   that pass today with a malformed or invalid `examples` would then reject, so
   the feature should land with that migration in mind.

## See also

- [[description]] — owns the doc-comment assembly `examples` will render into.
- [[title]] — the sibling doc-comment annotation.
- [[default]] — may double as an example; the precedent for the "not yet
  supported" deferral and the validity-MUST stance.
- [[readOnly]] / [[writeOnly]] — metadata that *rejects* (directional intent
  that ignoring would violate), the contrast to this *inert, ignored*
  annotation.
- [[ref]] — an `examples` value is opaque data: no closure edge, and as a `$ref`
  sibling it drops without folding.
- [[PRINCIPLES.md]] — **P2** (idiomatic, hand-written-feeling output),
  **P7/P7.1** (the loud-reject stance, and the inert-annotation exception to
  it), **P12** (annotation — no adapter/validator effect).
