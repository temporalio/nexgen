# `contentSchema`

Source: JSON Schema 2020-12, Validation, §8.5 "A Vocabulary for the Contents of
String-Encoded Data → contentSchema".

Supplies a schema describing the structure of a JSON **string**'s *decoded*
contents — an embedded document (typically JSON) carried inside a string. In
2020-12 it is an **annotation by default**.
**Not supported — rejected at load.** It describes an embedded document whose
typed lowering is genuinely ambiguous (P6), and it is only meaningful alongside
[[contentMediaType]], which is itself rejected.

## Spec summary

Verbatim (2020-12 validation, §8.5):

> If the instance is a string, and if "contentMediaType" is present, this
> property contains a schema which describes the structure of the string.

> The value of this property MUST be a valid JSON Schema.

> [It] SHOULD be ignored if "contentMediaType" is not present.

Distilled:
- Value **MUST be a valid JSON Schema** (a subschema for the *decoded* content).
- **Only meaningful with [[contentMediaType]]** — the media type says how to
  parse the string; `contentSchema` describes the parse result.
- **Annotation by default** — it does not assert unless an implementation opts
  in, and even then only after decoding the embedded document.
- Describes a **string that embeds another document**, e.g.
  `contentMediaType:"application/json"` + `contentSchema` for the nested JSON.

## Support decision

**Support:** no — **rejected at load time (P6 / P7.1).**

- **The embedded document has no coherent lowering (P6).** A string with
  `contentMediaType:"application/json"` + `contentSchema` is a
  **document inside a document**: on the wire it is a *string* whose text is
  itself a JSON value matching the subschema. Two lowerings are possible and
  neither is right — keep a bare `string` (and silently drop the nested
  structure the author declared), or emit the decoded nested **type** (and
  double-encode on the wire, since the child is serialized to a string, then
  embedded in the parent's JSON). Which one the author meant is unrecoverable.
  This is precisely the ambiguous, non-lowerable construct the strict subset
  rejects, the same posture as [[not]] and the boolean-logic applicators.
- **It depends on a rejected keyword.** `contentSchema` is inert without
  [[contentMediaType]] (the spec says to ignore it when the media type is
  absent), and [[contentMediaType]] is itself rejected (no emit site). So the
  pairing that gives `contentSchema` meaning never reaches code generation.
- **Accepted-but-unenforced is the P10 footgun.** Retaining it without
  decoding-and-validating the embedded document would present a
  constrained-looking schema that enforces nothing on the nested content — the
  "looks constrained, silently isn't" case **P10** forbids.

Loader behavior:
- Any `contentSchema` present → **reject** with a fix-it: if the field really
  carries an embedded JSON document, model it as a **nested typed field**
  directly (a property of the decoded type) rather than a string with
  `contentSchema`; the subset has no embedded-document string type.
- The value is **not** inspected, and is never lowered. The keyword's own
  rejection is the diagnostic the author sees: grading a subschema that is being
  discarded can only replace that diagnostic with one about a keyword *inside* a
  construct the loader rejects outright, which sends the author to the wrong
  line. A non-schema value (`contentSchema: 42`) therefore rejects on the same
  rule as any other `contentSchema`.
- Present without [[contentMediaType]] → reject as inert (the spec would ignore
  it; a dead keyword signals author confusion, P7.1).

## Type mapping

None — rejected before any type is emitted. The reject is the shared loader's,
so C# fails the same way as every other target, with the same diagnostic.

## Validator mapping

None — rejected at load time, so there is no (de)serialize boundary and no
serialize-side behavior (**P12**).

## Property-testing matrix

### Rejected at load time (negative) — the whole surface

| Reason                                | Example                                                                                               |
| ------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| Embedded document (non-lowerable, P6) | `{type:"string", contentMediaType:"application/json", contentSchema:{type:"object", properties:{…}}}` |
| Inert without media type (P7.1)       | `{type:"string", contentSchema:{type:"object"}}`                                                      |
| Non-schema value                      | `{…, contentSchema:42}` (rejected on the keyword, not on the value)                                   |

There are no accepted or runtime fixtures: the keyword never reaches code
generation.

## Interactions

- **[[contentMediaType]]**: the keyword `contentSchema` depends on; both reject,
  so an embedded structured document is inexpressible. The media-type reject is
  owned there.
- **[[contentEncoding]]**: the other content keyword — supported for `base64` →
  native bytes. An embedded *structured* document (`contentSchema`) is the
  opposite case: structure to lower, not bytes, and it is the one that has no
  coherent type.
- **[[not]]** / **[[anyOf]]** / **[[if-then-else]]**: siblings in the "ambiguous
  / non-lowerable, reject per P6" family — `contentSchema` is the
  embedded-document member.
- **[[properties]]**: the fix-it target — model the nested document as a real
  typed field instead of a schema-annotated string.

## Ecosystem variance

| Source dialect         | Action                                                            |
| ---------------------- | ----------------------------------------------------------------- |
| JSON Schema 2020-12    | Annotation by default → reject (non-lowerable embedded document). |
| OpenAPI 3.1            | Adopts 2020-12; `contentSchema` native → reject.                  |
| OpenAPI 3.0 / draft-07 | No `contentSchema` keyword — nothing to reject.                   |

## See also

- [[contentMediaType]] — the keyword `contentSchema` requires; both reject.
- [[contentEncoding]] — the supported content keyword (`base64` → bytes); the
  contrast to the non-lowerable embedded document.
- [[not]] — a sibling non-lowerable reject (P6).
- [[properties]] — the fix-it: model an embedded document as a nested typed
  field.
- [[PRINCIPLES.md]] — **P6** (strict subset), **P7/P7.1** (reject loudly with
  fix-its), **P10** (enforced, not advisory).
