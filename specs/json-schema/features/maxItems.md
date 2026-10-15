# `maxItems`

Source: JSON Schema 2020-12, Validation vocabulary, §6.4.1 "Validation Keywords
for Arrays → maxItems".

Sets an **inclusive** upper bound on the **number of elements** an array
instance may have. A pure runtime count assertion — no type impact. The
canonical spec for the array-length pair; [[minItems]] shares the machinery
documented here and differs only in the comparison operator.

## Spec summary

Verbatim (2020-12 validation, §6.4.1):

> The value of this keyword MUST be a non-negative integer.

> An array instance is valid against "maxItems" if its size is less than, or
> equal to, the value of this keyword.

Distilled:
- Value MUST be a **non-negative integer**.
- Instance valid iff `elementCount(instance) ≤ maxItems`.
- "Size" is the number of top-level array elements — unlike the string-length
  family, this is a **directly portable** count: every target's native length
  primitive (`len` / `.length` / `len` / `.size()`) agrees on it, so there is no
  code-point-style unit hazard (contrast [[maxLength]]).
- Applies **only** to array instances; the spec silently ignores it for
  non-arrays. Per **P7.1** we instead reject a `maxItems` on a non-array
  [[type]] at load time.
- Pure assertion; no annotation behavior.

## Support decision

**Support:** yes — runtime element-count comparison.

Lowers to the same `≤` count predicate at two generator-owned call sites in
every language; no effect on emitted types. Both count
**the array's complete element sequence**: the parse adapter counts the wire
array rather than the partially-built typed collection — precisely so that a
failed element cannot change the count — and the serialize path counts the
decoded collection. One comparison, one kind of operand, one reason, in both
directions (**P10** / **P11** / **P12**).

Loader behavior:
- Value not a non-negative integer → reject: a non-number (`maxItems:"3"`,
  `maxItems:true`, `maxItems:null`), a **negative** value (`maxItems:-1`), or a
  **fractional** value (`maxItems:3.5`). `maxItems:3.0` is accepted (≡ `3`,
  honoring the `1.0`-as-integer rule from [[type]]).
- A bound greater than `9007199254740991` (2^53−1) → reject. Count bounds must
  be exactly representable in every target.
- A bound in `[2^31, 2^53−1]` is **accepted**, and the emitted constant must
  then be written in a form wide enough to hold it — in Java the `long` literal
  (`{n}L`), because a bare Java integer literal stops at 2^31−1. The ceiling
  above is a representability limit, not a licence to emit the bound bare.
- `maxItems` on a non-array [[type]] (`{type:"string", maxItems:3}`) → reject
  per **P7.1** (statically meaningless — the string-length analog is
  [[maxLength]], the object member-count analog is [[maxProperties]]).
- `maxItems` present without `type:"array"` → reject per [[type]]
  (missing/mismatched type); a `type:"array"` still requires [[items]].
- **`minItems` > `maxItems` on the same node → reject (unsatisfiable).**
  `minItems == maxItems` pins an **exact** length (accepted — a fixed-size
  array). See **Interactions → satisfiability**.
- `maxItems: 0` → accepted (array must be empty). Rarely the intent — an
  always-empty array carries no data — but it is a legitimate numeric ceiling,
  not a schema bug, so it is not rejected.

## Type mapping

None. The emitted collection type is [[items]]'s `[]T` / `T[]` / `list[T]` /
`List<T>`; the bound lives only in the validator.

## Validator mapping

