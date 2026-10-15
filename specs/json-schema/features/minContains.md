# `minContains`

Source: JSON Schema 2020-12, Validation vocabulary, §6.4.5 "Validation Keywords
for Arrays → minContains".

Sets an **inclusive** lower bound on the
**number of elements that match the [[contains]] matcher**. A pure runtime count
assertion — no type impact. Mirror of [[maxContains]] with `≥`; the shared
match-counting machinery (the tally over the matcher predicate, the per-language
primitive, the serialize-side re-check, the exact-count pin) is documented once
in [[maxContains]] and referenced here. `minContains` is the keyword that
**defines the [[contains]] existential floor**: omitting it is the same as
`minContains:1` (the "≥ 1 match" default), and setting it to `0` **relaxes** the
existential entirely.

## Spec summary

Verbatim (2020-12 validation, §6.4.5):

> The value of this keyword MUST be a non-negative integer.

> If "contains" is not present within the same schema object, then this keyword
> has no effect.

> An instance array is valid against "minContains" if the number of elements
> that are valid against the schema for "contains" is greater than, or equal to,
> the value of this keyword.

> A value of 0 is allowed, but is only useful for setting a range of occurrences
> from 0 to the value of "maxContains". A value of 0 causes the keyword to
> always pass validation (but validation can still fail against a "maxContains"
> keyword).

> Omitting this keyword has the same behavior as a value of 1.

Distilled:
- Value MUST be a **non-negative integer**.
- Instance valid iff `matchCount(instance) ≥ minContains`, where `matchCount` is
  the number of elements validating against the [[contains]] matcher (the same
  tally [[maxContains]] documents).
- **Omitting ≡ `1`** — this is exactly [[contains]]' default ≥ 1 existential.
  The default is part of the schema's **meaning**, not of its spelling: a node
  carrying [[contains]] and no `minContains` *has* a floor of `1`, and any
  transformation that rewrites or combines nodes carries that floor with it (see
  Interactions → [[allOf]]).
- **`minContains:0`** turns the existential off: [[contains]] then "always
  passes" and the only remaining teeth come from a paired [[maxContains]] (the
  `0..max` range). Meaningless on its own — see Loader behavior.
- **No effect without [[contains]].** Per **P7.1** a `minContains` with no
  sibling `contains` is a load reject, not a no-op.
- Pure assertion; no annotation behavior, no effect on the emitted collection
  type.

## Support decision

**Support:** yes — a runtime **match-count** comparison, [[maxContains]] with
`≥`, on the same scalar-only envelope [[contains]] draws (scalar matcher over a
scalar [[items]] element type; composite matchers / elements deferred there and
therefore here too). Same grounding: **P10** (enforced), **P11** (aggregated),
**P12** — one comparison over one kind of operand, the match tally over the
array's complete element sequence, applied at both boundaries with the same
reason. No effect on emitted types.

Loader behavior (mirror of [[maxContains]] with `≥`, plus the `0` case):
- Value not a non-negative integer → reject: non-number (`minContains:"1"`,
  `minContains:true`), **negative** (`minContains:-1`), or **fractional**
  (`minContains:1.5`). `minContains:2.0` accepted (≡ `2`).
- The portable count ceiling from [[maxItems]], and the emitted-literal
  obligation that comes with a bound above 2^31−1, apply here unchanged.
- **`minContains` without [[contains]]** on the same node → **reject**
  (**P7.1**, statically meaningless — the spec's "no effect" tightened to a loud
  error). Diagnostic: add a `contains` matcher or remove `minContains`.
- **`minContains:1`** → accepted; it is the [[contains]] default restated. Not
  rejected as "redundant" — it is a spec-sanctioned identity that templated
  tooling emits, so honoring it is friendlier than a diagnostic (mirrors
  [[minItems]] `0` / [[uniqueItems]] `false`).
