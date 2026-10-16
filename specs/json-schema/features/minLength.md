# `minLength`

Source: JSON Schema 2020-12, Validation vocabulary, §6.3.2 "Validation Keywords
for Strings → minLength".

Sets an **inclusive** lower bound on the **length of a string** instance,
counted in Unicode **code points** (RFC 8259). A pure runtime assertion — no
type impact. Mirror of [[maxLength]]; the shared string-length machinery
(code-point counting, the per-language primitive choice, serialize-side
re-check, exact-length pin) is documented once in [[maxLength]] and referenced
here.

## Spec summary

Verbatim (2020-12 validation, §6.3.2):

> The value of this keyword MUST be a non-negative integer.

> A string instance is valid against this keyword if its length is greater than,
> or exactly equal to, the value of this keyword.

> The length of a string instance is defined as the number of its characters as
> defined by RFC 8259.

> Omitting this keyword has the same behavior as a value of 0.

Distilled:
- Value MUST be a **non-negative integer**.
- Instance valid iff `codePointCount(instance) ≥ minLength`.
- Length = Unicode **code point** count, **no normalization** — exactly
  [[maxLength]]'s definition (see there for the NFC/NFD example).
- `minLength: 0` is a **no-op** (spec: same as omitting) — accepted but vacuous.
- Applies only to string instances; a `minLength` on a non-string [[type]] is
  rejected at load (**P7.1**).

## Support decision

**Support:** yes — runtime code-point-count comparison, [[maxLength]] with `≥`.
Same grounding: **P1** (identical code-point count across all four — the crux,
documented in [[maxLength]]), **P10** (enforced), **P11** (aggregated), **P12**
(shared `Validate` predicate, identical both directions). No effect on emitted
types.

Loader behavior (mirror of [[maxLength]] with `≥`):
- Value not a non-negative integer → reject: non-number, **negative**
  (`minLength:-1`), or **fractional** (`minLength:0.5`). `minLength:5.0`
  accepted (≡ `5`).
- A bound above the portable length ceiling → reject, and a bound in
  `[2^31, 2^53−1]` obliges Java to emit a `long` literal — both exactly as in
  [[maxLength]].
- `minLength` on a non-string [[type]] → reject (**P7.1**).
- **`minLength:0`** → accepted, treated as **omitted** (the spec's explicit
  equivalence); it constrains nothing. (Not rejected as "redundant" — it is a
  spec-sanctioned identity, and authors emit it from templated tooling; silently
  honoring it as a no-op is friendlier than a diagnostic.) "Treated as omitted"
  is a rule about the **emitted code**, not only about the verdict:
  **no comparison is emitted for it**. A dead `≥ 0` check is not merely wasted —
  it makes an inert keyword an input to whatever else the emitter derives from
  "this node has a length bound".
- **`minLength` > `maxLength` → reject (unsatisfiable)**;
  `minLength == maxLength` pins an **exact** length (accepted). All
  combined-length satisfiability lives in [[maxLength]].
- A `const`/`default`/`enum` string literal on the **same node** shorter than
  `minLength` → reject at load (e.g. `{type:"string", minLength:5, const:"ab"}`)
  — the string-length half of the deferred literal-vs-constraint obligation (see
  [[maxLength]] / [[const]]).

## Type mapping

None. Constraint lives only in the validator.

## Validator mapping

