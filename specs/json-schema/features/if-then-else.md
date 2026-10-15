# `if` / `then` / `else`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.2.2.1–3 "Keywords
for Applying Subschemas Conditionally → if, then, else".

The **conditional applicator** trio: evaluate the `if` subschema (without
asserting it), and depending on whether it passed, additionally require the
`then` or the `else` subschema. **Not supported** — the trio is rejected at load
time per **P6** (categorical exclusion, not a deferral). It is the canonical
example of runtime **conditional shape**: the constraints — and therefore the
effective type — of an instance fork on a predicate evaluated against that same
instance at runtime.

## Spec summary

Verbatim (2020-12 core, Applicator, §10.2.2.1–3):

> This validation outcome of this keyword's subschema ["if"] has no direct
> effect on the overall validation result. Rather, it controls which of the
> "then" or "else" keywords are evaluated.

> Instances that successfully validate against this keyword's subschema ["if"]
> MUST also be valid against the subschema value of the "then" keyword, if
> present.

> Instances that fail to validate against this keyword's subschema ["if"] MUST
> also be valid against the subschema value of the "else" keyword, if present.

> ["then"/"else"] has no effect when "if" is absent.

Distilled:
- `if` is a **non-asserting probe**: its own pass/fail never directly decides
  validity; it only selects which of `then` / `else` is applied.
- If the instance matches `if`, it must additionally satisfy `then`; otherwise
  it must additionally satisfy `else`. Either of `then`/`else` may be omitted
  (an omitted branch is vacuously satisfied).
- The net effect is a **runtime branch on instance content**: the set of
  members, types, and constraints in force depends on a condition tested against
  the very instance being validated. Two instances of the "same" schema can be
  required to have entirely different shapes.

## Support decision

**Support:** no — the whole `if`/`then`/`else` trio is
**rejected at load time (P6).**

Conditional shape has no coherent lowering across the five targets:

- **P6 (strict subset), conditional shape.** The effective type forks on a
  runtime predicate — "*if* field `kind` is `"a"` *then* these members are
  required with these types, *else* those." Modeling that faithfully needs
  conditional-shape machinery: Go and Java have no single static type that says
  "this object has one set of required fields/constraints under condition C and
  a different set otherwise." This is the same wall [[dependentSchemas]] hits
  (it is literally `if {required:[…]} then <subschema>`) and the reason
  [[anyOf]] and [[not]] are rejected. The only lowerings are to forfeit typing
  or to synthesize conditional variants that do not round-trip identically
  across languages (**P1**).
- **P7 / P7.1 (reject ambiguity loudly).** A schema whose shape depends on
  evaluating a probe subschema at runtime is exactly the construct we error on
  at generator time rather than approximate. Emitting a type that silently drops
  the conditional constraint (or applies it in only some languages) is the
  silently-wrong output the mission forbids.
- **A non-asserting probe has no home.** `if` deliberately contributes nothing
  to validity on its own; there is no field, type, or predicate it maps to. Its
  only job is to steer `then`/`else`, which is precisely the runtime steering
  the subset excludes.

`then`/`else` are meaningless without `if` (the spec makes them no-ops when `if`
is absent), so the trio is rejected as a unit. A stray `then` or `else` with no
sibling `if` is a dead keyword — also rejected, with a diagnostic that it has no
effect.

Loader behavior:
- Any `if` (with or without `then`/`else`) present → reject with a located
  diagnostic. This holds on a raw [[allOf]] branch, and on the implicit conjunct
  of a `$ref`-with-siblings: the location carries the branch index and the
  diagnostic is this one, never a merge conflict over the keyword's value.
- A `then` or `else` present **without** an `if` → reject as a no-op keyword (it
  can never fire).