Per **P10**/**P11**. A single `≤` comparison of the **element count** against
the fixed bound. On deserialize it counts the original wire array even if an
element fails [[items]] conversion; on serialize it uses the decoded
collection's native length. Unlike [[maxLength]] there is no unit subtlety, so
each language uses its bare length primitive.

| Language   | Strategy                                                                                                                                                                                                                                                                           |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | `UnmarshalJSON` checks `len(rawArray) > max` after collecting indexed item violations; serialize checks the typed slice in shared `Validate`. Both collect into one `PayloadValidationError` application failure.                                                                  |
| TypeScript | After the `Array.isArray` guard ([[items]]), deserialize checks `raw.length > max` after parsing the elements; serialize checks the typed array. A failure pushes ``Violation{path, reason: `must have at most ${max} items, got ${raw.length}`}``.                                |
| Python     | After the `isinstance(raw, list)` guard ([[items]]), the transfer converter checks `len(raw) > max` after parsing the elements; serialize checks the typed list. Both aggregate into the generated `PayloadValidationError` application failure.                                   |
| Java       | The per-POJO collecting deserializer (PRINCIPLES Java §5) checks `node.size() > max` after parsing the elements; serialize checks the typed `List<T>`. Both push a structured violation into the single `PayloadValidationError` application failure. Not bean-validation `@Size`. |

**Informative `reason` strings.** The `Violation` `reason` names the
**concrete bound and the offending count** — `must have at most 3 items, got 4`
— per the [[maxProperties]] count-family convention, so the aggregated error
tells the caller which ceiling was crossed and by how much. The bound is an
emitted compile-time constant; the count is computed at runtime.

### Serialize-side (P12)

The bound **re-runs before emit** over the decoded value — a model constructed
with an over-long slice/list in memory fails serialize with the same aggregated
primitive rather than emitting an out-of-bounds array. Real teeth in the
statically-typed targets, where in-memory construction is unchecked (identical
rationale to the [[type]] integer-cap re-check and the [[maxLength]] bound
re-check). The element count is the same in memory as on the wire (no
default-omission subtlety like [[maxProperties]] has), so the check is a plain
length comparison in both directions.

## Property-testing matrix

### Accepted (positive)

| Shape                       | Example                                                       |
| --------------------------- | ------------------------------------------------------------- |
| Inclusive max               | `{type:"array", items:{type:string}, maxItems:10}`            |
| `.0`-valued bound           | `{type:"array", items:{type:string}, maxItems:10.0}`          |
| Zero max (empty array only) | `{type:"array", items:{type:string}, maxItems:0}`             |
| Exact size (min==max)       | `{type:"array", items:{type:string}, minItems:2, maxItems:2}` |

### Rejected at load time (negative)

| Reason                           | Example                                                        |
| -------------------------------- | -------------------------------------------------------------- |
| Value not a number               | `maxItems:"3"`, `maxItems:true`, `maxItems:null`               |
| Negative value                   | `maxItems:-1`                                                  |
| Fractional value                 | `maxItems:3.5`                                                 |
| Above the portable count ceiling | `maxItems:9007199254740992`                                    |
| Type mismatch (P7.1)             | `{type:"string", maxItems:3}`, `{type:"integer", maxItems:3}`  |
| Unsatisfiable range              | `{type:"array", items:{type:string}, minItems:10, maxItems:2}` |

### Runtime fixtures (validator)

- Element count `== max` → OK (`≤` is inclusive).
- Element count `max+1` → one `PayloadValidationError` application failure whose
  reason names the bound and count (`must have at most 3 items, got 4`).
- Empty array `[]` against `maxItems:0` → OK.
- Combined with other failing assertions ([[minItems]], a failing element
  [[items]] check, a failing sibling field) → **all** reported in one shot
  (**P11**).
- Serialize of an in-memory over-long slice/list → rejected before emit
  (**P12**), not silently written.

## Interactions

- **[[minItems]]**: the paired lower bound over the same element count.
  `minItems > maxItems` is a load error; `minItems == maxItems` pins an
  **exact** size (accepted — a fixed-size array, the array analog of the
  string-length exact pin in [[maxLength]] and the numeric `minimum == maximum`
  pin in [[maximum]]).
- **[[items]]**: `items` types the elements, `maxItems` caps how many there may
  be — orthogonal; both apply and aggregate. `maxItems` counts elements
  regardless of whether each satisfies `items`.
- **[[type]]**: gates applicability — `maxItems` is meaningful only for
  `type:"array"`; a mismatch is a load reject (**P7.1**). The emitted collection
  type is unchanged; `maxItems` never narrows it.
- **[[uniqueItems]]**: an independent array assertion; both apply and aggregate.
  We do **not** attempt cross-satisfiability between a uniqueness constraint and
  a count bound (deciding whether enough distinct values exist to fill a
  `minItems`-with-`uniqueItems` array is out of scope — see [[minItems]]).
- **[[required]]**: orthogonal — `required` decides whether the array member is
  present; `maxItems` bounds its size. A present empty `[]` satisfies `required`
  regardless of `maxItems`.
- **`const`/`default`** (composite, deferred): the current subset has scalar
  `const`/`default` only ([[const]], [[default]]). When an array-valued literal
  lands, its element count MUST satisfy `maxItems` at load — the array-length
  half of the literal-vs-constraint obligation (mirroring the [[maxLength]]
  string-literal check). Enforcement deferred to that feature.

## Ecosystem variance

| Source dialect        | Action                                                                         |
| --------------------- | ------------------------------------------------------------------------------ |
| JSON Schema 2020-12   | Native.                                                                        |
| OpenAPI 3.1           | Adopts 2020-12 — `maxItems` identical. Native.                                 |
| OpenAPI 3.0 / draft-4 | `maxItems` present since draft-4 with identical semantics. Native, no rewrite. |
| Swagger 2.0           | Same as OAS 3.0.                                                               |

## See also

- [[minItems]] — the paired inclusive lower bound (shares this machinery).
- [[items]] — supplies the emitted collection type; types the elements this
  keyword counts.
- [[uniqueItems]] — the other array assertion (element uniqueness).
- [[type]] — gates applicability to `type:"array"`.
- [[maxProperties]] — the object member-count analog; same count-family
  `reason`-string convention.
- [[maxLength]] — the string-length analog; same inclusive-bound and exact-pin
  idea (but with a code-point unit hazard `maxItems` lacks).
- [[maximum]] — the numeric-bound family; same `reason`-string convention and
  single-value/exact pin idea.
