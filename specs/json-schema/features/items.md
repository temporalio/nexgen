# `items`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.3.1.2 "Keywords
for Applying Subschemas to Arrays → items".

Applies one subschema to every array element. The structural backbone of every
generated list type — the array analog of [[properties]]: it is what turns
`type:"array"` into a *typed* collection (`[]T` / `T[]` / `list[T]` / `List<T>`)
rather than an opaque one. The element type comes from recursively mapping the
`items` subschema.

## Spec summary

Verbatim (2020-12 core, Applicator):

> The value of "items" MUST be a valid JSON Schema.

> This keyword applies its subschema to all instance elements at indexes greater
> than the length of the "prefixItems" array in the same schema object, as
> reported by the annotation result of that "prefixItems" keyword. If no such
> annotation result exists, "items" applies its subschema to all instance array
> elements.

> The annotation result of this keyword is a boolean true if the subschema was
> applied to any positions of the instance array. This annotation affects the
> behavior of "unevaluatedItems".

> Omitting this keyword has the same assertion behavior as an empty schema.

Distilled:
- `items` constrains every element (in our subset there is no [[prefixItems]],
  so "every element" is literal — see Support).
- The value is a full subschema; the element type is that subschema's mapping,
  applied recursively.
- Omitting `items` = empty schema = **no constraint on elements** (spec default:
  an untyped array).
- 2020-12 note: the draft-7 array-of-schemas *tuple* spelling of `items` moved
  to [[prefixItems]]; in 2020-12 `items` is **always a single schema**. An
  array-valued `items` is a stale-draft artifact.

## Support decision

**Support:** yes — the homogeneous list case (`type:"array"` + a single `items`
subschema).

An array with `items` emits a typed collection whose element type is the
recursive mapping of the `items` subschema (itself rejected at load if it is out
of subset, per [[type]] and **P7.1**).

Rationale (citing [[PRINCIPLES.md]]):
- **P2 (idiomatic output)**: a homogeneous element schema lowers cleanly to the
  one collection type every target has (`[]T` / `T[]` / `list[T]` / `List<T>`).
- **P7 / P7.1 (strict schema)**: the element schema must carry an explicit,
  supported `type` (or the [[nullability]] `oneOf` pattern); a shapeless element
  (`{}` / `true` / `false`) leaves the element type undecidable and is rejected
  with a located diagnostic.

Loader behavior:
- `type:"array"` **without** `items` → **reject** per **P7.1**. Per spec this is
  a valid "array of anything", but the typed-codegen contract requires an
  explicit element type (the array parallel of `type:object` requiring a shape —
  see [[type]]). Diagnostic names the location and asks for `items:{...}`.
- `items` **without** `type:"array"` → reject per [[type]] (missing or
  mismatched `type`); require explicit `type:"array"`. The rule is not scoped to
  scalar types: a declared **`type:"object"`** carrying an `items` rejects too —
  an object's shape keywords ([[properties]], [[additionalProperties]]) do not
  license an array applicator beside them, and accepting one and dropping it is
  the silent passthrough **P7.1** forbids. It also holds however the `type`
  comes to be *absent*: a node carrying a [[oneOf]] or a [[ref]] declares no
  `type` of its own, so an `items` beside one is an authoring error and rejects
  rather than being dropped. An array *branch* carries its `items` inside the
  branch (see [[oneOf]]), and a sibling `items` folded onto a `$ref`'s merged
  node is subject to that node's own shape rules — a merged `type:"object"`
  rejects it like any other object.
- `items` value not a valid subschema → reject (recurse).
- `items` that is shapeless — `{}` / `true` / `false` → reject per **P7.1** (no
  element shape). Diagnostic names the array and asks for an explicit element
  `type`.
- `items` value that is an **array** (draft-7 tuple spelling) → reject;
  diagnostic notes 2020-12 moved tuples to [[prefixItems]], which is itself
  rejected per **P6** (below).
