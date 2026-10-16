# `enum`

Source: JSON Schema 2020-12, Validation vocabulary, §6.1.2 "Validation Keywords
for Any Instance Type → enum".

Pins an instance to one of a **fixed set** of values. The multi-value sibling of
[[const]]: `const` is the one-value specialization, `enum` lists several known
values in the **same closed machinery**. Supported for scalar values. `enum` is
a **pure assertion** — checked in both directions by the shared `Validate` layer
(**P12**): any value outside the set is **rejected** (**P13.1**). It carries
**no serialize-side special-casing**: a value reaches the wire because it is
*set in memory* (by the TS/Python literal type, by the Go/Java value constant),
not because the serializer rewrites it.

## Spec summary

Verbatim (2020-12 validation, §6.1.2):

> The value of this keyword MUST be an array. This array SHOULD have at least
> one element. Elements in the array SHOULD be unique.

> An instance validates successfully against this keyword if its value is equal
> to one of the elements in this keyword's array value.

> Elements in the array might be of any type, including null.

Distilled:
- A **closed set** assertion: the instance must **equal one of** the array
  elements, under the same value-not-spelling JSON equality [[const]] defines —
  so a member `1` is satisfied by the wire tokens `1`, `1.0` and `1e0`, and it
  is that comparison that makes `enum: [1, 1.0]` a duplicate.
- A single-element `enum` is functionally a [[const]]; see §6.1.3.
- Elements may be any JSON type. In our subset only **scalar** enums (string /
  number / integer / boolean), **homogeneous** with the declared [[type]], are
  supported; `null` members and composite (object / array) members are handled
  below.
- It is an **assertion**, not an annotation — unlike [[default]], an off-set
  value is a hard validation failure.

## Support decision

**Support:** yes (scalar values) — a runtime membership assertion, nothing more.
`null` members and composite members are rejected/deferred, exactly as
[[const]].

Rationale (citing [[PRINCIPLES.md]]):
- **P10 (enforced)**: the membership check runs at the (de)serializer boundary,
  aggregated per **P11**. It is a pure predicate over the decoded value,
  identical in both directions — the **shared `Validate`** layer of **P12**,
  with no serialize-side adapter logic of its own.
- **P13.1 (closed value set; unknown values rejected)**: `enum` is a
  **closed contract** — the field admits only the listed values, and any other
  value is a hard validation failure. The emitted type expresses that closedness
  in each language's idiom, for **every scalar kind**: a
  **closed union of literals** where literal types exist (TS
  `"red" | "green" | "blue"`; Python `Literal["red","green","blue"]`, with
  `float` the one exception — see Type mapping), a
  **defined type + one typed constant per value** in Go, and a **value class**
  carrying one constant per value in Java. Adding a value is a
  backward-compatible widening only for producers; for a consumer on the older
  schema an unrecognized value is **rejected**, not silently preserved — a
  changed value set is a contract change and surfaces as one (**P13.2**: the
  wire is forward-compatible for unknown *fields*, **not** for unknown
  `const`/`enum` *values* — **P13.1**). An **open enum** (accept-and-preserve
  unknown members, e.g. TS `"red" | (string & {})`) was considered and
  **rejected**: P13.1 governs, so the value set is closed in every language and
  the unknown value is a loud failure.
- **No auto-emit.** Like [[const]], `enum` is validated, not force-written: the
  generator validates that the value is one of the set on every model —
  constructed in-language or deserialized — and never force-writes a value on
  serialize. As with `const`, an absent required enum is a [[required]]
  violation (never auto-filled — for enum there is no single value to pick
  anyway); presence is governed by [[required]] like every other field.

Loader behavior:
- The array **MUST be non-empty** — an empty `enum: []` is statically
  unsatisfiable (no value validates) → reject.
- **Duplicate members** → reject as redundant (the spec's SHOULD-unique
  tightened to MUST). This includes members that are
  *distinct on the wire but collide after identifier encoding* (`"user-admin"` ⨯
  `"user_admin"` → both `UserAdmin`) — caught by the P15 collision pass; resolve
  with the per-value override (below).
- **Single-element** `enum` (`enum: ["v1"]`) → normalized to the [[const]]
  representation; `const` is the canonical spelling for the one-value case.
  (This is also the OpenAPI 3.0 / draft-4 const idiom — see Ecosystem variance.)
