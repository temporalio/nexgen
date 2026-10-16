# `prefixItems`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.3.1.1 "Keywords
for Applying Subschemas to Arrays → prefixItems".

Validates array elements **positionally** — the *tuple* form. Element `i` must
validate against the `i`-th subschema. **Not supported** — rejected at load time
per **P6** (categorical exclusion, not a deferral).

## Spec summary

Verbatim (2020-12 core, Applicator):

> The value of "prefixItems" MUST be a non-empty array of valid JSON Schemas.

> Validation succeeds if each element of the instance validates against the
> schema at the same position, if any. This keyword does not constrain the
> length of the array. If the array is longer than this keyword's value, this
> keyword validates only the prefix of matching length.

> The annotation result of this keyword is the largest index to which this
> keyword applied a subschema, or the boolean true if applied to every index.
> This annotation affects the behavior of "items" and "unevaluatedItems".

Distilled:
- A per-position list of subschemas: element 0 → schema 0, element 1 → schema 1,
  etc. — a heterogeneous tuple.
- Elements past the prefix are governed by [[items]] (in our subset, `items`
  covers all elements, since `prefixItems` is rejected).
- The draft-7 array-valued spelling of `items` is this keyword renamed.

## Support decision

**Support:** no — **rejected at load time (P6).**

A positional tuple has no coherent lowering across the four targets:
- **P6 (strict subset)**: Go and Java have **no tuple type**; the only
  representations are a heterogeneous `[]any` / `Object[]` (forfeiting the
  typing the subset exists to provide) or a generated positional struct (a shape
  mismatch from the wire array). TS's `[A, B]` and Python's `tuple[A, B]` have
  no cross-language counterpart, so a shared, typed representation does not
  exist.
- **P7 / P7.1 (reject ambiguity loudly)**: a tuple wearing array clothes is
  exactly the kind of construct we reject at generator time rather than
  approximate.

Loader behavior:
- Any `prefixItems` present → reject with a located diagnostic.
- The array-valued (draft-7 tuple) spelling of `items` is the same construct and
  is likewise rejected — see [[items]].
- The diagnostic offers the two coherent alternatives:
  1. An **object with named [[properties]]** — when the positions are really
     distinct, meaningful fields.
  2. A **homogeneous list** — `{type:array, items:{type:T}}` — when the elements
     share a type (see [[items]]).

## Type mapping

None — rejected before any type is emitted.

## Validator mapping

None — rejected at load time, so there is no (de)serialize boundary and no
serialize-side behavior (**P12**).

## Property-testing matrix

### Rejected at load time (negative) — the whole surface

| Reason                                        | Example                                                           |
| --------------------------------------------- | ----------------------------------------------------------------- |
| Tuple via `prefixItems` (P6)                  | `{type:array, prefixItems:[{type:string},{type:integer}]}`        |
| `prefixItems` + `items` tail                  | `{type:array, prefixItems:[{type:string}], items:{type:integer}}` |
| Array-valued `items` (draft-7 tuple spelling) | `{type:array, items:[{type:string},{type:integer}]}`              |

There are no accepted or runtime fixtures: the keyword never reaches code
generation.

## Interactions

- **[[items]]**: in 2020-12, `items` applies to elements *past* the
  `prefixItems` prefix. With `prefixItems` rejected, `items` applies to **all**
  elements — the homogeneous-list case is the supported form.
- **[[contains]]**: the array existential — supported for a scalar matcher (its
  own spec). **[[unevaluatedItems]]**: the other array applicator, also rejected
  per **P6**.
- **[[minItems]] / [[maxItems]] / [[uniqueItems]]**: array-level assertions over
  the element set; unaffected by this keyword's rejection.

## Ecosystem variance

| Source dialect        | Action                                                                                                                 |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12   | Reject (P6).                                                                                                           |
| OpenAPI 3.1           | `prefixItems` present (3.1 adopts 2020-12) → reject.                                                                   |
| OpenAPI 3.0           | No `prefixItems` keyword; single-schema `items` only — nothing to reject.                                              |
| Swagger 2.0 / draft-4 | The array-valued `items` tuple form (and `additionalItems` tail) → reject; rewrite to an object or a homogeneous list. |

## See also

- [[items]] — the supported homogeneous-list applicator; owns the
  array-valued-`items` rejection.
- [[contains]] — the supported scalar array existential.
- [[unevaluatedItems]] — the other rejected array applicator.
- [[properties]] — named-field alternative when positions are distinct fields.
