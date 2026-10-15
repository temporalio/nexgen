# `description`

Source: JSON Schema 2020-12, Validation vocabulary, §9.1 "A Vocabulary for Basic
Meta-Data Annotations → title and description".

Free-form prose explaining the schema it sits on — the primary human
documentation for a generated type, member, service, or operation. In the spec
it is a **pure annotation** — it never affects validation, and its text never
supplies a *name*. It is not, however, inert on the emitted shape everywhere:
beside a `$ref` it is a merge conjunct, and the merge materializes a type (see
Type mapping). Its single operational role is to become the
**body of the generated doc comment** (a Go `//` block, a TS JSDoc, a Python
class or attribute docstring, a Java Javadoc). Because it is the primary doc
source, this spec **owns the shared doc-comment machinery** — assembly order,
line-wrapping, and per-language escaping — that its sibling [[title]] defers to
it.

## Spec summary

Verbatim (2020-12 validation, §9.1):

> The value of both of these keywords MUST be a string.

> Both of these keywords can be used to decorate a user interface with
> information about the data produced by this user interface. A "title" will
> preferably be short, whereas a description will provide explanation for the
> purpose of the instance described by this schema.

Distilled:
- An **annotation**, not an assertion: per spec it
  **never changes whether an instance validates**. A wire value is judged by the
  schema's assertions alone, `description` present or not.
- Its value **MUST be a string** (the spec's own MUST).
- **Prose, possibly multi-line** — the spec's own role for it ("explanation for
  the purpose"). Unlike [[title]] (a short label, restricted to a single line),
  a `description` may span paragraphs.
- Applies to **any subschema** — a `$defs` type, a property subschema, an array
  `items` subschema — and, on the Nexus envelope, to **services and operations**
  (see [[services]]).

## Support decision

**Support:** yes — as the **doc-comment body only**. It emits no validator, no
adapter behavior, and no type/field name.

The defining choices (citing [[PRINCIPLES.md]]):
- **Annotation, no runtime effect (P10/P12).** `description` contributes **no**
  constraint predicate to the shared `Validate` and **no** parse/encode adapter
  logic. It is inert at runtime in both directions; it touches only the
  *generated source text*.
- **Never an identifier (P13, P15, P1).** Like [[title]], `description` never
  names a type, field, service, or operation — names come from the `$defs` key,
  the [[properties]] resolved policy, and the [[services]] keys. (It can still
  *cause* a name to be synthesized, beside a `$ref`; that name is
  position-derived — see Type mapping.) Routing prose into the identifier
  namespace would make a cosmetic copy-edit a breaking rename (P13), multiply
  the collision surface (P15), and bypass the case-mapping pipeline that keeps
  names identical across languages (P1). (`description` is even less tempting
  than `title` here — no ecosystem generator names types from it — but the rule
  is the same.)
- **The doc-comment body (P2).** `description` is the prose body of the
  generated doc comment, rendered to read like a comment a human wrote (P2). It
  composes with [[title]] as **summary line + body** (assembly order below).
  Authors' Markdown is passed through **verbatim** (escaped only for the comment
  block); the generator does not render or reinterpret it — what the author
  wrote is what lands in the source (see Ecosystem variance). Word-wrapping to
  the format line length is the one transformation applied, and it preserves the
  author's line breaks, paragraph breaks and leading indentation — see
  *Wrapping* below.

Loader behavior:
- `description` value **not a string** → **reject** (P7.1; and the spec's own
  MUST). `{description: 42}`, `{description: {...}}`, and `{description: null}`
  — an explicit `null` is a non-string *value*, not an absent keyword, so it
  rejects rather than being dropped.
- `description` an **empty or whitespace-only** string → **reject** as
  degenerate: it renders an empty doc body, which is dead metadata and signals
  author confusion (P7.1). Drop it, or give it text.
- `description` containing a **control character** — any code point below U+0020
  other than line feed and tab, notably U+0000 → **reject**. Degenerate metadata
  in the same class as the empty string, and unrepresentable in a comment in at
  least two targets, so rejecting once at load is the only form that keeps the
  four accept sets identical (P1); escaping it four different ways is not.
  Diagnostic names the offending code point.
