# `contains`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.3.1.3 "Keywords
for Applying Subschemas to Arrays → contains".

Asserts that **at least one element** of an array instance matches a subschema —
the array *existential*. A pure runtime assertion; no type impact. Where
[[items]] constrains **every** element and [[uniqueItems]] compares elements
**to each other**, `contains` asks only whether **some** element satisfies a
matcher — most often "the array holds this value" (`contains:{const:"admin"}`)
or "some element clears this bound" (`contains:{type:integer, minimum:5}`). Like
[[uniqueItems]], its support envelope is drawn at the **scalar/composite** line:
a scalar matcher over the (necessarily scalar) element type is a cheap,
trivially portable predicate; a composite matcher requires the deferred
deep-match surface.

## Spec summary

Verbatim (2020-12 core, Applicator, §10.3.1.3):

> The value of this keyword MUST be a valid JSON Schema.

> An array instance is valid against "contains" if at least one of its elements
> is valid against the given schema, except when "minContains" is present and
> has a value of 0, in which case an array instance MUST be considered valid
> against the "contains" keyword, even if none of its elements is valid against
> the given schema.

> This keyword produces an annotation value which is an array of the indexes to
> which this keyword's subschema is valid. […] The subschema MUST be applied to
> every array element even after the first match has been found, in order to
> collect annotations for use by other keywords.

> Omitting this keyword has the same assertion behavior as an empty schema.

Distilled:
- The value is a full subschema — the **matcher**. The array is valid iff
  **≥ 1 element** validates against it (the spec default, i.e. `minContains`
  omitted ≡ `1`).
- The match count can be re-floored/capped by [[minContains]] / [[maxContains]]
  (both supported — their own specs); `contains` **alone** means the
  **spec-default ≥ 1 match** (`minContains` omitted ≡ `1`).
- The annotation (matching indexes) exists only to feed [[unevaluatedItems]],
  which is **rejected** per **P6**. With no annotation consumer the indexes
  themselves are never collected; the scan keeps only the match **tally**, which
  is what a plain `contains` tests for nonzero and what [[minContains]] /
  [[maxContains]] bound (see Validator mapping).
- Applies only to array instances; a `contains` on a non-array [[type]] is
  rejected at load (**P7.1**), like every array keyword.

## Support decision

**Support:** yes for a **scalar matcher over a scalar element [[type]]** (string
/ number / integer / boolean); **composite** matchers or composite element types
(object / array) are **deferred** — a load reject with a "not yet supported"
diagnostic, exactly the stance [[uniqueItems]] / [[const]] / [[enum]] take on
composite values (a portable deep-match is correct in principle, just costly —
revisit on demand).

Because `type:"array"` **requires** [[items]] in this subset (no untyped
arrays), a `contains` always co-occurs with a known element type. Support is
therefore gated on that element type being **scalar**: the matcher runs over
values whose kind every target agrees on value-for-value (**P1**).

Grounding ([[PRINCIPLES.md]]): **P10** (enforced at the boundary), **P11**
(aggregated), **P12** (**one** matcher predicate, applied at both boundaries:
the deserialize scan reads the wire elements and the serialize scan the decoded
collection, with the same predicate, the same tally and the same reason). No
effect on emitted types. The scalar restriction is the **P1** line: a scalar
matcher is the same value/range comparison [[enum]] / [[minimum]] / [[pattern]]
already specify across all four targets, whereas a portable composite deep-match
is extra surface we don't yet commit to.

Loader behavior:
- `contains` value not a valid subschema (`contains:5`, `contains:"x"`,
  `contains:[…]`) → reject (recurse).
- `contains` on a non-array [[type]] (`{type:"string", contains:…}`) → reject
  per **P7.1** (statically meaningless).
- `contains` present without `type:"array"` → reject per [[type]]
  (missing/mismatched type); a `type:"array"` still requires [[items]].
