# `multipleOf`

Source: JSON Schema 2020-12, Validation vocabulary, §6.2.1 "Validation Keywords
for Numeric Instances → multipleOf".

Asserts that a numeric instance is an exact multiple of a divisor. A pure
runtime assertion — no type impact. The one numeric keyword with a genuine
floating-point hazard: **divisibility has no portable, intent-preserving answer
for fractional divisors**, so the supported form is narrowed to positive
**integer** divisors, where the check is exact and portable.

## Spec summary

Verbatim (2020-12 validation, §6.2.1):

> The value of "multipleOf" MUST be a number, strictly greater than 0.

> A numeric instance is valid only if division by this keyword's value results
> in an integer.

Distilled:
- Value MUST be a number `> 0`.
- Instance valid iff `instance / value` is an integer.
- Applies to any numeric instance (`integer` or `number`); a non-numeric
  [[type]] → load reject (**P7.1**).

## Support decision

**Support:** partial — **positive integer divisor only**. A fractional divisor
(`multipleOf: 0.1`, `2.5`, …) is **temporarily unsupported** (rejected at load,
deferred — *not* a categorical P6 exclusion). Applies to both `integer` and
`number` fields.

Rationale (citing [[PRINCIPLES.md]]):
- **P1 (identical cross-language validation).** Divisibility must produce the
  *same* accept/reject in C#, Go, TypeScript, Python, and Java for the same
  `(schema, instance)`. Empirically:
  - **Integer divisor → exact and portable.** Integer modulo (`integer` fields)
    and IEEE `fmod` (`number` fields) agree value-for-value across all five: Go
    `math.Mod`, Java `%`, C# `%`, JS `%`, and Python `math.fmod` return
    identical results for integer divisors (`10.0`/`6.0` accepted, `7.5`
    rejected, `1e300` accepted for divisor `2`).
  - **Fractional divisor → no defensible answer.** `fmod` treats the stored
    doubles literally, so `0.3 % 0.1 == 0.09999999999999998` and
    `1.1 % 0.1 == 2.77e-17` — i.e. C#/Go/Java/JS/Python all *reject* `0.3`
    against `multipleOf: 0.1`, which is not what an author writing
    `multipleOf: 0.1` means. The alternative — a **tolerant** divisibility
    check, of the kind several validation libraries ship — *accepts* `0.3`,
    `1.1` and `0.2`, but the tolerance is unspecified and per-library, so any
    target left on raw `fmod` then disagrees. Neither branch is reconcilable
    without imposing a shared decimal algorithm on every target.
- **P4 (minimal runtime deps).** A correct fractional check needs decimal
  scaling / big-decimal arithmetic; TypeScript and Go have no native decimal
  type, so we would have to ship one (P4 tension) *and* re-implement it
  identically four times to preserve P1. Not worth it for a rare form in v1.
- **P7 / P7.1 (reject ambiguity loudly).** A fractional divisor that "looks"
  like it should accept `0.3` but rejects it (or accepts it in one language
  only) is exactly the silently-incorrect output the mission forbids. Reject at
  load with a clear diagnostic instead.

Loader behavior:
- `multipleOf` not a number → reject.
- `multipleOf ≤ 0` (`0`, `-2`) → reject (spec MUST be `> 0`).
- `multipleOf` fractional (`0.1`, `2.5`, any non-integer-valued number) →
  **reject — temporarily unsupported**, with a "not yet supported" diagnostic
  (distinct from the `≤ 0` error). `multipleOf: 2.0` is accepted (≡ `2`,
  integer-valued, honoring the `1.0`-as-integer rule from [[type]]).
- `multipleOf` on a non-numeric [[type]] → reject (**P7.1**).
- Combined with a range ([[minimum]]/[[maximum]]/exclusive*): if
  **no multiple of the divisor lies in the accepted interval**, the schema is
  unsatisfiable → reject. The check is cheap: `floor(hi/m)*m ≥ lo` over the
  effective integer interval `[lo,hi]` (e.g.
  `{type:"integer", minimum:3, maximum:3, multipleOf:2}` → the only value `3` is
  not a multiple of `2` → reject). **The check binds over the whole binary64
  domain, not only over small magnitudes.** It is decided with the same
  `fmod`-based primitive as the runtime predicate, never by taking the
  fractional part of a `bound / divisor` quotient: above `2^52` a double
  quotient has no fractional part at all, so a rounding-to-integer step over it
  degenerates into a no-op and every large bound reports "satisfiable". That is
  a third divisibility semantics, disagreeing with all five runtimes.
  `{type:"number", minimum:1e23, maximum:1e23, multipleOf:5}` is a reject —
  `1e23` is not a multiple of `5` — exactly as its small-magnitude twin is.

