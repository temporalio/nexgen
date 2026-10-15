# `minItems`

Source: JSON Schema 2020-12, Validation vocabulary, §6.4.2 "Validation Keywords
for Arrays → minItems".

Sets an **inclusive** lower bound on the **number of elements** an array
instance must have. A pure runtime count assertion — no type impact. Mirror of
[[maxItems]]; the shared array-count machinery (portable element count,
per-language primitive, serialize-side re-check, exact-size pin) is documented
once in [[maxItems]] and referenced here.

## Spec summary

Verbatim (2020-12 validation, §6.4.2):

> The value of this keyword MUST be a non-negative integer.

> An array instance is valid against "minItems" if its size is greater than, or
> equal to, the value of this keyword.

> Omitting this keyword has the same behavior as a value of 0.

Distilled:
- Value MUST be a **non-negative integer**.
- Instance valid iff `elementCount(instance) ≥ minItems`.
- "Size" is the portable top-level element count — exactly [[maxItems]]'s
  definition (no unit hazard, unlike the string-length family).
- `minItems: 0` is a **no-op** (spec: same as omitting) — accepted but vacuous.
- Applies only to array instances; a `minItems` on a non-array [[type]] is
  rejected at load (**P7.1**).

## Support decision

**Support:** yes — runtime element-count comparison, [[maxItems]] with `≥`. Both
directions count the array's complete element sequence — the wire array inbound
(never the partially-built typed collection), the decoded collection outbound —
with the same comparison and reason (**P10** / **P11** / **P12**). No effect on
emitted types.

Loader behavior (mirror of [[maxItems]] with `≥`):
- Value not a non-negative integer → reject: non-number, **negative**
  (`minItems:-1`), or **fractional** (`minItems:0.5`). `minItems:2.0` accepted
  (≡ `2`).
- The portable count ceiling from [[maxItems]], and the emitted-literal
  obligation that comes with a bound above 2^31−1, apply here unchanged.
- `minItems` on a non-array [[type]] → reject (**P7.1**).
- `minItems` present without `type:"array"` → reject per [[type]]; a
  `type:"array"` still requires [[items]].
- **`minItems:0`** → accepted, treated as **omitted** (the spec's explicit
  equivalence); it constrains nothing. Not rejected as "redundant" — it is a
  spec-sanctioned identity that templated tooling emits, so honoring it as a
  no-op is friendlier than a diagnostic (mirrors [[minLength]] `0`).
- **`minItems` > `maxItems` → reject (unsatisfiable)**; `minItems == maxItems`
  pins an **exact** size (accepted). All combined-size satisfiability lives in
  [[maxItems]].

## Type mapping

None. Constraint lives only in the validator.

## Validator mapping

