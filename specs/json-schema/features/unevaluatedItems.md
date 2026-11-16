# `unevaluatedItems`

Source: JSON Schema 2020-12, Core (Unevaluated vocabulary), §11.2 "Keywords for
Applying Subschemas to Arrays with Unevaluated Items → unevaluatedItems".

The **annotation-aware** array catch-all: it constrains elements left
unevaluated after *all* array applicators in scope have run. **Not supported** —
rejected at load time per **P6** (categorical exclusion, not a deferral).

## Spec summary

Verbatim (2020-12 core, Unevaluated vocabulary):

> The value of "unevaluatedItems" MUST be a valid JSON Schema.

> The behavior of this keyword depends on the annotation results of adjacent
> keywords that apply to the instance location being validated. Specifically,
> the annotations from "prefixItems", "items", and "contains", which can come
> from those keywords when they are adjacent to the "unevaluatedItems" keyword.
> Those three annotations, as well as "unevaluatedItems", can also result from
> any and all adjacent in-place applicator keywords.

> If no relevant annotations are present, the "unevaluatedItems" subschema MUST
> be applied to all locations in the array. If a boolean true value is present
> from any of the relevant annotations, "unevaluatedItems" MUST be ignored.
> Otherwise, the subschema MUST be applied to any index greater than the largest
> annotation value for "prefixItems", which does not appear in any annotation
> value for "contains".

> This means that "prefixItems", "items", "contains", and all in-place
> applicators MUST be evaluated before this keyword can be evaluated.

> The annotation result of this keyword is the boolean true if any items in the
> array were evaluated with this schema's subschema. This annotation affects the
> behavior of "unevaluatedItems" in parent schemas.

> Omitting this keyword has the same assertion behavior as an empty schema.

Distilled:
- Like the tail form of [[items]], but the set of elements it governs is
  *whatever no applicator evaluated* — computed from the **transitive**
  annotation results of [[prefixItems]], [[items]], [[contains]], and every
  in-place applicator (`allOf`, `anyOf`, `oneOf`, `if`/`then`/ `else`, `$ref`).
- The spec is explicit that all those keywords **MUST be evaluated first** —
  this keyword is defined in terms of their annotation outputs, not the raw
  array.

## Support decision

**Support:** no — **rejected at load time (P6).**

`unevaluatedItems` has no coherent lowering in the strict subset:

- **P6 (strict subset), annotation dependency.** Like its object sibling
  [[unevaluatedProperties]], correct behavior requires a runtime
  annotation-collection pass over every array applicator before this schema can
  be applied to the remaining indices — a schema-interpreter runtime, not the
  flat (de)serializers the subset emits (**P1**/**P2**).
- **Degenerate in this subset.** [[prefixItems]], `anyOf`, `not`, and
  `if`/`then`/`else` are rejected. The admitted applicators leave no residual
  evaluated-index set to track: [[allOf]] is flattened at load, [[oneOf]] is a
  closed sum type, and a [[ref]] either names a model or folds its siblings into
  that same `allOf` merge. [[items]] already applies to **all** elements of a
  homogeneous list, and [[contains]] is a pure existential that constrains no
  positional tail. So no element is ever "unevaluated" in a way [[items]] does
  not already cover; `unevaluatedItems` would be either redundant with [[items]]
  or reach for the rejected [[prefixItems]] tuple tail. Rejecting it is the
  honest, unambiguous outcome (**P7**/**P7.1**).

Loader behavior:
- Any `unevaluatedItems` present → reject with a located diagnostic. This holds
  on a raw [[allOf]] branch, and on the implicit conjunct of a
  `$ref`-with-siblings: the location carries the branch index and the diagnostic
  is this one, never a merge conflict over two branches' differing values — a
  message that would imply the keyword is supported.
- The diagnostic points to [[items]] as the supported array element schema
  (`{type:array, items:{type:T}}` — a homogeneous list).

## Type mapping

None — rejected before any type is emitted. The reject is the shared loader's,
so C# fails the same way as every other target, with the same diagnostic.

## Validator mapping

None — rejected at load time, so there is no (de)serialize boundary and no
serialize-side behavior (**P12**).

## Property-testing matrix

### Rejected at load time (negative) — the whole surface

| Reason                               | Example                                                                      |
| ------------------------------------ | ---------------------------------------------------------------------------- |
| Annotation-dependent tail (P6)       | `{type:array, items:{type:string}, unevaluatedItems:false}`                  |
| Reaches for rejected tuple tail (P6) | `{type:array, prefixItems:[{type:string}], unevaluatedItems:{type:integer}}` |
| Bare `unevaluatedItems:true` (P6)    | `{type:array, unevaluatedItems:true}`                                        |

There are no accepted or runtime fixtures: the keyword never reaches code
generation.

## Interactions

- **[[items]]**: the supported homogeneous-list applicator. In this subset it
  already governs **every** element, so there is no unevaluated tail for this
  keyword to add — use [[items]] instead.
- **[[prefixItems]]**: rejected tuple applicator; its absence is why no
  positional tail is ever left unevaluated. Its spec already cross- references
  this keyword as rejected.
- **[[contains]]**: the supported scalar existential — evaluates matching
  elements but leaves no positional tail for this keyword to govern.
- **[[unevaluatedProperties]]**: the object-side sibling, also rejected per
  **P6** for the same annotation-dependency reason.
- **[[minItems]] / [[maxItems]] / [[uniqueItems]]**: array-level assertions,
  unaffected by this keyword's rejection.

## Ecosystem variance

| Source dialect           | Action                                                                                                               |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12      | Reject (P6).                                                                                                         |
| OpenAPI 3.1              | `unevaluatedItems` present (3.1 adopts 2020-12) → reject.                                                            |
| OpenAPI 3.0              | No `unevaluatedItems` keyword — nothing to reject.                                                                   |
| Swagger 2.0 / draft-4..7 | No `unevaluatedItems` keyword (draft-6/7 used `additionalItems` for the tuple tail — rejected with [[prefixItems]]). |

## See also

- [[items]] — the supported homogeneous-list applicator; use it instead.
- [[prefixItems]] — the rejected tuple applicator; owns the tuple-tail
  rejection.
- [[contains]] — the supported scalar array existential.
- [[unevaluatedProperties]] — the rejected object-side sibling.
