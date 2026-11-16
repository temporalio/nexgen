# `unevaluatedProperties`

Source: JSON Schema 2020-12, Core (Unevaluated vocabulary), §11.3 "Keywords for
Applying Subschemas to Objects with Unevaluated Properties →
unevaluatedProperties".

The **annotation-aware** object catch-all: it constrains members left
unevaluated after *all* applicators in scope have run. **Not supported** —
rejected at load time per **P6** (categorical exclusion, not a deferral).

## Spec summary

Verbatim (2020-12 core, Unevaluated vocabulary):

> The value of "unevaluatedProperties" MUST be a valid JSON Schema.

> The behavior of this keyword depends on the annotation results of adjacent
> keywords that apply to the instance location being validated. Specifically,
> the annotations from "properties", "patternProperties", and
> "additionalProperties", which can come from those keywords when they are
> adjacent to the "unevaluatedProperties" keyword. Those three annotations, as
> well as "unevaluatedProperties", can also result from any and all adjacent
> in-place applicator keywords. This includes but is not limited to the in-place
> applicators defined in this document.

> Validation with "unevaluatedProperties" applies only to the child values of
> instance names that do not appear in the "properties", "patternProperties",
> "additionalProperties", or "unevaluatedProperties" annotation results that
> apply to the instance location being validated.

> For all such properties, validation succeeds if the child instance validates
> against the "unevaluatedProperties" schema.

> The annotation result of this keyword is the set of instance property names
> validated by this keyword's subschema. This annotation affects the behavior of
> "unevaluatedProperties" in parent schemas.

> Omitting this keyword has the same assertion behavior as an empty schema.

Distilled:
- Like [[additionalProperties]], but the set of members it governs is
  *whatever no applicator evaluated* — the difference of the instance's member
  set and the **transitive** evaluated-name annotation gathered across
  `properties`, `patternProperties`, `additionalProperties`,
  **and every in-place applicator** (`allOf`, `anyOf`, `oneOf`, `if`/`then`/
  `else`, `$ref`, nested `unevaluatedProperties`).
- Its meaning at a location cannot be computed without first fully evaluating
  those siblings and collecting their annotation results — it is defined
  *in terms of* other keywords' outputs, not the raw instance.

## Support decision

**Support:** no — **rejected at load time (P6).**

`unevaluatedProperties` has no coherent lowering in the strict subset:

- **P6 (strict subset), annotation dependency.** Correct behavior requires a
  runtime **annotation-collection pass** — evaluate every in-place applicator,
  union the property names each one marked evaluated, then apply this schema to
  the remainder. That is a schema-interpreter runtime, the opposite of the flat,
  hand-written-feeling (de)serializers the subset emits (**P1**/**P2**). There
  is no static struct shape that captures "the members no branch happened to
  touch."
- **Redundant or incoherent in this subset — never in between.** The in-place
  applicators that would make it *differ* from [[additionalProperties]] either
  leave **no unevaluated residue** or are **rejected**. `anyOf`, `not`,
  `if`/`then`/`else`, [[patternProperties]], and [[prefixItems]] are rejected.
  The admitted applicators do not reintroduce the problem: [[allOf]] is
  **flattened to a single schema at load** (its branches' `properties` are
  merged in, so nothing survives as an in-place sibling), [[oneOf]] is a
  **closed sum type** whose object branches are complete, discriminated types
  (no shared residual object shape at the location), and a [[ref]] either names
  a model or folds its siblings into that same `allOf` merge. With no residual
  in-place applicator, the transitive evaluated set collapses to exactly
  [[properties]]'s matched names, so `unevaluatedProperties` would mean
  *precisely* what [[additionalProperties]] already means. Supporting it would
  add a second spelling of one behavior (violating the one-canonical-spelling
  stance the subset takes elsewhere) while its only *distinct* uses depend on
  rejected keywords. Rejecting it is the honest, unambiguous outcome
  (**P7**/**P7.1**).

Loader behavior:
- Any `unevaluatedProperties` present → reject with a located diagnostic. This
  holds on a raw [[allOf]] branch, and on the implicit conjunct of a
  `$ref`-with-siblings: the location carries the branch index and the diagnostic
  is this one, never a merge conflict over two branches' differing values — a
  message that would imply the keyword is supported.
- The diagnostic points to [[additionalProperties]] as the supported object
  catch-all: `false` for a closed struct, `true`/`{type:T}` for an open or typed
  one.

## Type mapping

None — rejected before any type is emitted. The reject is the shared loader's,
so C# fails the same way as every other target, with the same diagnostic.

## Validator mapping

None — rejected at load time, so there is no (de)serialize boundary and no
serialize-side behavior (**P12**).

## Property-testing matrix

### Rejected at load time (negative) — the whole surface

| Reason                                 | Example                                                                              |
| -------------------------------------- | ------------------------------------------------------------------------------------ |
| Annotation-dependent catch-all (P6)    | `{type:object, properties:{id:{type:integer}}, unevaluatedProperties:false}`         |
| Typed unevaluated schema (P6)          | `{type:object, properties:{id:{type:integer}}, unevaluatedProperties:{type:string}}` |
| Bare `unevaluatedProperties:true` (P6) | `{type:object, unevaluatedProperties:true}`                                          |

There are no accepted or runtime fixtures: the keyword never reaches code
generation.

## Interactions

- **[[additionalProperties]]**: the supported subset of this behavior. It looks
  only at the *local* [[properties]]/[[patternProperties]] matched names, which
  is statically knowable and lowers to a named catch-all member;
  `unevaluatedProperties` reaches across applicators and does not. The
  [[additionalProperties]] spec already records this keyword as rejected.
- **[[properties]]**: in this subset the transitive evaluated set is just its
  matched names, which is why `unevaluatedProperties` collapses to
  [[additionalProperties]] here.
- **[[patternProperties]]** / **[[prefixItems]]**: rejected applicators whose
  absence removes the only cases where `unevaluatedProperties` would differ from
  [[additionalProperties]].
- **[[unevaluatedItems]]**: the array-side sibling, also rejected per **P6** for
  the same annotation-dependency reason.

## Ecosystem variance

| Source dialect           | Action                                                         |
| ------------------------ | -------------------------------------------------------------- |
| JSON Schema 2020-12      | Reject (P6).                                                   |
| OpenAPI 3.1              | `unevaluatedProperties` present (3.1 adopts 2020-12) → reject. |
| OpenAPI 3.0              | No `unevaluatedProperties` keyword — nothing to reject.        |
| Swagger 2.0 / draft-4..7 | No `unevaluatedProperties` keyword — nothing to reject.        |

## See also

- [[additionalProperties]] — the supported, statically-knowable object
  catch-all; use it instead.
- [[properties]] — declares the members whose matched names are the transitive
  evaluated set in this subset.
- [[unevaluatedItems]] — the rejected array-side sibling.
- [[dependentSchemas]] — the other rejected conditional/annotation-driven object
  applicator.
