# `anyOf`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.2.1.2 "Keywords
for Applying Subschemas With Boolean Logic → anyOf".

The **inclusive-or** boolean-logic applicator: an instance is valid iff it
validates against **at least one** branch — and, unlike [[oneOf]], any number of
branches may match at once. **Not supported** — rejected at load time per **P6**
(categorical exclusion, not a deferral). It is the sibling [[oneOf]] earns its
place *against*: "exactly one" is a closed sum type with a decidable selector,
whereas "at least one" is an overlapping, non-discriminated choice with no
coherent typed lowering.

## Spec summary

Verbatim (2020-12 core, Applicator, §10.2.1.2):

> This keyword's value MUST be a non-empty array. Each item of the array MUST be
> a valid JSON Schema.

> An instance validates successfully against this keyword if it validates
> successfully against at least one schema defined by this keyword's value.

> Note that when annotations are being collected, all successful validations
> against subschemas MUST continue and their annotations collected.

Distilled:
- The value is a non-empty array of subschemas — the **branches**. The instance
  is valid iff **at least one** branch validates; **two or more** matching is
  explicitly fine (and, per the annotation note, *all* matching branches
  contribute — there is no single "the" branch).
- This is inclusive-or, not the exclusive-or of [[oneOf]] and not the
  intersection of [[allOf]]. The branches are free to **overlap**: a value may
  satisfy several at once, so there is no well-defined "which one" to bind to.
- Validity is defined by brute-force validation against the branches; the
  keyword says nothing about telling them apart, and — because overlap is
  permitted — nothing *can* be relied on to tell them apart.

## Support decision

**Support:** no — **rejected at load time (P6).**

`anyOf` has no coherent typed lowering in the strict subset:

- **P6 (strict subset), no decidable selector.** A faithful *typed* lowering of
  a choice needs to bind a wire value to **one** in-memory representation.
  [[oneOf]] can do this because "exactly one" guarantees the branches are
  disjoint and the generator further requires a decidable selector (a JSON type
  token, or a shared required-`const` tag) so a deserializer routes each value
  to a single branch without guessing. `anyOf` gives up both halves: branches
  **may overlap**, so even a brute-force "first branch that validates" is
  order-dependent and arbitrary, and there is no closed, disjoint selector to
  key on. There is no static type across the five targets for "a value that
  matches one *or more* of these possibly-overlapping shapes" — the only
  lowerings are to forfeit typing (a bare `any`/`interface{}`/`Object`) or to
  synthesize a union that silently changes meaning (turning inclusive-or into
  the exclusive-or of [[oneOf]], which does not round-trip identically, **P1**).
- **P7 / P7.1 (reject ambiguity loudly).** A choice whose branches can be
  simultaneously satisfied is exactly the ambiguity we refuse to guess at:
  binding it would require trial-validating every branch and then picking one by
  an invented rule (declaration order, "most fields matched"), which is
  silently-incorrect output. We reject at generator time instead.

Loader behavior:
- Any `anyOf` present → reject with a located diagnostic. This holds on a raw
  [[allOf]] branch, and on the implicit conjunct of a `$ref`-with-siblings: the
  location carries the branch index and the diagnostic is this one, never a
  merge conflict over the keyword's value.
- The diagnostic offers the coherent alternatives:
  1. **[[oneOf]]** — when the branches are genuinely a *closed, exclusive*
     choice: make them disjoint (distinct JSON kinds, or a shared required
     **`const`-tag** across object branches) and the choice is a supported sum
     type.
  2. **[[allOf]]** — when the intent was "must satisfy all of these" (the
     branches were being *combined*, not chosen between); `allOf` merges them
     into one schema at load ([[allOf]]).
  3. A single widened branch — when the branches differ only in a constraint
     that is really optional, drop to the common supertype and express the
     variation with ordinary optional members.

## Type mapping

None — rejected before any type is emitted.

## Validator mapping

None — rejected at load time, so there is no (de)serialize boundary and no
serialize-side behavior (**P12**).

## Property-testing matrix

### Rejected at load time (negative) — the whole surface

| Reason                                               | Example                                                                                                           |
| ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Inclusive-or, no decidable selector (P6)             | `{anyOf:[{type:string},{type:integer}]}` (use [[oneOf]] — disjoint kinds)                                         |
| Overlapping object branches (P6/P7.1)                | `{anyOf:[{type:object,properties:{a:{…}}},{type:object,properties:{b:{…}}}]}` (both match `{}`)                   |
| Constraint-only union that was really a combine (P6) | `{anyOf:[{type:string,minLength:3},{type:string,maxLength:10}]}` (use [[allOf]] to intersect, or a single branch) |
| Single-branch wrapper (P7.1)                         | `{anyOf:[{type:string}]}` (use the branch directly)                                                               |
| Empty array (invalid schema)                         | `{anyOf:[]}`                                                                                                      |

There are no accepted or runtime fixtures: the keyword never reaches code
generation.

## Interactions

- **[[oneOf]]**: the supported neighbor. `oneOf` is admitted because "exactly
  one" is a closed sum type *and* the generator requires a decidable selector;
  `anyOf` relaxes "exactly one" to "at least one" and permits overlap, removing
  exactly the disjointness/selector that makes the lowering coherent. Converting
  a genuinely exclusive `anyOf` to a discriminated [[oneOf]] is the primary fix.
- **[[allOf]]**: the intersection applicator. When branches were meant to be
  *combined* rather than chosen between, `allOf` is the coherent keyword — it
  merges/flattens to one schema at load.
- **[[nullability]]**: nullability is expressed with the two-branch
  `oneOf:[{T},{null}]` pattern, never `anyOf`.
- **[[dependentSchemas]]** / [[not]] / `if`/`then`/`else`: the other
  boolean-logic / conditional applicators rejected per **P6**; see
  [[if-then-else]] for the conditional-shape rationale.

## Ecosystem variance

| Source dialect      | Action                                                                                         |
| ------------------- | ---------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12 | Reject (P6).                                                                                   |
| OpenAPI 3.1         | `anyOf` present (3.1 adopts 2020-12) → reject; the fix-it points at a discriminated [[oneOf]]. |
| OpenAPI 3.0         | `anyOf` exists with the same inclusive-or semantics → reject.                                  |
| Swagger 2.0         | No `anyOf` keyword — nothing to reject.                                                        |
| draft-4..7          | `anyOf` present since draft-4 with identical semantics → reject.                               |

## See also

- [[oneOf]] — the supported exclusive-choice counterpart (closed sum type with a
  decidable selector); the primary fix for a genuinely exclusive `anyOf`.
- [[allOf]] — the intersection applicator; the fix when branches were meant to
  be combined, not chosen between.
- [[if-then-else]] — the rejected conditional applicator, sharing the P6
  no-coherent-lowering rationale.
- [[dependentSchemas]] — another rejected applicator with the same P6 grounding.
- [[PRINCIPLES.md]] — **P1** (polyglot wire), **P6** (strict subset),
  **P7/P7.1** (reject ambiguity).