- `description` as a **sibling of `$ref`** is **not** rejected: a
  sibling-bearing `$ref` is rewritten to an implicit `allOf` and **merged** (see
  [[ref]]), so a use-site `description` folds into the merged schema under the
  [[allOf]] rule below — deduped against the target's own `description` when
  identical, and when they differ the use-site `description` **wins** (it is
  last in the rewrite; see [[ref]]). This is the idiomatic way to override a
  shared type's prose at one use site.
- Multiple `description`s applicable to one node after an [[allOf]] merge (P6),
  whether from an explicit `allOf` or a `$ref` sibling → **last-wins**:
  identical values dedup, and when they differ the **last-merged** `description`
  survives (see [[allOf]]). A differing description is a deterministic override,
  never a reject. Mirrors [[title]] and [[default]].

Unlike [[title]], a **multi-line** `description` is **accepted** — prose is its
purpose. Embedded blank lines are preserved as paragraph breaks (see Doc-comment
assembly).

## Type mapping

**None of its own**, and the prose contributes no identifier (rationale in
Support decision). Beside a `$ref` the merge does materialize a type, but the
shape and the name are the merge's, never the prose's (see the P15 note below).
Its own materialization is the **body of the doc comment**, which carries no
type information:

| Placement                        | Where the body lands                                                                                                                                                                                                                                                         |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `$defs` type                     | doc comment on the generated **type** (Go `type` / TS `interface` / Python class / Java class)                                                                                                                                                                               |
| property subschema               | doc comment on the generated **field/member** — unless the subschema itself materializes a type (an inline object, or a `$ref` with siblings), in which case the prose travels with that type and the member takes its synthesized fallback line ([[properties]], [[allOf]]) |
| service (envelope)               | doc comment on the language-specific service binding defined by [[services]]                                                                                                                                                                                                 |
| operation (envelope)             | doc comment on the language-specific operation entry defined by [[services]]                                                                                                                                                                                                 |
| `items` / other inline subschema | doc comment on the declaration synthesized from that subschema, if any; otherwise dropped (nowhere to attach)                                                                                                                                                                |

Per-language block and placement:

| Language   | Doc-comment mechanism                                                                                                                                                                                                          |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Go         | `// ` line-comment block above the `type`/field/method, **name-led first line** (see below).                                                                                                                                   |
| TypeScript | `/** … */` JSDoc above the `interface`/field.                                                                                                                                                                                  |
| Python     | class **docstring** for a type/service; for a **field**, an **attribute docstring** — a bare string literal on the line(s) immediately after the field declaration, which every Python documentation tool and editor picks up. |
| Java       | `/** … */` Javadoc above the class/getter/method.                                                                                                                                                                              |

On an ordinary declaration `description` synthesizes no identifier and has no
P15 surface. **Beside a `$ref` it does**: `description` is not in the inert
sibling class, so the sibling triggers the implicit merge, which materializes a
standalone use-site type and its package-level identifiers ([[ref]], [[allOf]]).
Those names are derived from the **position** — the enclosing type plus the
member — never from the prose. So `description` **does** carry a P15 collision
surface at that one position, and a documentation-only edit can change the
emitted namespace and turn a loading schema into a load reject. [[ref]] owns
what that reject must say.

## Doc-comment assembly (shared machinery)

This is the mechanism [[title]] defers to. A generated doc comment is assembled
from up to three parts, in order:

1. **Summary line** — the [[title]], if present.
2. **Body** — the `description`, if present (may be multiple paragraphs).
3. **Tags** — generator-emitted trailers (e.g. an experimental/deprecated
   warning), if any.

Layout rules:
- With **both** a title and a description: summary line, one blank comment line,
  then the body.
- With **only** a description: the body opens the comment (its first line
  becomes the summary in doc tools that treat the first sentence as one —
  Javadoc/JSDoc/PEP 257).
