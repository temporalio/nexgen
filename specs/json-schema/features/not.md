# `not`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.2.1.4 "Keywords
for Applying Subschemas With Boolean Logic → not".

The **negation** boolean-logic applicator: an instance is valid iff it **fails**
to validate against the subschema. **Not supported** — rejected at load time per
**P6** (categorical exclusion, not a deferral). Where [[allOf]] (intersection)
and [[oneOf]] (a closed sum type) construct a type out of positive assertions,
`not` does the opposite: it names a **complement** — the open set of everything
a subschema *rejects* — which has no members, no kind, and no shape to lower to.

## Spec summary

Verbatim (2020-12 core, Applicator, §10.2.1.4):

> This keyword's value MUST be a valid JSON Schema.

> An instance is valid against this keyword if it fails to validate successfully
> against the schema defined by this keyword's value.

Distilled:
- The value is a single subschema. The instance is valid iff it does **not**
  validate against that subschema — validity is inverted.
- This is set **complement**, not a choice ([[oneOf]]/[[anyOf]]) or a
  combination ([[allOf]]). `not: S` denotes
  *the universe minus everything S accepts* — an open, unbounded set defined
  entirely by exclusion.
- The keyword asserts what an instance **is not**. It contributes no type, no
  member, no positive constraint of its own; its only content is the subschema
  being negated.

## Support decision

**Support:** no — **rejected at load time (P6).**

Negation has no coherent typed lowering in the strict subset:

- **P6 (strict subset), a complement is not a type.** The subset builds each
  type out of **positive assertions** — a [[type]] token, [[properties]],
  bounds, a [[const]]/[[enum]] value set. `not` supplies none of these; it
  describes the *anti-set* of a subschema. Even the simplest case,
  `not: {type: string}` ("anything except a string"), has no static type across
  the four targets: it spans every other JSON kind at once, so the only
  lowerings are to forfeit typing (`any`/`interface{}`/`Object`) or to invent a
  positive enumeration of "all kinds but one" that no source actually stated
  (**P1** — it would not round-trip identically). Negating a *constraint*
  (`not: {maximum: 10}`, `not: {const: "x"}`) is no better: the result is still
  an open complement — "any number strictly greater than 10, or not a number at
  all", "any value that is not the string `x`" — with no bounded, materializable
  shape.
- **P7 / P7.1 (reject ambiguity loudly).** A schema whose admissible values are
  defined by what they *exclude* cannot be turned into a concrete type without
  guessing which positive shape the author meant. Emitting `any` (or silently
  dropping the negation) is exactly the silently-wrong output the mission
  forbids; we error at generator time instead.
- **No positive content to attach a validator to.** Unlike a bound or a pattern,
  `not` has no value to check *for* — only a subschema to check *against* and
  then invert. There is no field, type, or predicate it maps to; its whole job
  is to reject, which is the anti-constructive move the subset excludes.

Two degenerate forms are incoherent rather than merely un-lowerable, and are
rejected as such:

- **`not: {}` / `not: true`** — negating the always-valid schema means
  **nothing** validates: the enclosing schema rejects every instance. An
  unsatisfiable type is a dead type — reject (**P7.1**).
- **`not: false`** — negating the always-invalid schema means **everything**
  validates: the keyword is a no-op that constrains nothing. A dead keyword —
  reject with a diagnostic that it has no effect.

Loader behavior:
- Any `not` present → reject with a located diagnostic (recurse into the negated
  subschema for validity, but never lower it). This holds on a raw [[allOf]]
  branch, and on the implicit conjunct of a `$ref`-with-siblings: the location
  carries the branch index and the diagnostic is this one, never a merge
  conflict over the keyword's value.
- `not: {}` / `not: true` → reject as **unsatisfiable** (accepts no instance).
- `not: false` → reject as a **no-op** (accepts every instance).
- The diagnostic offers the coherent alternatives:
  1. **Positive [[type]] / constraints** — state what values *are* allowed
     rather than what they are not (`not: {type: string}`, where the intent was
     "a number", → `type: number`).
  2. **[[enum]] / [[const]]** — for a *closed* value set, enumerate the
     admissible values instead of excluding others; note that excluding a value
     from an **open** domain ("any string except `x`") is *not* a closed set and
     remains inexpressible.
  3. **The complementary bound** — when negating a bound is really the opposite
     bound, use it directly (`not: {maximum: 10}` meaning "> 10" →
     [[exclusiveMinimum]] `10`; `not: {minimum: 0}` meaning "< 0" →
     [[exclusiveMaximum]] `0`).

