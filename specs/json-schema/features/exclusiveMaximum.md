# `exclusiveMaximum`

Source: JSON Schema 2020-12, Validation vocabulary, §6.2.3 "Validation Keywords
for Numeric Instances → exclusiveMaximum".

Sets an **exclusive** (strict) upper bound on a numeric instance. The strict-`<`
sibling of [[maximum]]; it shares that spec's numeric-bound machinery and adds
one ecosystem wrinkle: in draft-4 / OpenAPI 3.0 `exclusiveMaximum` was a
**boolean**, and that form is rejected here.

## Spec summary

Verbatim (2020-12 validation, §6.2.3):

> The value of "exclusiveMaximum" MUST be a number, representing an exclusive
> upper limit for a numeric instance.

> If the instance is a number, then the instance is valid only if it has a value
> strictly less than (not equal to) "exclusiveMaximum".

Distilled:
- Value MUST be a JSON **number** (2020-12) — *not* a boolean (that was
  draft-4).
- Instance valid iff `instance < exclusiveMaximum`.
- Applies only to numeric instances; a non-numeric [[type]] → load reject
  (**P7.1**).

## Support decision

**Support:** yes — runtime comparison assertion, identical to [[maximum]] but
with `<` instead of `≤`. Grounding: **P10**, **P11**, **P12** (shared `Validate`
predicate).

Loader behavior (the [[maximum]] rules with `<`, plus the boolean-form reject):
- **Boolean value → reject (draft-4 / OAS 3.0 form).**
  `exclusiveMaximum: true`/`false` is the *old* meaning (a modifier that made a
  sibling `maximum` exclusive). In 2020-12 it MUST be a number. We reject the
  boolean form at load with a fix-it:
  **rewrite `{maximum: N, exclusiveMaximum: true}` as `{exclusiveMaximum: N}`**,
  and `{maximum: N, exclusiveMaximum: false}` as `{maximum: N}`. This is a
  common shape in imported OpenAPI 3.0 documents, so the diagnostic is explicit
  rather than a generic "not a number".
- Value neither number nor boolean → reject.
- Over an `integer` position the bound MUST be integer-valued (`5.0` ok, `5.5`
  reject), keyed on the *effective* kind of the bounded value — see [[maximum]].
- Over a `number` position any finite numeric bound is accepted.
- **`exclusiveMaximum` and `maximum` both present → reject (redundant)** — both
  are upper bounds, one always dominates; keep exactly one (**P7.1**, the
  [[maximum]] rule). Not to be confused with the lower+upper interval
  `exclusiveMaximum` + `minimum`/`exclusiveMinimum`, which is fine.
- Satisfiability with the *lower* bounds (empty interval → reject) is the
  [[maximum]] rule. Note the strict operator makes
  `exclusiveMinimum == exclusiveMaximum`, `minimum == exclusiveMaximum`, and an
  integer field with no integer strictly below the bound and above the floor all
  **unsatisfiable** → reject.

## Type mapping

None. Constraint lives only in the validator.

## Validator mapping