- [[prefixItems]] present (tuple form) → reject per **P6**. Heterogeneous
  positional tuples have no single coherent lowering — Go and Java have no tuple
  type, and a mixed `[]any`/`Object[]` would forfeit the typing the subset
  exists to provide. This is a categorical P6 exclusion, not a deferral; the
  diagnostic suggests an object with named [[properties]] instead.

## Type mapping

A `{type:"array", items:S}` schema emits the language-native ordered collection
over `T`, where `T` is the recursive mapping of the element subschema `S`.
Optional/nullable wrapping of the **array itself** is owned by [[required]] +
[[nullability]]; this table is the bare collection type.

| Element `T` from `items` | Go    | TypeScript | Python    | Java      |
| ------------------------ | ----- | ---------- | --------- | --------- |
| any supported `T`        | `[]T` | `T[]`      | `list[T]` | `List<T>` |

Notes:
- **Elements recurse.** `T` is produced by the same pipeline as any field type:
  a scalar (`items:{type:string}` → `[]string` / `string[]` / `list[str]` /
  `List<String>`), an object (`items` is a `{type:object, properties:{...}}` → a
  slice/array/list of the emitted aggregate), a nested array
  (`items:{type:array, items:{...}}` → `[][]T` / `T[][]` / `list[list[T]]` /
  `List<List<T>>`), or a `$ref` (see [[ref]]).
- **Element nullability is the element's own concern.** An element schema that
  is the recognized [[nullability]] `oneOf` pattern makes the *elements*
  nullable — `[]*T` (Go), `(T | null)[]` (TS), `list[T | None]` (Python),
  `List<@Nullable T>` (Java) — distinct from the array field itself being
  optional/nullable, which wraps the whole collection. The two axes compose (an
  optional array of nullable elements is legal), and neither implies the other:
  an optional array of nullable elements is `list[T | None] | None`, not
  `list[T | None]`.
