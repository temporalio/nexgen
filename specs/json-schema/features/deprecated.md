# `deprecated`

Source: JSON Schema 2020-12, Validation, §9.3 "A Vocabulary for Basic Meta-Data
Annotations → deprecated".

Marks the schema location — a type, member, service, or operation — as
**discouraged for continued use**: still valid on the wire, but callers should
migrate away. In the spec it is a **pure annotation** — it never affects
validation. **Supported** — it lowers into each target's
**native deprecation marker** (Go's `// Deprecated:` godoc paragraph, a TS JSDoc
`@deprecated` tag, a Java `@Deprecated` annotation, a Python PEP 702
`@deprecated(…, category=None)` marker), so editors, compilers, and type
checkers surface the marker as far as that native mechanism reaches. Two of
those reaches are genuinely limited: TypeScript's JSDoc deprecation is a
language-service signal rather than a `tsc` diagnostic, and PEP 702 defines no
field form. That is a limit of the mechanism, not a licence for a marker the
generator has placed where its tool cannot see it — Go's `// Deprecated:` is
only live as its own godoc paragraph, so emitting it without the preceding blank
`//` line is a generator defect, not a toolchain limit. The marker has no
validation, serialization, or access-warning effect. It is a **tag** in the
doc-comment assembly [[description]] owns; because `deprecated` is boolean-only
it carries no message of its own, so the human rationale comes from the sibling
[[description]].

## Spec summary

Verbatim (2020-12 validation, §9.3):

> The value of this keyword MUST be a boolean.

> When multiple occurrences of this keyword are applicable to a single
> sub-instance, applications SHOULD consider the instance location to be
> deprecated if any occurrence specifies a true value.

> If "deprecated" has a value of boolean true, it indicates that applications
> SHOULD refrain from usage of the declared property. It MAY mean the property
> is going to be removed in the future.

> A root schema containing "deprecated" with a value of true indicates that the
> entire resource being described MAY be removed in the future.

Distilled:
- Value **MUST be a boolean**; default `false`.
- Multiple applicable occurrences **OR together**: **any** `true` ⇒ the location
  is deprecated. This is *not* the last-wins merge that [[description]] /
  [[title]] / [[default]] use (see Loader behavior).
- An **annotation**, not an assertion: per spec it
  **never changes whether an instance validates**. A wire value is judged by the
  schema's assertions alone, `deprecated` present or not.
- Applies to **any subschema** — a `$defs` type, a property subschema — and, on
  the Nexus envelope, to **services and operations** (see [[services]]). A
  root-schema `deprecated: true` deprecates the whole generated type.

## Support decision

**Support:** yes — as a **native deprecation marker** (plus a doc-comment tag)
on the declaration it decorates. It emits no validator and no adapter behavior.

The defining choices (citing [[PRINCIPLES.md]]):
- **Idiomatic native markers (P2).** Unlike [[examples]] — the other metadata
  annotation that is merely *ignored* — `deprecated` maps to a
  **first-class deprecation mechanism in every target**: godoc's `Deprecated:`
  convention, JSDoc `@deprecated`, `java.lang.Deprecated`, PEP 702
  `@deprecated`. Emitting the native marker is what makes the generated code
  read like code a human wrote, and it makes the deprecation *actionable*
  wherever the target tooling can carry it — and where the generator controls
  the placement, it must produce the form the tool recognizes rather than one it
  ignores. TypeScript exposes JSDoc deprecation through language services rather
  than a `tsc` diagnostic; Python's PEP 702 signal applies to decorated types
  and callables, while its field/operation `Annotated` form is
  documentation-only. So we emit, not ignore.
- **No modeling problem to block it.** [[readOnly]] / [[writeOnly]] reject
  because their directional intent has no single-type lowering; `deprecated` has
  no such fork — it merely *marks* a location and leaves the type, its fields,
  and the wire shape unchanged. Every target has an **authored** declaration to
  attach the marker to: the type or the member the keyword was written on. It
  never propagates to a *synthesized* declaration — a per-member closed-value
  type, a union-branch wrapper — because the author did not declare those, so a
  marker on them is gratuitous and would make the marker count target-dependent.
