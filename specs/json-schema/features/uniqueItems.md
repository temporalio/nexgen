# `uniqueItems`

Source: JSON Schema 2020-12, Validation vocabulary, §6.4.3 "Validation Keywords
for Arrays → uniqueItems".

Asserts that **no two elements** of an array instance are equal under JSON
equality. A pure runtime assertion — no type impact. The third array assertion
alongside the count pair [[minItems]] / [[maxItems]]; unlike them its cost and
portability depend on the **element type**, which is why its support envelope is
drawn at the scalar/composite line rather than being unconditional.

## Spec summary

Verbatim (2020-12 validation, §6.4.3):

> The value of this keyword MUST be a boolean.

> If this keyword has boolean value false, the instance validates successfully.
> If it has boolean value true, the instance validates successfully if all of
> its elements are unique.

> Omitting this keyword has the same behavior as a value of false.

Distilled:
- Value MUST be a **boolean**.
- `uniqueItems: true` → instance valid iff all elements are
  **pairwise distinct under JSON equality** (by type and value — the same
  equality [[const]] / [[enum]] use for membership, lifted to
  element-vs-element).
- `uniqueItems: false` → **no-op** (spec: same as omitting) — accepted but
  vacuous, mirroring [[minItems]] `0`.
- Applies only to array instances; a `uniqueItems` on a non-array [[type]] is
  rejected at load (**P7.1**), like the count pair.