- Every member must be type-compatible with the declared [[type]] (**P7.1**); a
  **mixed-type** array (`enum: [1, "a", true]`) or a member the type rejects →
  load reject. Numeric compatibility is **directional** and owned by [[type]]:
  an integral member inhabits `integer` and `number` alike, a fractional one
  inhabits `number` only, so `{type:"integer", enum:[1, 1.5]}` rejects —
  including when the pairing arrives through an [[allOf]] merge. An `integer`
  member outside the `±(2^53−1)` cap likewise rejects, because it names a value
  the field can never hold. Each member is additionally run through every
  *constraint* keyword present on the same node — [[pattern]],
  [[minLength]]/[[maxLength]], [[minimum]]/[[maximum]],
  [[exclusiveMinimum]]/[[exclusiveMaximum]], [[multipleOf]] — using that
  keyword's own load-time validator; a violation on **any** member is a load
  reject. **Each constraint keyword owns its half of this check** (its spec
  states the rule and lists the const/default/enum load reject); the same
  obligation applies to [[const]] and [[default]].
- `enum` **and** [[const]] both present → reject as redundant (const is a
  single-value enum; pick one spelling). Diagnostic points at the equivalence
  (see [[const]]).
- A **`null` member** (`enum: ["a", null]`) → **reject** in v1: a `null` member
  means "nullable", which is owned by the [[nullability]] pattern (wrap a
  non-null enum), not encoded as an enum element. Diagnostic points at the
  nullability pattern. (Parallel to `const: null`.)
- `enum` **on a [[oneOf]] node** — a sum type *or* a [[nullability]] wrapper — →
  **reject**, with a fix-it naming the branch to move it to. A union node
  carries no scalar `type`, so the compatibility gate above has nothing to check
  the members against; a closed value set belongs on the branch whose kind it
  closes. Same rule and same **P7.1** grounding as [[const]]'s; see [[oneOf]]
  and [[nullability]].
- **Composite members** (an element that is an **object or array**) →
  **temporarily unsupported**; reject at load with a "not yet supported"
  diagnostic — the deep structural-equality membership check is correct in
  principle, just costly (deferred past v1, revisit on demand). Same stance as
  [[const]].

## Type mapping

The emitted type is **closed to the value set**, in each language's idiom and
for **every scalar kind** (**P13.1**). Optional vs required wrapping is owned by
[[required]] / [[nullability]].

| enum value kind | Go                                                | TypeScript                   | Python                          | Java                   |
| --------------- | ------------------------------------------------- | ---------------------------- | ------------------------------- | ---------------------- |
| string          | `type Color string` + one typed const per value   | `"red" \| "green" \| "blue"` | `Literal["red","green","blue"]` | value class `Color`    |
| integer         | `type Priority int64` + one typed const per value | `1 \| 2 \| 3`                | `Literal[1,2,3]`                | value class `Priority` |
| number          | `type Ratio float64` + one typed const per value  | `1.5 \| 2.5`                 | `float`                         | value class `Ratio`    |
| boolean         | `type Flag bool` + typed const per value          | `true \| false`              | `Literal[True,False]`           | value class `Flag`     |

**Go.** A **defined type** over the primitive plus
**one typed value constant per member**, for every scalar kind:

```go
type Color string
const (
    ColorRed   = Color("red")
    ColorGreen = Color("green")
    ColorBlue  = Color("blue")
)
```

The field is typed with the defined type (`C Color`); the value constants are
the idiomatic way to set it (`Palette{C: ColorRed}`) and the values the
validator compares against. This is exactly [[const]]'s shape with more than one
constant grouped under the type. Each value-constant name is
`{Type}{EncodedValue}` (see Naming and encoding). Doc comments follow
[[const]]'s rule: the type gets a name-led comment (from the owning property's
[[title]]/[[description]], else a fallback naming the model and field), and
**each** value constant gets its own name-led comment stating the value it holds
(PRINCIPLES.md, Go §1) — not just a comment on the `const (...)` block.

**TypeScript.** The **closed union of literals** —
`c: "red" | "green" | "blue"`. The union closes the field in-language (a wrong
value is a compile error); the deserialize validator compares the wire value
against the same set.

**Python.** The **closed literal set** via `Literal` —
`Literal["red","green","blue"]`. **`float` members are the exception:**
`Literal` forbids float members (PEP 586), so a number enum is plain `float` and
closedness rests on the converter's membership check alone (as in [[const]]).

