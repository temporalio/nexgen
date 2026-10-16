# `contentMediaType`

Source: JSON Schema 2020-12, Validation, §8.4 "A Vocabulary for the Contents of
String-Encoded Data → contentMediaType".

Labels a JSON **string** with the media type (RFC 2046) of its contents —
`application/json`, `image/png`, `text/html`. In 2020-12 it is an
**annotation by default** (it does not assert).
**Not supported — rejected at load.** A media type describes a
*string in transit*, and there is
**no place in the generated model to emit it**: it is a container / transport
concern (an HTTP `Content-Type` header, a Temporal payload's metadata), not a
field type or a validator. Emitting nothing would silently drop it; the strict
subset rejects loudly instead (P6 / P7.1).

## Spec summary

Verbatim (2020-12 validation, §8.4):

> If the instance is a string, this property indicates the media type of the
> contents of the string.

> The value of this property MUST be a string.

And the vocabulary-wide annotation rule (§8.2): the content keywords
**function as annotations** and MUST NOT be evaluated automatically.

Distilled:
- Value **MUST be a string** naming a media type.
- Applies only to a **string** instance; a no-op on any other type.
- **Annotation by default** — purely descriptive metadata about what the string
  *contains*, with no validation effect unless an implementation opts in.
- Pairs with [[contentEncoding]] (how the string is encoded) and
  [[contentSchema]] (the structure of the decoded content).

## Support decision

**Support:** no — **rejected at load time (P6 / P7.1).**

- **Nowhere to emit it (P2 / P6).** The generator lowers a schema to a **type**
  and a **validator**. A media type is neither: it annotates the string's
  payload for a *transport layer* that lives outside the model — an HTTP
  `Content-Type`, a multipart part header, a Temporal payload metadata entry.
  There is no field, type, or member it maps to, so keeping it would mean
  silently discarding author intent — exactly the silently-wrong output the
  mission forbids.
- **It cannot become a validator either (P1 / P10).** Enforcing
  `contentMediaType: "application/json"` would mean
  *parsing the string as that media type* — a per-media-type parser (JSON, XML,
  PNG, …) we will not own, and whose accept line diverges wildly across
  languages/libraries (the same divergence that made [[format]]'s native
  validators unusable as an oracle). An accepted-but-unenforced media type is
  the "looks constrained, silently isn't" **P10** footgun.
- **Not a doc comment.** Unlike [[description]], a media type is not prose
  *about* the type — it is a machine-facing transport attribute. Rendering
  `image/png` into a doc comment would be dead metadata that no consumer acts
  on. Reject rather than bury it in a comment.

Loader behavior:
- Any `contentMediaType` present → **reject** with a fix-it: remove it; the
  media type belongs on the transport / payload envelope, not the model. (Future
  direction: carry it as **Temporal payload metadata** — see Open questions.)
- The diagnostic must say that the **schema is rejected** and name that remedy.
  It must not describe the keyword as ignored, tolerated, or "carried verbatim":
  nothing is carried, no code is generated, and a message implying otherwise
  sends the author looking for a value that does not exist. "Validate the media
  type in application code" is also the wrong advice — the media type belongs to
  the envelope, not to application-level validation of the string.
- The reject holds **regardless of [[contentEncoding]]**: a base64 blob labeled
  with a media type (`{contentEncoding:"base64", contentMediaType:"image/png"}`
  — the canonical "embedded binary file" shape) rejects **here**, on the label.
  The [[contentEncoding]] materialization only applies to *unlabeled* base64 in
  v1.
- `contentMediaType` is required for a meaningful [[contentSchema]]; since both
  reject, an embedded structured document rejects on either keyword (see
  [[contentSchema]]).

## Type mapping

None — rejected before any type is emitted.

## Validator mapping

None — rejected at load time, so there is no (de)serialize boundary and no
serialize-side behavior (**P12**).

## Property-testing matrix

### Rejected at load time (negative) — the whole surface

| Reason                            | Example                                                                                                |
| --------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Media type present (no emit site) | `{type:"string", contentMediaType:"application/json"}`                                                 |
| Labeled base64 blob               | `{type:"string", contentEncoding:"base64", contentMediaType:"image/png"}`                              |
| With `contentSchema`              | `{type:"string", contentMediaType:"application/json", contentSchema:{…}}` (also see [[contentSchema]]) |
| Value not a string                | `{contentMediaType:5}`                                                                                 |

There are no accepted or runtime fixtures: the keyword never reaches code
generation.

## Interactions

- **[[contentEncoding]]**: `base64` alone materializes to a native bytes type;
  adding `contentMediaType` rejects the node here, so v1 bytes are **unlabeled**
  binary. This spec owns the "no emit site" rationale that blocks the label.
- **[[contentSchema]]**: only meaningful alongside `contentMediaType`; both
  reject, so an embedded structured document is inexpressible in the subset.
  [[contentSchema]] owns the embedded-document / non-lowerable rationale.
- **[[description]]**: the contrast — `description` *is* emittable (the
  doc-comment body); a media type is machine-facing transport metadata with no
  doc-comment home, so it rejects rather than being rendered as a comment.

## Ecosystem variance

| Source dialect      | Action                                                                                                                                                                                                       |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| JSON Schema 2020-12 | Annotation by default → reject (no emit site).                                                                                                                                                               |
| OpenAPI 3.1         | Adopts 2020-12; `contentMediaType` native → reject. OAS carries media types structurally (the `content` map keyed by media type), which is the transport-level home this keyword lacks in a bare schema.     |
| OpenAPI 3.0         | No `contentMediaType`; media types live in the `content` map — nothing to reject at the schema level.                                                                                                        |
| draft-07            | Human porting note: the keyword existed there, but a document that declares draft-07 rejects on the dialect pin before this keyword is inspected. In a schema with no `$schema`, the keyword itself rejects. |

## Open questions

1. **Media type as Temporal payload metadata.** A media type is a natural fit
   for a Temporal payload's metadata map rather than the model type. Wiring it
   there would let `contentMediaType` (and a labeled [[contentEncoding]] base64
   blob) be **accepted** as transport metadata instead of rejected — the
   intended future direction.

## See also

- [[contentEncoding]] — the supported sibling (`base64` → native bytes); a
  media-type label blocks its materialization in v1.
- [[contentSchema]] — the embedded-document keyword; rejected, and meaningless
  without `contentMediaType`.
- [[description]] — the emittable annotation (doc-comment body), the contrast to
  a non-emittable transport attribute.
- [[format]] — why native content/format validators are unusable as a portable
  oracle (divergent accept lines).
- [[PRINCIPLES.md]] — **P1** (polyglot wire), **P2** (idiomatic,
  hand-written-feeling output), **P6** (strict subset), **P7/P7.1** (reject
  loudly with fix-its), **P10** (enforced, not advisory).
