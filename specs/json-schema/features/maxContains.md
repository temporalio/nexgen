# `maxContains`

Source: JSON Schema 2020-12, Validation vocabulary, §6.4.4 "Validation Keywords
for Arrays → maxContains".

Sets an **inclusive** upper bound on the
**number of elements that match the [[contains]] matcher**. A pure runtime count
assertion — no type impact. The canonical spec for the *contains-count* pair:
[[minContains]] shares the match-counting machinery documented here and differs
only in the comparison operator (and in the special `0` case it owns). Where
[[maxItems]] caps the **total** element count, `maxContains` caps only the
**matching** subset — it is meaningless without a [[contains]] on the same node.

## Spec summary

Verbatim (2020-12 validation, §6.4.4):

> The value of this keyword MUST be a non-negative integer.

> If "contains" is not present within the same schema object, then this keyword
> has no effect.

> An instance array is valid against "maxContains" in two ways, depending on the
> form of the value of "contains". In the first form, if the value of
> "maxContains" is greater than or equal to the number of elements which
> validate against the "contains" subschema, then the array is valid. In the
> second form, if the value of "maxContains" is less than the number of elements
> which validate against the "contains" subschema, then the array is invalid.

Distilled:
- Value MUST be a **non-negative integer**.
- Instance valid iff `matchCount(instance) ≤ maxContains`, where `matchCount` is
  the number of elements validating against the [[contains]] matcher (the same
  predicate [[contains]] defines).
- **No effect without [[contains]].** Per **P7.1** we do not silently ignore it:
  a `maxContains` with no sibling `contains` is a load reject (below), not a
  no-op.