Per **P10**/**P11**. Exactly [[maximum]]'s per-language strategy with the
comparison changed to `≥` as the failing test (`v ≥ exclusiveMaximum` → a
`Violation` reading `must be < <exclMax>, got <v>`):

| Language   | Strategy                                                                                                                                                                                                                                                                          |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | The `if v >= exclMax { push(Violation{Reason: fmt.Sprintf("must be < %v, got %v", exclMax, v)}) }` predicate lives in the shared `Validate`, applied identically on both directions' paths (**P12.2**); violations collect into one `PayloadValidationError` application failure. |
| TypeScript | ``if (v >= exclMax) push(Violation{path, reason: `must be < ${exclMax}, got ${v}`})``.                                                                                                                                                                                            |
| Python     | `if v >= exclMax: violations.append(Violation(path=…, reason=f"must be < {exclMax}, got {v}"))` in the transfer type converter, after `_parse_spec_integer` normalizes an integer field's wire value (see [[type]]).                                                              |
| Java       | Collecting deserializer (PRINCIPLES Java §5) checks `v >= exclMax` via the [[type]] `SpecNumbers` helper, pushing a `Violation{path, "must be < " + exclMax + ", got " + v}` into the `PayloadValidationError` application failure.                                               |

Reason strings name the bound and offending value (`must be < 10, got 10`), per
the [[maximum]] convention. Integer-field-vs-float-bound exactness and the
serialize-side re-check are [[maximum]]'s (identical, `<` operator). No
parse/encode-adapter-only logic.

### Serialize-side (P12)

As [[maximum]]: the `<` predicate re-runs before emit; an in-memory value
`≥ exclusiveMaximum` fails serialize.

## Property-testing matrix

### Accepted (positive)

| Shape                 | Example                                                 |
| --------------------- | ------------------------------------------------------- |
| Integer exclusive max | `{type:"integer", exclusiveMaximum:10}` (max valid = 9) |
| Number exclusive max  | `{type:"number", exclusiveMaximum:1.0}`                 |
| With inclusive lower  | `{type:"integer", minimum:0, exclusiveMaximum:10}`      |

### Rejected at load time (negative)

| Reason                            | Example                                                                                                      |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Draft-4 boolean form              | `{type:"integer", maximum:10, exclusiveMaximum:true}`                                                        |
| Value not a number                | `exclusiveMaximum:"10"`, `exclusiveMaximum:null`                                                             |
| Type mismatch (P7.1)              | `{type:"string", exclusiveMaximum:5}`                                                                        |
| Fractional bound on integer field | `{type:"integer", exclusiveMaximum:5.5}`                                                                     |
| Redundant same-axis pair          | `{type:"integer", maximum:10, exclusiveMaximum:12}`                                                          |
| Empty interval                    | `{type:"integer", exclusiveMinimum:1, exclusiveMaximum:2}`, `{type:"number", minimum:5, exclusiveMaximum:5}` |

### Runtime fixtures (validator)

- `v == exclMax` → **reject** (strict; the boundary itself is invalid) — the key
  difference from [[maximum]].
- `v == exclMax-1` (integer) / just below (number) → OK.
- `v == exclMax+1` → reject.
- Combined failures aggregate in one shot (P11); serialize re-check (P12).

## Interactions

- **[[maximum]]**: same-axis (both upper) — both present on one node is a
  **load reject** (one always dominates; keep exactly one). All shared machinery
  and lower-vs-upper satisfiability live in [[maximum]].
- **[[exclusiveMinimum]]**: the strict lower sibling;
  `exclusiveMinimum ≥ exclusiveMaximum` is an empty interval → load reject.
- **[[multipleOf]]**: combines for satisfiability (see [[multipleOf]]).
- **[[type]]**: gates applicability; integer cap is the implicit outer bound.
- **[[const]] / [[default]] / [[enum]]**: a supplied numeric literal MUST
  satisfy `< exclusiveMaximum` at load (e.g.
  `{type:"integer", exclusiveMaximum:5, const:5}` → reject — the boundary is
  excluded).

## Ecosystem variance

| Source dialect        | Action                                                                                                                                                                                                                                                 |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| JSON Schema 2020-12   | Native (numeric form).                                                                                                                                                                                                                                 |
| OpenAPI 3.1           | Adopts 2020-12 — numeric `exclusiveMaximum`. Native.                                                                                                                                                                                                   |
| OpenAPI 3.0 / draft-4 | **`exclusiveMaximum` is a boolean** paired with `maximum`. Rejected at load with the rewrite fix-it above (`{maximum:N, exclusiveMaximum:true}` → `{exclusiveMaximum:N}`). This is the single largest source-dialect difference in the numeric family. |
| Swagger 2.0           | Same boolean form as OAS 3.0; same rewrite.                                                                                                                                                                                                            |

## See also

- [[maximum]] — the inclusive sibling; owns the shared numeric-bound machinery.
- [[exclusiveMinimum]] — the strict lower variant (same boolean-form wrinkle).
- [[minimum]], [[multipleOf]], [[type]], [[const]], [[default]], [[enum]] — as
  in [[maximum]].
