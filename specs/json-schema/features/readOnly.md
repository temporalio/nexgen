# `readOnly`

Source: JSON Schema 2020-12, Validation, §9.4 "A Vocabulary for Basic Meta-Data
Annotations → readOnly and writeOnly".

Marks a value as **managed by the owning authority**: it may appear in a
response but a client should not send it in a request (a server-assigned `id`,
`createdAt`, `etag`). In 2020-12 it is a **pure annotation** — it never affects
validation. **Not supported — rejected at load.** Its only operational meaning
is a **request/response directional asymmetry** that does not lower into the
generator's one-type-per-schema model; in the Nexus/Temporal setting an
operation's request and response are **already separate types**, so the
difference is expressed by modeling them separately, not by annotating one
shared type. This spec owns the shared rationale its mirror [[writeOnly]] defers
to.

## Spec summary

Verbatim (2020-12 validation, §9.4):

> The value of these keywords MUST be a boolean.

> If "readOnly" has a value of boolean true, it indicates that the value of the
> instance is managed exclusively by the owning authority, and attempts by an
> application to modify the value of this property are expected to be ignored or
> rejected by that owning authority.

> These keywords can be used to assist in user interface instance generation. …
> An instance document that is marked as "readOnly" for the entire document MAY
> be ignored if sent to the owning authority, or MAY result in an error, at the
> authority's discretion.

Distilled:
- Value **MUST be a boolean**; default `false`. Multiple applicable occurrences
  OR together (any `true` ⇒ `true`).
- An **annotation**, not an assertion: per spec it
  **never changes whether an instance validates**. A wire value is judged by the
  schema's assertions alone, `readOnly` present or not.
- Its meaning is **directional**: the value belongs on the response/read side
  and should not be sent on the request/write side.
- Paired mirror of [[writeOnly]] (write-side-only). Both describe
  *which direction of the wire a field belongs to*, relative to an owning
  authority.

## Support decision

**Support:** no — **rejected at load time (P6 / P7.1).** Deferred, not a
categorical [[not]]-style exclusion: a request/response-types feature could
admit it later (Open questions).

The defining choices (citing [[PRINCIPLES.md]]):
- **A single type cannot hold a per-direction field (P6).** The generator emits
  **one type per schema**, (de)serialized over the **same** fields in both
  directions (the shared `Validate` flanked by a parse and an encode adapter,
  P12). `readOnly` asks for a field that is
  **present on the way out but absent/ignored on the way in** — two distinct
  wire shapes for one type. There is no coherent single-type lowering: the
  field's presence is not a property of the value, it is a property of
  *which direction the value is crossing*, which the type system cannot express
  on one struct.
- **The two escape hatches both fail the subset.** (a)
  **Split into request and response types** — a real feature (doubles the type
  surface, needs a naming scheme, and interacts with [[ref]]/P15 collisions and
  P13 evolution); out of scope for now. (b) **Doc-only annotation** — accept
  `readOnly` but enforce nothing; that is the "looks directional, silently
  isn't" footgun **P10** forbids (a client *will* send a `readOnly` field and
  nothing stops it).
- **The Nexus/Temporal shape makes it redundant anyway (P2).** An operation
  already has **distinct input and output types** ([[services]]). The
  request/response distinction `readOnly` reaches for is therefore modeled by
  putting the field on the **output** type and leaving it off the **input** type
  — the idiomatic, statically-enforced expression of the same intent. A shared
  type annotated `readOnly` is the awkward encoding of a distinction the type
  layer already draws.
- **No assertion to attach (P10).** As a §9.4 annotation it contributes no
  constraint predicate to the shared `Validate` and no adapter check; there is
  nothing to enforce even if we kept it.

Loader behavior:
- Any `readOnly: true` present → **reject** with a fix-it: model the read-side
  field on the operation's **output** type and omit it from the **input** type,
  rather than annotating one shared type.