- Pure assertion; no annotation behavior, no effect on the emitted collection
  type ([[items]]'s `[]T` / `T[]` / `list[T]` / `List<T>`).
- **The equality is where the cost lives.** For a scalar element [[type]]
  (string / number / integer / boolean) uniqueness is a cheap, trivially
  portable comparison — every target agrees value-for-value (**P1**). For a
  **composite** element (object / array) it requires
  **deep structural equality** with number normalization and key-order
  independence — the same "correct in principle, just costly" surface [[const]]
  / [[enum]] defer for composite members.

## Support decision

**Support:** yes for a **scalar element [[type]]** (string / number / integer /
boolean); **composite** element types (object / array) are **deferred** —
`uniqueItems: true` over them is a load reject with a "not yet supported"
diagnostic, exactly the stance [[const]] / [[enum]] take on composite members
(the deep structural-equality check is correct in principle, just costly —
revisit on demand).

Grounding ([[PRINCIPLES.md]]): **P10** (enforced at the boundary), **P11**
(aggregated), **P12** (**one** all-distinct walk over one kind of operand — the
array's complete element sequence — applied at both boundaries with the same
equality and the same reason). No effect on emitted types. The scalar
restriction is the **P1** line: element-vs-element equality over a scalar type
is the same value comparison [[enum]] already specifies value-for-value across
all five targets, whereas a portable composite deep-equal is extra surface we
don't yet commit to.

A scalar element carrying a **temporal [[format]]** is also **deferred**, for a
different reason: the element materializes to a native value that no longer
holds the authored lexeme. Two distinct JSON strings — `"…T00:00:00Z"` and
`"…T00:00:00.000Z"` — are distinct under the JSON equality this keyword asserts,
and collapse to one canonical native value, so no single projection is both
element-to-element JSON equality inbound and available outbound; the pair would
parse and then refuse to re-serialize. A [[contentEncoding]] element is
**supported**: its codec admits only the canonical spelling, so the authored and
canonical wire forms coincide and one equality serves both directions. That
support is contingent on that strictness — a codec that ever admitted two
spellings of one value would fall under the same deferral.

Loader behavior:
- Value not a boolean (`uniqueItems:"true"`, `uniqueItems:1`,
  `uniqueItems:null`) → reject.
- **`uniqueItems: false`** → accepted, treated as **omitted** (the spec's
  explicit equivalence); it constrains nothing. Not rejected as "redundant" — it
  is a spec-sanctioned identity that templated OpenAPI tooling emits, so
  honoring it as a no-op is friendlier than a diagnostic (mirrors [[minItems]]
  `0`).
- `uniqueItems` on a non-array [[type]] (`{type:"string", uniqueItems:true}`) →
  reject per **P7.1** (statically meaningless).
- `uniqueItems` present without `type:"array"` → reject per [[type]]
  (missing/mismatched type); a `type:"array"` still requires [[items]].
- **`uniqueItems: true` with a composite element [[items]]**
  (`items:{type:"object", …}` or `items:{type:"array", …}`) → **reject** with a
  "not yet supported" diagnostic (deferred; see Support decision). A
  `uniqueItems: false` over the same composite `items` is still a no-op and
  accepted — nothing is asserted, so no equality is needed.
- **`uniqueItems: true` with a temporal [[format]] on a scalar `items`** →
  **reject** with a "not yet supported" diagnostic (deferred; see Support
  decision). A `uniqueItems: false` over the same element is a no-op and
  accepted.

## Type mapping

None. The emitted collection type is [[items]]'s `[]T` / `T[]` / `list[T]` /
`List<T>`; the assertion lives only in the validator.

## Validator mapping

Per **P10**/**P11**. A single **all-distinct** check. Deserialize compares the
original wire elements, including elements that fail [[items]] conversion;
serialize compares decoded elements. Every element that repeats an earlier one
is reported, naming both indexes; equality is the same value comparison [[enum]]
uses (exact `==` for numbers — see below). Because the element [[type]] is
scalar, Go, TypeScript and Java track seen values in their native hash/set
primitive; Python uses a JSON-aware equality walk over a list so `true` remains
distinct from `1`, and so raw composite values from elements that fail `items`
conversion remain comparable.

| Language   | Strategy                                                                                                                                                                                                                                                                                                                                                  |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | Deserialize normalizes each original `json.RawMessage` to a JSON value key and tracks the first raw index; serialize performs the equivalent typed-slice walk in shared `Validate`. Both collect duplicate-index violations into one `PayloadValidationError` application failure.                                                                        |
| TypeScript | After the `Array.isArray` guard ([[items]]), deserialize walks the raw array with a `Map` before returning the typed result; serialize walks the typed array. `Map` uses value equality for the supported scalar elements.                                                                                                                                |
| Python     | The runtime's `_check_unique_items(value, path, violations)` is called with the raw list on deserialize and the typed list on serialize. It uses a JSON-aware equality walk, **not** a `set`/`dict`, so booleans stay distinct from numbers and composite raw values compare structurally. Arrays are small and correctness beats the O(n²) (**P2**).     |
| Java       | The per-POJO collecting deserializer walks the original `JsonNode` array using `SpecNumbers.valueKey` for JSON numeric equality; serialize walks the typed `List<T>`. Both report the colliding indexes in the single `PayloadValidationError` application failure. Not bean-validation.                                                                  |
| C#         | `Converter.Parse` walks the original `JsonElement` array keyed by `SpecJson.ValueKey` (numbers mathematical, signed zero folded); `Validate` walks the in-memory list through `ValueEquality.Comparer` (a materialized value by its canonical wire string). Both report the colliding indexes in the single `PayloadValidationError` application failure. |

Reason strings name the **colliding positions**
(`duplicate items: element at index 3 equals index 1`) per the count-family
convention — the aggregated error tells the caller which elements clashed, not a
bare keyword.

**Float exactness.** For a `number` element type, equality is **exact `==`**,
never an epsilon — identical to [[enum]] / [[const]]: the wire values are
IEEE-754 binary64 from correctly-rounded decimal→double parsing, so the same
decimal yields the identical bit pattern in every target. The comparison key is
that binary64 value on **both** sides, whatever numeric width the host language
would otherwise give the wire token: two integer tokens that round to one
`double` are one value in every target. Equality is over the mathematical number
rather than the authored spelling (as in [[enum]]), so `[1, 1.0]` in a `number`
array is a runtime duplicate (both are the same value), not two distinct
elements. `-0.0` equals `0.0`; `NaN`/`±Infinity` cannot appear.

### Serialize-side (P12)

Identical to the count pair: the predicate **re-runs before emit** over the
decoded value, so an in-memory slice/list holding duplicates fails serialize
with the same aggregated primitive rather than being written. Real teeth in
every target: building the collection in memory is unchecked in all five (same
rationale as the [[maxItems]] bound re-check). Both directions walk the array's
complete element sequence and compare the same values — for a
[[contentEncoding]] element the byte value and its one canonical spelling
partition the elements identically — so the check is the identical all-distinct
walk in both directions.

## Property-testing matrix

### Accepted (positive)

| Shape                                              | Example                                                                                                                            |
| -------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| Scalar string uniqueness                           | `{type:"array", items:{type:string}, uniqueItems:true}`                                                                            |
| Scalar integer uniqueness                          | `{type:"array", items:{type:integer}, uniqueItems:true}`                                                                           |
| Scalar number uniqueness (exact `==`)              | `{type:"array", items:{type:number}, uniqueItems:true}`                                                                            |
| Boolean uniqueness (degenerate, ≤2 distinct)       | `{type:"array", items:{type:boolean}, uniqueItems:true}`                                                                           |
| No-op false                                        | `{type:"array", items:{type:string}, uniqueItems:false}`                                                                           |
| No-op false over composite (nothing asserted)      | `{type:"array", items:{type:object, …}, uniqueItems:false}`                                                                        |
| Combined with count bounds                         | `{type:"array", items:{type:string}, minItems:1, maxItems:10, uniqueItems:true}`                                                   |
| Nullable scalar element                            | `{type:"array", items:{oneOf:[{type:string},{type:"null"}]}, uniqueItems:true}` — `null` is one value; two `null`s are a duplicate |
| `contentEncoding` element (one canonical spelling) | `{type:"array", items:{type:string, contentEncoding:"base64"}, uniqueItems:true}`                                                  |

### Rejected at load time (negative)

| Reason                              | Example                                                                                                               |
| ----------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| Value not a boolean                 | `uniqueItems:"true"`, `uniqueItems:1`, `uniqueItems:null`                                                             |
| Type mismatch (P7.1)                | `{type:"string", uniqueItems:true}`                                                                                   |
| Composite element, deferred         | `{type:"array", items:{type:object, …}, uniqueItems:true}`, `{type:"array", items:{type:array, …}, uniqueItems:true}` |
| Temporal `format` element, deferred | `{type:"array", items:{type:"string", format:"date-time"}, uniqueItems:true}`                                         |

### Runtime fixtures (validator)

- All elements distinct → OK (both directions).
- A repeated element → one `PayloadValidationError` application failure naming
  the colliding indexes (`duplicate items: element at index 3 equals index 1`).
- Empty array `[]` and single-element array → OK (vacuously unique).
- `number` array with `[1, 1.0]` → duplicate (both the same value); with
  `[1, 2]` → OK.
- `boolean` array `[true, false]` → OK; `[true, true]` → duplicate.
- Combined with a failing count bound ([[minItems]] / [[maxItems]]) or a failing
  sibling field → **all** reported in one shot (**P11**).
- Serialize of an in-memory slice/list containing duplicates → rejected before
  emit (**P12**), not silently written.

## Interactions

- **[[minItems]] / [[maxItems]]**: independent array assertions; all apply and
  aggregate. We do **not** cross-check uniqueness against a count bound —
  deciding whether the element type admits enough distinct values to fill a
  `minItems`-with-`uniqueItems` array is out of scope, and that non-check is
  owned by [[minItems]] (parallel to the regex-vs-length non-check in
  [[maxLength]]). The degenerate `boolean` case (`minItems:3`
  + `uniqueItems:true` over ≤2 distinct values) is unsatisfiable at runtime but
    still not caught at load, keeping the rule uniform.
- **[[items]]**: gates the element type and thus applicability —
  `uniqueItems:true` is supported only when `items` is scalar; a composite
  `items` defers it. `items` types the elements, `uniqueItems` requires them
  distinct; orthogonal where both apply.
- **[[type]]**: gates applicability to `type:"array"`; a mismatch is a load
  reject (**P7.1**). The emitted collection type is unchanged.
- **[[format]] / [[contentEncoding]]**: a temporal [[format]] on the element is
  a load reject (Support decision) — the native value it materializes to cannot
  answer element-to-element JSON equality. A [[contentEncoding]] element is
  supported because its codec accepts one spelling per value.
- **[[enum]] / [[const]]**: share the **scalar value-equality** definition used
  here (exact `==` for numbers, normalized integer-valued numbers). An array
  whose `items` carries an `enum` may also set `uniqueItems`; the two compose —
  `enum` closes each element to a set, `uniqueItems` forbids repeats within the
  instance. Composite equality is deferred in all three.
- **[[required]]**: orthogonal — `required` decides whether the array member is
  present; `uniqueItems` shapes its value. A present empty `[]` satisfies both
  `required` and `uniqueItems`.
- **[[nullability]]**: if the element schema is the nullable [[nullability]]
  pattern, a `null` element is one value for uniqueness purposes — two `null`
  elements are a duplicate. Otherwise orthogonal.

## Ecosystem variance

| Source dialect        | Action                                                                            |
| --------------------- | --------------------------------------------------------------------------------- |
| JSON Schema 2020-12   | Native (scalar elements).                                                         |
| OpenAPI 3.1           | Adopts 2020-12 — `uniqueItems` identical. Native.                                 |
| OpenAPI 3.0 / draft-4 | `uniqueItems` present since draft-4 with identical semantics. Native, no rewrite. |
| Swagger 2.0           | Same as OAS 3.0.                                                                  |

## See also

- [[minItems]] / [[maxItems]] — the paired array **count** assertions; they own
  the combined-size satisfiability rules and the uniqueness-vs-count non-check.
- [[items]] — supplies the emitted collection type and the element type that
  gates uniqueItems' scalar-only support envelope.
- [[type]] — gates applicability to `type:"array"`.
- [[enum]] / [[const]] — share the scalar value-equality definition; both defer
  composite equality, as this keyword does.
- [[required]] — presence of the member, distinct from element distinctness.
- [[nullability]] — a nullable element counts `null` as a value for uniqueness.
