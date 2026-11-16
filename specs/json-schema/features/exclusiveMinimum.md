# `exclusiveMinimum`

Source: JSON Schema 2020-12, Validation vocabulary, §6.2.5 "Validation Keywords
for Numeric Instances → exclusiveMinimum".

Sets an **exclusive** (strict) lower bound on a numeric instance. The strict-`>`
sibling of [[minimum]] and the mirror of [[exclusiveMaximum]]; it shares
[[maximum]]'s numeric-bound machinery and the draft-4 boolean-form rejection.

## Spec summary

Verbatim (2020-12 validation, §6.2.5):

> The value of "exclusiveMinimum" MUST be a number, representing an exclusive
> lower limit for a numeric instance.

> If the instance is a number, then the instance is valid only if it has a value
> strictly greater than (not equal to) "exclusiveMinimum".

Distilled:
- Value MUST be a JSON **number** (2020-12) — not a boolean (draft-4).
- Instance valid iff `instance > exclusiveMinimum`.
- Non-numeric [[type]] → load reject (**P7.1**).

## Support decision

**Support:** yes — runtime comparison assertion, [[minimum]] with `>` instead of
`≥`. Grounding: **P10**, **P11**, **P12**.

Loader behavior ([[minimum]] rules with `>`, plus the boolean reject):
- **Boolean value → reject (draft-4 / OAS 3.0 form).**
  `exclusiveMinimum: true`/`false` was the old modifier of a sibling `minimum`.
  Fix-it: rewrite `{minimum:N, exclusiveMinimum:true}` as
  `{exclusiveMinimum:N}`, and `{minimum:N, exclusiveMinimum:false}` as
  `{minimum:N}` (see [[exclusiveMaximum]] for the mirror rationale).
- Value neither number nor boolean → reject.
- Over an `integer` position the bound MUST be integer-valued (`0.0` ok, `0.5`
  reject), keyed on the *effective* kind of the bounded value — see [[maximum]].
- Over a `number` position any finite numeric bound is accepted.
- **`exclusiveMinimum` and `minimum` both present → reject (redundant)** — both
  are lower bounds, one always dominates; keep exactly one (**P7.1**, the
  [[maximum]] rule). The lower+upper interval `exclusiveMinimum` +
  `maximum`/`exclusiveMaximum` is fine.
- Satisfiability with the *upper* bounds (empty interval → reject) is the
  [[maximum]] rule: `exclusiveMinimum ≥ exclusiveMaximum`,
  `exclusiveMinimum ≥ maximum`, and an integer field with no integer strictly
  above the bound and below the ceiling are all unsatisfiable → reject.

## Type mapping

None. Constraint lives only in the validator.

## Validator mapping

Per **P10**/**P11**. [[minimum]]'s per-language strategy with `≤` as the failing
test (`v ≤ exclusiveMinimum` → a `Violation` reading
`must be > <exclMin>, got <v>`):

| Language   | Strategy                                                                                                                                                                                                                                                                 |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Go         | `if v <= exclMin { push(Violation{Reason: fmt.Sprintf("must be > %v, got %v", exclMin, v)}) }` — a predicate in the shared `Validate`, applied identically on both directions' paths (**P12.2**), collecting into one `PayloadValidationError` application failure.      |
| TypeScript | ``if (v <= exclMin) push(Violation{path, reason: `must be > ${exclMin}, got ${v}`})``.                                                                                                                                                                                   |
| Python     | `if v <= exclMin: violations.append(Violation(path=…, reason=f"must be > {exclMin}, got {v}"))` in the transfer type converter, after `_parse_spec_integer` normalizes an integer field's wire value (see [[type]]).                                                     |
| Java       | Collecting deserializer (PRINCIPLES Java §5) checks `v <= exclMin` via the `SpecNumbers` helper, pushing a `Violation{path, "must be > " + exclMin + ", got " + v}` into the `PayloadValidationError` application failure.                                               |
| C#         | `Converter.Parse` reads the value via `SpecJson.SpecLong`/`SpecDouble` and checks `v <= exclMin`, recording `Violation{path, "must be > " + exclMin + ", got " + v}` into the `PayloadValidationError` application failure; `Validate` repeats the check before writing. |

Reason strings name the bound and offending value (`must be > 0, got 0`), per
the [[maximum]] convention. Integer-vs-float exactness and the serialize-side
re-check are [[maximum]]'s (identical, `>` operator).

### Serialize-side (P12)

As [[maximum]]: the `>` predicate re-runs before emit; an in-memory value
`≤ exclusiveMinimum` fails serialize.

## Property-testing matrix

### Accepted (positive)

| Shape                 | Example                                                  |
| --------------------- | -------------------------------------------------------- |
| Integer exclusive min | `{type:"integer", exclusiveMinimum:0}` (min valid = 1)   |
| Number exclusive min  | `{type:"number", exclusiveMinimum:0.0}` (positive reals) |
| With inclusive upper  | `{type:"integer", exclusiveMinimum:0, maximum:100}`      |

### Rejected at load time (negative)

| Reason                            | Example                                                                                                      |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Draft-4 boolean form              | `{type:"integer", minimum:0, exclusiveMinimum:true}`                                                         |
| Value not a number                | `exclusiveMinimum:"0"`, `exclusiveMinimum:null`                                                              |
| Type mismatch (P7.1)              | `{type:"string", exclusiveMinimum:0}`                                                                        |
| Fractional bound on integer field | `{type:"integer", exclusiveMinimum:0.5}`                                                                     |
| Redundant same-axis pair          | `{type:"integer", minimum:0, exclusiveMinimum:2}`                                                            |
| Empty interval                    | `{type:"integer", exclusiveMinimum:1, exclusiveMaximum:2}`, `{type:"number", exclusiveMinimum:5, maximum:5}` |

### Runtime fixtures (validator)

- `v == exclMin` → **reject** (strict; boundary excluded) — the key difference
  from [[minimum]].
- `v == exclMin+1` (integer) / just above (number) → OK.
- `v == exclMin-1` → reject.
- Aggregation (P11) and serialize re-check (P12) as elsewhere.

## Interactions

- **[[minimum]]**: same-axis (both lower) — both present on one node is a
  **load reject** (one always dominates; keep exactly one).
- **[[exclusiveMaximum]]**: the strict upper sibling;
  `exclusiveMinimum ≥ exclusiveMaximum` → empty interval → load reject.
- **[[multipleOf]]**: combines for satisfiability (see [[multipleOf]]).
- **[[type]]**: gates applicability; integer cap is the implicit floor.
- **[[const]] / [[default]] / [[enum]]**: a supplied numeric literal MUST
  satisfy `> exclusiveMinimum` at load (e.g.
  `{type:"integer", exclusiveMinimum:0, default:0}` → reject).

## Ecosystem variance

| Source dialect        | Action                                                                                                     |
| --------------------- | ---------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12   | Native (numeric form).                                                                                     |
| OpenAPI 3.1           | Native.                                                                                                    |
| OpenAPI 3.0 / draft-4 | **`exclusiveMinimum` is a boolean** paired with `minimum`. Rejected at load with the rewrite fix-it above. |
| Swagger 2.0           | Same boolean form; same rewrite.                                                                           |

## See also

- [[minimum]] — the inclusive sibling.
- [[exclusiveMaximum]] — the mirror strict-upper variant (same boolean-form
  wrinkle); shared machinery lives in [[maximum]].
- [[maximum]], [[multipleOf]], [[type]], [[const]], [[default]], [[enum]] — as
  elsewhere in the family.