- `readOnly` value **not a boolean** → **reject** (P7.1; the spec's own MUST).
  `{readOnly: "true"}`, `{readOnly: 1}`.
- `readOnly: false` → **reject** as a **no-op** (it is the default; a dead
  annotation signals author confusion, P7.1). Diagnostic notes it has no effect.
- `readOnly` **and** [[writeOnly]] both `true` on one node → **reject**
  (contradictory: managed-by-authority *and* never-returned — the value could
  never legitimately appear in either direction). See [[writeOnly]].

These are four different author mistakes with four different remedies, so one
shared message is not sufficient. The **no-op** case in particular must not be
told to split the type into request/response shapes — the fix for
`readOnly: false` is to *delete* it, because it already means nothing. The
non-boolean case names the offending value (per the `reason` convention), and
the contradictory pair names the contradiction and both keywords.

## Type mapping

None — rejected before any type is emitted. The reject is the shared loader's,
so C# fails the same way as every other target, with the same diagnostic.

## Validator mapping

None — rejected at load time, so there is no (de)serialize boundary and no
serialize-side behavior (**P12**).

## Property-testing matrix

### Rejected at load time (negative) — the whole surface

| Reason                                      | Example                                                                               |
| ------------------------------------------- | ------------------------------------------------------------------------------------- |
| Directional field (no single-type lowering) | `{properties:{id:{type:"string", readOnly:true}}}` (put `id` on the output type only) |
| Value not a boolean                         | `{readOnly:"true"}`, `{readOnly:1}`                                                   |
| No-op (equals default)                      | `{readOnly:false}`                                                                    |
| Contradictory pair                          | `{type:"string", readOnly:true, writeOnly:true}` (see [[writeOnly]])                  |

There are no accepted or runtime fixtures: the keyword never reaches code
generation.

## Interactions

- **[[writeOnly]]**: the write-side mirror; defers to this spec for the shared
  directional-asymmetry rationale. Both `true` on one node is a contradiction
  and rejects.
- **[[services]]**: the reason the keyword is redundant here — an operation's
  **input** and **output** are already separate types, which is where a
  read-only vs write-only distinction belongs.
- **[[required]]**: orthogonal in principle (presence vs direction) but
  `readOnly` + `required` is exactly the case single-type modeling mangles —
  required-in-responses yet forbidden-in-requests — which the reject sidesteps.
- **[[default]]**: a `readOnly` field with a server-supplied value is the
  archetype; here it is modeled as an output-type field, not an annotation on a
  shared type.
- **[[description]]**: the contrast — an annotation that *is* emittable
  (doc-comment body). `readOnly` has no emittable single-type form.

## Ecosystem variance

| Source dialect      | Action                                                                                                                                                                                                                                                       |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| JSON Schema 2020-12 | Annotation (§9.4) → reject (directional, no single-type lowering).                                                                                                                                                                                           |
| OpenAPI 3.1         | Adopts 2020-12; `readOnly` native and widely used → reject with the split-types fix-it. OpenAPI's own request/response schema reuse is the pattern this keyword serves — the Nexus model draws that line with separate operation input/output types instead. |
| OpenAPI 3.0         | `readOnly` present with the same semantics → reject.                                                                                                                                                                                                         |
| draft-07            | `readOnly` present since draft-07 (annotation) → reject.                                                                                                                                                                                                     |

## Open questions

1. **Request/response types.** The clean path to supporting `readOnly` /
   [[writeOnly]] is a feature that derives a **request view** and a
   **response view** from one annotated schema (dropping `readOnly` fields from
   the request, `writeOnly` fields from the response). This is a substantial
   addition (type-surface doubling, naming, P15 collisions, P13 evolution) and
   is deferred; until then the reject stands, with the
   separate-input/output-types fix-it as the idiomatic workaround.

## See also

- [[writeOnly]] — the write-side mirror; shares this rationale.
- [[services]] — operation input/output are already separate types, the
  idiomatic home for a read/write-side distinction.
- [[required]] — the presence axis that `readOnly` cuts across by direction.
- [[default]] — server-supplied values, the archetypal `readOnly` case, modeled
  as an output-type field here.
- [[description]] — the emittable annotation, the contrast to a non-lowerable
  directional one.
- [[PRINCIPLES.md]] — **P2** (idiomatic output), **P6** (strict subset),
  **P7/P7.1** (reject loudly with fix-its), **P10** (enforced, not advisory),
  **P12** (parse/encode adapters).