- **Annotation, no runtime effect (P10/P12/P1).** `deprecated` contributes
  **no** constraint predicate to the shared `Validate` and **no** parse/encode
  adapter logic. A deprecated value is accepted and round-trips exactly like a
  non-deprecated one. Its entire effect is at generation time, in the emitted
  declaration — with no runtime validation, serialization, or access-warning
  effect in any target. Python is held to the same bar: it uses PEP 702's
  `category=None` so even its `@deprecated` marker raises no access-time
  `DeprecationWarning` (see Type mapping); we deliberately do **not** emit the
  runtime `Field(deprecated=True)` form, keeping the four targets in parity
  (P1).
- **Never an identifier (P13/P1/P15).** `deprecated` never names a type, field,
  service, or operation. On an ordinary declaration it therefore synthesizes
  nothing and adds nothing to the per-scope collision pass. Beside a `$ref` it
  must stay that way too: the marker belongs on the **member** bound to the
  reference, so the reference survives and no clone, and no new identifier, is
  created (see Loader behavior).

Loader behavior:
- `deprecated` value **not a boolean** → **reject** (P7.1; the spec's own MUST).
  Diagnostic names the offending value:
  `deprecated must be a boolean, got "true"` for `{deprecated: "true"}`,
  likewise `{deprecated: 1}`.
- `deprecated: false` → **accepted and ignored** — it is the explicit form of
  the default (not deprecated), so no marker is emitted and no diagnostic is
  raised. This is the [[examples]] ergonomic exception to P7.1:
  `deprecated: false` is pervasive in OpenAPI / imported schemas as an explicit
  "not deprecated" flag, and it is unambiguous and inert, so rejecting it would
  force authors to strip meaningful, harmless metadata just to generate.
  (Contrast [[readOnly]] / [[writeOnly]], whose `false` rejects — but those
  keywords are rejected wholesale regardless.)
- `deprecated` as a **sibling of `$ref`**, **of either value**, leaves the
  reference intact — the keyword is never a merge conjunct, because it
  deprecates the *member* and asserts nothing about the referenced value. The OR
  is applied at the **use site**: the member keeps the referenced type and the
  marker lands on the member, which is what the Type-mapping table below says
  and what all four targets can express (a Go struct-field doc comment, a TS
  property JSDoc tag, a Python `Annotated`, a Java getter annotation). The
  sibling must **not** trigger the implicit-`allOf` fold ([[ref]], [[allOf]]): a
  fold would clone the target into a use-site type, move the marker off the
  member the author annotated, and — for `deprecated: false`, which means
  exactly what omitting the keyword means — change a member's type with no
  marker and no diagnostic to explain it.
- `deprecated` on a [[nullability]] `null` branch → **reject**, of either value:
  a `null` branch must be exactly `{type: "null"}` with no siblings, an
  invariant [[nullability]] owns and this keyword does not override. To mark a
  **nullable member**, write `deprecated` on the wrapper beside the `oneOf`; on
  the **non-null branch** it is an inline subschema with no declaration and is
  therefore dropped, so the two spellings are not interchangeable.
- `deprecated` at a **document root** — a definitions-only root or a Nexus
  envelope root — is **accepted**: `true` deprecates the generated root type
  where there is one, and is otherwise dropped like any marker with no
  declaration to attach to. It never makes a root "model-shaped": an imported
  document routinely carries a document-level annotation, and a reject there
  would have to describe a model the author never wrote.
- Multiple `deprecated` occurrences applicable to one node — the branches of an
  explicit [[allOf]] merge (P6), or a `$ref` sibling ORed against the resolved
  target's own value — → **OR**: if **any** applicable occurrence is `true`, the
  location is deprecated. This differs from [[description]] / [[title]] /
  [[default]], which resolve last-wins; `deprecated` follows the spec's own
  any-true-wins rule (§9.3). A merged `false` occurrence is ignored (as in
  isolation), so it never suppresses a sibling `true` — the location is
  deprecated iff some applicable occurrence is `true`.
- **Orthogonal** to [[required]], [[default]], [[const]], and [[nullability]]: a
  deprecated member may still be required, carry a default, and be nullable.
  `deprecated` changes none of those; it only adds the marker.

## Type mapping

**None.** `deprecated` does not change the emitted type and contributes no
identifier. Its sole materialization is the **native deprecation marker** (and a
doc-comment tag) on the declaration generated for the schema location it was
written on:

| Placement                            | What gets marked                                                                                                                                                                                                                                                                         |
| ------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `$defs` type                         | the generated **type** (Go `type` / TS `interface` / Python class / Java class)                                                                                                                                                                                                          |
| property subschema                   | the generated **field / getter / member**                                                                                                                                                                                                                                                |
| service (envelope)                   | the generated **service interface**; see [[services]]                                                                                                                                                                                                                                    |
| operation (envelope)                 | the generated **operation method**; see [[services]]                                                                                                                                                                                                                                     |
| inline subschema with no declaration | dropped — nowhere to attach a marker. This is the rule for every such position: an [[items]] element, a typed-map value, a [[contains]] matcher, a [[propertyNames]] key subschema, and the non-null branch of a degenerate nullability `oneOf`. None of them rejects; all of them drop. |

Per-language marker (the meat — a native construct, not just comment text):

| Language   | Native marker                                                                                                                                                                                                      | Effect for consumers                                                                                                                                                                                                                             |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Go         | a `// Deprecated: This <kind> is deprecated.` paragraph in the doc comment — **its own paragraph**, preceded by a blank `//` line, at every declaration kind (godoc convention; generic reason — see below)        | `staticcheck` SA1019 flags every use and `go doc` renders it, but **only** because the marker opens a paragraph: the analyzer splits the comment on blank lines and requires a part to *begin* `Deprecated: `. A doc-comment tag, not a keyword. |
| TypeScript | a bare JSDoc `@deprecated` tag in the `/** … */` block                                                                                                                                                             | Editors and language services strike through or suggest at call sites; `tsc` emits no deprecation diagnostic.                                                                                                                                    |
| Python     | PEP 702 `@deprecated("…", category=None)` (`typing_extensions` backport) on a generated type/service; fields and operation attributes use `Annotated[T, deprecated("…", category=None)]` as documentation metadata | Type/service decorators are recognized by PEP 702-aware checkers. Field and operation attribute metadata is not a PEP 702 warning construct. `category=None` suppresses access-time `DeprecationWarning`.                                        |
| Java       | the `@Deprecated` annotation on the type / getter / method, paired with a Javadoc `@deprecated This <kind> is deprecated.` tag                                                                                     | `javac` warns at every use; IDEs strike-through; `javadoc` renders the tag text in the "Deprecated." block and `deprecated-list.html`.                                                                                                           |

The marker attaches to a declaration that already exists; no new identifier is
synthesized for it, in any position — including beside a `$ref`, where the
reference survives and the marker lands on the member (see Loader behavior). So
`deprecated` contributes nothing to the per-scope collision pass (P15).

## Doc-comment tag and the message limitation

`deprecated` is the **tag** slot (assembly part 3) of the doc comment
[[description]] owns — summary line ([[title]]) → body ([[description]]) →
**tags**. It renders as a trailer in the doc comment *and*, where the target has
one, the native marker (annotation / decorator / godoc paragraph). The shared
placement, wrapping, and escaping rules are those [[description]] specifies;
this keyword only fills the tag slot.

`deprecated` is **boolean-only** — the spec gives it no message string — and the
sibling [[description]] is **never copied** into the marker's message slot: the
rationale, replacement pointer, or removal timeline is prose that lives in the
doc-comment body ([[description]]) directly above the marker, and copying it in
would repeat the same text twice in the generated source.

The marker is emitted **bare** only where the native construct carries no text
slot at all — a JSDoc `@deprecated` tag (TS). The other three targets have a
text slot that their tooling renders, so they carry a **generic** reason,
`This <kind> is deprecated.` — where `<kind>` ∈ type / field / service /
operation, already known from the placement:
- **Java** — the `@Deprecated` annotation paired with
  `@deprecated This <kind> is deprecated.`; `javadoc` renders the tag text into
  the type's "Deprecated." block and into `deprecated-list.html`, and a bare tag
  renders an empty explanation there.
- **Go** — the godoc convention is a `// Deprecated: <text>` paragraph; a bare
  `Deprecated:` reads incomplete, so the generic reason fills it:
  `// Deprecated: This field is deprecated.`
- **Python** — PEP 702 `deprecated(msg, category=None)` takes a **mandatory**
  message argument, so both forms carry the generic string:
  `@deprecated("This type is deprecated.", category=None)` on a declaration,
  `Annotated[T, deprecated("This field is deprecated.", category=None)]` on a
  field. (`category=None` is what keeps the marker static-only — see Type
  mapping.)

The rationale the author actually wrote still travels with the symbol, in the
doc-comment body, for every target — adjacent to the marker in Go, TypeScript
and Java, and *below* it in Python, whose decorator necessarily precedes the
docstring it documents.

## Validator mapping

`deprecated` emits **no validator** and **no adapter behavior**. It is an
annotation (§9.3): it never appears in the shared `Validate`, never runs in the
parse or encode adapter, and never causes a runtime pass/fail in either
direction (**P12**). A deprecated value deserializes, validates, and
re-serializes identically to a non-deprecated one.

There is no runtime validation, serialization, or access-warning effect in any
target. Python emits `category=None`, which suppresses access-time
`DeprecationWarning`; Python also retains `__deprecated__`, and Java's
`@Deprecated` is runtime-visible metadata. Generated code does not inspect
either artifact.

## Property-testing matrix

### Accepted (positive)

| Shape                                         | Handling                                                                                                                                                      |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Deprecated `$defs` type                       | `{deprecated:true, type:"object", …}` → marker on the generated type                                                                                          |
| Deprecated property                           | `properties:{legacyId:{deprecated:true, type:"string"}}` → marker on the field/getter                                                                         |
| Deprecated + description                      | `{deprecated:true, description:"Use `uuid` instead."}` → body carries the rationale, tag/marker below it                                                      |
| Deprecated + required                         | `required:["legacyId"]` with `legacyId:{deprecated:true, …}` → still required; marker added (orthogonal)                                                      |
| Deprecated + default                          | `{deprecated:true, type:"string", default:"x"}` → default machinery unchanged; marker added                                                                   |
| Merged occurrences OR to true                 | `allOf:[{deprecated:true},{…}]` → deprecated (any true wins; see [[allOf]])                                                                                   |
| `deprecated: false` (explicit not-deprecated) | `{deprecated:false, type:"string"}` → accepted, no marker, no diagnostic                                                                                      |
| Deprecated service / operation                | envelope `services.ChatService.operations.legacyCall.deprecated` → marker on the generated method (see [[services]])                                          |
| Sibling of a `$ref` (either value)            | `{$ref:"#/$defs/User", deprecated:true}` → member keeps type `User`, marker on the member; `deprecated:false` leaves the member byte-identical to omitting it |
| At a document root                            | a definitions-only or Nexus envelope root carrying `deprecated` → accepted; marks the root type if there is one, else dropped                                 |
| Member with a closed value set                | `{type:"string", enum:["a","b"], deprecated:true}` → one marker, on the member; the synthesized closed-value type is left unmarked                            |

### Rejected at load time (negative)

| Reason                                        | Example                                                                           |
| --------------------------------------------- | --------------------------------------------------------------------------------- |
| Value not a boolean                           | `{deprecated:"true"}`, `{deprecated:1}`, `{deprecated:null}`                      |
| On a nullability `null` branch (either value) | `{oneOf:[{type:"string"},{type:"null", deprecated:false}]}` (see [[nullability]]) |

### Runtime fixtures

None. `deprecated` has no runtime behavior — it neither validates nor
(de)serializes. Its only observable output is the generated declaration
(marker + doc tag), covered by generation-snapshot tests. The Python snapshot
asserts the `category=None` form, which is a text assertion: proving that
accessing a deprecated symbol raises no `DeprecationWarning` needs a runtime
assertion that executes the access.

## Interactions

- **[[description]]**: owns the doc-comment assembly `deprecated` plugs into as
  a **tag**, and supplies the rationale prose the bare marker lacks (the boolean
  carries no message). The tag renders directly below the `description` body.
- **[[title]]**: the summary line of the same doc comment; unaffected by
  `deprecated`.
- **[[properties]]**: `deprecated` decorates a **member**, never renames it —
  the field name still comes from the resolved naming policy. The marker
  attaches to the generated field/getter.
- **[[services]]**: the Nexus envelope can mark a service or operation
  `deprecated`; the marker lands on the generated interface / method.
- **[[allOf]]** / **[[ref]]**: an explicit `allOf` can bring multiple
  `deprecated` occurrences onto one node; they **OR** (any true ⇒ deprecated),
  unlike the last-wins annotations. As a `$ref` **sibling** the keyword does not
  merge at all — the reference stays a reference and the OR is applied at the
  use site (see Loader behavior).
- **[[required]]**, **[[default]]**, **[[const]]**, **[[nullability]]**: fully
  orthogonal — a deprecated member keeps its presence, default, fixed value, and
  nullability; only the marker is added.
- **[[examples]]** / **[[readOnly]]** / **[[writeOnly]]**: the sibling metadata
  annotations. `deprecated` is the **supported** one — it lowers cleanly to a
  native marker, where [[examples]] is ignored (inert) and [[readOnly]] /
  [[writeOnly]] reject (directional).

## Ecosystem variance

| Source dialect      | Action                                                                                                                                 |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12 | Native annotation (§9.3, added in 2019-09) → native deprecation marker.                                                                |
| OpenAPI 3.1         | Adopts 2020-12 — `deprecated` native on schemas → marker. OpenAPI's own `deprecated` on operations/parameters carries the same intent. |
| OpenAPI 3.0         | Has `deprecated` (boolean) on schemas, operations, and parameters, same intent → marker.                                               |
| draft-07            | **No `deprecated` keyword** (introduced in 2019-09); a draft-07 source simply has nothing to lower.                                    |

`deprecated` is a boolean with a single, consistent intent across every dialect
that has it, so no rewrite is needed — only the per-language marker choice
above.

## See also

- [[description]] — owns the doc-comment assembly `deprecated` renders into as a
  tag, and supplies the rationale prose the bare boolean lacks.
- [[title]] — the summary line of the same doc comment.
- [[services]] — the Nexus envelope marks services/operations `deprecated`; the
  marker lands on the generated interface/method.
- [[examples]] — the sibling metadata annotation that is *ignored* (inert), the
  contrast to this *supported* one.
- [[readOnly]] / [[writeOnly]] — the sibling metadata annotations that *reject*
  (directional, no single-type lowering).
- [[PRINCIPLES.md]] — **P2** (idiomatic, native markers), **P7/P7.1** (reject
  loudly with fix-its), **P10** (annotation, no advisory validation), **P12**
  (no adapter/validator effect), **P13/P15** (no synthesized identifier, no
  collision surface).