**Java.** A generated **value class** wrapping the primitive, carrying one known
constant per member — a private constructor, a membership lookup, and Jackson
`@JsonCreator`/`@JsonValue` for wire mapping:

```java
public final class Color {
    public static final Color RED   = new Color("red");
    public static final Color GREEN = new Color("green");
    public static final Color BLUE  = new Color("blue");
    private final String value;
    private Color(String value) { this.value = value; }

    @JsonCreator                              // standalone / interop decode
    public static Color fromString(String v) {
        if (v == null) return null;
        if ("red".equals(v)) return RED;
        if ("green".equals(v)) return GREEN;
        if ("blue".equals(v)) return BLUE;
        throw new IllegalArgumentException(
            "must be one of [\"red\",\"green\",\"blue\"], got \"" + v + "\"");
    }
    @JsonValue public String getValue() { return value; }
    // equals/hashCode/toString by value (omitted)
}
```

Where a member receives this carrier, the private constructor makes the known
constants the only obtainable instances, so a value outside the set
**cannot be constructed** in-language. The carrier is emitted for an
**object property**; an **array element** and a **typed-map value** keep the
primitive Java type, so in those positions closedness rests on the runtime
membership check alone, and narrowing the set surfaces there at runtime rather
than as a compile break. A **union variant** is expected to behave as those two
do; that leg is not established. This is the shared carrier of [[const]] (one
constant) and `enum` (several). Numeric and boolean value classes wrap
`long`/`double`/`boolean`, with `@JsonCreator` over the corresponding primitive.
How the aggregating deserialize path validates (without the throw defeating
aggregation) is in Validator mapping.

**Float exactness (all languages).** Number members are compared with
**exact `==`**, never an epsilon — identical to [[const]]: the wire value and
each literal are IEEE-754 binary64 from correctly-rounded decimal→double
parsing, so the same decimal yields the identical bit pattern everywhere. `-0.0`
equals `0.0`; `NaN`/`±Infinity` cannot appear; an integer-valued number member
such as `1.0` **is the same value as** `1` — equality is over the mathematical
number, not the authored spelling. That is a claim about the comparison rather
than a rewriting step, and it leaves the carrier untouched: it stays the node's
own `type` (see [[type]]).

### Naming and collisions (P15)

An `enum` synthesizes identifiers that do not exist in the input schema — a
**named type** (Go defined type / Java value class) and
**one value constant per member** — for every scalar kind. (TS and Python close
the type to an inline union of literals and synthesize no type or constants; the
values live in the literal and the validator.)
**Type-name derivation follows the [[properties]] resolved policy:** reuse the
`$defs` name when the enum is a **named** definition; when it is **anonymous**
(inline on a property), nest the synthesized type inside its enclosing model
where the language allows it, so it leaves the package/module namespace.

> **`$defs`-named scalar closed values are unimplemented.** A `$defs` entry must
> currently be `type: object`, a `oneOf` union, or a bare `$ref`, so
> `$defs: {Color: {type: string, enum: [red, green, blue]}}` is a load reject in
> all four languages. Every "named definition" branch below — the `$defs`-name
> reuse, the P15 row for a `$defs`-named type, and `x-<lang>-const-name` on a
> `$defs` node — describes the intended design and is unreachable until scalar
> `$defs` entries are admitted.

| Target     | Synthesized identifier(s)                                        | Placement / scope                                       |
| ---------- | ---------------------------------------------------------------- | ------------------------------------------------------- |
| Go         | defined type `Color` **+** one const per value (`ColorRed`, …)   | **flat package** (Go has no nested types); P15 backstop |
| Java       | value class `Color` + class-scoped constants (`RED`, …)          | **nested** `Palette.Color`                              |
| TypeScript | none — the type is the inline union `"red" \| "green" \| "blue"` | —                                                       |
| Python     | none — the type is the inline `Literal[…]`                       | —                                                       |

Per **P15** every synthesized name enters the **same per-scope namespace** as
the declared names and as one another; the generator runs a single collision
pass (after case-mapping) and **rejects at load** with a fix-it diagnostic on
any coincidence. A **type-name** collision is resolved by the [[properties]]
`x-<lang>-name` override on the declaring member (it moves the member-derived
type). **Value-constant** collisions are resolved by the per-value override
(below). This is the class-body surface where **many** constants can case-map
together that [[const]] defers here: two members whose encodings fold to the
same identifier (`"user"` + `"USER"` → both `USER`; `"user-admin"` ⨯
`"user_admin"` → both `UserAdmin`) are a load reject, never auto-mangled (a
`USER2` suffix would be unstable across schema revisions, **P13**). Nesting
shrinks the surface (a nested `Palette.Color` cannot clash with a top-level
`Color`); **Go** stays flat and relies on the P15 backstop.