Per **P10**/**P11**. A single `≥` comparison of the **element count** against
the fixed bound. On deserialize the count is the original wire array's length
even if an element fails [[items]] conversion; on serialize it is the decoded
collection's native length. Same per-language strategy as [[maxItems]] with
`< min` as the failing comparison (`len(v)` / `v.length` / `len(v)` /
`v.size()`).

| Language   | Strategy                                                                                                                                                                                                                                                                           |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | `UnmarshalJSON` checks `len(rawArray) < min` after collecting indexed item violations; serialize checks the typed slice in shared `Validate`. Both collect into one `PayloadValidationError` application failure.                                                                  |
| TypeScript | After the `Array.isArray` guard ([[items]]), deserialize checks `raw.length < min` after parsing the elements; serialize checks the typed array. A failure pushes ``Violation{path, reason: `must have at least ${min} items, got ${raw.length}`}``.                               |
| Python     | After the `isinstance(raw, list)` guard ([[items]]), the transfer converter checks `len(raw) < min` after parsing the elements; serialize checks the typed list. Both aggregate into the generated `PayloadValidationError` application failure.                                   |
| Java       | The per-POJO collecting deserializer (PRINCIPLES Java §5) checks `node.size() < min` after parsing the elements; serialize checks the typed `List<T>`. Both push a structured violation into the single `PayloadValidationError` application failure. Not bean-validation `@Size`. |

Reason strings name the concrete bound and offending count
(`must have at least 2 items, got 1`), per the [[maxProperties]] count-family
convention.

### Serialize-side (P12)

Identical to [[maxItems]]: the predicate re-runs before emit over the decoded
value; an in-memory under-filled slice/list fails serialize rather than being
written. A Go `nil` slice counts as length 0, so a required non-nullable array
with `minItems ≥ 1` and a `nil` in-memory value fails serialize — the correct,
loud outcome (the `nil`-vs-`[]` encoder decision itself is owned by
[[nullability]], see [[items]]).

## Property-testing matrix

### Accepted (positive)

| Shape                     | Example                                                       |
| ------------------------- | ------------------------------------------------------------- |
| Inclusive min (non-empty) | `{type:"array", items:{type:string}, minItems:1}`             |
| `.0`-valued bound         | `{type:"array", items:{type:string}, minItems:2.0}`           |
| Zero min (no-op)          | `{type:"array", items:{type:string}, minItems:0}`             |
| Exact size (min==max)     | `{type:"array", items:{type:string}, minItems:2, maxItems:2}` |

### Rejected at load time (negative)

| Reason                           | Example                                                        |
| -------------------------------- | -------------------------------------------------------------- |
| Value not a number               | `minItems:"1"`, `minItems:false`                               |
| Negative value                   | `minItems:-1`                                                  |
| Fractional value                 | `minItems:0.5`                                                 |
| Above the portable count ceiling | `minItems:9007199254740992`                                    |
| Type mismatch (P7.1)             | `{type:"string", minItems:1}`                                  |
| Unsatisfiable range              | `{type:"array", items:{type:string}, minItems:10, maxItems:2}` |

### Runtime fixtures (validator)

- Element count `== min` → OK (`≥` is inclusive).
- Element count `min-1` → one `PayloadValidationError` application failure
  naming the bound and count (`must have at least 2 items, got 1`).
- Empty array `[]` against `minItems:1` → rejected; against `minItems:0` → OK.
- Combined with other failing assertions → all reported in one shot (**P11**).
  Serialize of an under-filled in-memory value → rejected before emit (**P12**).

## Interactions

- **[[maxItems]]**: the paired upper bound; `minItems > maxItems` load error;
  `minItems == maxItems` pins an exact size (accepted). All combined-size
  satisfiability rules live in [[maxItems]].
- **[[items]]**: orthogonal — `items` types the elements, `minItems` requires a
  floor count. Both apply and aggregate.
- **[[required]]**: distinct axes — `required` is *presence* of the array
  member, `minItems` is *non-emptiness* of its value. `minItems:1` is the
  "non-empty array" idiom and does **not** imply the member is required; a
  member may be optional (absent is fine) yet, when present, be required to hold
  at least one element (**P8**: optional ≠ the value constraint).
- **[[uniqueItems]]**: independent; both apply and aggregate. No
  uniqueness-vs-count satisfiability cross-check — determining whether the
  element type admits enough distinct values to fill a
  `minItems`-with-`uniqueItems` array is undecidable in general and out of scope
  (parallel to the regex-vs-length non-check in [[maxLength]]).
- **[[type]]**: gates applicability; the emitted collection type is unchanged.

## Ecosystem variance

| Source dialect        | Action                                                         |
| --------------------- | -------------------------------------------------------------- |
| JSON Schema 2020-12   | Native.                                                        |
| OpenAPI 3.1           | Native.                                                        |
| OpenAPI 3.0 / draft-4 | `minItems` present since draft-4, identical semantics. Native. |
| Swagger 2.0           | Same as OAS 3.0.                                               |

## See also

- [[maxItems]] — the paired inclusive upper bound; owns the shared element-count
  machinery and combined-size satisfiability.
- [[items]] — supplies the emitted collection type; types the counted elements.
- [[uniqueItems]] — the other array assertion (element uniqueness).
- [[required]] — presence of the member, distinct from non-emptiness.
- [[type]] — gates applicability to `type:"array"`.
- [[maxProperties]] / [[minProperties]] — the object member-count analog (same
  count-family reason strings).
- [[minLength]] — the string-length analog (same inclusive-bound and no-op-zero
  conventions).