- Pure assertion; no annotation behavior, no effect on the emitted collection
  type ([[items]]'s `[]T` / `T[]` / `list[T]` / `List<T>`).

## Support decision

**Support:** yes — a runtime **match-count** comparison over the [[contains]]
matcher, on the same scalar-only envelope [[contains]] draws (scalar matcher
over a scalar [[items]] element type; composite matchers / elements are deferred
there and therefore here too).

Lowers to a single `≤` count comparison after tallying matches; no effect on
emitted types. Citing [[PRINCIPLES.md]]: **P10** (enforced at the boundary),
**P11** (aggregated), **P12** — one comparison over one kind of operand, the
match tally over the array's complete element sequence, applied at both
boundaries with the same reason.

Loader behavior:
- Value not a non-negative integer → reject: a non-number (`maxContains:"3"`,
  `maxContains:true`, `maxContains:null`), a **negative** value
  (`maxContains:-1`), or a **fractional** value (`maxContains:2.5`).
  `maxContains:2.0` is accepted (≡ `2`, honoring the `1.0`-as-integer rule from
  [[type]]).
- The portable count ceiling from [[maxItems]], and the emitted-literal
  obligation that comes with a bound above 2^31−1, apply here unchanged.
- **`maxContains` without [[contains]]** on the same node → **reject**
  (**P7.1**, statically meaningless — the spec's "no effect" tightened to a loud
  error). Diagnostic: add a `contains` matcher or remove `maxContains`.
- `maxContains` on a non-array [[type]] → reject per **P7.1** (it can only
  co-occur with `contains`, which itself requires `type:"array"` + [[items]] —
  see [[contains]]).
- **`minContains` > `maxContains` on the same node → reject (unsatisfiable).**
  `minContains == maxContains` pins an **exact** match count (accepted —
  "exactly N elements match"). All combined satisfiability for the pair lives
  here — see **Interactions → satisfiability**.
- **`maxContains:0`** → "**no** element may match the matcher" (a
  *must-not- contain* assertion). Legitimate, but only satisfiable together with
  **`minContains:0`**: the [[contains]] default is `minContains:1`, so a bare
  `maxContains:0` is `min 1 > max 0` → **reject** as unsatisfiable. With
  `minContains:0` present it is accepted (see [[minContains]] and the fixtures
  below).

## Type mapping

None. The emitted collection type is [[items]]'s `[]T` / `T[]` / `list[T]` /
`List<T>`; the bound lives only in the validator.

## Validator mapping

Per **P10**/**P11**. A single `≤` comparison of the **match count** against the
fixed bound. Deserialize tallies every original wire element; serialize tallies
the decoded collection. The same matcher predicate, comparison, and reason apply
in both directions (**P12**).

| Language   | Strategy                                                                                                                                                               |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | `UnmarshalJSON` tallies the original `json.RawMessage` elements; shared `Validate` tallies the typed slice before serialize. Both push the same bound/count violation. |
| TypeScript | The transfer converters tally the raw array inbound and the typed array outbound after the `Array.isArray` guard. `max` is an emitted numeric constant.                |
| Python     | `_check_contains` receives the raw list inbound and the typed list outbound, appending the same bound/count violation.                                                 |
| Java       | The collecting deserializer tallies the original Jackson array node; serialize tallies the typed `List<T>`. Not bean-validation.                                       |

**Informative `reason` strings.** The `reason` names the
**concrete bound and the offending match count** —
`too many matching items: at most 2, got 3` — per the [[maxItems]] count-family
convention, distinct from [[maxItems]]' *total*-count message so the caller can
tell a match-count violation from a size violation. The bound is an emitted
compile-time constant; the count is computed at runtime.

### Serialize-side (P12)

Identical to [[maxItems]]: the predicate **re-runs before emit** over the
decoded value — a model constructed in memory whose elements yield too many
matches fails serialize with the same aggregated primitive rather than being
written. Real teeth in the statically-typed targets (Go/TS/Java), where
in-memory construction is unchecked. Both directions tally over the array's
complete element sequence — the wire array inbound, the decoded collection
outbound — so the tally-and-compare is the identical predicate in both
directions.

## Property-testing matrix

### Accepted (positive)

| Shape                                   | Example                                                                                   |
| --------------------------------------- | ----------------------------------------------------------------------------------------- |
| Inclusive match cap                     | `{type:"array", items:{type:integer}, contains:{minimum:5}, maxContains:3}`               |
| `.0`-valued bound                       | `{type:"array", items:{type:string}, contains:{const:"x"}, maxContains:2.0}`              |
| Exact match count (min==max)            | `{type:"array", items:{type:string}, contains:{const:"x"}, minContains:2, maxContains:2}` |
| Range 0..max (must-not-exceed, zero OK) | `{type:"array", items:{type:string}, contains:{const:"x"}, minContains:0, maxContains:2}` |
| Must-not-contain (exactly zero matches) | `{type:"array", items:{type:string}, contains:{const:"x"}, minContains:0, maxContains:0}` |

### Rejected at load time (negative)

| Reason                                                         | Example                                                                                   |
| -------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| Value not a number                                             | `maxContains:"3"`, `maxContains:true`, `maxContains:null`                                 |
| Negative value                                                 | `maxContains:-1`                                                                          |
| Fractional value                                               | `maxContains:2.5`                                                                         |
| Above the portable count ceiling                               | `maxContains:9007199254740992`                                                            |
| No sibling `contains` (P7.1)                                   | `{type:"array", items:{type:string}, maxContains:2}`                                      |
| Unsatisfiable range                                            | `{type:"array", items:{type:string}, contains:{const:"x"}, minContains:3, maxContains:1}` |
| `maxContains:0` at the default `minContains:1` (unsatisfiable) | `{type:"array", items:{type:string}, contains:{const:"x"}, maxContains:0}`                |

### Runtime fixtures (validator)

- Match count `== max` → OK (`≤` is inclusive).
- Match count `max+1` → one `PayloadValidationError` application failure naming
  the bound and count (`too many matching items: at most 2, got 3`).
- Non-matching elements never count — an array full of non-matching elements has
  match count `0`, trivially `≤ max`.
- `minContains:0, maxContains:0` (must-not-contain): an array with **no**
  matching element → OK; **one** matching element → fail
  (`too many matching items: at most 0, got 1`).
- Combined with a failing sibling ([[minContains]], [[minItems]] / [[maxItems]],
  a bad element per [[items]]) → **all** reported in one shot (**P11**).
- Serialize of an in-memory value with too many matches → rejected before emit
  (**P12**), not silently written.

## Interactions

- **[[contains]]**: the gate — `maxContains` is meaningless without it and is a
  load reject on its own. It reuses [[contains]]' matcher predicate and its
  scalar-only support envelope and its full match tally. `contains` alone is the
  spec-default `minContains:1` with no ceiling.
- **[[minContains]] — satisfiability (canonical, owned here)**: the paired lower
  bound over the same match count. `minContains > maxContains` is a load error;
  `minContains == maxContains` pins an **exact** match count (accepted). Because
  the [[contains]] default is `minContains:1`, a `maxContains:0` is
  unsatisfiable unless `minContains:0` is set. We do **not** cross-check a
  `minContains`/`maxContains` bound against how many elements *could* match the
  matcher (undecidable in general — parallel to the uniqueness-vs-count
  non-check in [[minItems]]).
- **[[minItems]] / [[maxItems]]**: bound the **total** element count;
  `maxContains` bounds the **matching** subset. Independent — all apply and
  aggregate. We do not collapse them (a `maxContains:0` does not imply any
  `maxItems`).
- **[[items]]**: types (and constrains) every element; `maxContains` caps how
  many of them additionally match the [[contains]] matcher. Supported only when
  `items` is scalar (via [[contains]]).
- **[[type]]**: gates applicability to `type:"array"` (through [[contains]]); a
  mismatch is a load reject (**P7.1**). The emitted collection type is
  unchanged.
- **[[uniqueItems]] / [[required]] / [[nullability]]**: orthogonal, exactly as
  for [[contains]] — uniqueness of elements, presence of the member, and null of
  the member are all independent of the match-count ceiling.

## Ecosystem variance

| Source dialect                      | Action                                                                                                    |
| ----------------------------------- | --------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12                 | Native.                                                                                                   |
| OpenAPI 3.1                         | Adopts 2020-12 — `maxContains` identical. Native.                                                         |
| draft 2019-09                       | `maxContains` introduced here with identical semantics. Native.                                           |
| draft-7 / draft-6                   | `contains` exists but `maxContains` does not — nothing to map (a bare `contains` is the ≥ 1 existential). |
| OpenAPI 3.0 / Swagger 2.0 / draft-4 | No `contains` family — nothing to map.                                                                    |

## See also

- [[minContains]] — the paired inclusive lower bound (shares this match-counting
  machinery; owns the `0` relaxation and the default of 1).
- [[contains]] — supplies the matcher predicate and the scalar-only support
  envelope; `maxContains` caps the match tally that scan yields.
- [[maxItems]] / [[minItems]] — the **total**-element-count analog; same
  count-family `reason`-string convention and exact-pin idea.
- [[items]] — supplies the emitted collection type and the scalar element type
  this keyword's matches are drawn from.
- [[type]] — gates applicability to `type:"array"`.