**The pass runs over the value set that is emitted** — the resolved node, after
an [[allOf]] merge, a `$ref` resolution or a [[nullability]] push-down — never
over each authored `allOf` branch. Encodability and fold collisions are
properties of the members a constant is actually emitted for, so
`allOf: [{enum:["user","USER","guest"]}, {enum:["user","guest"]}]` loads: the
merged set is `["user","guest"]`, which folds cleanly, and the `USER` member
that would have collided is not in it. Rejecting the branch would refuse a
schema whose emitted form has no collision at all. Shape checks that a merge
could silently discard still run per branch — see [[allOf]], which owns the
split. Same rule as [[const]]'s.

### Naming and encoding (value → identifier)

Identical to [[const]]: Go and Java name each value constant after the **value**
— Go `{Type}{EncodedValue}`, Java a class-scoped `{EncodedValue}` — encoded
through the [[properties]] Stage 1–4 pipeline, with a per-kind front-end and a
**constant recasing scope** (Go `PascalCase`, Java `UPPER_SNAKE`). The per-kind
token rules (word-splitting, digits, `Neg` for negatives, `.` → `_`, the Java
`V_` leading-letter prefix, ASCII-without-whitespace strings, many-to-one
readability) are exactly [[const]]'s — see [[const]] "Naming and encoding".
Because a whole array of values is encoded, the collision surface is larger,
which is why the class-body case above is exercised here.

### Overriding value constants

[[const]] overrides its single value constant with `x-<lang>-const-name` (a
string). `enum` has **many** values, so the override is a **map**:
**`x-<lang>-enum-names`** (`x-go-enum-names` / `x-java-enum-names`), an object
on the enum schema keyed by the **member value** and mapping to the override
identifier. Keys are the member's canonical JSON string — the string itself for
string members, the shortest round-trippable decimal for numbers,
`"true"`/`"false"` for booleans. Only members that need an override need appear;
unlisted members use the Stage 1–4 encoding. Like `x-<lang>-name` each override
is used **verbatim** (it skips Stage 1–4), must itself be a legal, non-reserved
identifier in that language, and participates in the P15 collision pass. Only
**Go** and **Java** synthesize value constants; **TS** and **Python** carry the
values in inline literals and have nothing to override, so the keyword is inert
there. This is the only way to admit a member that encodes to an empty or
illegal token (`"-"`) or to separate two members whose encodings fold together
(`"user"` + `"USER"`), consistent with the **no-auto-mangling** rule (P13/P15).
Example:

```jsonc
{
  "type": "string",
  "enum": ["user", "USER", "-"],
  "x-go-enum-names":   { "USER": "UserUpper", "-": "Dash" },
  "x-java-enum-names": { "USER": "USER_UPPER", "-": "DASH" }
}
```

## Validator mapping