- **An inline element shape is named.** An element schema that is an **object**
  or a `oneOf` *sum type* (two or more non-`null` branches) is named after its
  position at load — `<EnclosingType><Property>Item`, `…ItemItem` for a nested
  array — moved into `$defs`, and the element rewritten to a `$ref`, so the
  element type is an ordinary named model or union in every target (see
  [[properties]] §"Naming an inline object shape" and [[oneOf]] §"Unions in
  element positions"). This holds for **every** array position, not only a
  declared property: an array that is a [[oneOf]] branch or a typed-map member
  has its inline element shape named and hoisted the same way, at any depth. No
  element position falls back to an untyped map or a bare `Object`/`String` — if
  a name cannot be synthesized the load fails, rather than the element being
  emitted opaque and its declared members lost. Go and Java decode a *union*
  element through the union's dispatcher one value at a time — neither can
  allocate a sealed interface from a whole-collection decode.
- **Java** uses `List<T>` (interface type; the concrete `ArrayList` is an
  implementation detail of the deserializer). `List<T>` is a reference type, so
  it carries a non-null validator rather than boxing (see PRINCIPLES Java §3);
  element boxing follows `T`'s own [[type]] mapping.
- **Empty vs absent.** `[]` (present, empty) is distinct from an absent array
  (owned by [[required]]) and from `null` (owned by [[nullability]]); see the Go
  nil-slice substitution under Serialize-side.

## Validator mapping

Per **P10** the array type and every element are validated at the (de)serializer
boundary; per **P11** element failures aggregate. `items` contributes the
per-element dispatch; the outer array-type check (`Array.isArray` / typed slice
/ `isinstance(v, list)` / typed `List` binding) comes from [[type]]'s `"array"`
row.

| Language   | Strategy                                                                                                                                                                                                                                                                                                              |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | Custom `UnmarshalJSON` decodes the field into a shadow `[]*json.RawMessage`, then dispatches each element through `T`'s runtime helper, collecting `Violation{Path, Reason}` into the one `PayloadValidationError` application failure. `Path` threads the index: `tags[2]`.                                          |
| TypeScript | Hand-emitted `Array.isArray` guard, then a per-element loop running `T`'s checks; push `Violation { path: "tags[2]", reason }` per bad element into the list, throw one `PayloadValidationError` application failure.                                                                                                 |
| Python     | The transfer type converter (PRINCIPLES Python §3) guards `isinstance(v, list)`, then loops the raw elements through `T`'s parse helper / converter, appending `Violation(path="tags[2]", reason=…)` per bad element and raising one generated `PayloadValidationError` application failure. The TypeScript parallel. |
| Java       | the per-POJO collecting deserializer (PRINCIPLES Java §5) reads the array node, walks each element through `T`'s spec-strict/constraint helper, and collects `Violation{path:"tags[2]", reason}` into the one `PayloadValidationError` application failure. The Go parallel.                                          |

- **Path convention.** Element failures use bracketed indices appended to the
  field path (`tags[2]`, and for nested arrays `matrix[1][2]`), distinct from
  the dotted member paths [[properties]] uses — so a caller can locate the
  offending element unambiguously (**P11**).
- **Reason convention.** An element takes the *same* checks — and so the same
  `reason` text — the value in that position would take anywhere else: a
  mistyped element reads `expected string` / `expected number` from [[type]]'s
  row for `T`, and a constraint failure reads that keyword's own reason
  (`must have length >= 3, got 1`). Nothing about the reason marks it as an
  element: the bracketed index in the path already does that, which leaves the
  reason free to name the type or bound that was missed. This holds for an
  element of a `oneOf` array branch exactly as for a declared array member (see
  [[oneOf]]). Reason *text* is not held byte-identical across targets (**P11**),
  but the shape is the same one everywhere.
- **Element recursion.** Each element validates recursively — an array of
  objects runs each object's own `Validate`, an array of arrays recurses again,
  an array of `$ref` follows the reference (see [[ref]]). A nested array decodes
  one loop per level, each level's loop variables carrying their depth so an
  inner element never shadows the level above it, and each level appending its
  own index to the path (`matrix[1][2]`).
- **Sibling array keywords inspect the original instance.** The parse adapter
  may build a partial typed collection while aggregating bad `items`, but
  [[minItems]], [[maxItems]], [[uniqueItems]], and [[contains]] evaluate the
  complete wire array. A failed element is neither removed nor replaced by a
  typed placeholder for those checks. Indexed element violations are emitted
  first, followed by array-level violations in keyword order. The rule applies
  recursively, including arrays in union branches and typed-map members.
- **Materializing elements use their ordinary adapters.** A temporal [[format]]
  or [[contentEncoding]] in `items` is parsed and serialized through the same
  generator-owned adapter as a declared property, at every nesting depth.
  Required runtime support is discovered recursively from the element schema,
  rather than only from top-level properties.
- **The wrapper does not change what the elements require.** Wrapping the array
  in the [[nullability]] wrapper leaves the element schema's demands on the
  emitter untouched: every package-level compiled-regex static the elements or a
  [[contains]] matcher need — [[pattern]], [[format]], [[contentEncoding]] — is
  still emitted for a **nullable** array, at the same position the code
  referencing it uses, or the package refers to an undeclared identifier. And a
  nullable array is still `[]T`, including as a typed [[additionalProperties]]
  value, so the element traversal ranges over the value and never over `*value`.
  The same holds however the array is reached — a declared property, a typed-map
  member, a [[oneOf]] branch, or a deeper element.
- **Empty array.** The element loop is vacuous; an empty `[]` passes the `items`
  check (array-length floors, when supported, live in their own specs — see
  Interactions).

### Serialize-side (P12)

`items` is symmetric across directions: serialize recurses the shared `Validate`
into each element (a nested aggregate element runs its own
`MarshalJSON`/`toTransferType`/`to_transfer_type`; a scalar element re-runs the
same predicate the deserializer used) **before emitting a byte**, failing with
the same aggregated primitive (**P11**), and re-emits elements in order (arrays
are ordered — unlike object members, element order is part of the value and is
preserved).

This includes arrays that are branches of a [[oneOf]]: branch selection does not
bypass the ordinary recursive array parser/mapper, so **element conversion**,
constraints, materialization, nested models, and indexed aggregation remain
identical to a declared array property. A branch whose elements are models is
converted element by element, never handed to the encoder as the in-memory
collection.

- **Go nil-slice substitution.** A `nil` `[]T` marshals to JSON `null` under
  `encoding/json`, not `[]`. For a **required, non-nullable** array that is the
  wrong wire form, so the generated `MarshalJSON` writes `[]` — Go has no
  non-nil empty-slice type, and the substitution is the deliberate, implemented
  design for that one target (see the [[nullability]] serialize table). It
  preserves the absent-vs-zero distinction **P9** forces everywhere. What is
  **Go-only** is this **empty-vs-`null` aliasing**, not the wider question: the
  other three targets can each hold an empty reference on a required
  non-nullable array (Java `null`, Python `None`, TypeScript
  `undefined`/`null`), and there the serialize side must **reject** rather than
  write an emptiness the schema forbids. Each emits an `expected array` or
  `required` violation before conversion/iteration, so an invalid native value
  remains inside the aggregate rather than throwing from the host runtime. The
  decision itself is owned by [[required]] + [[nullability]], recorded here
  because the array type is where it bites.

## Property-testing matrix

### Accepted (positive)

| Shape                                                     | Example                                                                                                                                                                   |
| --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| List of scalars                                           | `{type:array, items:{type:string}}`                                                                                                                                       |
| List of objects (element named after the position)        | `{type:array, items:{type:object, properties:{id:{type:integer}}}}`                                                                                                       |
| Nested array                                              | `{type:array, items:{type:array, items:{type:integer}}}`                                                                                                                  |
| Element with assertions                                   | `{type:array, items:{type:string, minLength:1}}`                                                                                                                          |
| Nullable elements                                         | `{type:array, items:{oneOf:[{type:string},{type:null}]}}`                                                                                                                 |
| Union elements (named after the position)                 | `{type:array, items:{oneOf:[{type:string},{type:integer}]}}`                                                                                                              |
| Array member of a struct                                  | `{type:object, properties:{tags:{type:array, items:{type:string}}}}`                                                                                                      |
| Self-reference via `items`, **required** OK (see [[ref]]) | tree node — the empty array terminates the recursion: `{type:object, properties:{value:{type:string}, children:{type:array, items:{$ref:"#"}}}}` with `children` required |

### Rejected at load time (negative)

| Reason                                                      | Example                                                                                    |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| `type:array` without `items` (P7.1)                         | `{type:array}`                                                                             |
| `items` without `type:array` (per [[type]])                 | `{items:{type:string}}` (no `type`)                                                        |
| Shapeless element (P7.1)                                    | `{type:array, items:{}}`, `…items:true`, `…items:false`                                    |
| Element not a schema                                        | `{type:array, items:"string"}`, `…items:5`                                                 |
| Out-of-subset element (P7.1, recurse)                       | `{type:array, items:{type:object}}` (object with no shape)                                 |
| Array-valued `items` (draft-7 tuple)                        | `{type:array, items:[{type:string},{type:integer}]}`                                       |
| Tuple via `prefixItems` (P6)                                | `{type:array, prefixItems:[{type:string},{type:integer}]}`                                 |
| `items` paired with non-array `type`                        | `{type:string, items:{type:string}}`, `{type:object, properties:{…}, items:{type:string}}` |
| `items` beside a `oneOf` or a `$ref` (no `type` of its own) | `{oneOf:[{type:string},{type:integer}], items:{type:string}}`                              |

### Runtime fixtures (validator)

- Element valid / invalid against the element schema; a bad element reports at
  its index path (`tags[2]`).
- Multiple bad elements → all reported in one shot (**P11**).
- Empty array `[]` → accepted (element check vacuous).
- Non-array instance (`{}`, `"x"`, `5`) → rejected by the [[type]] `"array"`
  check, not by `items`.
- Nested array element failure → nested index path (`matrix[1][2]`).
- Array of objects → each element runs its aggregate's own `Validate`; failures
  thread the index into the member path (`items[0].id`).
- Self-reference via `items` (see [[ref]]): a recursive instance of bounded
  depth validates and round-trips; the empty array terminates the chain, so a
  required `items:{$ref:"#"}` field is satisfiable (contrast the direct
  self-reference rule in [[properties]] / [[ref]]).
- Round-trip preserves element **order** and empty-vs-populated shape (Go
  required-array `nil` → `[]`, per Serialize-side).

## Interactions

- **[[type]]**: `items` is only meaningful under `type:"array"`; pairing with
  any other `type` is a generator-time error, and `type:"array"` **requires**
  `items` (no untyped arrays — the array parallel of the object-shape
  requirement in [[type]]).
- **[[prefixItems]]**: the 2020-12 positional-tuple keyword — **rejected** per
  **P6** (no coherent cross-language tuple lowering). With it rejected, `items`
  applies to *all* elements, so the annotation-dependent "elements past the
  prefix" clause never engages in our subset.
- **[[contains]]**: the array **existential** — supported for a scalar matcher
  over a scalar `items` element type (its own spec); layers over the same
  element set `items` types. **[[unevaluatedItems]]**: **rejected** per **P6**
  (annotation-dependent semantics don't lower).
- **[[minItems]] / [[maxItems]]**: array-level count assertions that layer over
  the same element set (see their specs); both count the full array including
  every `items`-validated element, and are orthogonal to the element typing
  `items` supplies. **[[uniqueItems]]**: element-uniqueness assertion over the
  same element set (its own spec).
- **[[nullability]]**: owns optional/nullable wrapping of the array field;
  element-level nullability is expressed by the element subschema (the `oneOf`
  null pattern) and composes with the field wrapping. The empty-vs-`null`
  decision on a required non-nullable array — Go's `[]` substitution and the
  other three targets' reject (Serialize-side) — is made there. Wrapping the
  array does not change what its elements require of the emitter (Validator
  mapping).
- **[[required]]**: orthogonal — decides whether the array member must be
  present; `items` types its elements. A present-but-empty `[]` satisfies
  `required` (presence, not non-emptiness — non-emptiness is [[minItems]]).
- **[[ref]]**: `items` may be a `$ref`, including one resolving back to the
  containing type. A self-reference wrapped in an array **terminates** (the
  empty array ends the chain), so — unlike a direct object self-reference — it
  may be **required and non-nullable** (see [[properties]] / [[ref]]).

## Ecosystem variance

| Source dialect        | Action                                                                                                                                                                                 |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12   | Native (`items` = single schema over all elements).                                                                                                                                    |
| OpenAPI 3.1           | Aligns with 2020-12. Native.                                                                                                                                                           |
| OpenAPI 3.0           | `items` is single-schema-only (no tuple form) — matches our list case. Native; `nullable:true` on the element → reject (rewrite to the nullability `oneOf`).                           |
| Swagger 2.0 / draft-4 | Single-schema `items` maps natively; the **array** `items` tuple form (and its `additionalItems` tail) → reject, rewrite to a homogeneous list or an object with named [[properties]]. |

## See also

- [[type]] — gates `items` to `type:"array"`; supplies the array-type check and
  the element base type.
- [[properties]] — the object analog; array-of-objects elements are emitted
  aggregates.
- [[prefixItems]], [[unevaluatedItems]] — rejected per **P6**; `items` is the
  supported per-element applicator. [[contains]] — the supported scalar
  existential over the element set.
- [[minItems]], [[maxItems]], [[uniqueItems]] — array-level assertions layered
  over the element set.
- [[required]], [[nullability]] — own the array field's optional/nullable
  wrapping and the empty-vs-absent-vs-`null` distinction.
- [[ref]] — element `$ref`, including array-terminated recursion.
