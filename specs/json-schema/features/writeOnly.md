# `writeOnly`

Source: JSON Schema 2020-12, Validation, §9.4 "A Vocabulary for Basic Meta-Data
Annotations → readOnly and writeOnly".

Marks a value as **write-side-only**: it may be sent in a request but is never
returned in a response (the archetype is a `password`). In 2020-12 it is a
**pure annotation** — it never affects validation.
**Not supported — rejected at load.** It is the mirror of [[readOnly]] and fails
the strict subset for the same reason: its only meaning is a request/response
**directional asymmetry** that does not lower into the generator's
one-type-per-schema model. The shared rationale lives in [[readOnly]]; this spec
states the mirror-case specifics.

## Spec summary

Verbatim (2020-12 validation, §9.4):

> The value of these keywords MUST be a boolean.

> If "writeOnly" has a value of boolean true, it indicates that the value is
> never present when the instance is retrieved from the owning authority. It may
> be sent to the owning authority to set or update the value, but it is not
> returned.

> [E.g.] "writeOnly" would be used to mark a password input field.

Distilled:
- Value **MUST be a boolean**; default `false`. Multiple applicable occurrences
  OR together (any `true` ⇒ `true`).
- An **annotation**, not an assertion: it
  **never changes whether an instance validates**.
- Its meaning is **directional**: the value belongs on the request/write side
  and is absent from the response/read side — the exact mirror of [[readOnly]].

## Support decision

**Support:** no — **rejected at load time (P6 / P7.1).** Deferred, not a
categorical exclusion — see [[readOnly]] for the shared reasoning (a single type
cannot hold a per-direction field; the split-types and doc-only escape hatches
both fail the subset; an operation's input and output are already separate types
in the Nexus model).

Mirror-case specifics:
- The canonical case is a **`password`**: present on the operation's **input**
  type, absent from its **output** type. That is exactly how it is modeled here
  — put the field on the input type and leave it off the output — rather than
  annotating one shared type `writeOnly`.
- A **doc-only** acceptance would be worse than for [[readOnly]]: a `writeOnly`
  field silently echoed back in a response is a **data-exposure** footgun (a
  password round-tripped to the client), the strongest form of the "looks
  directional, silently isn't" **P10** hazard.

Loader behavior:
- Any `writeOnly: true` present → **reject** with a fix-it: model the write-side
  field on the operation's **input** type and omit it from the **output** type.
- `writeOnly` value **not a boolean** → **reject** (P7.1). `{writeOnly: "yes"}`,
  `{writeOnly: 0}`.
- `writeOnly: false` → **reject** as a **no-op** (equals the default).
- `writeOnly` **and** [[readOnly]] both `true` on one node → **reject** as
  contradictory (never sent to the client *and* managed by the authority — no
  direction the value legitimately appears in).

As in [[readOnly]], the four rejects carry four different remedies and must be
distinguishable: `writeOnly: false` is told to delete a dead annotation, never
to split the type by direction.

## Type mapping

None — rejected before any type is emitted. The reject is the shared loader's,
so C# fails the same way as every other target, with the same diagnostic.

## Validator mapping

None — rejected at load time, so there is no (de)serialize boundary and no
serialize-side behavior (**P12**).

## Property-testing matrix

### Rejected at load time (negative) — the whole surface

| Reason                                      | Example                                                                                           |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Directional field (no single-type lowering) | `{properties:{password:{type:"string", writeOnly:true}}}` (put `password` on the input type only) |
| Value not a boolean                         | `{writeOnly:"yes"}`, `{writeOnly:0}`                                                              |
| No-op (equals default)                      | `{writeOnly:false}`                                                                               |
| Contradictory pair                          | `{type:"string", readOnly:true, writeOnly:true}`                                                  |

There are no accepted or runtime fixtures: the keyword never reaches code
generation.

## Interactions

- **[[readOnly]]**: the read-side mirror; owns the shared directional-asymmetry
  rationale. Both `true` on one node is a contradiction and rejects.
- **[[services]]**: the write-side field belongs on the operation's **input**
  type, which is already distinct from the output type.
- **[[required]]**: `writeOnly` + `required` (a required-on-input,
  absent-on-output field like a `password`) is precisely the case single-type
  modeling mangles; the reject sidesteps it.

## Ecosystem variance

| Source dialect      | Action                                                                                          |
| ------------------- | ----------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12 | Annotation (§9.4) → reject (directional, no single-type lowering).                              |
| OpenAPI 3.1         | Adopts 2020-12; `writeOnly` native (the `password` idiom) → reject with the split-types fix-it. |
| OpenAPI 3.0         | `writeOnly` present with the same semantics → reject.                                           |
| draft-07            | `writeOnly` present since draft-07 (annotation) → reject.                                       |

## See also

- [[readOnly]] — the read-side mirror; owns the shared rationale and the
  request/response-types open question.
- [[services]] — operation input/output are already separate types, the home for
  a write-side-only field.
- [[required]] — the presence axis `writeOnly` cuts across by direction.
- [[PRINCIPLES.md]] — **P6** (strict subset), **P7/P7.1** (reject loudly with
  fix-its), **P10** (enforced, not advisory; the data-exposure hazard), **P12**
  (parse/encode adapters).