- **`minContains:0`** →
  - **with a sibling [[maxContains]]** → **accepted**: the `0..max` range (or,
    with `maxContains:0`, the *must-not-contain* assertion). The [[contains]]
    existence check is relaxed; [[maxContains]] carries the teeth.
  - **without [[maxContains]]** → **reject** (**P7.1**): `minContains:0` alone
    makes [[contains]] "always pass" with no upper bound, so the whole
    `contains` block asserts **nothing**. Diagnostic: remove the vacuous
    `contains`, or add a `maxContains`. (Contrast [[minItems]] `0`, which is a
    harmless no-op on a still-meaningful array type; here the *only* reason
    `contains` exists is the assertion `minContains:0` erases.)
- **`minContains` > [[maxContains]] → reject (unsatisfiable)**;
  `minContains == maxContains` pins an **exact** match count (accepted). All
  combined satisfiability for the pair lives in [[maxContains]].
- We do **not** cross-check `minContains` against how many elements *could*
  match the matcher (undecidable in general — parallel to the
  uniqueness-vs-count non-check in [[minItems]]).

## Type mapping

None. Constraint lives only in the validator.

## Validator mapping

Per **P10**/**P11**. A single `≥` comparison of the **match count** against the
fixed bound. Deserialize tallies the original wire elements and serialize
tallies the decoded collection, using the same matcher, comparison, and reason
in both directions (**P12**).

| Language   | Strategy                                                                                                                                                               |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | `UnmarshalJSON` tallies the original `json.RawMessage` elements; shared `Validate` tallies the typed slice before serialize. Both push the same bound/count violation. |
| TypeScript | The transfer converters tally the raw array inbound and the typed array outbound after the `Array.isArray` guard.                                                      |
| Python     | `_check_contains` receives the raw list inbound and the typed list outbound, appending the same bound/count violation.                                                 |
| Java       | The collecting deserializer tallies the original Jackson array node; serialize tallies the typed `List<T>`. Not bean-validation.                                       |

Reason strings name the concrete bound and offending match count
(`too few matching items: at least 2, got 1`), per the [[maxContains]]
count-family convention — distinct from [[minItems]]' *total*-count message.
When `minContains` is absent (the default `1`), the failure is [[contains]]' own
existential violation (`no element matches the required schema`), owned by
[[contains]]; an explicit `minContains` produces the count-family message.

### Serialize-side (P12)

Identical to [[maxContains]]: the predicate re-runs before emit over the decoded
value; an in-memory value with too few matches fails serialize rather than being
written. Real teeth in the statically-typed targets (Go/TS/Java), where
in-memory construction is unchecked. Both directions tally over the array's
complete element sequence — the wire array inbound, the decoded collection
outbound — so the check is the identical `≥` comparison in both directions.

## Property-testing matrix

### Accepted (positive)

| Shape                              | Example                                                                                   |
| ---------------------------------- | ----------------------------------------------------------------------------------------- |
| Match floor ≥ 2                    | `{type:"array", items:{type:integer}, contains:{minimum:5}, minContains:2}`               |
| `.0`-valued bound                  | `{type:"array", items:{type:string}, contains:{const:"x"}, minContains:2.0}`              |
| Explicit default (≡ contains)      | `{type:"array", items:{type:string}, contains:{const:"x"}, minContains:1}`                |
| Exact match count (min==max)       | `{type:"array", items:{type:string}, contains:{const:"x"}, minContains:2, maxContains:2}` |
| Range 0..max (existential relaxed) | `{type:"array", items:{type:string}, contains:{const:"x"}, minContains:0, maxContains:2}` |

### Rejected at load time (negative)

| Reason                                          | Example                                                                                   |
| ----------------------------------------------- | ----------------------------------------------------------------------------------------- |
| Value not a number                              | `minContains:"1"`, `minContains:true`                                                     |
| Negative value                                  | `minContains:-1`                                                                          |
| Fractional value                                | `minContains:1.5`                                                                         |
| Above the portable count ceiling                | `minContains:9007199254740992`                                                            |
| No sibling `contains` (P7.1)                    | `{type:"array", items:{type:string}, minContains:2}`                                      |
| `minContains:0` with no `maxContains` (vacuous) | `{type:"array", items:{type:string}, contains:{const:"x"}, minContains:0}`                |
| Unsatisfiable range                             | `{type:"array", items:{type:string}, contains:{const:"x"}, minContains:3, maxContains:1}` |

### Runtime fixtures (validator)

- Match count `== min` → OK (`≥` is inclusive).
- Match count `min-1` → one `PayloadValidationError` application failure naming
  the bound and count (`too few matching items: at least 2, got 1`).
- Empty array `[]` against `minContains:1` (or the bare [[contains]] default) →
  rejected; against `minContains:0, maxContains:N` → OK (zero matches allowed).
- `minContains:0, maxContains:2`: zero, one, or two matching elements → OK;
  three → fail (via [[maxContains]]).
- Combined with other failing assertions ([[maxContains]], [[minItems]] /
  [[maxItems]], a bad element per [[items]]) → **all** reported in one shot
  (**P11**). Serialize of an under-matching in-memory value → rejected before
  emit (**P12**).

## Interactions

- **[[contains]]**: `minContains` **is** the [[contains]] existential floor —
  omitting it ≡ `minContains:1`. It reuses [[contains]]' matcher predicate and
  scalar-only envelope, is a load reject without a sibling `contains`, and
  `minContains:0` relaxes [[contains]] to always-pass (meaningful only with
  [[maxContains]]).
- **[[maxContains]]**: the paired upper bound; owns the shared match-counting
  machinery and all combined satisfiability (`minContains > maxContains` reject;
  `minContains == maxContains` exact pin; the `maxContains:0` ⇒ `minContains:0`
  requirement).
- **[[allOf]] — the omitted default survives the merge.** Branches combine by
  their **effective** bounds: a branch carrying [[contains]] and no
  `minContains` contributes a floor of `1`, not "no opinion". The merged floor
  is the **greatest** of the branches' effective floors, the merged ceiling the
  **least** of their effective ceilings, and a merged floor above the merged
  ceiling rejects as unsatisfiable ([[maxContains]]). So a `minContains:0` in
  one branch cannot relax a sibling branch that omits the keyword: the merged
  schema is never weaker than a branch it merges. [[allOf]] owns the general
  rule this instantiates.
- **[[minItems]] / [[maxItems]]**: bound the **total** element count;
  `minContains` bounds the **matching** subset. Independent — all apply and
  aggregate. A `minContains:2` does not imply `minItems:2` (the two matches
  suffice, but the array's total size is a separate axis).
- **[[items]]**: types every element; `minContains` requires a floor number of
  them to also match the [[contains]] matcher. Supported only when `items` is
  scalar (via [[contains]]).
- **[[type]]**: gates applicability to `type:"array"` (through [[contains]]); a
  mismatch is a load reject (**P7.1**). The emitted collection type is
  unchanged.
- **[[uniqueItems]] / [[required]] / [[nullability]]**: orthogonal, exactly as
  for [[contains]] — element uniqueness, member presence, and member null are
  independent of the match-count floor.

## Ecosystem variance

| Source dialect                      | Action                                                                                                                      |
| ----------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12                 | Native.                                                                                                                     |
| OpenAPI 3.1                         | Adopts 2020-12 — `minContains` identical. Native.                                                                           |
| draft 2019-09                       | `minContains` introduced here with identical semantics. Native.                                                             |
| draft-7 / draft-6                   | `contains` exists but `minContains` does not — a bare `contains` is the fixed ≥ 1 existential (no way to change the floor). |
| OpenAPI 3.0 / Swagger 2.0 / draft-4 | No `contains` family — nothing to map.                                                                                      |

## See also

- [[maxContains]] — the paired inclusive upper bound; owns the shared
  match-counting machinery, the count-family `reason` strings, and the pair
  satisfiability rules.
- [[contains]] — supplies the matcher predicate and scalar-only envelope;
  `minContains` is its existential floor (default 1) and `minContains:0` relaxes
  it.
- [[minItems]] / [[maxItems]] — the **total**-element-count analog; same
  inclusive-bound and exact-pin conventions.
- [[items]] — supplies the emitted collection type and the scalar element type
  the matches are drawn from.
- [[type]] — gates applicability to `type:"array"`.