Per **P10**/**P11**. A single `≥` comparison of the **code-point count** against
the fixed bound, identical in both directions (shared `Validate`, **P12**). Same
per-language strategy as [[maxLength]] with `< min` as the failing comparison —
how each language counts code points is the load-bearing detail
(`utf8.RuneCountInString` / Python `len` / `codePointCount(0,length())`; TS the
shared `codePointLength` scan with early-exit, see [[maxLength]]), never the
bare `len`/`.length`.

| Language   | Strategy                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Go         | `if n := utf8.RuneCountInString(v); n < min { push(Violation{Reason: fmt.Sprintf("must have length >= %d, got %d", min, n)}) }` — a predicate in the shared `Validate`, which `UnmarshalJSON` calls after decoding, collecting into one `PayloadValidationError` application failure.                                                                                                                                                                                                                                                      |
| TypeScript | The shared `codePointLength` surrogate-aware scan (see [[maxLength]]) with **early-exit** the moment the running count reaches `min` (pass — no need to count the rest). If the string ends first the full count `n` is already in hand, so the failure path needs **no second pass** (the asymmetry with [[maxLength]], where the over-length case must recount): ``push(Violation{path, reason: `must have length >= ${min}, got ${n}`})``, throw one `PayloadValidationError` application failure. **Never `v.length`** (UTF-16 units). |
| Python     | `if (n := len(v)) < min: violations.append(Violation(path=…, reason=f"must have length >= {min}, got {n}"))` in the transfer type converter — `len` on a `str` counts code points (see [[maxLength]]); aggregates into the single generated `PayloadValidationError` application failure.                                                                                                                                                                                                                                                  |
| Java       | The per-POJO collecting deserializer (PRINCIPLES Java §5) reads the `String`, checks `int n = v.codePointCount(0, v.length()); if (n < min)`, pushing a `Violation{path, "must have length >= " + min + ", got " + n}` into the single `PayloadValidationError` application failure. Not bean-validation `@Size`.                                                                                                                                                                                                                          |

Reason strings name the concrete bound and offending count
(`must have length >= 5, got 2`), per the [[maximum]] convention.

### Serialize-side (P12)

Identical to [[maxLength]]: the predicate re-runs before emit over the decoded
value; an in-memory under-length string fails serialize rather than being
written. See [[maxLength]] serialize note (symmetric), including the rule that
on a materialized node both boundaries count the **canonical** wire string —
projected **once per member** and shared with the closed-value comparison, since
two projections are two operands (**P12.2**).

## Property-testing matrix

### Accepted (positive)

| Shape                   | Example                                     |
| ----------------------- | ------------------------------------------- |
| Inclusive min           | `{type:"string", minLength:1}` (non-empty)  |
| `.0`-valued bound       | `{type:"string", minLength:1.0}`            |
| Zero min (no-op)        | `{type:"string", minLength:0}`              |
| Exact length (min==max) | `{type:"string", minLength:5, maxLength:5}` |

### Rejected at load time (negative)

| Reason                            | Example                                                       |
| --------------------------------- | ------------------------------------------------------------- |
| Value not a number                | `minLength:"1"`, `minLength:false`                            |
| Negative value                    | `minLength:-1`                                                |
| Fractional value                  | `minLength:0.5`                                               |
| Above the portable length ceiling | `minLength:9007199254740992`                                  |
| Type mismatch (P7.1)              | `{type:"integer", minLength:1}`                               |
| Unsatisfiable range               | `{type:"string", minLength:10, maxLength:2}`                  |
| Literal below bound               | `{type:"string", minLength:5, const:"ab"}`, `{…, default:""}` |

### Runtime fixtures (validator)

- `codePointCount(v) == min` → OK (`≥` inclusive).
- `v` one code point under `min` → one `PayloadValidationError` application
  failure naming the bound and count (`must have length >= 5, got 4`).
- **Astral fixtures:** `"😀"` counts as **1** (satisfies `minLength:1`); the
  empty string `""` counts as **0**. Every language agrees (see [[maxLength]]).
- Combined with other failing assertions → all reported in one shot (**P11**).
  Serialize of an under-length in-memory value → rejected before emit (**P12**).

## Interactions

- **[[maxLength]]**: the paired upper bound; `minLength > maxLength` load error;
  `minLength == maxLength` pins an exact length (accepted). All combined-length
  satisfiability rules live in [[maxLength]].
- **[[pattern]]**: independent; both apply and aggregate. No regex-vs-length
  satisfiability cross-check (see [[maxLength]]).
- **[[type]]**: gates applicability; `minLength` never changes the emitted type
  — `string`, unless a materializing sibling ([[format]] / [[contentEncoding]])
  governs it.
- **[[const]] / [[default]] / [[enum]]**: a supplied string literal MUST satisfy
  `minLength` at load (rule above). Closes the string-length portion of the
  deferred literal-vs-constraint obligation (see [[maxLength]], which also owns
  the rule for a **closed value set** on the same node — the count is taken over
  the value's underlying `string`).

## Ecosystem variance

| Source dialect        | Action                                                                       |
| --------------------- | ---------------------------------------------------------------------------- |
| JSON Schema 2020-12   | Native.                                                                      |
| OpenAPI 3.1           | Native.                                                                      |
| OpenAPI 3.0 / draft-4 | `minLength` present since draft-4, identical (code-point) semantics. Native. |
| Swagger 2.0           | Same as OAS 3.0.                                                             |

## See also

- [[maxLength]] — the paired inclusive upper bound; owns the shared code-point
  machinery and the astral/normalization discussion.
- [[pattern]] — the other string assertion (regex).
- [[type]] — supplies the emitted `string`; gates applicability.
- [[const]], [[default]], [[enum]] — supplied string literals validated against
  `minLength` at load.
- [[maximum]] / [[minimum]] — the numeric-bound family (same reason-string +
  exact-pin conventions).