- **Shapeless matcher** — `contains:{}` / `contains:true` / `contains:false` →
  reject per **P7.1**. `{}` / `true` match every element (so `contains`
  degenerates to "non-empty" — the diagnostic points at `minItems:1`); `false`
  matches nothing (unsatisfiable at the default `minContains:1`). No element
  shape → no matcher.
- **Composite matcher** (`contains:{type:"object", …}` or
  `contains:{type:"array", …}`) → **reject** with a "not yet supported"
  diagnostic (deferred; see Support decision).
- **Composite element type** — `contains` paired with a composite [[items]]
  (`items:{type:"object", …}` / `items:{type:"array", …}`) → **reject**,
  deferred, exactly as [[uniqueItems]] defers composite elements.
- **Matcher type incompatible with the element type** — a scalar matcher whose
  [[type]] (or `const`/`enum` value kind) cannot match the [[items]] scalar
  element type (`items:{type:"string"}` + `contains:{type:"integer"}`, or
  `+ contains:{const:5}`) → reject as **statically unsatisfiable** (**P7.1**):
  no element can ever match. Parallel to [[enum]]'s member/type compatibility
  check. (A number matcher over an `integer` element, and vice-versa, follows
  [[type]]'s numeric rules — equality is over the mathematical number, so
  `contains:{const:1.0}` over `items:{type:integer}` is fine.)
- **A numeric bound on the matcher is held to the element's effective kind**,
  not to the matcher's own declared [[type]], so a fractional bound over
  `integer` [[items]] is the same load reject as on an `integer` property — see
  [[maximum]], which owns the rule. Resolving the bounded position's kind once
  at load is what keeps each target from deriving that kind, and the bound
  literal with it, on its own.
- **Matcher value outside a closed element domain** — when the element carries a
  closed value set ([[const]] / [[enum]]), a matcher member outside that set can
  never match. If **no** member survives — a matcher `const` outside the set, or
  a matcher `enum` all of whose members are outside it — the schema is
  statically **unsatisfiable** and rejects (**P7.1**), with the same "no element
  can ever match" diagnostic as the kind mismatch above. A matcher that
  *partially* overlaps the element's set is satisfiable and **accepted**; the
  emitted scan then compares only the surviving members, since a comparison
  against a value the element type excludes is not merely dead but ill-typed in
  the targets that emit the closed type.
- **Materializing keywords.** A [[contentEncoding]] or a temporal [[format]] on
  the **matcher** → **reject**: a matcher is a predicate with no slot to
  materialize into; assert the wire string with [[pattern]], [[const]] or
  [[enum]] instead, or move the assertion onto [[items]]. A materializing
  keyword on the **element** is **accepted**, and the matcher measures the
  element's **canonical wire form** — the exact bytes the generator's own
  serializer produces — in both directions, the same form [[const]] / [[enum]]
  compare over a materialized value.
- **[[minContains]] / [[maxContains]]** present → validated by their own specs
  (non-negative integers, range satisfiability, the `minContains:0` relaxation).
  They require a sibling `contains` and cannot appear without one — see
  Interactions.

## Type mapping

None. The emitted collection type is [[items]]'s `[]T` / `T[]` / `list[T]` /
`List<T>`; the existential lives only in the validator.

## Validator mapping