Per **P10**/**P11**. A single **membership** check against the fixed set,
identical in both directions (a pure predicate over the decoded value — the
**shared `Validate`** layer of **P12**).

| Language   | Strategy                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ---------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | A predicate in the shared `Validate`, applied identically on both directions' paths (**P12.2**: sharing is a requirement on the predicate, not on the call graph): `switch v { case ColorRed, ColorGreen, ColorBlue: default: … Violation{Path, Reason: fmt.Sprintf("must be one of [%s], got %q", set, v)} }`, collected into one `PayloadValidationError` application failure. The field is the defined type; the typed constants are both the compared set and the idiomatic setters.                                               |
| TypeScript | the emitted validator tests membership against the literal set with an inline inequality chain, throwing one `PayloadValidationError` application failure. The field's union type closes it in-language.                                                                                                                                                                                                                                                                                                                               |
| Python     | the transfer type converter (PRINCIPLES Python §3) tests membership against an inline tuple and appends a `Violation` into the single generated `PayloadValidationError` application failure. The field is the closed `Literal` (`float` enums are plain `float`, validated the same way).                                                                                                                                                                                                                                             |
| Java       | the aggregating path is the per-POJO collecting deserializer (PRINCIPLES Java §5): a **non-throwing membership lookup** — known value → the constant, otherwise record a `Violation{path, "must be one of [...], got …"}` — so multiple bad fields collect into the single `PayloadValidationError` application failure. The value class's `@JsonCreator fromString` *throws* only on the **standalone/interop** path, where fail-fast is expected. Serialize needs no separate check: the value class can only hold a known constant. |

The reason string names the **expected set and the offending value**
(`must be one of ["red","green","blue"], got "purple"`), never a bare keyword.
Go renders the set with no space after the comma; TypeScript and Python render
`["red", "green", "blue"]` — a divergence in the set's rendering only, never in
the accepted value set.

**Co-authored assertions on the same node** follow [[const]]'s rule unchanged: a
co-authored string or numeric assertion is **still emitted** even though the
load gate has already validated every member against it — closedness changes the
*carrier*, so where a target gives the value set its own named type the
predicate is evaluated over that value's underlying primitive; and on a
materialized [[format]]/[[contentEncoding]] node both checks read one
canonical-wire projection per member.

### Serialize-side (P12)

There is **no enum-specific serialize logic**. `enum` rides the same encode path
as every other field: a set field is emitted, an unset optional field is
omitted. Presence is owned entirely by [[required]] — a required+enum is always
present (so always emitted), an optional+enum emits iff the consumer opted it
in. A value reaches the wire because each language guarantees it is
*set in memory*, never because the adapter rewrites it:

| declaration     | in-memory                                   | serialize                                                |
| --------------- | ------------------------------------------- | -------------------------------------------------------- |
| required + enum | always set (by type / `final` / consumer)   | emitted by the normal encode path                        |
| optional + enum | present iff the consumer opts the member in | emitted **if set**, omitted if unset (normal omit-unset) |

As with [[const]], there is **no auto-inject on absence** — so an absent
required enum is a [[required]] violation, and Python has no auto-fill step (for
enum the generator could not pick which member to fill in any case). The
serialize membership check has teeth wherever an out-of-set value can be set in
memory before emit: an optional+enum mutated to a wrong value, a Go
zero-value/mutated field (`Color("")` is not a member), or any Python in-memory
assignment (a dataclass validates nothing on construction, PRINCIPLES Python
§1). Where a closing carrier *is* emitted — the TypeScript literal union, and
the Java value class on an object property — the value cannot be out of set in
memory, so the check is effectively a deserialize-direction guard. Java
array-element and typed-map positions keep the primitive carrier (Type mapping),
so there the check has teeth in both directions and is emitted on both.

## Property-testing matrix

### Accepted (positive)

| Shape                                    | Example                                                                                                                                                      |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| String set                               | `{type:"string", enum:["red","green","blue"]}` → `ColorRed`/`ColorGreen`/`ColorBlue`                                                                         |
| Member needing encoding                  | `{type:"string", enum:["user-admin","guest"]}` → `UserAdmin`/`Guest`                                                                                         |
| Integer set                              | `{type:"integer", enum:[1,2,3]}`                                                                                                                             |
| Boolean set (degenerate but legal)       | `{type:"boolean", enum:[true,false]}`                                                                                                                        |
| Float set (exact `==`)                   | `{type:"number", enum:[1.5,2.5]}`                                                                                                                            |
| Integer-valued number members            | `{type:"number", enum:[1.0,2.0]}` → values `1`/`2`, names `Ratio1`/`Ratio2`, carrier still `number`                                                          |
| Merge that removes a folding member      | `{allOf:[{type:"string", enum:["user","USER","guest"]},{type:"string", enum:["user","guest"]}]}` → `["user","guest"]`; the fold check runs on the merged set |
| Single-element → const                   | `{type:"string", enum:["v1"]}` (normalized to [[const]])                                                                                                     |
| enum + default (member)                  | `{type:"string", enum:["a","b"], default:"a"}` (default is in the set)                                                                                       |
| Case-folding members rescued by override | `{type:"string", enum:["user","USER"], x-go-enum-names:{"USER":"UserUpper"}, x-java-enum-names:{"USER":"USER_UPPER"}}`                                       |

### Rejected at load time (negative)

| Reason                                                 | Example                                                                                                                                                                                                                                                                                                                                                    |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Empty array (unsatisfiable)                            | `{type:"string", enum:[]}`                                                                                                                                                                                                                                                                                                                                 |
| Duplicate members                                      | `{type:"string", enum:["a","a"]}`, `{type:"number", enum:[1, 1.0]}` (one value, two spellings)                                                                                                                                                                                                                                                             |
| Mixed member types (P7.1)                              | `{type:"string", enum:["a",1,true]}`                                                                                                                                                                                                                                                                                                                       |
| Member type-incompatible with `type` (P7.1)            | `{type:"integer", enum:[1,"x"]}`                                                                                                                                                                                                                                                                                                                           |
| Fractional member on `integer` (directional, [[type]]) | `{type:"integer", enum:[1,1.5]}`                                                                                                                                                                                                                                                                                                                           |
| `integer` member outside the `±(2^53−1)` cap           | `{type:"integer", enum:[1,9007199254740992]}`                                                                                                                                                                                                                                                                                                              |
| `enum` on a [[oneOf]] node                             | `{oneOf:[{type:"string"},{type:"integer"}], enum:["x"]}`, `{oneOf:[{type:"string"},{type:"null"}], enum:["x"]}` — author it on the branch                                                                                                                                                                                                                  |
| Disjoint closed sets over one position                 | `{type:"array", items:{type:"string", enum:["a","b"]}, contains:{type:"string", const:"z"}}`                                                                                                                                                                                                                                                               |
| A member fails a sibling constraint                    | `{type:"string", minLength:3, enum:["ok","no"]}`, `{type:"integer", minimum:0, enum:[1,-2]}`                                                                                                                                                                                                                                                               |
| With `const` (redundant)                               | `{type:"string", const:"a", enum:["a"]}`                                                                                                                                                                                                                                                                                                                   |
| `null` member (use nullability)                        | `{type:"string", enum:["a",null]}`                                                                                                                                                                                                                                                                                                                         |
| Composite member (deferred)                            | `{type:"object", enum:[{a:1}]}`, `{type:"array", enum:[[1]]}`                                                                                                                                                                                                                                                                                              |
| Non-ASCII / whitespace string member                   | `{type:"string", enum:["a b","café"]}`                                                                                                                                                                                                                                                                                                                     |
| Member un-encodable as an identifier                   | `{type:"string", enum:["-","+"]}` (empty tokens → Stage 3 reject; override: `x-<lang>-enum-names`)                                                                                                                                                                                                                                                         |
| Synthesized-name collision (P15)                       | Two members folding together (`"user"` + `"USER"` → both `USER`; `"user-admin"` ⨯ `"user_admin"` → both `UserAdmin`); a `$defs`-named enum reusing an existing type name; Go flat type/const ⨯ a declared top-level name. (Nesting removes the Java anonymous case; Go stays flat → still caught. TS/Python close the type inline and synthesize nothing.) |

### Runtime fixtures (validator)

- Wire value in the set → OK (both directions).
- Wire value present but **out of set** → one `PayloadValidationError`
  application failure naming the set and the actual value
  (`must be one of ["red","green","blue"], got "purple"`).
- required+enum **absent on the wire** → required violation (see [[required]]),
  reported as a presence error, not an enum error (there is no auto-fill, the
  same as [[const]]).
- Serialize of a correctly-set enum → that value on the wire (TS/Java cannot be
  out of set; Go set via a value constant).
- Serialize of a Go zero-value / bypassed required enum (`Color("")`, not a
  member) → rejected **loudly** by `Validate`, not silently rewritten.
- Serialize after mutating an optional+enum to an out-of-set value → rejected
  before emit (**P12**).
- Override-named members (`enum:["user","USER"]` + `x-go-enum-names`/
  `x-java-enum-names`): both values round-trip and validate; the overrides
  rename the constants (Go `UserUpper`, Java `USER_UPPER`), never the compared
  values. Confirms the override affects only synthesized identifiers, never the
  membership check.

## Interactions

- **[[const]]**: `const` ≡ a single-element `enum` (spec §6.1.3), and the two
  **share one representation** — a closed value set that rejects any
  unrecognized value (TS/Python literal, Go defined type + value consts, Java
  value class). `const` is the single-value specialization; a single-element
  `enum` normalizes to it. We reject the two together; pick one spelling.
- **[[type]]**: every member must be assignable to the declared type; mismatch
  or a mixed-type array is a load-time reject (**P7.1**). The emitted type is
  closed to the value set — a union of literals / defined type / value class
  over `type`'s primitive mapping (**P13.1**).
- **[[required]]**: owns presence entirely. required+enum is always set in
  memory (so always emitted); optional+enum is validated-if-present and
  emit-if-set. As with [[const]], no value is injected on absence, so an absent
  required enum is a required violation. `enum` adds no serialize behavior; it
  only asserts membership.
- **[[default]]**: **compatible** (unlike [[const]], which is mutually
  exclusive). A `default` supplies which member applies on absence and
  **MUST itself be a member of the set** (load reject otherwise); `default` is
  off the wire (omit-unset) and materialized on read.
- **[[nullability]]**: a `null` member is rejected. A nullable enum is the
  nullability `oneOf` with the `enum` authored **on the non-null branch**; the
  wrapper node must not carry it (Loader behavior). The two axes stay orthogonal
  there: the branch's value set is closed and the field still admits an explicit
  `null`, which the membership check never sees.
- **[[pattern]] / [[minLength]] / [[maxLength]] / [[minimum]] / [[maximum]] /
  [[exclusiveMinimum]] / [[exclusiveMaximum]] / [[multipleOf]]**: every member
  is validated against each sibling constraint at load; a violation on any
  member is a reject. Each keyword owns its half of the check.
- **[[propertyNames]]**: `enum` is reused as a **key** assertion when it appears
  under `propertyNames` — the map's keys must be one of the set, same closed
  machinery applied to keys instead of values.
- **[[oneOf]] / discriminated unions**: a per-branch discriminator is a
  single-value [[const]] (an equivalent single-value `enum` also qualifies); a
  multi-value `enum`-typed member narrows a value to a closed set but does not
  by itself select a branch. Where a single-value `enum` is the discriminator,
  the pairwise distinctness [[oneOf]] requires is decided by **value equality**
  — `[1]` and `[1.0]` are one value, so those branches are not distinct (see
  [[const]]). `enum` on the union node itself is a reject.
- **[[items]] / [[contains]]**: two closed value sets over the same value
  position must **intersect**. A closed-value element type and a closed-value
  [[contains]] matcher whose sets are disjoint — `items` closed to `["a","b"]`
  with a matcher requiring `"z"` — describe an array no value can satisfy, so
  the schema is unsatisfiable and rejects at load. Left to runtime it is a
  comparison between provably non-overlapping literal types, which the closed
  carriers make a type error rather than a failing check. Only **emptiness**
  rejects: a matcher that merely narrows the element's set is satisfiable and
  accepted — [[contains]] owns that case.
- **[[ref]]**: a `$defs`-named enum's synthesized type (Go defined type / Java
  value class) reuses the `$defs` name and enters the same per-package namespace
  (P15); recursion/anonymity follow the shared [[properties]] nesting rule.
  **Unimplemented** — a scalar `$defs` entry is a load reject today (see
  *Naming and collisions*).

## Ecosystem variance

| Source dialect        | Action                                                                                                          |
| --------------------- | --------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12   | Native (scalar, homogeneous).                                                                                   |
| OpenAPI 3.1           | Adopts 2020-12 — `enum` native.                                                                                 |
| OpenAPI 3.0           | `enum` native; also the idiom for `const` (single-element `enum`) — a one-element enum normalizes to [[const]]. |
| Swagger 2.0 / draft-4 | `enum` native (no `const` until draft-6); single-element `enum` → [[const]].                                    |

## See also

- [[const]] — the single-value sibling; `enum` is the same closed representation
  with more than one known value (both reject unrecognized values). Shares the
  naming/encoding pipeline; `enum` exercises the class-body multi-constant
  collision surface `const` defers here.
- [[properties]] — owns the identifier case-mapping + collision/escape-hatch
  policy that governs enum's synthesized type/value-const names (P15).
- [[type]] — supplies the emitted primitive type; gates member compatibility and
  homogeneity.
- [[required]] — owns presence; no auto-fill for an absent required enum.
- [[default]] — compatible; a default must be a member of the set.
- [[nullability]] — a `null` member is rejected; use the nullability pattern for
  a nullable enum.
- [[propertyNames]] — `enum` reused as a key-shape assertion.
- [[pattern]] / [[minLength]] / [[maxLength]] / [[minimum]] / [[maximum]] /
  [[exclusiveMinimum]] / [[exclusiveMaximum]] / [[multipleOf]] — sibling
  constraints each member is validated against at load.