- With **only** a title: just the summary line (see [[title]]).
- With **neither**: Go still emits a comment — see the fallback rule below
  (PRINCIPLES.md, Go §1). TypeScript/Python/Java simply emit none (there is no
  per-language mandate that every declaration carry a doc comment in those
  languages, unlike Go's `golint`-driven convention).
- **Paragraphs** in the body (blank lines in the source string) are preserved as
  blank comment lines.

**Go — the name-led first line.** Idiomatic Go doc comments begin with the
identifier being described (`godoc`/`golint`: `// User is …`). The rule applies
to **every exported declaration the generator emits** — type, field, method,
service binding, operation entry and client — and governs whatever text
**opens** that declaration's comment:
- title present → the summary line is `// <Name> <title>` (see [[title]]).
- title absent, description present → the identifier is prefixed to the first
  line of the description: `// <Name> <first line of description>`.
- **neither present** → the generator supplies a minimal name-led fallback line
  instead of leaving the declaration undocumented (e.g.
  `// <Name> is generated from the corresponding JSON Schema definition.` for a
  type, `// <Name> corresponds to the "<jsonName>" JSON property.` for a field).
  This is a Go-wide mandate, not a `title`/`description` behavior — see
  PRINCIPLES.md, Go §1.
- **Stutter guard:** if that opening text already begins with `<Name>`
  (case-insensitively) **as a whole word**, the identifier is not doubled — the
  text is emitted as-is. Same guard, and the same word-boundary requirement, as
  [[title]].

**Wrapping.** Word-wrapping to the format line length is the **only**
transformation the generator applies to authored prose. Each part is wrapped to
88 columns minus the current indent and the comment prefix width (`// `, ` * `,
docstring indent). Wrapping is **per source line**: an author's explicit line
breaks, paragraph breaks, and each line's **leading indentation** are preserved
— a wrapped continuation repeats the indent and reflows against the reduced
width — so an indented code block stays a code block and a nested list item
stays nested. Runs of interior whitespace are normalized to single spaces; no
other rewriting occurs.

Wrapping must not change what a doc tool *means*. Javadoc reads a `@`-tag as a
block tag only when it opens a line, so a wrap point that lifts an authored
mid-sentence `@`-tag onto its own line converts inert prose into a live tag —
and because the wrap point depends on the enclosing identifier's length and the
declaration's indent, the identical authored string can land inert at one site
and live at another. Our decision: **the wrapper never breaks a line immediately
before a token a target's doc tool treats as positional.**
*(Status: unimplemented — the wrapper is position-blind today.)*

**Escaping.** Each part is escaped so the prose cannot break out of the comment,
break the host language's lexer, or
**become a directive the toolchain acts on**. The three are one obligation, and
the table below is the current spelling of it, not its limit: a host construct
not listed here still has to be neutralized.

| Language   | Escaping                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | `//` line comments have no terminator to escape, but they *do* have a **directive grammar**: a physical line whose first token is `+build` is a legacy build constraint and one beginning `go:` is a tool directive, either of which `gofmt` may hoist above the `package` clause. Each physical line is prefixed `// `, and the directive punctuation is backslash-neutralized (`+build` → `\+build`, `go:` → `go\:`) so the text cannot be acted on. Leading whitespace is not a sufficient escape because Go's constraint parser trims it. |
| TypeScript | replace `*/` → `* /` so the body can't close the JSDoc block early.                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Python     | escape `\` → `\\` and `"""` → `\"\"\"` so the body can't terminate the docstring.                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Java       | replace `*/` → `* /`; HTML-escape `&`→`&amp;`, `<`→`&lt;`, `>`→`&gt;`, and `\`→`&#92;` (applied after the `&` pass). The backslash is not cosmetic: JLS §3.3 translates Unicode escapes in a first lexical phase that covers comments, so a `\u` not followed by four hex digits is a hard compile error, and a well-formed one is silently substituted — neither of which any other target does. A Javadoc body is HTML, so the entity renders as the character the author wrote.                                                            |

## Validator mapping

`description` emits **no validator** and **no adapter behavior**. It is an
annotation (§9.1): it never appears in the shared `Validate`, never runs in the
parse or encode adapter, and never causes a runtime pass/fail in either
direction. Its entire effect is at generation time, in the emitted doc comment.

## Property-testing matrix

### Accepted (positive)

| Shape                                               | Example                                                                                                                                  |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| Description on a `$defs` type                       | `{description:"A registered user account.", type:"object", …}`                                                                           |
| Description on a property                           | `properties:{email:{description:"Contact address; used for password resets.", type:"string"}}`                                           |
| Title + description                                 | `{title:"User", description:"A registered user account.", type:"object"}` → summary + body                                               |
| Multi-paragraph description                         | a `description` with an embedded blank line → two comment paragraphs                                                                     |
| Description sibling of a `$ref` (merged, last-wins) | `{$ref:"#/$defs/User", description:"Use-site note."}` → the use-site `description` overrides the target's when they differ (see [[ref]]) |
| Differing merged descriptions (last-wins)           | `allOf:[{description:"A"},{description:"B"}]` → `"B"` (see [[allOf]])                                                                    |
| Description on a service/operation                  | envelope `services.ChatService.description` → service interface doc comment (see [[services]])                                           |

### Rejected at load time (negative)

| Reason                         | Example                                                         |
| ------------------------------ | --------------------------------------------------------------- |
| Non-string value               | `{description:42}`, `{description:{...}}`, `{description:null}` |
| Empty / whitespace-only        | `{description:""}`, `{description:"   "}`                       |
| Control character in the prose | a `description` whose text carries U+0000                       |

### Runtime fixtures

None. `description` has no runtime behavior — it neither validates nor
(de)serializes. Its only observable output is the generated doc comment, covered
by generation-snapshot tests, not runtime fixtures.

## Interactions

- **[[title]]**: the sibling annotation. `title` is the summary line;
  `description` is the body. This spec owns the shared assembly, wrapping, and
  escaping; [[title]] fills the summary slot.
- **[[properties]]**: owns the field/type naming policy that `description` does
  not participate in. A property's `description` decorates the generated
  member's doc comment; it never renames the member.
- **[[ref]]**: a `description` **sibling** of a `$ref` is not rejected — the
  `$ref` and its siblings are rewritten to an implicit [[allOf]] and merged (see
  [[ref]]); because the rewrite puts the siblings last, a use-site `description`
  **overrides** the referenced target's under the last-wins rule below.
- **[[allOf]]**: merges (P6) — explicit or via a `$ref` sibling — can bring
  multiple `description`s onto one node; identical values dedup, differing ones
  resolve **last-wins** (see [[allOf]]), never a reject.
- **[[services]]**: the Nexus envelope decorates services and operations with
  `description` (the envelope's recognized doc member — it uses `description`,
  not `title`); each becomes the doc comment on the generated service interface
  / operation method.
- **[[type]]**, **[[nullability]]**, **[[required]]**: fully orthogonal —
  `description` changes neither the type, its nullability, nor its presence.

## Ecosystem variance

| Source dialect        | Action                                                      |
| --------------------- | ----------------------------------------------------------- |
| JSON Schema 2020-12   | Native annotation (§9.1). Rendered as the doc-comment body. |
| OpenAPI 3.1           | Adopts 2020-12 — `description` native; CommonMark per OAS.  |
| OpenAPI 3.0           | `description` present; CommonMark per OAS.                  |
| Swagger 2.0 / draft-4 | `description` present; same annotation semantics.           |

`description` is universal across dialects and needs no rewrite, and — unlike
[[title]] — no generator repurposes it as an identifier, so there is no naming
footgun to guard against.

The one convention to note is **Markdown**: OpenAPI declares `description` to be
CommonMark. We **pass the text through verbatim** into the doc comment (escaped
only for the comment block, and word-wrapped per *Wrapping*), and do **not**
render it to another format or strip the markup. This is the P2 choice — the
author's prose lands in the generated source as written, which is what a
hand-written comment would contain; downstream doc tooling (godoc, TypeDoc,
Sphinx, Javadoc) interprets whatever markup it supports,
**at the position the author wrote it**.

## See also

- [[title]] — the sibling annotation; supplies the summary line that opens the
  doc comment whose body is `description`.
- [[services]] — the Nexus envelope's service/operation `description`s become
  the doc comments on the generated interface and methods.
- [[properties]] — owns the naming policy `description` does not touch.
- [[ref]] — a `description` sibling of a `$ref` merges via the implicit-`allOf`
  rewrite; use-site value wins (last-wins).
- [[default]] — the other basic-metadata annotation (§9.2); unlike `description`
  it carries operational semantics, whereas `description` is purely a doc
  comment.