- The diagnostic offers the coherent alternatives:
  1. **[[oneOf]]** — when the conditional was encoding a *closed, exclusive*
     choice between shapes: model it as a discriminated union (distinct JSON
     kinds, or a shared required **`const`-tag** across object branches), which
     is a supported sum type.
  2. **[[dependentRequired]]** — when the only conditional effect was making
     other members required given a trigger member's *presence*
     (`if {required:["a"]} then {required:["b"]}` → `{"a":["b"]}`).
  3. **Unconditional [[properties]] + [[required]]** — when the fields are
     really always part of the shape and the condition was incidental.

## Type mapping

None — rejected before any type is emitted.

## Validator mapping

None — rejected at load time, so there is no (de)serialize boundary and no
serialize-side behavior (**P12**).

## Property-testing matrix

### Rejected at load time (negative) — the whole surface

| Reason                                           | Example                                                                                              |
| ------------------------------------------------ | ---------------------------------------------------------------------------------------------------- |
| Conditional shape (P6)                           | `{type:object, if:{properties:{kind:{const:"a"}}}, then:{required:["x"]}, else:{required:["y"]}}`    |
| Conditional required via presence (P6)           | `{type:object, if:{required:["a"]}, then:{required:["b"]}}` (use [[dependentRequired]])              |
| Conditional shape that is really a union (P6)    | `{if:{properties:{kind:{const:"cat"}}}, then:{…Cat…}, else:{…Dog…}}` (use a discriminated [[oneOf]]) |
| `if` with no `then`/`else` (non-asserting no-op) | `{type:object, if:{properties:{a:{type:string}}}}`                                                   |
| `then`/`else` with no `if` (dead keyword)        | `{type:object, then:{required:["x"]}}`                                                               |

There are no accepted or runtime fixtures: the keywords never reach code
generation.

## Interactions

- **[[dependentSchemas]]**: the closest supported/rejected boundary —
  `dependentSchemas` is exactly `if {required:[key]} then <subschema>`, and is
  rejected for this same conditional-shape reason. Its supported subset,
  [[dependentRequired]] (name-presence dependency only), is the fix when the
  conditional effect was purely "these other members become required."
- **[[oneOf]]**: the supported way to express a *closed* choice between shapes —
  a discriminated union. Most `if`/`then`/`else` written to switch an object's
  shape on a tag value should be a `oneOf` with a `const`-tag discriminator.
- **[[anyOf]]** / [[not]]: the other boolean-logic applicators rejected per
  **P6**; `if`/`then`/`else` is the *conditional* member of that rejected
  family, [[anyOf]] the *inclusive-or* member.
- **[[allOf]]** / **[[oneOf]]**: the two admitted applicators — an
  *unconditional* intersection that flattens at load ([[allOf]]) and a *closed*
  sum type with a decidable selector ([[oneOf]]). Neither has a
  runtime-conditional shape, which is the line `if`/`then`/`else` crosses.

## Ecosystem variance

| Source dialect            | Action                                                                |
| ------------------------- | --------------------------------------------------------------------- |
| JSON Schema 2020-12       | Reject (P6).                                                          |
| OpenAPI 3.1               | `if`/`then`/`else` present (3.1 adopts 2020-12) → reject.             |
| OpenAPI 3.0 / Swagger 2.0 | No `if`/`then`/`else` keywords — nothing to reject.                   |
| draft-7                   | `if`/`then`/`else` introduced here with identical semantics → reject. |
| draft-4..6                | No `if`/`then`/`else` keywords — nothing to reject.                   |

## See also

- [[dependentSchemas]] — the same conditional-shape rejection; its supported
  subset [[dependentRequired]] is the fix for presence-triggered required
  members.
- [[oneOf]] — the supported closed-choice-between-shapes construct
  (discriminated union); the fix for shape-switching conditionals.
- [[dependentRequired]] — name-presence dependency, the coherent lowering of
  `if {required} then {required}`.
- [[anyOf]] — the rejected inclusive-or applicator; the sibling rejection in the
  boolean-logic family.
- [[PRINCIPLES.md]] — **P1** (polyglot wire), **P6** (strict subset),
  **P7/P7.1** (reject ambiguity).
