# `minimum`

Source: JSON Schema 2020-12, Validation vocabulary, §6.2.4 "Validation Keywords
for Numeric Instances → minimum".

Sets an **inclusive** lower bound on a numeric instance. A pure runtime
comparison assertion — no type impact. Mirror of [[maximum]]; the shared
numeric-bound machinery (per-language comparison, integer-vs-float exactness,
serialize-side re-check, combined-bound satisfiability) is documented once in
[[maximum]] and referenced here.

## Spec summary

Verbatim (2020-12 validation, §6.2.4):

> The value of "minimum" MUST be a number, representing an inclusive lower limit
> for a numeric instance.

> If the instance is a number, then this keyword validates only if the instance
> is greater than or exactly equal to "minimum".

Distilled:
- Value MUST be a JSON number.
- Instance valid iff `instance ≥ minimum`.
- Applies only to numeric instances; a `minimum` on a non-numeric [[type]] is
  rejected at load (**P7.1**).

## Support decision

**Support:** yes — runtime comparison assertion.

Same grounding as [[maximum]]: **P10** (enforced), **P11** (aggregated), **P12**
(shared `Validate` predicate, identical both directions). No effect on emitted
types.

Loader behavior (mirror of [[maximum]] with `≥`):
- Value not a number → reject.
- `minimum` on a non-numeric [[type]] → reject (**P7.1**).
- **Over an `integer` position the bound MUST be integer-valued** —
  `minimum:0.0` accepted (≡ `0`), `minimum:0.5` rejected with a fix-it (same
  exact-comparison rationale as [[maximum]], which also owns the rule that the
  *effective* position kind governs when the bound's own node declares a wider
  one).
- Over a `number` position any finite bound is accepted.
- `minimum` below the [[type]] integer cap `−(2^53−1)` over an `integer`
  position is redundant (cap already rejects) but allowed; a `minimum` *above*
  `+(2^53−1)` empties the accepted set and rejects ([[maximum]]).
- **`minimum` and `exclusiveMinimum` both present on the same node → reject
  (redundant)** — both are lower bounds, one always dominates; keep exactly one
  (**P7.1**, mirror of the [[maximum]] rule). The lower+upper mix `minimum` +
  `exclusiveMaximum` is *not* redundant (interval) and stays a satisfiability
  check. allOf tightening caveat: see [[maximum]] / [[allOf]].
- Combined-bound satisfiability and [[multipleOf]] emptiness are the [[maximum]]
  rules (see **Interactions → satisfiability** there): `minimum > maximum` →
  reject; `minimum ≥ exclusiveMaximum` → reject; integer interval with no
  integer → reject.

## Type mapping

None. Constraint lives only in the validator.

## Validator mapping

