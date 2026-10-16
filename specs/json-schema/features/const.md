# `const`

Source: JSON Schema 2020-12, Validation vocabulary, §6.1.3 "Validation Keywords
for Any Instance Type → const".

Pins an instance to a single fixed value. The discriminator primitive: a `const`
string on a member is how a typed object announces its variant on the wire.
Supported for scalar values. `const` is a **pure assertion** — a single-value
[[enum]] — checked in both directions by the shared `Validate` layer (**P12**):
any value other than the fixed one is **rejected** (**P13.1**). It carries
**no serialize-side special-casing**: the value reaches the wire because it is
*set in memory* (by the TS literal type, by the Java value constant, or by the
consumer in Go and Python — a required+const is a required field), not because
the serializer rewrites it.

## Spec summary

Verbatim (2020-12 validation, §6.1.3):

> The value of this keyword MAY be of any type, including null.

> Use of this keyword is functionally equivalent to an "enum" (Section 6.1.2)
> with a single value.

> An instance validates successfully against this keyword if its value is equal
> to the value of the keyword.

Distilled:
- A single-value assertion: the instance must **equal** the keyword's value —
  JSON equality, which compares **values and not their written form**. For the
  three scalar kinds that matters: a string compares by code points, a boolean
  by itself, and a **number by its mathematical value**, so a `const: 1` is
  satisfied by the wire tokens `1`, `1.0` and `1e0` alike, and `-0.0` equals
  `0.0` ([[type]]'s identity rule). A numeric `const` names one number, never
  one spelling of it; anywhere the generator has to decide whether two `const`
  values are *the same value* — the [[oneOf]] discriminator's pairwise
  distinctness above all — this is the comparison it owes, and comparing JSON
  representations instead would treat one value as two.
- Equivalent to `enum: [<value>]`; see [[enum]].
- Value may be any JSON type. In our subset only **scalar** consts (string /
  number / integer / boolean) are supported; `null` and composite (object /
  array) consts are handled below.
- It is an **assertion**, not an annotation — unlike [[default]], a non-matching
  value is a hard validation failure.

## Support decision

**Support:** yes (scalar values) — a runtime equality assertion, nothing more.
`const: null` and composite consts are rejected/deferred.

Rationale (citing [[PRINCIPLES.md]]):
- **P10 (enforced)**: the equality check runs at the (de)serializer boundary,
  aggregated per **P11**. It is a pure predicate over the decoded value,
  identical in both directions — the **shared `Validate`** layer of **P12**,
  with no serialize-side adapter logic of its own.
- **P13.1 (closed value set; unknown values rejected)**: `const` is a
  **closed contract** — the field admits only the fixed value, and any other
  value is a hard validation failure. The emitted type expresses that closedness
  in each language's idiom, for **every scalar kind**: a **closed literal**
  where literal types exist (TS `"v1"` / `1.5` / `true`; Python `Literal["v1"]`,
  with `float` the one exception — see Type mapping), a
  **defined type + typed constant** in Go (`type UserEventKind string` +
  `UserEventKindUser`), and a **value class** in Java (a known constant,
  obtainable only through the class). A schema revision that bumps the value
  (`"v1"`→`"v2"`) is a **breaking change** to the contract and surfaces as one —
  a compile error at stale call sites (the literal changes in TS/Python; the
  value-derived constant is renamed in Go/Java) — the correct, loud outcome: a
  changed value contract is not backward compatible, and the generator does not
  pretend otherwise. [[enum]] is the same closed machinery with more than one
  known value.
- **No auto-emit.** Like [[enum]], `const` is validated, not force-written:
  validate that the value equals the keyword on every model — whether
  constructed in-language or deserialized over the wire. The generator does
  **not** force-write the fixed value on serialize. The value lands on the wire
  because it is *set in memory*, and **presence is governed by [[required]]**,
  like every other field — a required+const is always present (so always
  emitted) for the same reason any required field is. Not force-writing keeps
  the serializer free of const special-casing and stops it from
  **silently rewriting** a wrong in-memory value: setting `kind="admin"` on a
  type whose const is `"user"` fails `Validate` loudly instead of being masked.

Loader behavior:
- `const` value type-incompatible with the declared [[type]] → reject per
  **P7.1** (`{type:"integer", const:"x"}` is statically unsatisfiable). Numeric
  compatibility is **directional** and owned by [[type]]: an integral value
  inhabits `integer` and `number` alike, a fractional one inhabits `number`
  only, so `{type:"integer", const:1.5}` rejects — including when the pairing
  arrives through an [[allOf]] merge. An `integer` value outside the `±(2^53−1)`
  cap likewise rejects, because it names a value the field can never hold. The
  const value must validate against the **rest** of the field's own schema too
  (e.g. `{type:"string", minLength:5, const:"ab"}` → reject — the fixed value
  can never satisfy the field). The const value is run through every
  *constraint* keyword present on the same node — [[pattern]],
  [[minLength]]/[[maxLength]], [[minimum]]/[[maximum]],
  [[exclusiveMinimum]]/[[exclusiveMaximum]], [[multipleOf]] — using that
  keyword's own load-time validator over the fixed value; a violation is a load
  reject. **Each constraint keyword owns its half of this check** (its spec
  states the rule and lists the const/default/enum load reject); `const`
  supplies the fixed value and inherits every such check that is present. The
  same obligation applies to [[default]] and [[enum]].
- `const` **and** [[default]] both present → reject. A const fixes the value; a
  default is then either redundant (equal) or contradictory (unequal).
  Diagnostic: drop the `default`; the const already determines the value.
- `const` **and** [[enum]] both present → reject as redundant (const is a
  single-value enum; pick one spelling). Diagnostic points at the equivalence.
- `const: null` → **reject**. A field that is *always* `null` carries no
  information — the same degenerate case as a standalone `{type:"null"}` (see
  [[type]]). If the intent is "nullable", use the [[nullability]] pattern; if
  "absent", omit the field.
- `const` **on a [[oneOf]] node** — a sum type *or* a [[nullability]] wrapper —
  → **reject**, with a fix-it naming the branch to move it to. A union node
  carries no scalar `type`, so the compatibility gate above has nothing to check
  the value against; a closed value set belongs on the branch whose kind it
  closes. This is [[oneOf]]'s sibling rule and [[nullability]]'s
  wrapper-versus-branch rule applied to `const`, and it is a **P7.1** obligation
  rather than a style preference: authored on the union node the value set is
  neither uniformly enforced nor uniformly dropped, so one schema names a
  different accepted value set in each target.
- Composite const (`const` whose value is an **object or array**) →
  **temporarily unsupported**; reject at load with a "not yet supported"
  diagnostic (not a categorical P6 exclusion — the deep structural-equality
  check is correct in principle, just costly; deferred past v1 and revisit on
  demand). Contrast [[default]], which explicitly avoids deep-equals; for
  `const` the deep-equals would be a genuine assertion, not an omission
  heuristic.

## Type mapping

The emitted type is **closed to the const value**, in each language's idiom and
for **every scalar kind** (**P13.1**). Optional vs required wrapping is owned by
[[required]] / [[nullability]].

| const value kind | Go                                        | TypeScript | Python           | Java                        |
| ---------------- | ----------------------------------------- | ---------- | ---------------- | --------------------------- |
| string           | `type UserEventKind string` + typed const | `"<v>"`    | `Literal["<v>"]` | value class `UserEventKind` |
| integer          | `type Priority int64` + typed const       | `<v>`      | `Literal[<v>]`   | value class `Priority`      |
| number           | `type Ratio float64` + typed const        | `<v>`      | `float`          | value class `Ratio`         |
| boolean          | `type Enabled bool` + typed const         | `<v>`      | `Literal[<v>]`   | value class `Enabled`       |

**Go.** A **defined type** over the primitive plus a typed value constant, for
every scalar kind:

```go
type UserEventKind string
const UserEventKindUser = UserEventKind("user")

type Priority int64
const Priority3 = Priority(3)
```

The field is typed with the defined type (`Kind UserEventKind`); the value
constant is the idiomatic way to set it (`UserEvent{Kind: UserEventKindUser}`)
and the value the validator compares against. The defined type gives the value
its own name, groups the constant(s) under it, and provides
**safety on initialization** — a bare primitive requires a deliberate cast
(`Priority(4)`) to reach the field, so a wrong value is a conscious act, caught
by `Validate`. It generalizes directly to [[enum]], which lists several
constants under one type. The value-constant name is `{Type}{EncodedValue}` (see
Naming and encoding). Both the defined type and each value constant are
exported, so both carry a name-led doc comment (PRINCIPLES.md, Go §1) — the
type's from the owning property's [[title]]/[[description]] when present, else a
fallback naming the declaring model and field; each constant's states the value
it holds (e.g. `// UserEventKindUser is the UserEventKind value "user".`).

**TypeScript.** The **closed literal** of the value — `kind: "user"`, `n: 1.5`,
`ok: true`. The literal type closes the field in-language (a wrong value is a
compile error); the deserialize validator compares the wire value against the
same literal.

**Python.** The **closed literal** via `Literal` — `Literal["user"]`,
`Literal[1]`, `Literal[True]` — carried as the dataclass field's default so a
consumer never has to restate the fixed value.
**`float` consts are the exception:** `Literal` forbids float members (PEP 586),
so a number const is plain `float` and closedness rests on the converter's
equality check alone.

**Java.** A generated **value class** wrapping the primitive, for every scalar
kind — a known constant, a private constructor, and Jackson
`@JsonCreator`/`@JsonValue` for wire mapping:

```java
public final class UserEventKind {
    public static final UserEventKind USER = new UserEventKind("user");
    private final String value;
    private UserEventKind(String value) { this.value = value; }

    @JsonCreator                              // standalone / interop decode
    public static UserEventKind fromString(String v) {
        if (v == null) return null;
        if ("user".equals(v)) return USER;
        throw new IllegalArgumentException(
            "must equal \"user\", got \"" + v + "\"");
    }
    @JsonValue public String getValue() { return value; }
    // equals/hashCode/toString by value (omitted)
}
```

Where a member receives this carrier, the private constructor makes the known
constant the only obtainable instance, so a wrong value
**cannot be constructed** in-language. The carrier is emitted for an
**object property**; an **array element** and a **typed-map value** keep the
primitive Java type, so in those positions closedness rests on the runtime
equality check alone ([[enum]] states the same scope for the multi-value form).
The value class is the shared carrier of `const` (one constant) and [[enum]]
(several). Numeric and boolean value classes wrap `long`/`double`/`boolean`,
with `@JsonCreator` over the corresponding primitive. How the aggregating
deserialize path validates (without the throw defeating aggregation) is in
Validator mapping.

**Float exactness (all languages).** A number const is compared with
**exact `==`**, never an epsilon — it is an equality assertion, and an epsilon
would admit near-but-unequal values. The comparison is deterministic and
portable: the wire value and the const literal are both IEEE-754 binary64
produced by correctly-rounded decimal→double parsing, so the same decimal yields
the identical bit pattern in every language and on every architecture (the path
only parses and compares, never computes). The loader carries the value through
its shortest round-trippable decimal, so every target re-parses to the same
double. `-0.0` equals `0.0`; `NaN`/`±Infinity` cannot appear (not JSON
literals); an integer-valued number such as `1.0` **is the same value as** `1` —
equality is over the mathematical number, never the authored spelling. That is a
claim about the comparison, not about a rewriting step, and it leaves the
carrier alone: on a `type: number` node the constant and the field stay the
target's binary64 type (see [[type]]). A float const asserts *exact* equality to
one specific double, so it is intended for values transmitted as authored — not
values arrived at by upstream computation.

### Naming and collisions (P15)

A `const` synthesizes identifiers that do not exist in the input schema — a
**named type** (Go defined type / Java value class) and a **value constant** —
for every scalar kind. (TS and Python close the type to an inline literal and
synthesize no type or constant; the value lives in the literal and the
validator.) **Type-name derivation follows the [[properties]] resolved policy:**
reuse the `$defs` name when the const is a **named** definition; when it is
**anonymous** (inline on a property), nest the synthesized type inside its
enclosing model where the language allows it, so it leaves the package/module
namespace.

> **`$defs`-named scalar closed values are unimplemented.** A `$defs` entry must
> currently be `type: object`, a `oneOf` union, or a bare `$ref`, so
> `$defs: {Color: {type: string, enum: [red, green, blue]}}` is a load reject in
> all four languages. Every "named definition" branch below — the `$defs`-name
> reuse, the P15 row for a `$defs`-named type, and `x-<lang>-const-name` on a
> `$defs` node — describes the intended design and is unreachable until scalar
> `$defs` entries are admitted.

| Target     | Synthesized identifier(s)                                          | Placement / scope                                       |
| ---------- | ------------------------------------------------------------------ | ------------------------------------------------------- |
| Go         | defined type `UserEventKind` **+** value const `UserEventKindUser` | **flat package** (Go has no nested types); P15 backstop |
| Java       | value class `Kind` + class-scoped constant (`USER`)                | **nested** `UserEvent.Kind`                             |
| TypeScript | none — the type is the inline literal `"user"`                     | —                                                       |
| Python     | none — the type is the inline `Literal["user"]`                    | —                                                       |

Per **P15** every synthesized name enters the **same per-scope namespace** as
the declared names and as one another; the generator runs a single collision
pass (after case-mapping) and **rejects at load** with a fix-it diagnostic on
any coincidence. A **type-name** collision is resolved by the [[properties]]
`x-<lang>-name` override on the declaring member — the synthesized type is named
from the member, so re-mapping it moves the type. A **value-constant** collision
is resolved by the `x-<lang>-const-name` override on the const schema (see
[Overriding the value constant](#overriding-the-value-constant)) — the value
constant is named from the *value*, a separate axis that `x-<lang>-name` does
not touch. Nesting shrinks the surface (a nested `UserEvent.Kind` cannot clash
with a top-level `UserEventKind`); **Go** stays flat and relies on the P15
backstop. **No auto-mangling** — a synthesized `UserEventKind2` would be
unstable across schema revisions (P13). The class-body surface where **many**
constants can case-map together (`"user"` + `"USER"` → both `USER`) is exercised
by
[[enum]].

**The pass runs over the schema that is emitted.** Naming derivation,
encodability and collision are properties of the value set the generator
actually emits a constant for, so they are decided **once**, on the resolved
node — after an [[allOf]] merge and after a `$ref` resolution. An authored
`allOf` branch is an input to that merge and not an emitted namespace of its
own: a branch value that would encode to a colliding or illegal token, but that
the merge removes, is not a collision, and rejecting it would refuse a schema
whose emitted form is clean. Shape checks that a merge could silently discard
still run per branch — see [[allOf]], which owns the split.

### Naming and encoding (value → identifier)

Go and Java name the value constant after the **value** — Go
`{Type}{EncodedValue}`, Java a class-scoped `{EncodedValue}` — so the value is
encoded as an identifier through the [[properties]] Stage 1–4 pipeline, with a
per-kind front-end producing the token and a **constant recasing scope** (Go
`PascalCase`, Java `UPPER_SNAKE`):

| kind    | token                                                              | Go                  | Java             |
| ------- | ------------------------------------------------------------------ | ------------------- | ---------------- |
| string  | value split on **any non-alphanumeric ASCII** into words (Stage 1) | `UserEventKindUser` | `USER`           |
| integer | the digits                                                         | `Priority3`         | `V_3`            |
| number  | shortest round-trippable decimal, `.` → `_`                        | `Ratio3_14`         | `V_3_14`         |
| boolean | `True` / `False`                                                   | `EnabledTrue`       | `TRUE` / `FALSE` |

Rules:
- **Negatives** encode the sign as the word `Neg`: `-3` → `PriorityNeg3` /
  `V_NEG_3`; `-3.14` → `RatioNeg3_14`.
- **Numbers** encode from the shortest round-trippable decimal; the `.` becomes
  `_` and is **kept** (so `3_14` stays distinct from `314`). A magnitude that
  canonicalizes to exponent form encodes `e` as `E`, drops a positive exponent
  sign, and encodes a negative one via `Neg` (`1e-7` → `Ratio1ENeg7`, `1e+20` →
  `Ratio1E20`). The decimal comes from the **value**, not the authored spelling:
  P1 makes `1`, `1.0` and `1e0` one number, so an integral value collapses to
  its integer form and all three spellings name one constant (`Score1` / `V_1`).
  Re-spelling a `const` is a no-op on the wire and must not rename a constant
  out from under callers (P13). **This holds at every magnitude.** There is no
  threshold above which the authored spelling is used instead — falling back to
  it past `2^53` would both reintroduce the instability the canonical decimal
  exists to remove (`1e+20` and `100000000000000000000` are one number and must
  name one constant) and let characters that are legal in a JSON number but not
  in an identifier — `+` above all — reach the emitted token, where Go and Java
  do not even parse.
- **Java leading-letter guarantee.** Java constants are class-scoped with no
  type prefix, and Stage 3 rejects an identifier beginning with a digit. A token
  that does not start with an ASCII letter (every numeric, and digit-leading
  strings) is prefixed `V_`; string and boolean tokens that already start with a
  letter are used as-is (`USER`, `TRUE`). Go needs no prefix — the `{Type}`
  prefix always supplies a leading letter.
- **Empty or illegal** encodings are **rejected at load** by Stage 3 — e.g. a
  string const `"-"` (all separators → empty token) — with a diagnostic pointing
  at the `x-<lang>-const-name` override (below), which names the value constant
  directly. `x-<lang>-name` cannot rescue it: that override moves the
  synthesized *type*, whereas the empty token belongs to the *value constant*
  (in Java the constant has no member-derived component at all — it is purely
  the encoded value).
- The encoding is a **readable handle, not a lossless round-trip**: it is
  intentionally many-to-one (`"user-admin"`, `"user_admin"`, `"userAdmin"` all →
  `UserAdmin`; string `"3"` and integer `3` both → `…3`). Genuine clashes are
  caught by the P15 collision pass and **rejected**, never auto-mangled. Because
  a number token derives from the canonical round-trippable decimal, the name is
  stable across schema revisions (P13).
- `const` string values are restricted to **ASCII without whitespace** (rejected
  at load otherwise), keeping the string front-end to the Stage 1 word-splitter.

### Overriding the value constant

The value constant is named from the **value**, on a different axis from the
member name. The [[properties]] `x-<lang>-name` override moves the synthesized
**type** (which *is* derived from the member) but leaves the value constant
untouched — in **Java** the constant is purely the encoded value with no
member-derived component, and a **`$defs`**-named const has no declaring member
at all (the motivating case, though a scalar `$defs` entry does not load yet —
see *Naming and collisions*). So the escape hatch for the value-constant axis is
a separate override, **`x-<lang>-const-name`** (`x-go-const-name` /
`x-java-const-name`), placed on the **const schema** — the node carrying
`const`, whether inline on a property or a named `$defs` definition. Like
`x-<lang>-name` it is used **verbatim** (it skips the Stage 1–4 encoding), must
itself be a legal, non-reserved identifier in that language, and participates in
the P15 collision pass. Only **Go** and **Java** synthesize a value constant;
**TS** and **Python** carry the value in an inline literal and have nothing to
override, so the keyword is inert there. This is the only way to admit a value
that encodes to an empty or illegal token (`const "-"`) or to separate two
values whose encodings case-map together (`"user-admin"` ⨯ `"user_admin"` → both
`UserAdmin`), consistent with the **no-auto-mangling** rule (P13/P15).

[[enum]] runs the same value→constant encoding per member, so it inherits the
same empty/illegal-token and encoding-clash cases; the per-value override
mechanism for the multi-value form is specified there.

## Validator mapping

Per **P10**/**P11**. A single equality check against the fixed value, identical
in both directions (it is a pure predicate over the decoded value — the
**shared `Validate`** layer of **P12**).

| Language   | Strategy                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Go         | A predicate in the shared `Validate`, applied identically on both directions' paths (**P12.2**: sharing is a requirement on the predicate, not on the call graph, so reaching it through `Validate` or emitting the same check inline is an emission choice): `if v != UserEventKindUser { … Violation{Path, Reason: fmt.Sprintf("must equal %q, got %q", UserEventKindUser, v)} }`, collected into one `PayloadValidationError` application failure. The field is the defined type; the typed constant is both the compared value and the idiomatic setter (`UserEvent{Kind: UserEventKindUser}`).    |
| TypeScript | the shared `Validate` predicate compares against the literal: ``if (v !== "user") push(Violation{path, reason: `must equal "user", got ${JSON.stringify(v)}`})``, throwing one `PayloadValidationError` application failure. The field's literal type closes it in-language.                                                                                                                                                                                                                                                                                                                           |
| Python     | the transfer type converter (PRINCIPLES Python §3) compares against the literal — `v != "user"` → `Violation(path=…, reason='must equal "user", got <json>')`, the same reason string TypeScript emits — aggregated into the single generated `PayloadValidationError` application failure. The field is the closed `Literal` (`float` consts are plain `float`, validated the same way).                                                                                                                                                                                                              |
| Java       | the aggregating path is the per-POJO collecting deserializer (PRINCIPLES Java §5), which does a **non-throwing membership lookup** — known value → the constant, otherwise record a `Violation{path, "must equal \"user\", got …"}` — so multiple bad fields all collect into the single `PayloadValidationError` application failure, consistent with every other §5 constraint helper. The value class's `@JsonCreator fromString` *throws* only on the **standalone/interop** path, where fail-fast is expected. Serialize needs no separate check: the value class can only hold a known constant. |

### Co-authored assertions on the same node

A closed value set may sit beside a string or numeric assertion
([[minLength]]/[[maxLength]], [[pattern]], [[format]], [[minimum]]/[[maximum]],
[[multipleOf]], …). The load gate has already run every such keyword's own
validator over the fixed value (see Loader behavior), so the assertion cannot
fail for an admissible value — and it is nonetheless **still emitted**.
**Closedness changes the carrier, never the assertion.** Where a target gives
the closed set its own named type — Go's defined type, Java's value class — the
co-emitted predicate is evaluated over that value's **underlying primitive**,
converting at the call site. Handing the named type straight to a primitive that
takes the underlying type does not compile, and dropping the predicate instead
would make the emitted validator depend on the load-time check having been
exact. [[maxLength]]/[[minLength]], [[pattern]] and [[format]] state this same
rule from their side; it is one rule, not four.

On a **materialized** node — a temporal [[format]] or a [[contentEncoding]] —
both the assertion and the closed-value comparison measure the **canonical**
wire string rather than the authored literal. That string is projected
**once per member** and both predicates read that one projection. Two
independent projections are two operands, which is exactly the drift **P12.2**
forbids, and a second projection in the same scope is also a redeclaration in
the targets that emit straight-line code.

### Serialize-side (P12)

There is **no const-specific serialize logic**. `const` rides the same encode
path as every other field: a set field is emitted, an unset optional field is
omitted. Presence is owned entirely by [[required]] — a required+const is always
present (so always emitted), an optional+const emits iff the consumer opted it
in. The fixed value reaches the wire because each language guarantees it is
*set in memory*, never because the adapter rewrites it:

| declaration      | in-memory                                   | serialize                                                |
| ---------------- | ------------------------------------------- | -------------------------------------------------------- |
| required + const | always set (by type / `final` / consumer)   | emitted by the normal encode path                        |
| optional + const | present iff the consumer opts the member in | emitted **if set**, omitted if unset (normal omit-unset) |

How each language guarantees "set in memory" — and validates on the way in:

| Language   | Mechanism                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | Field typed with the defined type (`Kind UserEventKind`), set idiomatically via the typed value constant (`UserEvent{Kind: UserEventKindUser}`). A forgotten field is the zero value (`UserEventKind("")`), which the shared `Validate` rejects **loudly** on serialize — consistent with how Go treats every required field. optional+const uses a pointer to the defined type + `,omitempty`, validated when non-nil.                                                                                                                                             |
| TypeScript | The field is the closed literal (`kind: "user"`); a wrong value is a compile error, so a required+const is always correct in memory and emitted by the normal `toTransferType`. optional+const emits when not `undefined`.                                                                                                                                                                                                                                                                                                                                          |
| Python     | Presence follows [[required]] like any field — **no auto-fill on parse**: a required+const absent on the wire is a `"required"` violation, an optional+const absent stays omitted, and `from_transfer_type` enforces `== "user"` whenever the value is present. In memory the dataclass field carries the const as its **default**, so a consumer never has to restate it and `to_transfer_type` always has the right value to write; it re-checks equality before emitting the key. This all runs under the **default Temporal converter** (PRINCIPLES Python §3). |
| Java       | `private final UserEventKind kind = UserEventKind.USER;` for required+const, getter only. The value class can only hold a known constant, so the getter (via `@JsonValue`) emits `"user"` by the normal path. On the way in, the collecting deserializer's membership lookup records a `Violation` for a non-`"user"` wire value. optional+const is a `@Nullable UserEventKind` constructor parameter, validated if non-null. Numeric/boolean consts use their value classes the same way.                                                                          |

The serialize equality check has teeth only where a wrong value can be set in
memory before emit: an optional+const set to a wrong value, a Go
zero-value/mutated field, or any Python in-memory assignment (a dataclass
validates nothing on construction, PRINCIPLES Python §1). Where a closing
carrier *is* emitted — the TypeScript literal, and the Java value class on a
required+const object property (`final`) — the value cannot be wrong in memory,
so the check is effectively a deserialize-direction guard. Java array-element
and typed-map positions keep the primitive carrier (Type mapping), so there it
has teeth in both directions.

## Property-testing matrix

### Accepted (positive)

| Shape                                  | Example                                                                                                                                                       |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| String discriminator                   | `{type:"string", const:"user"}`                                                                                                                               |
| String needing encoding                | `{type:"string", const:"user-admin"}` → `UserAdmin`                                                                                                           |
| Integer const                          | `{type:"integer", const:3}`                                                                                                                                   |
| Boolean const                          | `{type:"boolean", const:true}`                                                                                                                                |
| Float const (exact `==`)               | `{type:"number", const:3.14}` → `Ratio3_14`                                                                                                                   |
| Negative float const                   | `{type:"number", const:-3.14}` → `RatioNeg3_14`                                                                                                               |
| Integer-valued number const            | `{type:"number", const:1.0}` → value `1`, name `Ratio1`, carrier still `number`                                                                               |
| Un-encodable value rescued by override | `{type:"string", const:"-", x-go-const-name:"Dash", x-java-const-name:"DASH"}` → Go `Dash`, Java `DASH` (empty token would otherwise reject; TS/Python inert) |

### Rejected at load time (negative)

| Reason                                                | Example                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ----------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Type-incompatible (P7.1)                              | `{type:"integer", const:"x"}`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Fractional value on `integer` (directional, [[type]]) | `{type:"integer", const:1.5}`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `integer` value outside the `±(2^53−1)` cap           | `{type:"integer", const:9007199254740992}`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `const` on a [[oneOf]] node                           | `{oneOf:[{type:"string"},{type:"integer"}], const:"x"}`, `{oneOf:[{type:"string"},{type:"null"}], const:"x"}` — author it on the branch                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Fails own subschema's constraint                      | `{type:"string", minLength:5, const:"ab"}`, `{type:"integer", minimum:5, const:2}`, `{type:"string", pattern:"^[a-z]+$", const:"A1"}`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| With `default`                                        | `{type:"string", const:"v1", default:"v1"}`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| With `enum` (redundant)                               | `{type:"string", enum:["a"], const:"a"}`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `const: null` (degenerate)                            | `{type:"null", const:null}`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Composite const (deferred)                            | `{type:"object", const:{a:1}}`, `{type:"array", const:[1]}`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Non-ASCII / whitespace string value                   | `{type:"string", const:"user admin"}`, `{type:"string", const:"café"}`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Value un-encodable as an identifier                   | `{type:"string", const:"-"}` (empty token → Stage 3 reject; override: `x-<lang>-const-name`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Synthesized-name collision (P15)                      | Go flat `UserEventKind`/`UserEventKindUser` ⨯ a declared top-level name; a `$defs`-named const reusing an existing type name; two values whose encodings collide (`"user-admin"` ⨯ `"user_admin"` → both `UserAdmin`). Type-name clash → `x-<lang>-name` on the declaring member, which moves the synthesized type because it is named `<Type><Member>` off the *emitted* member identifier (`kind` + `x-go-name: Category` → `ProbeCategory`); value-constant clash → `x-<lang>-const-name`. (Nesting removes the Java anonymous case; Go stays flat → still caught. TS and Python close the type inline and synthesize no named type; TS still emits a module-scope `<FIELD>_CONST` binding for the value, which joins the same collision pass — two of them can coincide through the model-name disambiguator, e.g. a prefixed `A.kind` → `A_KIND_CONST` against an unprefixed `C.aKind`, and emitting both would be a duplicate `const` in one module.) |

### Runtime fixtures (validator)

- Wire value equals const → OK (both directions).
- Wire value present but `!= const` → one `PayloadValidationError` application
  failure naming the expected and actual value
  (`must equal "user", got "admin"`).
- required+const **absent on the wire** → required violation (see [[required]]),
  reported as a presence error, not a const error.
- Serialize of a correctly-set required const → the fixed value on the wire
  (TS/Java cannot be wrong; Python requires the field, so the consumer sets it;
  Go set via the value constant).
- Serialize of a Go zero-value / bypassed required const
  (`Kind == UserEventKind("")`) → rejected **loudly** by `Validate`, not
  silently rewritten (the generator never force-writes the value).
- Serialize after mutating an optional+const to a wrong value → rejected before
  emit (**P12**).
- Override-named const (`const:"-"` + `x-go-const-name:"Dash"` /
  `x-java-const-name:"DASH"`): wire value `"-"` round-trips — validates equal in
  both directions (the override renames the constant, not the compared value),
  and the value constant (Go `Dash`, Java `DASH`) sets it in memory; any other
  wire value → one `PayloadValidationError` application failure
  (`must equal "-", got …`). Confirms the override affects only the synthesized
  identifier, never the equality check.

## Interactions

- **[[enum]]**: `const` ≡ a single-element `enum` (spec §6.1.3), and the two
  **share one representation** — a closed value set that rejects any
  unrecognized value (TS/Python literal, Go defined type + value consts, Java
  value class). `const` is the single-value specialization; [[enum]] lists
  several known values in the same machinery. We reject the two together;
  `const` is the canonical spelling for the one-value case, [[enum]] for the
  multi-value case. An off-set value is a hard reject in both.
- **[[type]]**: the const value must be assignable to the declared type;
  mismatch is a load-time reject (**P7.1**). The emitted type is closed to that
  value — a literal / defined type / value class over `type`'s primitive mapping
  (**P13.1**).
- **[[required]]**: owns presence entirely. required+const is always set in
  memory (so always emitted) — the discriminator — for the same reason any
  required field is; optional+const is validated-if-present and emit-if-set.
  const itself adds no serialize behavior; it only asserts the value.
- **[[default]]**: mutually exclusive (load reject). `const` fixes the value;
  `default` supplies one for absence — combining them is redundant or
  contradictory. The two sit at opposite ends: a required `const` is always
  present and asserted; a `default` value is *off the wire* (omit-unset) and
  only materialized on read. That opposition is exactly why they don't co-occur.
- **[[oneOf]] / discriminated unions**: a per-branch `const` on a shared
  required member name **is** the discriminator [[oneOf]] keys on for object
  tagged unions — `const` specifies the *value* contract, and [[oneOf]] reuses
  it (unchanged) as the selector. Because the discriminator type is **closed**
  (**P13.1**), bumping a branch's `const` value is a deliberate breaking change
  to that branch, surfaced at compile time — the intended, loud outcome for a
  changed contract. The pairwise distinctness [[oneOf]] requires of those
  discriminators is decided by **value equality**, the same equality this spec's
  Float-exactness rule and [[type]]'s identity rule define: `const: 1` and
  `const: 1.0` are one value, so two branches carrying them are not distinct and
  the union rejects. Comparing JSON *representations* instead would admit that
  union while the emitted dispatch — which selects numerically — could never
  reach its second branch. A `const` on the union node itself is a reject
  (Loader behavior); the discriminator lives inside each branch.
- **[[items]] / [[contains]]**: two closed value sets over the same value
  position must **intersect** — a closed-value element type and a closed-value
  [[contains]] matcher with disjoint sets describe an array no value can
  satisfy, so the schema is unsatisfiable and rejects at load. Only emptiness
  rejects; a matcher that narrows the element's set is accepted ([[contains]]
  owns that case, [[enum]] the multi-value form of this one).
- **[[minProperties]] / [[maxProperties]]**: an **object-level** const would pin
  the exact member set, making the count statically decidable (noted in both
  specs) — but object-level const is deferred (see Loader behavior above), so
  that interaction is dormant in v1. A **property-level** const only constrains
  a value if present; it affects the count only when paired with [[required]].
- **[[nullability]]**: `const: null` is rejected (degenerate). A nullable member
  with a fixed value is the nullability `oneOf` with the `const` authored
  **on the non-null branch**; the wrapper node must not carry it (Loader
  behavior). The two axes stay orthogonal there: the branch's value set is
  closed and the field still admits an explicit `null`, which the closed check
  never sees.

## Ecosystem variance

| Source dialect        | Action                                                                                                                                         |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12   | Native (scalar). Native.                                                                                                                       |
| OpenAPI 3.1           | Adopts 2020-12 — `const` native.                                                                                                               |
| OpenAPI 3.0           | No `const` keyword; the idiom is `enum: [<value>]`. A single-element `enum` → accept as the equivalent const (single-value [[enum]] handling). |
| Swagger 2.0 / draft-4 | No `const` (draft-6+); single-element `enum` → same as OAS 3.0.                                                                                |

## See also

- [[enum]] — the multi-value sibling; `const` ≡ single-element enum, sharing one
  closed representation (both reject unrecognized values).
- [[properties]] — owns the identifier case-mapping + collision/escape-hatch
  policy that governs const's synthesized type/value-const names (P15).
- [[type]] — supplies the emitted primitive type; gates value compatibility.
- [[required]] — owns presence; a required+const is the always-present
  discriminator (because it is required, not because const says so).
- [[default]] — the semantic opposite (off-the-wire/omit-unset vs
  always-present-and-asserted); mutually exclusive with `const`.
- [[nullability]] — `const: null` rejected; otherwise orthogonal.
- [[oneOf]] — object discriminated-union dispatch keys on a required `const`.
- [[minProperties]] / [[maxProperties]] — object-level const (deferred) would
  make counts static.