**Deferred, not excluded.** A future lowering could support fixed-precision
fractional divisors via decimal scaling (multiply instance and divisor by `10^k`
to integers, then integer-divide), gated on all five targets agreeing. Revisit
on demand — mirrors [[patternProperties]]' "temporarily unsupported, plausibly
lowerable later" posture.

## Type mapping

None. The emitted field type is [[type]]'s primitive; the divisor lives only in
the validator.

## Validator mapping

Per **P10**/**P11**. A single divisibility predicate against the fixed integer
divisor `m`, identical in both directions (shared `Validate`, **P12**). The
predicate is defined per field kind:
- **`integer` field:** `v % m == 0` in exact integer arithmetic.
- **`number` field:** `fmod(v, m) == 0` (IEEE remainder — exact for the stored
  double).

| Language   | Strategy                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Go         | A predicate in the shared `Validate`, applied identically on both directions' paths (**P12.2**: sharing is a requirement on the predicate, not on the call graph). Integer field: `if v % m != 0 { push(Violation{Reason: fmt.Sprintf("must be a multiple of %v, got %v", m, v)}) }` (`int64`). Number field: same message when `math.Mod(v, m) != 0` (`float64`). Violations collect into one `PayloadValidationError` application failure.                                                                                                                                                                                                                                                                                                                                                                                         |
| TypeScript | ``if (v % m !== 0) push(Violation{path, reason: `must be a multiple of ${m}, got ${v}`})`` — `%` is IEEE `fmod`, and integer fields are safe integers so it is exact for both kinds. `m` is an emitted numeric constant. Throw one `PayloadValidationError` application failure.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Python     | An inline check in the transfer type converter (PRINCIPLES Python §3), emitted the same way TypeScript emits it rather than behind a runtime helper, appending `Violation(path, reason=f"must be a multiple of {m}, got {v}")`. **Integer field:** exact Python-`int` modulo, over the value `_parse_spec_integer` has already normalized. **Number field:** `math.fmod(v, m) != 0` — deliberately the same primitive as the other four rather than any *tolerant* native divisibility check, so the predicate is **bit-identical `fmod`** across targets (a tolerant check would only be safe because we reject the fractional divisors where the tolerance would bite; standardizing on `fmod` keeps the predicate provably identical instead). Aggregates into the single generated `PayloadValidationError` application failure. |
| Java       | The per-POJO collecting deserializer (PRINCIPLES Java §5) reads the field via the [[type]] `SpecNumbers` helper, then checks `v % m != 0` — `long % long` (integer field) or `double % double` (number field, IEEE `fmod`, matching the others) — pushing a `Violation{path, "must be a multiple of " + m + ", got " + v}` into the single `PayloadValidationError` application failure. Not `BigDecimal.remainder` (would risk decimal-vs-`fmod` divergence on the number path).                                                                                                                                                                                                                                                                                                                                                    |
| C#         | `Converter.Parse` reads the value via `SpecJson.SpecLong`/`SpecDouble`, then checks `v % m != 0` — `long % long` (integer field) or `double % double` (number field, IEEE `fmod`, matching the others) — recording `Violation{path, "must be a multiple of " + m + ", got " + v}`. Not `decimal` remainder.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |

Reason strings name the divisor and offending value
(`must be a multiple of 2, got 3`), per the [[maximum]] convention.

For `integer` fields the divisor and value are both within the [[type]] cap
`±(2^53−1)`, so integer modulo is exact everywhere (TS included — safe
integers). For `number` fields IEEE `fmod` is exact and portable across all five
(verified, including large integer-valued doubles like `1e300`).

### Serialize-side (P12)

The divisibility predicate is a shared-`Validate` check, so it
**re-runs before emit** over the decoded value — an in-memory value that is not
a multiple of `m` (a Go `int64` / Java `long` / Python `int` mutated or
constructed off-grid) fails serialize with the aggregated primitive rather than
being written. No parse-adapter-only or encode-adapter-only logic.

## Property-testing matrix

### Accepted (positive)

| Shape                      | Example                            |
| -------------------------- | ---------------------------------- |
| Integer divisor on integer | `{type:"integer", multipleOf:2}`   |
| Larger integer divisor     | `{type:"integer", multipleOf:100}` |
| `.0`-valued divisor        | `{type:"integer", multipleOf:2.0}` |
| Integer divisor on number  | `{type:"number", multipleOf:5}`    |

### Rejected at load time (negative)

| Reason                                    | Example                                                                |
| ----------------------------------------- | ---------------------------------------------------------------------- |
| Not `> 0`                                 | `multipleOf:0`, `multipleOf:-2`                                        |
| Fractional divisor (deferred)             | `multipleOf:0.1`, `multipleOf:2.5`, `{type:"number", multipleOf:0.01}` |
| Value not a number                        | `multipleOf:"2"`, `multipleOf:true`                                    |
| Type mismatch (P7.1)                      | `{type:"string", multipleOf:2}`                                        |
| Unsatisfiable with range                  | `{type:"integer", minimum:3, maximum:3, multipleOf:2}`                 |
| Unsatisfiable with range, large magnitude | `{type:"number", minimum:1e23, maximum:1e23, multipleOf:5}`            |

### Runtime fixtures (validator)

- **Integer field, `multipleOf:2`:** accept `4`, `0`, `-2`, `4.0` (spec-int
  normalizes then passes); reject `5`, `3`, `5.0`.
- **Number field, `multipleOf:3`:** accept `6.0`, `-9.0`, `0.0`,
  `1e300`-if-divisible; reject `7.5`, `4.0`.
- **No fractional-divisor surprises:** because `multipleOf:0.1` is a load
  reject, the `0.3 %` footgun never reaches runtime — the accepted forms are all
  `fmod`/integer-exact and agree across the five languages.
- Combined with a failing bound / sibling field → all reported in one shot
  (**P11**); serialize of an off-grid in-memory value → rejected (**P12**).

## Interactions

- **[[type]]**: gates applicability (`integer`/`number` only); a mismatch is a
  load reject (**P7.1**). The emitted type is `type`'s primitive. Integer fields
  ride the `±(2^53−1)` cap, which keeps integer modulo exact.
- **[[minimum]] / [[maximum]] / [[exclusiveMinimum]] / [[exclusiveMaximum]]**:
  combine for satisfiability — no multiple of the divisor in the accepted
  interval → load reject (rule above). This is the numeric analog of the
  count-satisfiability checks in [[maxProperties]] / [[minProperties]].
- **[[const]] / [[default]] / [[enum]]**: a supplied numeric literal MUST be a
  multiple of `m` at load — e.g. `{type:"integer", multipleOf:2, const:3}` and
  `{…, default:5}` are load rejects. Closes the numeric portion of the deferred
  literal-vs-constraint obligation the [[const]] and [[default]] specs flagged.

## Ecosystem variance

| Source dialect        | Action                                                                                                                                                                                                                                                                                      |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12   | Native. Integer divisors accepted; fractional divisors rejected (deferred).                                                                                                                                                                                                                 |
| OpenAPI 3.1           | Adopts 2020-12 — same.                                                                                                                                                                                                                                                                      |
| OpenAPI 3.0 / draft-4 | `multipleOf` present since draft-4 with identical semantics — **but** other toolchains accept fractional divisors (`multipleOf: 0.01` for currency). Such schemas need a rewrite to an integer divisor (e.g. model cents as an `integer`) or must wait for the deferred fractional support. |
| Swagger 2.0           | Same as OAS 3.0.                                                                                                                                                                                                                                                                            |

## See also

- [[maximum]], [[minimum]], [[exclusiveMaximum]], [[exclusiveMinimum]] — the
  numeric bound family; combine with `multipleOf` for satisfiability.
- [[type]] — supplies the emitted primitive and the integer cap; gates
  applicability.
- [[const]], [[default]], [[enum]] — supplied numeric literals are validated
  against `multipleOf` at load.
- [[patternProperties]] — the sibling "temporarily unsupported, plausibly
  lowerable later" decision posture.