## Type mapping

None — rejected before any type is emitted.

## Validator mapping

None — rejected at load time, so there is no (de)serialize boundary and no
serialize-side behavior (**P12**).

## Property-testing matrix

### Rejected at load time (negative) — the whole surface

| Reason                                   | Example                                                                                      |
| ---------------------------------------- | -------------------------------------------------------------------------------------------- |
| Negated kind — open complement (P6)      | `{not:{type:string}}` (state the positive [[type]])                                          |
| Negated bound (P6)                       | `{not:{maximum:10}}` (use [[exclusiveMinimum]] `10` if "> 10")                               |
| Negated value — open exclusion (P6/P7.1) | `{not:{const:"x"}}`, `{not:{enum:["a","b"]}}` (enumerate the *admissible* set with [[enum]]) |
| Negated object shape (P6)                | `{not:{required:["a"]}}` ("must not have `a`" — no positive shape)                           |
| Unsatisfiable — negates always-valid     | `{not:{}}`, `{not:true}` (accepts no instance)                                               |
| No-op — negates always-invalid           | `{not:false}` (accepts every instance)                                                       |
| Double negation                          | `{not:{not:{type:string}}}` (still a combinator; reject — write `type:string`)               |

There are no accepted or runtime fixtures: the keyword never reaches code
generation.

## Interactions

- **[[allOf]]**: the admitted intersection applicator — it *combines* positive
  schemas and flattens to one materialized type at load. `not` is its opposite:
  it removes from the universe rather than combining, so it does not flatten and
  has nothing to merge. An [[allOf]] branch that is itself a `not` is rejected
  for this reason ([[allOf]]).
- **[[oneOf]]**: the admitted closed sum type with a decidable selector. It
  earns its place by being *constructive and disjoint*; `not` is neither — it
  names a complement, not one of a closed set of positive branches.
- **[[anyOf]]** / **[[if-then-else]]**: the other boolean-logic / conditional
  applicators rejected per **P6**. `not` is the **negation** member of that
  rejected family — [[anyOf]] the *inclusive-or*, [[if-then-else]] the
  *conditional* — all three lacking the positive, decidable shape that admits
  [[allOf]] and [[oneOf]].
- **[[enum]] / [[const]]**: the coherent way to express a *closed* set of
  admissible values; the fix when `not: {const}` / `not: {enum}` was reaching
  for exclusion but the real intent was a bounded, positive value set.
- **[[exclusiveMinimum]] / [[exclusiveMaximum]]**: the complementary bounds a
  negated [[maximum]]/[[minimum]] should be rewritten to when the negation was
  really "the other side of the bound".

## Ecosystem variance

| Source dialect      | Action                                                                                                           |
| ------------------- | ---------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12 | Reject (P6).                                                                                                     |
| OpenAPI 3.1         | `not` present (3.1 adopts 2020-12) → reject; the fix-it points at a positive [[type]]/constraint or an [[enum]]. |
| OpenAPI 3.0         | `not` exists with the same negation semantics → reject.                                                          |
| Swagger 2.0         | No `not` keyword — nothing to reject.                                                                            |
| draft-4..7          | `not` present since draft-4 with identical semantics → reject.                                                   |

## See also

- [[allOf]] — the admitted intersection applicator that *combines* positive
  schemas and flattens at load; `not` is the non-combining complement.
- [[oneOf]] — the admitted closed sum type; constructive and disjoint where
  `not` is neither.
- [[anyOf]] — the rejected inclusive-or applicator; a sibling rejection in the
  boolean-logic family.
- [[if-then-else]] — the rejected conditional applicator; the other member of
  the rejected boolean-logic / conditional family.
- [[enum]] / [[const]] — the closed-value-set fix for exclusion-by-`not`.
- [[exclusiveMinimum]] / [[exclusiveMaximum]] — the complementary bounds a
  negated [[maximum]]/[[minimum]] rewrites to.
- [[PRINCIPLES.md]] — **P1** (polyglot wire), **P6** (strict subset),
  **P7/P7.1** (reject ambiguity).
