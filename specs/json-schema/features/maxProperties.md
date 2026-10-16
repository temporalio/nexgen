# `maxProperties`

Source: JSON Schema 2020-12, Validation vocabulary, §6.5.1 "Validation Keywords
for Objects → maxProperties".

Caps the number of members an object instance may have. A pure runtime count
assertion — no type impact.

## Spec summary

Verbatim (2020-12 validation, §6.5.1):

> The value of this keyword MUST be a non-negative integer.

> An object instance is valid against "maxProperties" if its number of
> properties is less than, or equal to, the value of this keyword.

Distilled:
- Counts **all** members of the instance object, including ones not named in
  [[properties]] (i.e. preserved extras count too).
- No annotation behavior; pure assertion.

## Support decision

**Support:** yes — runtime assertion.

Lowers to a boundary count check in every language; no effect on emitted types.
Citing [[PRINCIPLES.md]]: **P10** (enforced at the boundary), **P11**
(aggregated).

Loader behavior:
- Value not a non-negative integer (per spec; honors the `1.0`-as-integer rule
  and the integer cap, see [[type]]) → reject.
- The portable count ceiling from [[maxItems]] applies, and so does the
  `long`-literal obligation the ceiling does not discharge: a bound in
  `[2^31, 2^53−1]` does not compile in Java unless the emitter suffixes the
  literal.
- The keyword requires `type: "object"`; a missing or different type rejects at
  load time under **P7.1**. One diagnostic covers the four object-constraint
  keywords.
- `maxProperties: 0` → accepted (object must be empty). Note this is a
  near-equivalent of a closed empty object; prefer `additionalProperties:false`
  with no [[properties]] when *emptiness* is the intent, and `maxProperties`
  when a numeric ceiling is.
- `maxProperties < minProperties` (both present) → reject (unsatisfiable).
  Diagnostic names both.
- `maxProperties` less than the count of [[required]] members → reject
  (unsatisfiable: required forces more members than the cap allows).
- **The cap is reconciled against every keyword that forces keys, not only
  [[required]].** If the smallest key count the schema forces exceeds the cap,
  the object is uninhabitable and rejects at load. Besides `required`'s
  unconditional presence that is [[dependentRequired]]: take the largest closure
  a single trigger forces, since the trigger plus its dependents must all be
  present together. The count keywords own this duty because the count is what
  makes the combination decidable, so the forcing keyword's own spec need not
  restate it. The mirror duty — a floor against a capped key space — is in
  [[minProperties]]. **Status: implemented**: the comparison uses the union of
  the trigger's transitive dependency closure and every always-present
  [[required]] key.

**Which object is counted, in both directions.** The count is over the
**wire object at the boundary being validated**: inbound, the raw decoded object
before default population; outbound, the object the encoder will actually write.
A member the encoder omits is not a wire key and does not count — that covers an
unset default, and in TypeScript any value the encoder drops (`undefined`, a
function, a `toJSON` returning `undefined`) or replaces (a `toJSON` returning
another object, whose members *do* reach the wire and *do* count). **P12.2**
names the failure mode directly: a predicate that counts wire keys on one side
and in-memory fields on the other is not conformant, however the two sides share
code.

The two counts can disagree where the [[nullability]] collapse omits a key the
wire carried. That direction only ever *removes* keys, so it cannot push a
parsed object over a **cap** — it is the floor that diverges, and the open
status is recorded in [[minProperties]].

## Type mapping

None. The emitted aggregate is unchanged; the constraint lives only in the
validator.

## Validator mapping