Per **P10**/**P11**. A single **existence** scan. Deserialize scans the original
wire elements, including elements that fail [[items]] conversion; serialize
scans decoded elements. Each element is tested against the scalar matcher
predicate — the same shared predicate the matcher's own keywords define
([[const]]/[[enum]] equality, [[minimum]]/[[maximum]] range, [[pattern]] regex,
[[minLength]]/[[maxLength]], [[multipleOf]]) — and the scan tallies **all**
matches. A plain `contains` uses only whether that tally is nonzero;
[[minContains]] and [[maxContains]] apply their bounds to the same tally. If no
element matches, one `Violation` is pushed.

**A `null` element is never a candidate.** On the wire side the scan reads each
element as a JSON token, so a `null` is **skipped**, never coerced to a zero
value (`""` / `0` / `false`) and offered to the matcher. This is what makes a
nullable element (the [[nullability]] `oneOf` pattern) safe: a `null` matches
nothing, in every target and in both directions.

| Language   | Strategy                                                                                                                                                                                                                                                                    |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | Deserialize scans the original `json.RawMessage` elements with the matcher's scalar parser and predicates; serialize scans the typed slice in shared `Validate`. A miss collects one violation into `PayloadValidationError` application failure.                           |
| TypeScript | After the `Array.isArray` guard ([[items]]), deserialize scans the raw array and serialize scans the typed array with the same scalar matcher predicates. A miss pushes one violation.                                                                                      |
| Python     | The transfer converter calls `_check_contains` with the raw list on deserialize and the typed list on serialize. The matcher uses the same scalar predicates in either direction, including for typed-map members and [[oneOf]] branches.                                   |
| Java       | The per-POJO collecting deserializer scans the original `JsonNode` elements with the matcher type guard and predicates; serialize scans the typed `List<T>`. A miss pushes one violation into the single `PayloadValidationError` application failure. Not bean-validation. |

Reason strings name **what was required**, not a bare keyword — the matcher is
described by its own constraint (`no element equals "admin"` for a `const`
matcher, `no element matches minimum 5` for a range matcher,
`no element matches the required schema` as the general fallback), per the
informative-reason convention the constraint families use.

**Float exactness.** When the matcher is a `const`/`enum` over a `number`
element, equality is **exact `==`**, never an epsilon — identical to
[[enum]] / [[uniqueItems]]: the wire value and the matcher literal are both
IEEE-754 binary64 from correctly-rounded decimal→double parsing, so the same
decimal yields the identical bit pattern in every target. An integer-valued
number matcher such as `const:1.0` matches the element `1`: equality is over the
mathematical number, not the authored spelling (as in
[[enum]]).

### Serialize-side (P12)

Identical to [[uniqueItems]]: the predicate **re-runs before emit** over the
decoded value, so an in-memory slice/list that holds no matching element fails
serialize with the same aggregated primitive rather than being written. Real
teeth in the statically-typed targets (Go/TS/Java), where in-memory construction
is unchecked. Each element is measured in the wire form it has or would have —
its own bytes inbound, its canonical rendering outbound for a materialized
element — so the existence scan is the identical predicate in both directions.

## Property-testing matrix

### Accepted (positive)

| Shape                                                    | Example                                                                                                                                     |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Contains a required value (`const` matcher)              | `{type:"array", items:{type:string}, contains:{const:"admin"}}`                                                                             |
| Contains a member of a set (`enum` matcher)              | `{type:"array", items:{type:string}, contains:{enum:["admin","root"]}}`                                                                     |
| Contains an element clearing a bound                     | `{type:"array", items:{type:integer}, contains:{type:integer, minimum:5}}`                                                                  |
| Contains a pattern-matching element                      | `{type:"array", items:{type:string}, contains:{type:string, pattern:"^x"}}`                                                                 |
| Integer-valued number matcher normalizes                 | `{type:"array", items:{type:integer}, contains:{const:1.0}}`                                                                                |
| Match-count bounds ([[minContains]] / [[maxContains]])   | `{type:"array", items:{type:integer}, contains:{minimum:5}, minContains:2, maxContains:4}`                                                  |
| Combined with count/uniqueness assertions                | `{type:"array", items:{type:string}, minItems:1, uniqueItems:true, contains:{const:"admin"}}`                                               |
| Array member of a struct                                 | `{type:"object", properties:{roles:{type:array, items:{type:string}, contains:{const:"admin"}}}}`                                           |
| Nullable scalar element                                  | `{type:"array", items:{oneOf:[{type:string},{type:"null"}]}, contains:{const:"admin"}}` — a `null` element never matches the scalar matcher |
| Matcher partially overlapping a closed element set       | `{type:"array", items:{type:string, enum:["a","b"]}, contains:{enum:["b","z"]}}` — `"z"` can never match and is not compared                |
| Materialized element, matched on its canonical wire form | `{type:"array", items:{type:string, format:"date-time"}, contains:{type:string, pattern:"^2024"}}`                                          |

### Rejected at load time (negative)

| Reason                                                                           | Example                                                                                                                  |
| -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| Matcher not a schema                                                             | `contains:5`, `contains:"x"`, `contains:[{type:string}]`                                                                 |
| Type mismatch (P7.1)                                                             | `{type:"string", contains:{const:"a"}}`                                                                                  |
| Shapeless matcher (P7.1)                                                         | `{type:array, items:{type:string}, contains:{}}`, `…contains:true`, `…contains:false`                                    |
| Composite matcher (deferred)                                                     | `{type:array, items:{type:object,…}, contains:{type:object,…}}`                                                          |
| Composite element type (deferred)                                                | `{type:array, items:{type:object,…}, contains:{const:…}}`                                                                |
| Matcher type-incompatible with element (P7.1, unsatisfiable)                     | `{type:array, items:{type:string}, contains:{type:integer}}`, `…contains:{const:5}}`                                     |
| Matcher value outside a closed element set (P7.1, unsatisfiable)                 | `{type:array, items:{type:string, enum:["a","b"]}, contains:{const:"z"}}`, `…contains:{enum:["z"]}}`                     |
| Fractional matcher bound over `integer` elements (see [[maximum]] / [[minimum]]) | `{type:array, items:{type:integer}, contains:{type:number, minimum:5.5}}`                                                |
| Materializing keyword on the matcher                                             | `{type:array, items:{type:string}, contains:{type:string, format:"date-time"}}`, `…contains:{contentEncoding:"base64"}}` |

### Runtime fixtures (validator)

- At least one element matches → OK (both directions), regardless of how many.
- No element matches → one `PayloadValidationError` application failure naming
  the required matcher (`no element equals "admin"`).
- Empty array `[]` → **rejected** (nothing to match; the default is ≥ 1 match).
  Contrast [[minItems]] `0` / [[uniqueItems]], which pass vacuously on `[]`.
- Multiple matches → OK (existential; a plain `contains` asks only that the
  tally be nonzero).
- A `null` element under a nullable element schema → never counted as a match,
  in **both** directions and in every target, whatever the matcher's scalar kind
  (a zero value must not stand in for the missing element).
- A materialized element (temporal [[format]] / [[contentEncoding]]) → matched
  on its canonical wire form, so a value that parses re-serializes.
- Combined with a failing sibling ([[minItems]] / [[maxItems]] / [[uniqueItems]]
  / a bad element per [[items]]) → **all** reported in one shot (**P11**).
- Serialize of an in-memory slice/list with no matching element → rejected
  before emit (**P12**), not silently written.

## Interactions

- **[[items]]**: gates the element type and thus applicability — `contains` is
  supported only when `items` is scalar; a composite `items` defers it. `items`
  types (and constrains) **every** element; `contains` asserts **some** element
  additionally satisfies the matcher. Orthogonal where both apply: an element
  must satisfy `items` to be present at all, and at least one must also satisfy
  `contains`.
- **[[minContains]] / [[maxContains]]**: the 2020-12 count-of-matches bounds
  that generalize `contains` from "≥ 1" to a range — **supported** (their own
  specs). They own the match-count machinery, bound the same tally this scan
  produces, require a sibling `contains`, and `minContains:0` relaxes the
  existential (vacuous alone, meaningful as a `0..maxContains` range).
  `contains` **alone** is exactly the spec-default ≥ 1.
- **[[minItems]] / [[maxItems]]**: array-level **element**-count assertions;
  `contains` is a **match**-count existential over a subset. All apply and
  aggregate. We do **not** collapse `contains` into an implied `minItems:1` (a
  matching array is necessarily non-empty, but the two reasons stay distinct) —
  parallel to the no-cross-check stance between [[uniqueItems]] and the count
  pair.
- **[[uniqueItems]]**: independent scalar-gated array assertions; both apply and
  aggregate. `uniqueItems` forbids repeats, `contains` requires a match —
  orthogonal.
- **[[const]] / [[enum]]**: the natural `contains` matchers — "the array
  contains value X" is `contains:{const:X}`, "…one of a set" is
  `contains:{enum:[…]}`. They share the **scalar value-equality** definition
  used here (exact `==` for numbers, normalized integer-valued numbers), and
  defer composite values identically. On the **element** they close its domain,
  which bounds what a matcher can ever match (Loader behavior).
- **[[format]] / [[contentEncoding]]**: rejected on the matcher, supported on
  the element — where the matcher then measures the element's canonical wire
  form in both directions (Loader behavior). Contrast [[uniqueItems]], which
  defers a materializing element entirely, because its equality is
  element-to-element rather than element-to-matcher.
- **[[minimum]] / [[maximum]] / [[pattern]] / [[minLength]] / [[maxLength]] /
  [[multipleOf]]**: any of these on the matcher subschema defines "match" for a
  `contains` over a numeric/string element; the scan reuses that keyword's own
  shared predicate.
- **[[type]]**: gates applicability to `type:"array"`; a mismatch is a load
  reject (**P7.1**). The emitted collection type is unchanged.
- **[[required]]**: orthogonal — `required` decides whether the array member is
  present; `contains` shapes its value. A present array must hold a match; an
  optional **absent** array raises no `contains` violation (nothing to scan —
  **P8**: optional ≠ the value constraint).
- **[[nullability]]**: if the element schema is the nullable [[nullability]]
  pattern, a `null` element matches `contains` only if the matcher itself is the
  null pattern (out of this scalar subset). Otherwise a `null` element simply
  never matches a scalar matcher — orthogonal.
- **[[prefixItems]] / [[unevaluatedItems]]**: the other array applicators,
  **rejected** per **P6**. With [[unevaluatedItems]] gone there is no consumer
  for `contains`' index annotation, so the scan keeps a tally and never the
  index set.

## Ecosystem variance

| Source dialect                      | Action                                                                                                                                                |
| ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12                 | Native (scalar matcher; `minContains`/`maxContains` supported).                                                                                       |
| OpenAPI 3.1                         | Adopts 2020-12 — `contains` identical. Native.                                                                                                        |
| draft-7 / draft-6                   | `contains` present since draft-6 with the same ≥ 1 existential (no `minContains`/`maxContains` — those arrived in draft 2019-09). Native, no rewrite. |
| OpenAPI 3.0 / Swagger 2.0 / draft-4 | No `contains` keyword — nothing to map.                                                                                                               |

## See also

- [[items]] — supplies the emitted collection type and the scalar element type
  that gates `contains`' support envelope; constrains every element where
  `contains` asserts one.
- [[uniqueItems]] — the sibling scalar-gated array assertion; shares the
  scalar/composite support line and the scalar value-equality definition.
- [[minContains]] / [[maxContains]] — the supported count-of-matches bounds that
  generalize this keyword's ≥ 1 default.
- [[minItems]] / [[maxItems]] — array **element**-count assertions, distinct
  from the **match**-count existential.
- [[const]] / [[enum]] — the natural scalar matchers; share value-equality and
  defer composite values.
- [[minimum]] / [[maximum]] / [[pattern]] / [[minLength]] / [[maxLength]] /
  [[multipleOf]] — matcher constraints that define "match" for numeric and
  string elements.
- [[type]] — gates applicability to `type:"array"`.
- [[required]] / [[nullability]] — presence and null of the array member,
  distinct from the existential over its elements.
- [[prefixItems]] / [[unevaluatedItems]] — rejected array applicators; the
  absent `unevaluatedItems` is why the scan keeps only a tally.
