# `maxLength`

Source: JSON Schema 2020-12, Validation vocabulary, §6.3.1 "Validation Keywords
for Strings → maxLength".

Sets an **inclusive** upper bound on the **length of a string** instance, where
length is the count of Unicode **code points** (RFC 8259), *not* bytes, UTF-16
code units, or grapheme clusters. A pure runtime assertion — no type impact. The
canonical spec for the string-length pair; [[minLength]] shares the machinery
documented here and differs only in the comparison operator. The one
string-length hazard is that the naive "length" primitive counts the
**wrong unit** in three of the four targets (Go bytes, TS/Java UTF-16 units) —
so the shared predicate is pinned to a code-point count, verified across all
four targets.

## Spec summary

Verbatim (2020-12 validation, §6.3.1):

> The value of this keyword MUST be a non-negative integer.

> A string instance is valid against this keyword if its length is less than, or
> exactly equal to, the value of this keyword.

> The length of a string instance is defined as the number of its characters as
> defined by RFC 8259.

Distilled:
- Value MUST be a **non-negative integer**.
- Instance valid iff `codePointCount(instance) ≤ maxLength`.
- "Number of characters as defined by RFC 8259" = number of
  **Unicode code points** (RFC 8259 §7 defines a JSON string as a sequence of
  Unicode code points). **No normalization** is applied: precomposed `é` (NFC,
  `U+00E9`) is length 1, decomposed `é` (NFD, `e`+`U+0301`) is length 2 — and
  every target agrees on each, because none normalizes.
- Applies **only** to string instances; the spec silently ignores it for
  non-strings. Per **P7.1** we instead reject a `maxLength` on a non-string
  [[type]] at load time.
- Pure assertion; no annotation behavior.

## Support decision

**Support:** yes — runtime code-point-count comparison.

Lowers to a single length comparison in every language; no effect on emitted
types. Citing [[PRINCIPLES.md]]: **P1** (the count must be *identical* across
all four — the crux, see below), **P10** (enforced at the boundary), **P11**
(aggregated), **P12** (a pure predicate over the decoded value in the
**shared `Validate`** layer — identical in both directions, no parse/encode
adapter logic of its own).

**The P1 crux — count code points, not the native "length".** The naive
per-language length primitive counts a different unit in three of the four
targets, so a schema like `{type:"string", maxLength:3}` against the 4-byte /
2-UTF-16-unit / 3-code-point string `"a😀b"` would *disagree* value-for-value if
each language used its default. Verified in `string_probe` (`"a😀b"`): Go
`len` = **6** (UTF-8 bytes), JS `.length` = **4** and Java `.length()` = **4**
(UTF-16 code units), Python `len` = **3** (code points). Only the code-point
count (3) is spec-correct and portable, so the shared predicate is pinned to it
and each language uses its code-point-counting primitive (table below),
**never** the bare `len`/`.length`. This is the string analog of
[[multipleOf]]'s "standardize on the portable operation across all four"
decision.

Loader behavior:
- Value not a non-negative integer → reject: a non-number (`maxLength:"5"`,
  `maxLength:true`, `maxLength:null`), a **negative** value (`maxLength:-1`), or
  a **fractional** value (`maxLength:5.5`). `maxLength:5.0` is accepted (≡ `5`,
  honoring the `1.0`-as-integer rule from [[type]]).
- A bound greater than `9007199254740991` (2^53−1) → reject. A length bound must
  be exactly representable in every target, which is the same ceiling and the
  same reason the count keywords use ([[maxItems]] / [[maxProperties]]); the
  bound is emitted as a numeric literal into all four, and above 2^53−1 the
  literal no longer denotes the authored integer.
- **A bound in `[2^31, 2^53−1]` obliges Java to emit a `long` literal**
  (`{n}L`), exactly as in [[maxItems]]. Java integer literals stop at 2^31−1, so
  a bound above it emitted bare is a `javac` "integer number too large" error —
  the bound loads and the generated package does not compile. The ceiling above
  is *not* what makes that range safe.
- `maxLength` on a non-string [[type]] (`{type:"integer", maxLength:5}`) →
  reject per **P7.1** (statically meaningless — the array-length analog is
  `maxItems`, the member-count analog is [[maxProperties]]).
- **`minLength` > `maxLength` on the same node → reject (unsatisfiable).**
  `minLength == maxLength` pins an **exact** length (accepted — a fixed-width
  string). See **Interactions → satisfiability**.
- A `const`/`default`/`enum` string literal on the **same node** whose
  code-point length exceeds `maxLength` → reject at load. This closes, for the
  string-length constraints, the "validate the literal against constraint
  keywords" obligation the [[const]] and [[default]] specs deferred (e.g.
  `{type:"string", maxLength:2, const:"abc"}` is a load reject).

## Type mapping

None. The bound lives only in the validator and never changes the emitted field
type — [[type]]'s `string`, unless a materializing sibling ([[format]] temporal
/ [[contentEncoding]] bytes) governs it (the bound then checks the encoded wire
string).

## Validator mapping