Per **P10**/**P11**. The "number of properties" is the count of
**distinct member keys present on the wire**, taken at the deserialize boundary
**before** default population (see [[default]]) — a default-filled key is never
on the wire and does not count (see Interactions). Count the wire object as a
single number; do **not** sum a declared-fields bucket and an extras bucket
separately (case-mapping can route a key to either, and the declared-vs-extras
split is a language-side artifact, not a wire fact).

| Language   | Strategy                                                                                                                                                                                                                                                                                                                  |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | `UnmarshalJSON` applies the predicate to the raw wire-key count; `Validate` applies the same comparison and reason to the key set `MarshalJSON` will write.                                                                                                                                                               |
| TypeScript | `fromTransferType` applies the predicate to the wire object's own enumerable keys; `toTransferType` applies it to the keys the outbound conversion will actually emit. The comparison is inlined in each converter.                                                                                                       |
| Python     | `from_transfer_type` counts `len(raw)` on the raw wire dict — one number over the wire object, taken before any default is materialized — and appends `Violation(path="", reason=f"must have at most {max} properties, got {n}")` when `n > max`, into the single generated `PayloadValidationError` application failure. |
| Java       | the per-POJO collecting deserializer (PRINCIPLES Java §5) counts distinct keys in the parsed tree (`> max`) — one number over the wire object, **not** populated POJO fields + catch-all map summed post-bind; a violation joins the single `PayloadValidationError` application failure.                                 |

### Serialize-side (P12)

The count runs again before emit, over the keys that
**will actually be written** — i.e. *after* default omission and the
omit-vs-`null` decision (the serialize mirror of "before default population"). A
field whose default is unset is omitted and does **not** count, exactly as it
didn't on the way in. Each language counts the members its own encoder will emit
— in Python, `len(out)` on the dict `to_transfer_type` has built — and an
over-cap model fails `MarshalJSON`/`toTransferType`/`to_transfer_type` rather
than emitting an out-of-bounds object. Because the in-memory model can *read* a
default as present that *serializes* as absent, a model can legitimately fail
`maxProperties`/`minProperties` on serialize that "looked" satisfiable in memory
— correct, since the assertion is about the wire object.

## Property-testing matrix

### Accepted (positive)

| Shape         | Example                                                      |
| ------------- | ------------------------------------------------------------ |
| Cap with room | `{type:object, additionalProperties:true, maxProperties:3}`  |
| Cap = 0       | `{type:object, additionalProperties:false, maxProperties:0}` |

### Rejected at load time (negative)

| Reason                           | Example                                                      |
| -------------------------------- | ------------------------------------------------------------ |
| Not non-negative integer         | `maxProperties:-1`, `maxProperties:1.5`, `maxProperties:"3"` |
| Missing/mismatched object type   | `{maxProperties:3}`, `{type:"array", maxProperties:3}`       |
| `< minProperties`                | `minProperties:5, maxProperties:2`                           |
| `<` required count               | `required:["a","b","c"], maxProperties:2`                    |
| `<` the keys a dependency forces | `dependentRequired:{a:["b","c"]}, maxProperties:2`           |

### Runtime fixtures (validator)

- Member count `== max` → OK (≤ is inclusive).
- Member count `max+1` (including via extras) → one `PayloadValidationError`
  application failure whose reason names the cap and count
  (`must have at most 3 properties, got 4`).
- Combined with other failing assertions → all reported in one shot (P11).

## Interactions

- **[[minProperties]]**: paired bound; both count the same member set.
  `min > max` is a load error.
- **[[required]] / [[properties]]**: extras count toward the total, so an open
  struct ([[additionalProperties]] `true`) can exceed a cap the declared members
  alone wouldn't.
- **[[additionalProperties]] `false`**: bounds the max member count to the
  declared set; a `maxProperties` larger than that count is redundant (allowed,
  not an error).
- **[[dependentRequired]]**: a trigger forces its whole dependent set to be
  present at once, so the cap is reconciled against the largest such closure —
  per the loader-behavior rule above.
- **`default`**: `default` is an annotation, not an assertion, and defaults are
  dropped on serialize (see [[default]]) — a default-filled key is never on the
  wire, so it does **not** count toward the cap. The count is taken before
  default population on deserialize.
- **`const`** (future feature): an object-level `const` pins the exact member
  set, making `maxProperties` statically decidable — a const object with more
  members than `maxProperties` is unsatisfiable (load reject), mirroring the
  [[required]] / [[additionalProperties]] `false` satisfiability checks.
  Property-level `const` only constrains a value *if present*, so it has no
  count impact unless paired with [[required]]. Enforcement deferred to the
  `const` spec.

## Ecosystem variance

| Source dialect        | Action                             |
| --------------------- | ---------------------------------- |
| JSON Schema 2020-12   | Native.                            |
| OpenAPI 3.1 / 3.0     | `maxProperties` identical. Native. |
| Swagger 2.0 / draft-4 | `maxProperties` identical. Native. |

## See also

- [[minProperties]] — lower bound on member count.
- [[required]] — pins which members; interacts with the cap.
- [[additionalProperties]] — whether extras (which also count) are allowed.
