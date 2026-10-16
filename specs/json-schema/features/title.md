# `title`

Source: JSON Schema 2020-12, Validation vocabulary, §9.1 "A Vocabulary for Basic
Meta-Data Annotations → title and description".

A short human-readable label for the schema it sits on. In the spec it is a
**pure annotation** — it never affects validation, and its text never supplies a
*name*. It is not, however, inert on the emitted shape everywhere: beside a
`$ref` it is a merge conjunct, and the merge materializes a type (see Type
mapping). We give it exactly one operational role: it becomes the
**summary line of the generated doc comment** on the type or member it decorates
(a Go `//` comment, a TS/Java block comment, a Python docstring). Crucially, and
unlike much of the ecosystem, `title` is **never** used to derive a type or
field *name* — names come from the `$defs` key and the [[properties]] resolved
policy, never from free-form prose (see Type mapping and Ecosystem variance).

## Spec summary

Verbatim (2020-12 validation, §9.1):

> The value of both of these keywords MUST be a string.

> Both of these keywords can be used to decorate a user interface with
> information about the data produced by this user interface. A title will
> preferably be short, whereas a "description" will provide explanation for the
> purpose of the instance described by this schema.

Distilled:
- An **annotation**, not an assertion: per spec it
  **never changes whether an instance validates**. A wire value is judged by the
  schema's assertions alone, `title` present or not.
- Its value **MUST be a string** (the spec's own MUST, not a strengthening of
  ours).
- A **short label** — the spec's own guidance ("preferably be short"). Its
  sibling [[description]] carries the prose.
- Applies to **any subschema**: a `$defs` type, a property subschema, an array
  `items` subschema — anywhere a schema node appears.

## Support decision

**Support:** yes — as a **doc-comment summary only**. It emits no validator, no
adapter behavior, and no type/field name.

The defining choices (citing [[PRINCIPLES.md]]):
- **Annotation, no runtime effect (P10/P12).** `title` contributes **no**
  constraint predicate to the shared `Validate` and **no** parse/encode adapter
  logic. It is inert at runtime in both directions — even more inert than
  [[default]], which at least owns omit-unset + materialize-on- read. `title`
  touches only the *generated source text*.
- **Never a type name (P13, P15, P1).** The obvious ecosystem move — using
  `title` to name the generated model (openapi-generator does this for inline
  schemas; datamodel-code-generator under `--use-title-as-name` — see Ecosystem
  variance) — is **deliberately refused**:
  - **P13 (stability).** Type names must be stable across schema revisions.
    `title` is free-form UI prose; letting it name a type means a cosmetic
    copy-edit ("User" → "User account") silently **renames the type** and breaks
    every call site — a breaking change from a change that was never meant to be
    one. Our type names derive from the `$defs` key / property name, which
    authors already treat as an identifier.
  - **P15 (one namespace, no mangling).** A prose title routed into the
    identifier namespace multiplies the collision surface (two types titled
    "Request", a title colliding with a `$defs` key) with no stable escape hatch
    — `title` cannot double as both a display label and a de-collided
    identifier.
  - **P1 (polyglot consistency).** A title-derived name skips the [[properties]]
    Stage 1–4 case-mapping pipeline, so it would land differently per language —
    the exact footgun openapi-generator was filed for (issue #5248: title-named
    schemas bypass the naming transform and cause import failures). Our names go
    through one pipeline precisely so all four targets agree.
- **Summary line, co-owned with [[description]].** `title` and [[description]]
  feed the **same** generated doc comment: `title` is the
  **first/summary line**, [[description]] the body. The shared doc-comment
  machinery (placement, wrapping, escaping per language) is specified in
  [[description]]; this spec owns only `title`'s contribution (the summary line)
  and its load-time shape checks.