Per **P10**/**P11**. A single `≤` comparison of the **code-point count** against
the fixed bound, identical in both directions (a pure predicate over the decoded
value — the **shared `Validate`** layer of **P12**). The per-language row
differs only in how it counts code points; all four are verified equal in
`string_probe`. Go/Python/Java lean on a stdlib primitive
(`utf8.RuneCountInString` / `len` / `codePointCount`); TS has no allocation-free
code-point-count primitive, so it emits a small surrogate-aware scan once as a
shared `codePointLength` helper and, for the length assertions,
**early-exits against the bound** rather than counting the whole string (see the
TS row).

| Language   | Strategy                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | The comparison is a predicate in the shared `Validate(model)` (`if n := utf8.RuneCountInString(v); n > max { push(Violation{Path, Reason: fmt.Sprintf("must have length <= %d, got %d", max, n)}) }`), which the generated `UnmarshalJSON` calls after decoding; violations collect into one `PayloadValidationError` application failure. **`utf8.RuneCountInString`, not `len`** — `len` is the UTF-8 byte count (verified `len("a😀b") == 6`).                                                                                                                                                                                                                                                                                                                                                                                           |
| TypeScript | An **allocation-free** surrogate-aware pass that **early-exits** the moment the bound is crossed: walk `v` by UTF-16 unit counting code points (each well-formed high+low surrogate pair is one code point), and stop as soon as the running count exceeds `max`. On the (rare) failure path compute the exact count with the shared `codePointLength(v)` helper for the message: ``push(Violation{path, reason: `must have length <= ${max}, got ${codePointLength(v)}`})``; throw one `PayloadValidationError` application failure. **Never `v.length`** — that is the UTF-16 code-unit count (verified `"a😀b".length === 4`). `max` is an emitted numeric constant. This beats the obvious `[...v].length` (which allocates a full code-point array) ~3.5×, and early-exit bounds work on adversarially long input regardless of `max`. |
| Python     | `if (n := len(v)) > max: violations.append(Violation(path=…, reason=f"must have length <= {max}, got {n}"))` in the transfer type converter (PRINCIPLES Python §3). **`len` on a `str` is the code-point count** — a single astral emoji is 1 code point but 4 UTF-8 bytes / 2 UTF-16 units (`len("a😀b") == 3`) — so it is spec-correct with no extra scan, matching the other three. Aggregates into the single generated `PayloadValidationError` application failure.                                                                                                                                                                                                                                                                                                                                                                   |
| Java       | The per-POJO collecting deserializer (PRINCIPLES Java §5) reads the field node as a `String`, then checks `int n = v.codePointCount(0, v.length()); if (n > max)`, pushing a `Violation{path, "must have length <= " + max + ", got " + n}` into the single `PayloadValidationError` application failure. **`codePointCount(0, length())`, not `length()`** — `length()` is the UTF-16 code-unit count (verified `"a😀b".length() == 4`). Not bean-validation `@Size` — the check is hand-written in the collecting deserializer like every other constraint.                                                                                                                                                                                                                                                                               |

**Informative `reason` strings.** The `Violation` `reason` names the
**concrete bound and the offending count** — `must have length <= 2, got 3` —
per the [[maximum]] convention, so the aggregated error tells the caller which
limit was crossed and by how much. The bound is an emitted compile-time
constant; the count is computed at runtime.

### Serialize-side (P12)

The bound is a shared-`Validate` predicate, so it **re-runs before emit** over
the decoded value — a model constructed with an over-length string (a Go
`string` / Java `String` / Python `str` set past `maxLength` in memory) fails
serialize with the same aggregated primitive rather than emitting an
out-of-bounds value. Real teeth in the statically-typed targets, where in-memory
construction is unchecked (identical rationale to the [[type]] integer-cap
re-check and the [[maximum]] bound re-check). No parse-adapter-only or
encode-adapter-only logic: the comparison is pure and direction-agnostic.

**On a materialized node** ([[format]] temporal / [[contentEncoding]] bytes) the
decoded value is not a `string`, so the bound is a predicate over the
**canonical wire string** — the form the encode adapter produces for that value
— at **both** boundaries: on parse the value is parsed and re-canonicalized
*before* the count is taken, and on serialize the re-serialized wire string is
counted **before emit**, so the teeth hold there too (an in-memory bytes value
whose base64 exceeds `maxLength`, or a temporal whose canonical form does, fails
serialize). Still one predicate, still identical in both directions. Counting
the incoming form on parse and the canonical form on serialize would make that
false: `PT90M` (5) and its canonical spelling `PT1H30M` (7) are one [[format]]
`duration` value, so `{format:"duration", maxLength:5}` would admit a payload it
then cannot re-emit — a **P1** accept-set defect. A **literal** ([[const]],
[[enum]], [[default]]) on such a node is measured the same way: it is
canonicalized first, so the length checked at load is the length of the constant
the generator emits. Counting two different strings in the two directions is
exactly the operand drift **P12.2** forbids, "even though both sides call the
same helper".

That same string is the closed-value comparison's operand too ([[const]] /
[[enum]]), and it is projected **once per member**: two independent projections
are two operands (**P12.2**), and a second projection in one scope is a
redeclaration.

## Property-testing matrix

### Accepted (positive)

| Shape                        | Example                                     |
| ---------------------------- | ------------------------------------------- |
| Inclusive max                | `{type:"string", maxLength:10}`             |
| `.0`-valued bound            | `{type:"string", maxLength:10.0}`           |
| Zero max (empty string only) | `{type:"string", maxLength:0}`              |
| Exact length (min==max)      | `{type:"string", minLength:5, maxLength:5}` |

### Rejected at load time (negative)

| Reason                            | Example                                                                                         |
| --------------------------------- | ----------------------------------------------------------------------------------------------- |
| Value not a number                | `maxLength:"5"`, `maxLength:true`, `maxLength:null`                                             |
| Negative value                    | `maxLength:-1`                                                                                  |
| Fractional value                  | `maxLength:5.5`                                                                                 |
| Above the portable length ceiling | `maxLength:9007199254740992`                                                                    |
| Type mismatch (P7.1)              | `{type:"integer", maxLength:5}`, `{type:"array", maxLength:3}`                                  |
| Unsatisfiable range               | `{type:"string", minLength:10, maxLength:2}`                                                    |
| Literal exceeds bound             | `{type:"string", maxLength:2, const:"abc"}`, `{…, default:"abc"}`, `{…, enum:["ok","toolong"]}` |

### Runtime fixtures (validator)

- `codePointCount(v) == max` → OK (`≤` is inclusive).
- `v` one code point over `max` → one `PayloadValidationError` application
  failure whose reason names the bound and count
  (`must have length <= 2, got 3`).
- **Astral / multi-byte fixtures (the P1 core):** `"a😀b"` counts as **3** in
  all four (not 6/4/4); `"😀😀"` counts as **2**; NFC `"é"` counts as **1** and
  NFD `"e"+U+0301` as **2** — every language agrees on each because none
  normalizes.
- Combined with other failing assertions ([[minLength]], [[pattern]], a failing
  sibling field) → **all** reported in one shot (**P11**).
- Serialize of an in-memory over-length string → rejected before emit (**P12**),
  not silently written.

## Interactions

- **[[minLength]]**: the paired lower bound over the same code-point count.
  `minLength > maxLength` is a load error; `minLength == maxLength` pins an
  **exact** length (accepted — a fixed-width string, the string analog of the
  numeric `minimum == maximum` single-value pin in [[maximum]]).
- **[[pattern]]**: an independent string assertion; both apply and both
  aggregate. We do **not** attempt cross-satisfiability between a regex and a
  length bound (a pattern *can* imply a minimum length, but reasoning about it
  is undecidable in general and out of scope — see [[pattern]]).
- **[[type]]**: gates applicability — `maxLength` is meaningful only for
  `string`; a mismatch is a load reject (**P7.1**). `maxLength` never narrows
  the emitted type and does not force it to `string`: a materializing sibling
  ([[format]] / [[contentEncoding]]) may replace it with a native construct
  while the bound still checks the encoded wire string.
- **[[const]] / [[default]] / [[enum]]**: a string literal supplied by one of
  these on the **same node** MUST satisfy `maxLength` at load (rule above) — the
  string-length half of the deferred literal-vs-constraint obligation. A
  **closed value set** does not remove the bound: where a target gives the
  closed set its own named type, the count is taken over that value's underlying
  `string`. Handing the named type to the code-point-count primitive is a
  compile error, and dropping the check silently makes the emitted validator
  depend on the load-time check having been exact.
- **[[minProperties]] / [[maxProperties]]**: the *object member-count* analog;
  [[maxItems]] / [[minItems]] is the *array-length* analog. All are count
  assertions that share the "count the right thing, compare once" shape but
  count different things (and the array/object counts, unlike string length,
  need no unit normalization).

## Ecosystem variance

| Source dialect        | Action                                                                                       |
| --------------------- | -------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12   | Native.                                                                                      |
| OpenAPI 3.1           | Adopts 2020-12 — `maxLength` identical. Native.                                              |
| OpenAPI 3.0 / draft-4 | `maxLength` present since draft-4 with identical (code-point) semantics. Native, no rewrite. |
| Swagger 2.0           | Same as OAS 3.0.                                                                             |

Note: some non-conformant toolchains implement `maxLength` as a UTF-16 or byte
count. We follow the spec (code points); a schema authored against a
UTF-16-counting tool could differ only for astral-plane input, which is the same
class of edge the `string_probe` fixtures pin down.

## See also

- [[minLength]] — the paired inclusive lower bound (shares this machinery).
- [[pattern]] — the other string assertion; regex, with its own
  dialect/anchoring caveats.
- [[type]] — supplies the emitted `string` primitive; gates applicability.
- [[const]], [[default]], [[enum]] — supplied string literals are validated
  against `maxLength` at load.
- [[maximum]] — the numeric-bound family; same `reason`-string convention and
  single-value/exact pin idea.
- [[maxProperties]] — the object member-count analog.
