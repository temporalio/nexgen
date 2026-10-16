# `$comment`

Source: JSON Schema 2020-12, Core vocabulary, §8.3 "Comments With `$comment`".

A note left in the schema **for the schema's own maintainers** — the spec
reserves it strictly for authors and forbids implementations from acting on it.
It looks like a sibling of [[description]], but its role is the opposite:
`description` is the one annotation that **becomes** generated documentation,
whereas `$comment` is defined to **leave no trace** in validation, in the
emitted type, or in the generated source. This spec's job is to pin that
difference down: `$comment` is **accepted and silently dropped**, not rejected
and not surfaced.

## Spec summary

Verbatim (2020-12 core, §8.3):

> This keyword reserves a location for comments from schema authors to readers
> or maintainers of the schema.

> The value of this keyword MUST be a string. Implementations MUST NOT present
> this string to end users. […] Implementations MUST NOT take any other effect
> based on the presence, absence, or contents of `$comment` properties.

Distilled:
- The value **MUST be a string** (the spec's own MUST).
- It is **not an annotation in the collected-annotation sense**: unlike
  [[title]] / [[description]], the spec explicitly forbids presenting it to end
  users or letting it change any behavior. It is inert by mandate.
- It may appear on **any subschema** — a `$defs` type, a property subschema, an
  `items` subschema, the document root.

## Support decision

**Support:** yes — **accepted and ignored** (silently dropped at load).

This is deliberately *not* a reject. `$comment` is a known core keyword whose
spec-mandated behavior is precisely "do nothing," so honouring it means dropping
it, not refusing the schema:

- **No runtime effect (P10/P12).** `$comment` contributes **no** predicate to
  the shared `Validate` and **no** parse/encode adapter logic. A wire value is
  judged by the schema's assertions alone.
- **No type, no identifier (P13/P15/P1).** It never names or shapes a type,
  field, service, or operation, and adds nothing to the P15 collision surface.
- **Not a doc comment (P2).** This is the line that separates it from
  [[description]]: the spec forbids presenting `$comment` to end users, so it is
  **not** routed into the generated doc comment. A maintainer note that says
  "TODO: tighten this once the API stabilises" must not leak into a published
  Go/TS/Python/Java/C# doc block. Authors who want prose in the generated output
  use [[description]]; `$comment` stays in the source schema only.

Loader behavior:
- `$comment` value **not a string** → **reject** (P7.1; and the spec's own
  MUST). `{$comment: 42}`, `{$comment: {...}}`.
- `$comment` a **string** (any content, including empty) →
  **accepted and dropped**. Unlike an empty [[description]] — which is rejected
  because it renders a dead doc body — an empty `$comment` produces no output at
  all, so there is nothing degenerate to guard against.
- `$comment` as a **sibling of `$ref`** remains inert: it is dropped before
  deciding whether the reference needs an implicit merge, so adding a comment
  cannot clone the target into a new type or add a P15 identifier.
- `$comment` on a [[nullability]] `null` branch → **reject**. This is the one
  position the "any subschema" rule above does not reach: a `null` branch must
  be exactly `{type: "null"}` with no siblings, an invariant [[nullability]]
  owns and an inert annotation does not override.
- `$comment` on a **document root** — a definitions-only root or a Nexus
  envelope root — → **accepted and dropped**, exactly as [[description]] is
  there. A root-level maintainer note is what an imported OpenAPI document
  carries, and dropping it changes nothing; a reject at that position would have
  to describe a model the author never wrote.

## Type mapping

**None.** `$comment` changes neither the emitted type nor any identifier, and —
by mandate — never the generated doc comment.

## Validator mapping

`$comment` emits **no validator** and **no adapter behavior**. It never appears
in the shared `Validate`, never runs in the parse or encode adapter, and
produces no output in either direction (**P12**). Its entire lifetime ends at
load, when it is dropped.

## Property-testing matrix

### Accepted (positive)

| Shape                       | Example                                                                                            |
| --------------------------- | -------------------------------------------------------------------------------------------------- |
| Comment on a `$defs` type   | `{$comment:"internal: revisit bounds", type:"object", …}` → dropped, type unchanged                |
| Comment on a property       | `properties:{age:{$comment:"was int32 in v1", type:"integer"}}` → dropped                          |
| Empty comment               | `{$comment:"", type:"string"}` → accepted, dropped (no dead output)                                |
| Comment sibling of a `$ref` | `{$ref:"#/$defs/User", $comment:"use-site note"}` → comment dropped, reference unchanged ([[ref]]) |
| Comment at a document root  | a definitions-only or Nexus envelope root carrying `$comment` → dropped                            |

### Rejected at load time (negative)

| Reason                         | Example                                                                          |
| ------------------------------ | -------------------------------------------------------------------------------- |
| Non-string value               | `{$comment:42}`, `{$comment:{...}}`, `{$comment:["x"]}`, `{$comment:null}`       |
| On a nullability `null` branch | `{oneOf:[{type:"string"},{type:"null", $comment:"note"}]}` (see [[nullability]]) |

### Runtime fixtures

None. `$comment` has no runtime behavior and no generated output. A structural
generation regression should compare otherwise-identical schemas with and
without the annotation, including beside a `$ref`.

## Interactions

- **[[description]]** / **[[title]]**: the annotations that *do* surface.
  `$comment` is the deliberate opposite — same "note on a schema" shape, but the
  spec forbids presenting it, so it is dropped rather than rendered. Prose meant
  for the generated output belongs in [[description]].
- **[[ref]]**: a `$comment` sibling is dropped without triggering the
  implicit-`allOf` rewrite; the reference and its emitted identity stay
  unchanged.

## Ecosystem variance

| Source dialect            | Action                                                                        |
| ------------------------- | ----------------------------------------------------------------------------- |
| JSON Schema 2020-12       | Native core keyword (§8.3). Accepted, dropped.                                |
| OpenAPI 3.1               | Adopts 2020-12 — `$comment` recognised; dropped.                              |
| OpenAPI 3.0 / Swagger 2.0 | No `$comment` keyword — nothing to handle.                                    |
| draft-7                   | `$comment` present since draft-7 with identical "ignore" semantics → dropped. |

## See also

- [[description]] — the annotation `$comment` is most often confused with; it
  *becomes* the doc comment, whereas `$comment` is dropped.
- [[title]] — the summary-line annotation; also surfaced, unlike `$comment`.
- [[ref]] — a `$comment` sibling of a `$ref` is dropped and leaves the reference
  intact.
- [[PRINCIPLES.md]] — **P2** (readable generated source), **P10/P12** (no
  runtime effect), **P13/P15/P1** (never an identifier).