Loader behavior:
- `title` value **not a string** → **reject** (P7.1; and the spec's own MUST).
  `{title: 42}`, `{title: ["a"]}`, and `{title: null}` — an explicit `null` is a
  non-string *value*, not an absent keyword, so it rejects rather than being
  dropped.
- `title` an **empty or whitespace-only** string → **reject** as degenerate: it
  renders an empty summary line, which is dead metadata and signals author
  confusion (P7.1). Drop it, or give it text.
- `title` containing a **newline** → **reject**: `title` is a short label (spec:
  "preferably be short"), and a multi-line summary line is a category error —
  prose belongs in [[description]]. Diagnostic: move the body to `description`.
- `title` as a **sibling of `$ref`** is **not** rejected: a sibling-bearing
  `$ref` is rewritten to an implicit `allOf` and **merged** (see [[ref]]), so a
  use-site `title` folds into the merged schema under the [[allOf]] rule below —
  deduped against the target's own `title` when identical, and when they differ
  the use-site `title` **wins** (it is last in the rewrite; see [[ref]]).
- Multiple `title`s applicable to one node after an [[allOf]] merge (P6),
  whether from an explicit `allOf` or a `$ref` sibling → **last-wins**:
  identical values dedup, and when they differ the **last-merged** `title`
  survives (see [[allOf]]). A differing title is a deterministic override, never
  a reject. Mirrors [[default]] and [[description]].

## Type mapping

**None of its own**, and — the load-bearing negative — `title` does **not**
contribute the type's or field's *name* (rationale in Support decision). Beside
a `$ref` the merge does materialize a type, but the shape and the name are the
merge's, never the title's (see the P15 note below). The type comes from
[[type]] + [[nullability]]; the name comes from the `$defs` key / [[properties]]
resolved policy / [[ref]]. `title`'s sole materialization is the
**summary line of the doc comment**, which carries no type information:

| Placement                        | Where the summary line lands                                                                                  |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `$defs` type                     | doc comment on the generated **type** (Go `type` / TS `interface` / Python class / Java class)                |
| property subschema               | doc comment on the generated **field/member**                                                                 |
| `items` / other inline subschema | doc comment on the declaration synthesized from that subschema, if any; otherwise dropped (nowhere to attach) |

Per-language rendering of the summary line (the shared doc-comment mechanism —
placement, line-wrapping, escaping — is owned by [[description]]; this is only
`title`'s slot in it):

| Language   | Summary-line mechanism                                                                                                                                                                            |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | leading `// ` line of the doc comment above the `type`/field, **led by the identifier name** per Go convention — `// <Name> <title>` (see below).                                                 |
| TypeScript | first line of the `/** … */` JSDoc above the `interface`/field.                                                                                                                                   |
| Python     | first line of the class **docstring**; for a **field**, the first line of the string literal that follows the field declaration (the attribute-docstring convention documentation tooling reads). |
| Java       | first sentence of the `/** … */` Javadoc above the class/getter.                                                                                                                                  |

**Go — the identifier-led first line.** Idiomatic Go doc comments for an
exported identifier **begin with the name being described** (`godoc` / `golint`:
`// User is a registered account.`, not `// A registered account.`). So Go does
**not** emit the bare `title` as the summary line the way the other three
languages do — it prefixes the identifier:

```go
// User A registered user account.
type User struct { … }

// RoomID Identifier of the chat room.
RoomID string
```

- The line is `// <Name> <title>`, where `<Name>` is the generated type or field
  identifier. This keeps the comment name-led (satisfying `golint`'s "comment
  should be of the form `<Name> …`" and staying greppable by identifier) while
  still surfacing the author's `title`.
- **Redundancy guard:** if the `title` already begins with `<Name>`
  (case-insensitively) **as a whole word** — e.g. field `Email` with
  `title:"Email address"` — the identifier is **not** doubled; the `title` is
  emitted as-is (`// Email address`), since it is already name-led. This avoids
  the `// Email Email address` stutter. The word boundary is load-bearing: a
  bare character-prefix test would suppress the identifier for field `id` with
  `title:"Identifier of the room"`, or for a type `U` with any title starting in
  `U`, leaving a comment `golint`/`revive` flag as not of the form `<Name> …` —
  which is the outcome this guard and PRINCIPLES Go §1 both exist to prevent.
- Because a `title` is a short **label** (a noun phrase), not necessarily a
  grammatical predicate, the name-led join reads as a caption rather than a full
  sentence — acceptable and still idiomatic (the convention golint enforces is
  the *name-first* rule, not sentence grammar). Authors wanting a full-sentence
  doc put the prose in [[description]], which becomes the body under the same
  name-led first line.

The name-led first-line rule is a property of the **Go doc comment as a whole**
(it governs whichever text — `title` or, absent a title, [[description]] — opens
the comment), so the mechanism is co-owned with [[description]]; this spec fixes
only how `title` fills that opening slot.

When both `title` and [[description]] are present, the doc comment is the
summary line (`title`) then the body ([[description]]); when only `title` is
present, the doc comment is just the summary line.

On an ordinary declaration `title` synthesizes no identifier.
**Beside a `$ref` it does**: `title` is not in the inert sibling class, so the
sibling triggers the implicit merge, which materializes a standalone use-site
type and its package-level identifiers ([[ref]], [[allOf]]). Those names come
from the **position** — the enclosing type plus the member — never from the
title text. So `title` **does** carry a P15 collision surface at that one
position, and adding a `title` purely as documentation can change the emitted
namespace and turn a loading schema into a load reject. [[ref]] owns what that
reject must say.

## Validator mapping

`title` emits **no validator** and **no adapter behavior**. It is an annotation
(§9.1): it never appears in the shared `Validate`, never runs in the parse or
encode adapter, and never causes a runtime pass/fail in either direction. Its
entire effect is at generation time, in the emitted doc comment. There is
nothing to check at runtime and nothing to test at the (de)serializer boundary.

## Property-testing matrix

### Accepted (positive)

| Shape                                         | Example                                                                                                               |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| Title on a `$defs` type                       | `{title:"User account", type:"object", …}` → type doc summary                                                         |
| Title on a property                           | `properties:{email:{title:"Email address", type:"string"}}` → field doc summary                                       |
| Title + description                           | `{title:"User", description:"A registered user.", type:"object"}` → summary + body                                    |
| Title on a scalar member                      | `{title:"Age in years", type:"integer"}`                                                                              |
| Title sibling of a `$ref` (merged, last-wins) | `{$ref:"#/$defs/User", title:"Account"}` → the use-site `title` overrides the target's when they differ (see [[ref]]) |
| Differing merged titles (last-wins)           | `allOf:[{title:"A"},{title:"B"}]` → `"B"` (see [[allOf]])                                                             |

### Rejected at load time (negative)

| Reason                        | Example                                                                           |
| ----------------------------- | --------------------------------------------------------------------------------- |
| Non-string value              | `{title:42}`, `{title:["a"]}`, `{title:null}` — `null` is a value, not an absence |
| Empty / whitespace-only       | `{title:""}`, `{title:"  "}`                                                      |
| Multi-line (prose in a label) | `{title:"User\naccount"}` (use `description`)                                     |

### Runtime fixtures

None. `title` has no runtime behavior — it neither validates nor (de)serializes.
Its only observable output is the generated doc comment, covered by
generation-snapshot tests, not runtime fixtures.

## Interactions

- **[[description]]**: the sibling annotation and co-owner of the doc comment.
  `title` is the summary line; `description` is the body. The shared doc-comment
  machinery (placement, wrapping, escaping) is specified in [[description]];
  `title` only fills its summary slot.
- **[[properties]]**: owns the field/type **naming** policy that `title`
  deliberately does **not** participate in. A property's `title` decorates the
  generated member's doc comment; it never renames the member (the name stays
  the property key, per the resolved policy).
- **[[ref]]**: a `title` **sibling** of a `$ref` is not rejected — the `$ref`
  and its siblings are rewritten to an implicit [[allOf]] and merged (see
  [[ref]]); because the rewrite puts the siblings last, a use-site `title`
  **overrides** the referenced target's under the last-wins rule below.
- **[[allOf]]**: merges (P6) — explicit or via a `$ref` sibling — can bring
  multiple `title`s onto one node; identical values dedup, differing ones
  resolve **last-wins** (see [[allOf]]), never a reject.
- **[[services]]**: the Nexus envelope decorates services/operations with
  `description` (see the envelope's recognized members); `title` is a **schema**
  annotation and applies to the types under `$defs` and their members, not to
  the envelope root.
- **[[type]]**, **[[nullability]]**, **[[required]]**: fully orthogonal —
  `title` changes neither the type, its nullability, nor its presence.

## Ecosystem variance

| Source dialect        | Action                                                              |
| --------------------- | ------------------------------------------------------------------- |
| JSON Schema 2020-12   | Native annotation (§9.1). Rendered as the doc-comment summary line. |
| OpenAPI 3.1           | Adopts 2020-12 — `title` native, same annotation semantics.         |
| OpenAPI 3.0           | `title` present; same annotation semantics.                         |
| Swagger 2.0 / draft-4 | `title` present; same annotation semantics.                         |

`title` is universal across dialects and needs no rewrite. The
**one substantive divergence is what we deliberately do *not* do**: much of the
codegen ecosystem treats `title` as a **type/model-naming input**, which we
refuse.

- **openapi-generator** uses the `title` attribute when flattening inline /
  composed schemas to *name* the generated model (falling back to `InlineObject`
  when absent). This is a documented footgun: the title-derived name
  **skips the normal name-transformation function**, so it diverges from the
  generator's own naming conventions and can cause import failures — the subject
  of issue #5248, "Do not use the title attribute to control code generation."
- **datamodel-code-generator** exposes `--use-title-as-name`, an **opt-in** flag
  to use `title` as the class name — off by default, precisely because it is
  surprising.

We take neither behavior. Our type/field names come from the `$defs` key and the
[[properties]] resolved policy — stable across revisions (P13), run through one
case-mapping pipeline for cross-language agreement (P1), and de-collided in one
namespace pass (P15). `title` stays what the spec says it is: a display label,
surfaced only in the doc comment. This is stricter than the ecosystem in the
sense that a `title` will *never* influence an identifier here — a schema that
relied on openapi-generator's title-naming gets a name derived from its `$defs`
key instead (and keeps the `title` as its doc summary).

## See also

- [[description]] — the sibling annotation; co-owns the doc comment (`title` =
  summary line, `description` = body) and specifies the shared doc-comment
  machinery.
- [[properties]] — owns the field/type naming policy `title` deliberately does
  not touch.
- [[ref]] — a `title` sibling of a `$ref` merges via the implicit-`allOf`
  rewrite; use-site value wins (last-wins).
- [[default]] — the other basic-metadata annotation (§9.2); unlike `title` it
  carries operational semantics (omit-unset, materialize-on- read), whereas
  `title` is purely a doc comment.
- [[services]] — the Nexus envelope uses `description` on services/operations;
  `title` decorates the `$defs` types and members.