Per **P10**/**P11**. A single `≥` comparison against the fixed bound, identical
in both directions (shared `Validate`, **P12**). Same per-language strategy as
[[maximum]] with `< min` as the failing comparison:

| Language   | Strategy                                                                                                                                                                                                                                                                                                                     |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | `if v < min { push(Violation{Reason: fmt.Sprintf("must be >= %v, got %v", min, v)}) }` — a predicate in the shared `Validate`, applied identically on both directions' paths (**P12.2**), collecting into one `PayloadValidationError` application failure. Integer field compares `int64`; number field compares `float64`. |
| TypeScript | ``if (v < min) push(Violation{path, reason: `must be >= ${min}, got ${v}`})``, throw one `PayloadValidationError` application failure.                                                                                                                                                                                       |
| Python     | `if v < min: violations.append(Violation(path=…, reason=f"must be >= {min}, got {v}"))` in the transfer type converter, run after `_parse_spec_integer` normalizes an integer field's wire value (`0.0`→`0`, see [[type]]); aggregates into the single generated `PayloadValidationError` application failure.               |
| Java       | The per-POJO collecting deserializer (PRINCIPLES Java §5) reads the node via the [[type]] `SpecNumbers` helper and checks `v < min` (`long`/`double`), pushing a `Violation{path, "must be >= " + min + ", got " + v}` into the single `PayloadValidationError` application failure. Not bean-validation `@Min`.             |
| C#         | `Converter.Parse` reads the value via `SpecJson.SpecLong`/`SpecDouble` and checks `v < min` (`long`/`double`), recording `Violation{path, "must be >= " + min + ", got " + v}` into the single `PayloadValidationError` application failure; `Validate` repeats the check before writing.                                    |

Reason strings name the concrete bound and offending value
(`must be >= 0, got -1`), per the [[maximum]] convention.
Integer-field-vs-float-bound comparison is lossless within the cap — see
[[maximum]] (the `(double)cap == cap` guarantee applies identically).

### Serialize-side (P12)

Identical to [[maximum]]: the predicate re-runs before emit over the decoded
value; an in-memory value below `minimum` fails serialize rather than being
written. See [[maximum]] serialize note (symmetric).

## Property-testing matrix

### Accepted (positive)

| Shape                 | Example                                  |
| --------------------- | ---------------------------------------- |
| Integer inclusive min | `{type:"integer", minimum:0}`            |
| Non-negative via `.0` | `{type:"integer", minimum:0.0}`          |
| Number fractional min | `{type:"number", minimum:-1.5}`          |
| Single-value range    | `{type:"integer", minimum:5, maximum:5}` |

### Rejected at load time (negative)

| Reason                                    | Example                                                                                                          |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Value not a number                        | `minimum:"0"`, `minimum:false`                                                                                   |
| Type mismatch (P7.1)                      | `{type:"string", minimum:0}`                                                                                     |
| Fractional bound over an integer position | `{type:"integer", minimum:0.5}`, `{type:"array", items:{type:"integer"}, contains:{type:"number", minimum:1.5}}` |
| Range emptied by the integer cap          | `{type:"integer", minimum:9007199254740992}`                                                                     |
| Redundant same-axis pair                  | `{type:"integer", minimum:0, exclusiveMinimum:2}`, `{type:"integer", minimum:0, exclusiveMinimum:0}`             |
| Unsatisfiable range                       | `{type:"integer", minimum:10, maximum:2}`                                                                        |

### Runtime fixtures (validator)

- `v == min` → OK (`≥` inclusive).
- `v == min-1` (integer) / just below `min` (number) → one
  `PayloadValidationError` application failure whose reason names the bound and
  value (`must be >= 0, got -1`).
- Combined with other failing assertions → all reported in one shot (P11).
- Serialize of an in-memory value below `min` → rejected before emit (P12).

## Interactions

- **[[maximum]]**: the paired upper bound; `min > max` load error; `min == max`
  pins a single value (accepted). All combined-bound satisfiability rules live
  in [[maximum]].
- **[[exclusiveMinimum]]**: same-axis (both lower) — `minimum` and
  `exclusiveMinimum` on one node is a **load reject** (one always dominates;
  keep exactly one).
- **[[multipleOf]]**: combines for satisfiability (no multiple in range →
  reject; see [[multipleOf]]).
- **[[type]]**: gates applicability; integer cap is the implicit floor
  `−(2^53−1)` on integer fields.
- **[[const]] / [[default]] / [[enum]]**: a supplied numeric literal MUST
  satisfy `minimum` at load — e.g. `{type:"integer", minimum:5, const:2}` and
  `{…, default:0}` are load rejects. Closes the numeric portion of the deferred
  literal-vs-constraint obligation (see [[maximum]]).

## Ecosystem variance

| Source dialect        | Action                                                                                                                       |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12   | Native.                                                                                                                      |
| OpenAPI 3.1           | Native.                                                                                                                      |
| OpenAPI 3.0 / draft-4 | `minimum` (inclusive) identical. Their `exclusiveMinimum` is a *boolean* modifier — rewrite handled in [[exclusiveMinimum]]. |
| Swagger 2.0           | Same as OAS 3.0.                                                                                                             |

## See also

- [[maximum]] — the paired inclusive upper bound; owns the shared machinery.
- [[exclusiveMinimum]] — the strict (`>`) lower variant; owns the draft-4
  boolean-form rewrite.
- [[exclusiveMaximum]] — the strict (`<`) upper variant.
- [[multipleOf]], [[type]], [[const]], [[default]], [[enum]] — as in
  [[maximum]].
