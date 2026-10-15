# `oneOf`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.2.1.3 "Keywords
for Applying Subschemas With Boolean Logic → oneOf".

The **tagged-union / sum-type** keyword: an instance is valid iff it validates
against **exactly one** branch. Among the boolean-logic applicators, `oneOf` and
[[allOf]] are the two the subset admits — but by different mechanisms: `allOf`
(intersection) collapses to a single materialized schema at load ([[allOf]]),
whereas `oneOf` is *retained* as a closed sum type. [[anyOf]], [[not]],
[[if-then-else]] stay **rejected** per **P6** (see [[dependentSchemas]] for the
same rationale). `oneOf` earns its place because "exactly one" is a *sum type*,
which lowers coherently to all four targets —
**but only when the branches carry a decidable selector**, so a deserializer can
route a wire value to exactly one branch without trial-validating every
subschema. Two selectors are supported, composed in layers: the
**JSON type token** separates branches of different kinds, and — among two or
more **object** branches — a shared required **`const`-tag** property separates
them by value. Only the OpenAPI `discriminator` *object* is deferred (as
optional sugar over the const tag; see Support decision).

## Spec summary

Verbatim (2020-12 core, Applicator, §10.2.1.3):

> This keyword's value MUST be a non-empty array. Each item of the array MUST be
> a valid JSON Schema.

> An instance validates successfully against this keyword if it validates
> successfully against exactly one schema defined by this keyword's value.

Distilled:
- The value is a non-empty array of subschemas — the **branches**. The instance
  is valid iff **exactly one** branch validates: zero matches fails, and —
  unlike [[anyOf]] — **two or more** matches also fails.
- "Exactly one" is what makes it a sum type rather than an intersection
  ([[allOf]]) or an inclusive-or ([[anyOf]]). It is the only combinator whose
  semantics correspond to a single, closed, discriminated choice — the shape
  every target can model.
- The keyword says nothing about *how* to tell the branches apart; validity is
  defined by brute-force validation against all branches. A faithful *typed*
  lowering needs more than validity — it needs a **decidable selector** (below),
  or the deserializer is reduced to guessing (**P7**).

## Support decision

**Support:** yes, for a `oneOf` whose branches are separable by a
**decidable selector**. Concretely, the generator accepts a `oneOf` iff:

- it has **≥ 2 branches** (a single-branch `oneOf` is a pointless wrapper —
  reject per **P7.1**, diagnostic points at using the branch directly);
- **every branch declares a single recognized [[type]]** (directly, or via a
  `$ref` to a named typed definition) — a branch with no `type`, a boolean/empty
  schema (`true`/`false`/`{}`), or one that is itself a combinator has no
  classifiable kind and is rejected;
- **grouped by JSON kind** across {`null`, `boolean`, `string`, `number`,
  `array`, `object`}: every non-`object` kind holds **at most one** branch
  (type-token separable), and the `object` kind, if it holds **two or more**
  branches, is separated by a shared **`const`-tag discriminator** (below);
- every **object** branch has a **determinate type name**: a `$ref` brings its
  definition's name, a lone inline object branch derives `<Union>Object`, and
  two or more inline object branches must name themselves with `x-<lang>-name`
  (below).

Selection is layered. The outer selector is the wire token (`{`→object,
`[`→array, `"`→string, number→the numeric branch, `true`/`false`→boolean,
`null`→null). When the token is `{` and there are multiple object branches, the
inner selector reads the **discriminator property's value** and maps it to the
branch bearing that `const`. Because the branches are proven disjoint at load
(distinct kinds, and distinct `const` values within the object kind),
**at most one** branch can match, so "exactly one" decomposes into two decidable
checks:

- **≤ 1 (load time):** disjoint kinds + pairwise-distinct discriminator `const`s
  → no instance can satisfy two branches. Proven once, statically.
- **≥ 1 (runtime):** the token (then, for objects, the discriminator value)
  selects a branch and the value must validate against it; a token or
  discriminator value matching no branch is zero matches → a `Violation`.

This is why mixed-kind unions (`object | array`, `object | string`,
`string | integer`) and object tagged unions (`Cat | Dog`) are all supported.
The [[nullability]] pattern `oneOf:[{T},{null}]` is the
**degenerate two-kind instance** of the type-token layer (the `null` token is
the selector); it remains owned by [[nullability]].

### Object branches — naming the inline shape

An object branch is admitted whatever its shape (declared [[properties]], a
typed map, a free-form object, member-count bounds). The constraint is not the
shape, it is the **name**: every target has to materialize a *type* for a
**structured** object branch — Go a defined type to carry the marker method,
Java a class to `implement` the interface, TS an interface plus the converter
that validates its members, Python a dataclass plus the converter that validates
them — and a type needs a name. So every object branch must resolve to a
determinate name:

- **`$ref` to a named definition** — the definition's name *is* the branch type,
  already emitted with its own validation. The recommended form for any shape
  that is worth naming or is reused.
- **one inline object branch** — synthesizes `<Union>Object`, by the same rule
  the other inline branch kinds use (`<Union><Kind>`, [[properties]]
  §"Synthesized type names"). The name is determinate because the type-token
  layer admits a single object branch unless the branches are `const`-tagged.
  **P15** remains the backstop if `<Union>Object` collides with another emitted
  type: reject, with `x-<lang>-name` as the escape hatch — the same treatment
  every other synthesized name gets.
- **two or more inline object branches** (a `const`-tagged union written inline)
  — every branch would derive the same `Object` name, and there is nothing in a
  JSON Schema branch to derive a *distinguishing* name from: the discriminator's
  `const` value is a wire value, not an identifier, and ordinal names
  (`Object1`, `Object2`) reorder silently when a branch is inserted. So each
  branch must name itself with the Stage 4 override ([[properties]]) —
  `x-go-name` / `x-ts-name` / `x-py-name` / `x-java-name`, the key of whichever
  target is being generated — and is rejected naming the missing key otherwise.

A named branch is emitted as
**the same named model an authored definition of that shape would produce**: the
load moves the branch into `$defs` under its resolved name and rewrites the
branch to a `$ref` at it, so there is one object-model emitter per target rather
than a second, inline one. Everything downstream follows from that — the branch
validates, resolves, collides (**P15**), and joins the module's exported surface
exactly like an authored definition. It also means the inline form and the
`$ref` form emit *identical* code; the choice between them is only where the
shape reads best.

Note what the multi-branch case costs: an override key per branch is more typing
than a `$defs` entry per branch, which also gives every target a better name
than a union-derived one. Inline multi-object branches are admitted for
completeness, not recommended; the diagnostic on a missing override points at
both remedies.

The **free-form object** (`type: object` with `additionalProperties: true` and
no declared `properties`) is the exception that needs no name at all
*as a branch*: it declares no shape to emit, so TS and Python express it
structurally inside the value union (`Record<string, unknown>` /
`dict[str, Any]`) and only Go and Java wrap it — as `<Union>Object` over the
verbatim member map ([[additionalProperties]], **P13**). It is left inline
rather than hoisted. This is specific to the branch position: written in a value
position of its own — a property, an element, a map member — a free-form object
*is* named and hoisted like any other object ([[properties]] §"Naming an inline
object shape").

### Discriminated object unions — the `const`-tag

Two or more **object** branches are separated by a shared discriminator
property. The generator accepts them iff there is **exactly one** property name
that, in **every** object branch:

- is listed in that branch's [[required]] array (so it is always on the wire to
  select on), and
- has a scalar **`const`** (a single-value [[enum]] is equivalent), with the
  `const` **values pairwise-distinct** across the branches.

**Distinctness is by value, not by spelling.** The comparison is [[const]]'s
value equality — the same equality the emitted dispatcher applies to the wire
token — so numbers compare mathematically: `const: 1` and `const: 1.0` are
**one** discriminator value and two branches carrying them are *not* distinct.
Comparing JSON representations instead would admit a union whose second branch
no ordinary producer can ever select, because the runtime comparator cannot tell
the two apart; the payload would decode silently as the first branch. Such a
pair is the "no discriminator" reject below.

That property is the **discriminator**; its `const` value is the inner selector.
This is the pure-JSON-Schema tagged-union idiom — no OpenAPI `discriminator`
object required — and it is **validation-bearing**: the `const` is already
enforced both directions ([[const]], **P12**), and the discriminator is a
**closed value set** (**P13.1**), so an unknown discriminator value is
*rejected*, not preserved. Keying selection on the `const` value (not the `$ref`
name) keeps it stable under type renaming.

- **Zero** shared required-`const` properties across the object branches →
  reject: no discriminator (diagnostic shows the const-tag form).
- **More than one** qualifying property → reject as **ambiguous** (**P7.1**);
  the deferred OpenAPI `discriminator` object is how one would later name the
  intended tag explicitly.
- The discriminator property is otherwise an ordinary member of each branch
  (emitted as the [[const]] literal); the rest of each branch is a normal open
  object (**P13**) — extra keys are still preserved *within* the selected
  branch. The tag closes selection *across* branches; it does not close the
  branches themselves.

Grounding ([[PRINCIPLES.md]]): **P1** — a sum type with a decidable selector
round-trips identically across all four targets; **P6** — the selector
requirement is the strict-subset line that admits `oneOf` as a retained union
while still excluding the non-collapsing applicators (`anyOf`/`not`/`if`);
**P7 / P7.1** — a selector-less union (overlapping branches) would force
trial-validation guessing, which we reject loudly rather than approximate;
**P2** — the type-token rule is exactly the set of unions TypeScript can narrow
with native `typeof`/`Array.isArray` and Go/Java can decode by a single token
peek, so it reads like hand-written code in every language.

### Why a selector is required, not optional

The subset's objects are **open by default** ([[additionalProperties]],
**P13**): unknown keys are preserved, not rejected. Structural discrimination
("does this payload match branch A's shape?") is therefore unsound for object
branches — an extra key never disqualifies a match, so two object branches can
both plausibly accept the same payload. This is exactly why an explicit
**`const`-tag** is required to separate object branches: it is a closed value
the wire must carry, immune to extra keys. For *disjoint-kind* branches the JSON
token itself is the tag — self-describing, forward-compatible, and free.

### Null branches — nullable unions

`null` is one of the disjoint kinds, so a `null` branch is admitted like any
other. It doesn't add a sum-type *member*; it marks the whole field
**nullable**, and the remaining non-null branches form the value type:

- **exactly one non-null branch** (`[{T},{null}]`) → a plain nullable field,
  **owned by [[nullability]]** (its canonical two-branch pattern);
- **two or more non-null branches** (`[{object},{array},{null}]`) → a
  **nullable union**: the non-null branches form the sum type (and must
  themselves be pairwise-disjoint), and the field is nullable.

The nullable union needs no new machinery — every target already has a nullable
channel for the union type: a Go interface is nilable (`nil` = `null`), Python
adds `| None`, TS adds `| null`, and a Java reference is `@Nullable`. The `null`
token selects "no value" exactly as the value tokens select their branches;
decode/encode of the null state follow the [[nullability]] tables over the union
type (including the optional-vs-null collapse in Go/Java/Python and the faithful
round-trip in TS). Required-vs-optional and the nullable state remain orthogonal
(**P8**), so all four presence/null combinations apply to a union just as to a
scalar.

### Unions in element positions

A union is not always a definition or a property. It can also be the
**element type** of a collection — an array's [[items]], a map's typed
[[additionalProperties]] — and there it is admitted on the same terms, with one
addition: it needs a name, for the same reason a structured object branch does.
Go emits a union as a sealed interface plus a dispatcher, Java as an interface
with a static `fromNode`; a `[]T` / `List<T>` over that interface has to name
`T`.

So a `oneOf` **sum type** in an element position is *named after its position*,
moved into `$defs`, and the position rewritten to a `$ref` at it:

| Position                                           | Synthesized name    |
| -------------------------------------------------- | ------------------- |
| `items` of the property `values` on `Bag`          | `BagValuesItem`     |
| `items` of `items` (nested array)                  | `BagValuesItemItem` |
| `additionalProperties` of the definition `Entries` | `EntriesValue`      |
| `items` of a union's array branch                  | `<Union>ArrayItem`  |

The branch's own `x-<lang>-name` overrides the derived name, and **P15** rejects
a collision rather than mangling — the rules every synthesized name follows.
Because the hoist runs to a fixpoint, an inline *object branch* of a hoisted
element union is named in turn (`BagValuesItemObject`), so the element position
needs no `$defs` boilerplate from the author for any shape. An inline *object*
in an element position is named by the same rule and the same table
([[properties]] §"Naming an inline object shape") — the positions and their
names are shared; only what occupies them differs.

Two consequences worth stating:

- **The naming is uniform across targets.** An anonymous union on a *property*
  stays inline in TS/Python (only Go/Java synthesize `<Enclosing><Property>`),
  but an element union is a `$defs` definition before any backend sees it, so
  all four emit the same named type. That is the point: one hoist replaces four
  element-position emitters.
- **The nullability `oneOf` is not hoisted.** `items: {oneOf: [{T},{null}]}` has
  one non-null branch, so it is [[nullability]]'s degenerate pattern, not a sum
  type: it declares nothing to name and every target expresses it on the element
  itself (`[]*T`, `(T | null)[]`, `list[T | None]`, `List<@Nullable T>` — see
  [[items]]).

Element decoding is elementwise by necessity: a whole-collection decode
(`json.Unmarshal` into `[]T`, `readTreeAsValue(node, T.class)`) cannot allocate
a sealed interface. Each element/member is routed through the union's own
dispatcher, and its index or key is threaded into the violation path
(`shapes[1]`, `choices.primary`) per **P11**.

The reverse nesting is recursive too: when an
**array is itself a union branch**, the selected branch uses the ordinary
[[items]] parser and mapper in both directions. It does not pass the array
through verbatim. Element models, temporals, and binary values therefore
materialize normally, every element's constraints run, and failures aggregate at
indexed paths under the union.

### Deferred (reject with a "not yet supported" diagnostic)

- **The OpenAPI `discriminator: {propertyName, mapping}` object** — the explicit
  form that names the tag property and maps values to schemas. It is deferred as
  *optional sugar* over the const-tag form: when accepted, it must be
  **consistent with** the branch `const`s (each mapped value's branch must
  already carry that `const`), so the `const` stays the single
  validation-bearing source of truth and we don't inherit the discriminator
  object's `$ref`-name brittleness. Until then it is rejected with a diagnostic
  pointing at the const-tag form, which needs no OpenAPI extension.

- **A materializing keyword on a non-object branch of a sum type** — a temporal
  [[format]] (`date-time`/`date`/`time`/`duration`) or a [[contentEncoding]].
  Both replace the wire `string` with a native typed value (`time.Time` /
  `OffsetDateTime` / `datetime` / `Temporal.*`; `[]byte` / `byte[]` / `bytes`),
  and the synthesized `<Union><Kind>` wrapper has no such type: the branch would
  materialize in Python while Go, TypeScript, and Java carried an unvalidated
  `string`, which is exactly the silent per-target divergence **P1** forbids. So
  it is rejected with a located diagnostic naming the keyword and the two
  remedies — drop the keyword to keep the branch a plain (still fully validated)
  `string`, or carry the value as a *property* of an object branch, where
  materialization already works. Non-materializing string formats (`uuid`,
  `email`, `hostname`, `uri`, `ipv4`, `ipv6`) are **not** deferred: they assert
  and keep the `string`, so they ride along like any other branch constraint.
  This is scoped to the sum type — the [[nullability]] pattern
  `oneOf:[{T},{null}]` has a single non-null branch and synthesizes no wrapper,
  so a materialized nullable field is unaffected.

### Rejected outright (incoherent, not merely unsupported)

- **`integer | number`** — both are the JSON number token *and*
  `integer ⊂ number`, so any integer satisfies both branches: `oneOf`'s
  exactly-one is **unsatisfiable**. Reject per **P7.1** (not deferred — no
  discriminator can fix an overlap). Distinct numeric-vs-non-numeric pairs
  (`integer | string`, `number | boolean`) are fine.
- **Empty `oneOf: []`** — the value MUST be non-empty; reject as an invalid
  schema.

Loader behavior:
- `oneOf` value not a non-empty array of valid subschemas → reject (recurse into
  each branch).
- Single-branch `oneOf` → reject (**P7.1**, pointless wrapper).
- Any branch without a single recognized `type` (missing `type`,
  `true`/`false`/`{}`, a nested combinator) → reject: no classifiable kind.
- Two or more **object** branches → require the `const`-tag discriminator
  (exactly one shared required-`const` property whose values are pairwise
  distinct *by value*); zero such properties or more than one qualifying →
  reject (no discriminator / ambiguous).
- An **inline** structured object branch (declared `properties`, or a typed
  `additionalProperties`) → resolve its name (below), move it into `$defs` under
  that name, and rewrite the branch to a `$ref` at it. From that point it is an
  ordinary model. The free-form object is left inline.
- Two or more **inline** structured object branches without a per-branch
  `x-<lang>-name` for the target being generated → reject: the synthesized
  `<Union>Object` name is not determinate. The diagnostic names the missing key
  and points at `$defs` + `$ref` as the shorter remedy.
- A synthesized branch name already declared in `$defs`, or colliding with
  another emitted type → reject per **P15**, `x-<lang>-name` as the escape hatch
  (as for every synthesized name).
- **What a `oneOf` node may carry, in one rule.** A node that carries `oneOf` is
  a union: it is not simultaneously a leaf, an object, or a closed value set. It
  admits the annotations ([[title]], [[description]], [[comment]], [[examples]],
  [[deprecated]]), an `x-<lang>-name` override, and `default` — which
  **rejects** on a sum type, because the literal does not identify which branch
  to construct, and is **where a nullable member's `default` belongs** on the
  two-branch [[nullability]] wrapper (see [[nullability]] and [[default]]).
  **Every other keyword is a load reject**, whichever of the two shapes the node
  is: `type`, [[properties]], [[items]], [[additionalProperties]],
  [[const]]/[[enum]], and every count or string bound. The fix-it names the
  branch or branches the keyword constrains and says to move it there — a closed
  value set over a sum type is spelled by putting the `const`/`enum` in each
  branch, and on a nullability wrapper by putting it on the **non-null** branch,
  which is the spelling every target agrees on. Accepting such a sibling and
  dropping it is exactly the silent discard **P7.1** exists to prevent, and
  because [[type]]'s leaf-`type` and object-without-shape checks key on the
  node's `type`, a discarded sibling can also switch an unrelated guard off. The
  rule holds at every position a union can occupy — a property, a `$defs` entry,
  an `items`, a typed `additionalProperties`, an operation `input`/`output`.

  **And it is spelling-independent.** A node whose `$ref` resolves to a named
  union is adjudicated by the same rule: an annotation sibling attaches to the
  reference and is not a conjunct ([[ref]]), a `default` sibling gets the
  sum-type `default` reject, and any other sibling gets *this* reject — naming
  the union, the sibling, and the branch to move it into, never an [[allOf]]
  merge diagnostic over a keyword the author did not write (**P7.2**). The
  inline and reference spellings of one union accept and reject the same set of
  siblings.
- A `oneOf` **sum type** written inline in an element position (an array's
  `items` at any depth, an object's typed `additionalProperties`) → name it
  after its position (below), move it into `$defs`, and rewrite the position to
  a `$ref` at it, exactly as for an inline object branch. Its own inline object
  branches are then named in turn.
- An **inline** object branch in a position with no derivable name (a nested
  inline object's property, whose enclosing shape is itself not materialized —
  see [[properties]]) → reject unless it is the free-form object, which needs
  none; the diagnostic points at `$defs` + `$ref`.
- Two or more branches of the same **non-object** kind (two strings, two
  integers) → reject: a scalar same-kind choice is an [[enum]] (or `const`
  union), not a `oneOf` — diagnostic points at [[enum]].
- `integer` and `number` branches together → reject (unsatisfiable overlap).
- A `null` branch with exactly one other branch → hand off to [[nullability]]
  (its canonical two-branch pattern); with two or more others → accept as a
  **nullable union** (null marks the field nullable; the non-null branches form
  the sum type).
- Synthesized union type name collides after case-mapping → reject per **P15**
  (`x-<lang>-name` on the declaring member/definition is the escape hatch),
  exactly as [[const]]/[[enum]] synthesized types.

## Type mapping

A supported `oneOf` emits a **closed sum type** — a value that holds exactly one
of the branch types, structurally (the in-memory representation cannot hold two,
so the "exactly one" invariant is carried by the type itself; the runtime check
is only *which* branch on decode).

The union type is **named** by the same rule [[const]]/[[enum]] synthesized
types use ([[properties]] §"Synthesized type names"): a named `$defs` union
reuses the def name; an **anonymous** inline union on a property is named
`<EnclosingType><Property>` (Go flat / Java nested), while TS and Python inline
the union with no synthesized name. A union in an **element position** is named
and hoisted at load (§"Unions in element positions"), so by the time a backend
sees it, it *is* a named `$defs` union in every language.

Running example (named union in `$defs`):

```yaml
$defs:
  Widget: { type: object, properties: { size: { type: integer } } }
  Foo:
    oneOf:
      - { $ref: '#/$defs/Widget' }        # object branch
      - { type: string }                  # string branch
      - { type: array, items: { type: number } }   # array branch
```

### TypeScript

A bare union — inline on the field, or a `type` alias for a `$def`. No
synthesized name; narrowing is native (**PRINCIPLES TS §2** — structural types,
no runtime footprint):

```ts
export type Foo = Widget | string | number[];

function handle(f: Foo) {
  if (typeof f === "string") {        // f: string
  } else if (Array.isArray(f)) {      // f: number[]
  } else {                            // f: Widget
  }
}
```

The type-token selector maps 1:1 onto TS's built-in narrowing primitives
(`typeof`, `Array.isArray`, and — for object-vs-object — a discriminant literal
property). This is the best-fit target: the acceptance rule is *exactly* what TS
narrows without hand-written type guards (**P2**).

An inline object branch is the one place TS does need a name: the branch's
members have to be validated, and that validation lives in a
`TransferTypeConverter` keyed to a type. A **structured** inline branch is
therefore emitted as the interface + converter pair a named definition gets
(`<Union>Object`, or the branch's `x-ts-name`) and enters the union under that
name; the union still narrows structurally, on the object token or the
discriminant literal. Only the **free-form** branch stays anonymous — it has no
members to validate:

```ts
export interface FooObject { kind: "a"; value: string; additionalProperties: Record<string, unknown>; }
export type Foo = FooObject | string;                      // structured inline branch
export type Bar = Record<string, unknown> | string;        // free-form inline branch
```

A property-level (anonymous) union whose members need a transform gets one
module-private `serialize<Union>` function — the same dispatch a named union's
`toTransferType` performs — so an object member is written through its branch
converter rather than emitted with its in-memory `additionalProperties` member
intact.

### Python

A PEP 604 union, inline on the field; a named `$def` becomes a `TypeAlias`:

```python
Foo: typing.TypeAlias = Widget | str | list[float]     # TypeAlias for the $def
```

This is structurally the TypeScript design — a plain type plus an off-type
converter that owns both directions — with one mechanical difference: a
`TypeAlias` cannot carry a decorator and `type[A | B]` is not a valid
annotation, so a union gets **no** `_<Model>TransferTypeConverter` class
(**PRINCIPLES Python §3**). Its conversion is emitted as a pair of
module-private free functions instead:

```python
def _foo_from_transfer_type(
    value: typing.Any, path: str, violations: list[Violation]
) -> Foo | None: ...

def _foo_to_transfer_type(value: Foo) -> typing.Any: ...
```

The parse function classifies the wire token, delegates to the selected branch,
and appends any `Violation` to the caller's list — returning `None` on failure
so its siblings are still checked (**P11**); the serialize function dispatches
on the in-memory value's Python type. An inline property-level union gets the
same pair, named after its position (`_<model>_<property>_from_transfer_type` /
`_..._to_transfer_type`) — the analogue of TS's module-private
`serialize<Model><Property>`.

One consequence worth stating: a union carries no *registered* converter, so it
cannot be a top-level Nexus operation input/output type. That costs nothing —
the loader already requires operation I/O to be an object type and rejects a
`oneOf` there ([[services]]) — so a union only ever appears nested, where the
declaring model's converter calls these functions.

For an object tagged union the `const` tag stays a `typing.Literal` member of
each branch ([[const]]) and the parse function switches on the discriminant
value read out of the raw dict (see "Discriminated object unions" below).

Python inlines the union but **not** a structured object branch's shape: the
branch's members have to be validated, and that validation lives in a converter
keyed to a type, so such a branch becomes a module-level dataclass named by the
rule above (`<Union>Object`, or the branch's `x-py-name`) and enters the union
under that name. The free-form object is the exception — `dict[str, typing.Any]`
has no declared members to validate and needs no class.

```python
@_transfer_type_convertible(_FooObjectTransferTypeConverter)
@dataclasses.dataclass(slots=True, kw_only=True)
class FooObject: ...                      # the inline object branch

Foo: typing.TypeAlias = FooObject | str
```

### Go

A **sealed interface** with an unexported marker method (**Option B**); the
field is the bare interface, type-switched directly (no wrapper, no accessor):

```go
type Foo interface {
    isFoo()
    Validate() error
}

func (Widget) isFoo() {}          // a $ref/named branch implements the marker directly
type FooString string             // a non-nameable branch gets a synthesized variant type…
func (FooString) isFoo() {}
type FooArray []float64           // …named <Union><Kind>
func (FooArray) isFoo() {}

type Container struct {
    Value Foo `json:"value"`
}

switch v := c.Value.(type) {      // direct type switch on the field
case Widget:     use(v)
case FooString:  use(string(v))
case FooArray:   use([]float64(v))
}
```

The marker method is unexported, so
**only generator-emitted types implement the interface** — the set is closed by
construction. Any branch that is (or `$ref`s) a named Go type implements the
marker directly; an inline scalar/array/object branch synthesizes a variant type
named `<Union><Kind>` (Go has no nested types → flat, P15-backstopped, or the
branch's `x-go-name` when it carries one). Because at most one branch occupies
each kind, `<Union><Kind>` is unambiguous.

An inline **object** branch synthesizes `<Union>Object` as a **struct** — the
same struct a named definition of that shape gets, fields and validation
included — rather than a defined type over a map, because the interface requires
the standard `Validate`/`UnmarshalJSON`/`MarshalJSON` surface:

```go
type FooObject struct {                      // a structured inline branch
    Kind  FooObjectKind `json:"kind"`
    Value string        `json:"value"`
}
func (FooObject) isFoo() {}

type BarObject struct {                      // a free-form inline branch
    AdditionalProperties map[string]json.RawMessage
}
func (BarObject) isBar() {}
```

Both the interface type and each synthesized `<Union><Kind>` variant are
exported, so both carry a name-led doc comment (PRINCIPLES.md, Go §1): the
interface from the union schema's [[title]]/[[description]] when present, else a
fallback listing its admissible branch kinds
(`// Foo is one of: string, []float64, Widget.`); a synthesized variant from its
own branch schema's [[title]]/[[description]], else a fallback naming the union
and wrapped kind it belongs to. The unexported marker method needs none.

### Java

Java 8 baseline (**PRINCIPLES Java §1**) has no sealed interfaces, so the union
is a **plain interface, sealed by convention** — the `fromNode` dispatcher it
carries only ever constructs the known variants:

```java
public interface Foo {
    // The collecting dispatcher (§5): reads one JsonNode into a variant, or
    // records a Violation and returns null.
    static @Nullable Foo fromNode(JsonNode node, String path, List<Violation> violations, DeserializationContext context) { … }

    // Widget implements Foo (object branch — the POJO gains `implements Foo`)
    public static final class FooString implements Foo { private final String value; /* ctor, @JsonValue getter */ }
    public static final class FooArray  implements Foo { private final List<Double> value; /* … */ }
}
```

Java has no union type: an object `$ref` branch just gains `implements Foo` on
its existing POJO, while a **scalar/array branch must be wrapped** in a variant
class (`String` can't implement an interface). This wrapping is the cost only of
the *tagless token-based* form (mixed kinds); a pure object tagged union of
`$ref` branches has no wrappers — every branch is already a POJO. The verbosity
is hidden behind the POJO style (**P2**).

The wrapper's `getValue()` carries Jackson's `@JsonValue`, so a wrapper writes
back as the value it holds rather than as a bean around it; that makes the
serialize side a single runtime-class dispatch for every branch kind (an object
branch writes through its POJO's serializer, a wrapper through `@JsonValue`).

An inline object branch has no POJO to gain `implements`, so one is emitted for
it, named by the rule above (`<Union>Object`, or the branch's `x-java-name`). A
**structured** branch becomes a top-level POJO like any definition, taking part
in the interface's `fromNode` dispatch. A **free-form** branch instead becomes a
wrapper class over the catch-all member type an open POJO uses
([[additionalProperties]]), so its members round-trip verbatim:

```java
public final class FooObject implements Foo { /* declared members + collecting (de)serializer */ }
public static final class BarObject implements Bar { private final Map<String, JsonNode> value; /* … */ }
```

A **property-level** union is the same interface — `fromNode`, wrappers and all
— declared *nested in the enclosing POJO* (`Showcase.Detail`) rather than in its
own file, and the enclosing deserializer reads the member with the identical
`Detail.fromNode(…)` call a named union def gets. A structured branch of such a
union is still a top-level POJO; it names the nested interface through its
declaring class:

```java
public final class ShowcaseDetailObject implements Showcase.Detail { /* … */ }
```

### Nullable unions

A `null` branch adds no member type; it makes the union field nullable using
each target's existing nullable channel ([[nullability]]), applied to the union
type rather than a scalar:

|            | value type          | nullable form                                       |
| ---------- | ------------------- | --------------------------------------------------- |
| Go         | `Foo` (interface)   | already nilable — `nil` = `null`; no `*Foo` wrapper |
| TypeScript | `Foo`               | `Foo \| null` (optional adds `?`)                   |
| Python     | `Foo` (`TypeAlias`) | `Foo \| None`                                       |
| Java       | `Foo`               | `@Nullable Foo`                                     |

The presence/null state machine (required+nullable emits `null`, optional
collapses in Go/Java/Python, faithful in TS) is exactly the [[nullability]]
serialize/round-trip tables, unchanged — the union type simply takes the place
of the scalar.

### Discriminated object unions

Two or more object branches separated by a `const`-tag emit the same closed sum
type; the tag is an ordinary member of each branch, emitted as the [[const]]
literal, and drives selection.

```yaml
$defs:
  Cat: { type: object, required: [kind], properties: { kind: {type: string, const: cat}, meow: {type: string} } }
  Dog: { type: object, required: [kind], properties: { kind: {type: string, const: dog}, bark: {type: string} } }
  Animal: { oneOf: [ {$ref: '#/$defs/Cat'}, {$ref: '#/$defs/Dog'} ] }
```

- **TypeScript** — a discriminated union; the tag is a literal, narrowed by
  `switch`:
  ```ts
  interface Cat { kind: "cat"; meow: string; }
  interface Dog { kind: "dog"; bark: string; }
  export type Animal = Cat | Dog;
  switch (a.kind) { case "cat": /* Cat */ break; case "dog": /* Dog */ break; }
  ```
- **Python** — dataclass branches plus the union's parse function switching on
  the tag:
  ```python
  @dataclasses.dataclass(slots=True, kw_only=True)
  class Cat: kind: typing.Literal["cat"] = "cat"; meow: str
  @dataclasses.dataclass(slots=True, kw_only=True)
  class Dog: kind: typing.Literal["dog"] = "dog"; bark: str
  Animal: typing.TypeAlias = Cat | Dog
  # _animal_from_transfer_type: raw["kind"] == "cat" → Cat's converter ;
  #   "dog" → Dog's ; else a Violation naming the admissible values
  ```
- **Go** — the sealed interface; the container's `UnmarshalJSON` peeks the
  discriminator on an object token, then unmarshals into the concrete struct:
  ```go
  type Animal interface{ isAnimal() }
  func (Cat) isAnimal() {}
  func (Dog) isAnimal() {}
  // decode: raw["kind"] == "cat" → *Cat ; "dog" → *Dog ; else Violation
  ```
- **Java** — every branch is already a POJO implementing the interface; the
  collecting deserializer (**PRINCIPLES Java §5**) peeks the discriminator
  `JsonNode` and dispatches to the matching POJO's collecting deserializer
  (keeping P11 aggregation — the reason we peek rather than lean on Jackson's
  fail-fast `@JsonTypeInfo`, though the const tag is exactly what an
  `@JsonTypeInfo(use=NAME, include=EXISTING_PROPERTY)` form would key on).

## Validator mapping

Per **P10**/**P11**/**P12**. "Exactly one" is decomposed as above: the
disjointness invariant (distinct kinds + distinct discriminator `const`s) is
proven at **load** (so ≤ 1 is structural and needs no runtime work), and the
boundary enforces **≥ 1** by routing the wire token — then, for an object token
with a multi-branch object kind, the **discriminator value** — to its branch and
validating the value against that branch's own shared predicates. A token *or*
discriminator value matching no branch → one `Violation`. The discriminator is a
**closed value set** (**P13.1**): an unknown discriminator value is rejected,
not preserved (unlike a branch's own unknown *members*, which stay open per
**P13**). This is **selection then delegate**, never a trial-all-branches loop.

| Language   | Strategy                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | The container's collecting `UnmarshalJSON` reads the field from the raw `map[string]json.RawMessage`, peeks its first non-space token, and routes to the branch of that kind (`{`→object; `[`→array: `FooArray`; `"`→string: `FooString`; number→the numeric branch via `parseSpecInteger`/spec-number so `1.5` still yields a `Violation`). For an object token with 2+ object branches it further reads the discriminator property and selects the branch with that `const`. It then runs that branch's shared `Validate` and assigns the concrete type to the interface field. No matching kind / unknown discriminator value → `Violation` collected into the single `PayloadValidationError` application failure.                                                                                                                                                                                                                    |
| TypeScript | `fromTransferType` is the `typeof`/`Array.isArray` chain shown above; for an object it switches on the discriminant literal (`raw.kind`) and delegates to that branch's converter (e.g. `catTransferTypeConverter.fromTransferType`); the fall-through pushes one `Violation`. Plain checks only (**PRINCIPLES TS §1** — no runtime schema lib).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Python     | The union's module-private `_<union>_from_transfer_type(value, path, violations)`, called by the declaring model's `_<Model>TransferTypeConverter` (**PRINCIPLES Python §3**), classifies the raw value with `isinstance` — `dict`→object, `list`→array, `str`→string, a non-`bool` `int`/`float`→the numeric branch via `_parse_spec_integer`/the spec-number rule so `1.5` still yields a `Violation`, `bool`→boolean, `None`→null. For an object token with 2+ object branches it reads the discriminator key out of the raw dict and delegates to that branch's converter, re-pathing its `PayloadValidationError` application failure under the current path with `_collect`. No decidable branch / unknown discriminator value → a `Violation` appended and `None` returned, so sibling members are still checked and the model's converter raises the one `PayloadValidationError` application failure (**PRINCIPLES Python §2**). |
| Java       | The union interface's static `fromNode` (called by the enclosing POJO's collecting deserializer, **PRINCIPLES Java §5**) switches on the `JsonNode` kind (`isObject`/`isArray`/`isTextual`/`isNumber`/`isBoolean`); for an object with 2+ object branches it peeks the discriminator node and dispatches to the matching POJO's collecting deserializer. On no match / unknown discriminator it pushes a `Violation` into the single `PayloadValidationError` application failure and returns `null`. One dispatcher serves both positions: a named union def and a union written inline on a property.                                                                                                                                                                                                                                                                                                                                   |

Reason strings name **what was expected**, never a bare `oneOf`, per the
informative-reason convention the constraint families use: no decidable branch →
`expected one of: <labels>` over the admissible kinds / branch types; an object
whose discriminator value matches no branch →
`unknown discriminator <field> <value>: expected one of [...]` over the
admissible tag values. Both strings are identical in all four targets.

### Branch constraints

Selection is only half of it. A branch is an ordinary schema, so once the
selector routes a value to one, that value is held to
**everything that branch declares** — and to nothing the other branches declare.
An **object** branch gets this for free: it is a named model ([[properties]]),
so it validates through its own model's checks. A **non-object** branch has no
model, so the type each target synthesizes for it carries the branch's
predicates instead — the same emitters, with the same reasons, a *property* of
that type would use (`minLength`/`maxLength` /`pattern`/`format`, the numeric
bounds and `multipleOf`, `minItems`/`maxItems`/ `uniqueItems`/`contains`, a
`const`/`enum` value set), under the union's own violation path (`idOrName`,
`shapes[1]`, `choices.primary`):

| Language   | Where a non-object branch's constraints live                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | the synthesized `<Union><Kind>` wrapper's `Validate`, over a conversion back to the underlying type (`string(v)`, `[]float64(v)`). The dispatcher calls it on the selected branch, and the declaring model's `Validate` — which `MarshalJSON` runs first — calls it again before emit. A branch `pattern`/`format` compiles to a package-level regex var keyed by the wrapper type (`fooStringPattern`).                                                                                                                    |
| TypeScript | the narrowing chain itself: each `typeof`/`Array.isArray` arm runs the branch's checks over the narrowed value, in `fromTransferType` and again on the serialize side (a named union in its own converter's `toTransferType`, an inline one in the declaring model's, so a branch violation aggregates with its siblings).                                                                                                                                                                                                  |
| Python     | the classification arm itself, exactly as in TypeScript: each `isinstance` arm runs the branch's checks over the classified value — the numeric bounds, length bounds, `multipleOf`, and the `pattern`/`format` regex match all inline; only `uniqueItems` and `contains` go through a runtime helper (`_check_unique_items` / `_check_contains`) — in `_<union>_from_transfer_type` and again in `_<union>_to_transfer_type`, so a branch violation aggregates with its siblings. Selecting the branch *is* validating it. |
| Java       | a package-private `validate(path, violations)` on the wrapper class, with its compiled `pattern`/`format` `Pattern` statics. `fromNode` calls it on the wrapper it just built; the interface's static `validate` dispatches on the member's runtime class and is called by the declaring POJO's `Serializer` (and per element/member for a collection of unions) before any wire member is written.                                                                                                                         |

A **closed value set** (`const`/`enum`) on a branch closes the *type* where the
target can express that — a TypeScript literal union
(`"auto" | "manual" | number`), a Python `typing.Literal` — and is a membership
check in the validator in Go and Java, which have no field to hang a defined
type or value class off (the same treatment a typed map's member gets,
[[additionalProperties]] §"Per-member `T` validation"). The accepted value set
is identical in all four.

### Serialize-side (P12)

The in-memory value **is** a single branch member, so "exactly one" is
structurally guaranteed and the encode adapter simply emits the held variant: Go
`json.Marshal` on the interface marshals its dynamic type (a
`FooString`/`FooArray` named type serializes as its underlying JSON kind; an
object branch emits its fields); TS `toTransferType` branches on
`typeof`/`Array.isArray` and delegates to the member's converter; Java's
`Serializer` writes by runtime class; Python's `_<union>_to_transfer_type`
dispatches on the member's Python type with `isinstance` and calls the branch's
converter. The shared `Validate` still
**re-runs the selected branch's constraints before emit**, so an in-memory
member violating its own branch's rules fails serialize with the same aggregated
primitive rather than being written (real teeth where construction is
unchecked).

Python's serialize dispatch tests every branch but the last, then falls through
to it. The fallthrough is safe by **precondition**: `_<union>_to_transfer_type`
runs the union's checks — each branch's own constraints and the
no-branch-matched test — and raises the aggregated `PayloadValidationError`
application failure *before* the dispatch, so a value that reaches the guards is
already known to match some branch and the final `isinstance` is redundant.
Emitting it anyway would leave the guard and the `expected one of` raise behind
it statically unreachable.

Those checks live in that function for every union that has one, named or
inline, which is what puts them ahead of the dispatch — the only place that can
stop it. A union no branch of which transforms its value needs no such function
at all: the member is emitted as-is and the declaring model's converter runs the
checks inline.

A branch **transforms its value** whenever any part of it does, not only when
the branch is itself a model or a materialized scalar. An
**array branch whose elements are models**, temporals or binary values
transforms — per element, through the ordinary [[items]] mapper — so it is
emitted through its converter on the serialize side exactly as on the parse
side, with each element's constraints run and each violation pathed by index
under the member. Handing such a branch's in-memory value straight to the wire
emits language objects where JSON is required and skips the element validation
**P12** requires in both directions; a branch is "emitted as-is" only when its
value is already the wire form — a scalar, or a collection of scalars, that no
element mapper touches.

Either way the violations are reported under the member's path — a union
function collects them at the empty path and the declaring converter re-paths
them through `_collect` (**P11**) — and aggregate with the member's siblings.

The parse direction, which is handed untrusted input rather than a value of the
declared union type, tests every branch and raises `expected one of: <labels>`
with no fallthrough.

## Property-testing matrix

### Accepted (positive)

| Shape                                                                 | Example                                                                                                                      |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| Scalar ∪ scalar (disjoint kinds)                                      | `{oneOf:[{type:string},{type:integer}]}`                                                                                     |
| Object ∪ scalar                                                       | `{oneOf:[{$ref:"#/$defs/Widget"},{type:string}]}`                                                                            |
| Object ∪ array                                                        | `{oneOf:[{$ref:"#/$defs/Widget"},{type:array,items:{type:number}}]}`                                                         |
| Three disjoint kinds                                                  | `{oneOf:[{$ref:"#/$defs/Widget"},{type:string},{type:array,items:{type:number}}]}`                                           |
| Branch constraints carried through                                    | `{oneOf:[{type:string,minLength:3},{type:integer,minimum:0}]}`                                                               |
| An array branch's own bounds                                          | `{oneOf:[{type:array,items:{type:number},minItems:1,uniqueItems:true},{type:string}]}`                                       |
| Array-of-models branch (element converter both directions)            | `{oneOf:[{type:array,items:{$ref:"#/$defs/Address"}},{type:string}]}`                                                        |
| A closed value set on a branch                                        | `{oneOf:[{type:string,enum:[auto,manual]},{type:integer,minimum:0}]}`                                                        |
| An asserted (non-materializing) `format` on a branch                  | `{oneOf:[{type:string,format:uuid},{type:integer}]}`                                                                         |
| Named `$defs` union reused by `$ref`                                  | `{$defs:{Foo:{oneOf:[…]}}, properties:{f:{$ref:"#/$defs/Foo"}}}`                                                             |
| Object tagged union (`const`-tag)                                     | `{oneOf:[{$ref:"#/$defs/Cat"},{$ref:"#/$defs/Dog"}]}` with `kind:{type:string, const:…}` required in each                    |
| Tagged union mixed with a scalar kind                                 | `{oneOf:[{$ref:"#/$defs/Cat"},{$ref:"#/$defs/Dog"},{type:string}]}` (token picks object-vs-string; `const` picks Cat-vs-Dog) |
| Two-branch `[T,null]`                                                 | owned by [[nullability]] — the degenerate type-token case                                                                    |
| Nullable union — `null` among 3+ disjoint kinds                       | `{oneOf:[{$ref:"#/$defs/Widget"},{type:array,items:{type:number}},{type:"null"}]}`                                           |
| Inline free-form object ∪ scalar                                      | `{oneOf:[{type:object,additionalProperties:true},{type:string}]}`                                                            |
| Inline structured object ∪ scalar (branch type named `<Union>Object`) | `{oneOf:[{type:object,properties:{a:{type:string}}},{type:string}]}`                                                         |
| Inline tagged object branches, each self-named                        | `{oneOf:[{Cat…, x-go-name: Cat},{Dog…, x-go-name: Dog}]}` (the key of the target being generated)                            |
| Union as an array element (named)                                     | `{type:array, items:{$ref:"#/$defs/Foo"}}`                                                                                   |
| Union as an array element (inline, named `<Enclosing>Item`)           | `{type:array, items:{oneOf:[{type:string},{type:integer}]}}`                                                                 |
| Union as a map member (inline, named `<Enclosing>Value`)              | `{type:object, additionalProperties:{oneOf:[{$ref:"#/$defs/Cat"},{$ref:"#/$defs/Dog"}]}}`                                    |

### Rejected at load time (negative)

| Reason                                                                  | Example                                                                                                                                              |
| ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Single-branch wrapper (P7.1)                                            | `{oneOf:[{type:string}]}`                                                                                                                            |
| Empty array (invalid schema)                                            | `{oneOf:[]}`                                                                                                                                         |
| Branch with no classifiable kind                                        | `{oneOf:[{type:string},{minLength:3}]}`, `…{oneOf:[{type:string},true]}`, `…[{type:string},{}]`                                                      |
| Object branches with no shared required-`const` (no discriminator)      | `{oneOf:[{$ref:"#/$defs/A"},{$ref:"#/$defs/B"}]}`, neither carrying a `const` tag                                                                    |
| Two or more inline object branches missing a per-branch `x-<lang>-name` | `{oneOf:[{type:object,required:[kind],properties:{kind:{type:string,const:cat}…}},{…kind:{type:string,const:dog}…}]}` — both derive `<Union>Object`  |
| Synthesized `<Union>Object` collides with another emitted type (P15)    | a `$defs.FooObject` alongside an inline object branch of the `Foo` union                                                                             |
| Inline structured object branch with no derivable name                  | a branch inside a *nested inline object's* property, whose enclosing shape is itself not materialized                                                |
| Discriminator `const` not `required` in a branch                        | `{oneOf:[{Cat with kind required},{Dog with kind optional}]}`                                                                                        |
| Non-distinct discriminator values                                       | `{oneOf:[{kind:{type:string,const:"x"}…},{kind:{type:string,const:"x"}…}]}`                                                                          |
| Discriminator values distinct only in spelling                          | `{oneOf:[{kind:{const:1}…},{kind:{const:1.0}…}]}` — one value by [[const]] equality                                                                  |
| Non-annotation, non-`default` sibling of `oneOf` (sum type)             | `{oneOf:[{type:string},{type:integer}], type:object, properties:{ignored:{…}}}`; `…, const:"x"`; `…, minLength:3` — move the keyword into the branch |
| The same sibling on a [[nullability]] wrapper                           | `{oneOf:[{type:string},{type:"null"}], enum:[a,b]}`; `…, maxLength:8` — move the keyword to the non-null branch                                      |
| Ambiguous discriminator (2+ qualifying properties)                      | two object branches sharing both `kind` and `variant` as required-`const`                                                                            |
| A materialized temporal `format` on a sum-type branch (deferred)        | `{oneOf:[{type:string,format:"date-time"},{type:integer}]}`                                                                                          |
| A materialized `contentEncoding` on a sum-type branch (deferred)        | `{oneOf:[{type:string,contentEncoding:base64},{type:integer}]}`                                                                                      |
| `default` on a sum type                                                 | `{oneOf:[{type:string},{type:integer}], default:"x"}` — no branch is identified for construction                                                     |
| Two same-**scalar**-kind branches (use `enum`)                          | `{oneOf:[{type:string,const:"a"},{type:string,const:"b"}]}` → [[enum]]                                                                               |
| `integer`+`number` overlap (unsatisfiable, P7.1)                        | `{oneOf:[{type:integer},{type:number}]}`                                                                                                             |
| Duplicate `null` branches                                               | `{oneOf:[{type:string},{type:"null"},{type:"null"}]}` (same-kind, and a tautology)                                                                   |
| Branch is a nested combinator                                           | `{oneOf:[{type:string},{anyOf:[…]}]}`                                                                                                                |
| Synthesized union name collision (P15)                                  | two anonymous unions recasing to the same Go type name (fix: `x-go-name`)                                                                            |

### Runtime fixtures (validator)

- Wire value whose token selects a branch and validates → OK, bound to that
  variant (both directions).
- Token selects a branch but the value fails that branch's constraints (`""` for
  `{type:string,minLength:3}`) → one `Violation` from the branch predicate.
- Token matches **no** branch (`true` against `Widget | string | number[]`) →
  one `Violation` naming the admissible kinds.
- `1.5` against a `string | integer` union → routed to the integer branch,
  rejected by the spec-number rule (**P12** parse adapter), not truncated.
- Object with `kind:"cat"` against `Cat | Dog` → bound to `Cat`; extra unknown
  keys on the object are **preserved** (branch stays open, **P13**).
- Object against a free-form object branch → every member preserved verbatim,
  large integers untruncated, and re-emitted unchanged (**P13**).
- Object with `kind:"fish"` (unknown discriminator) → one `Violation` naming the
  admissible values (`cat`, `dog`) — closed value set (**P13.1**), not
  preserved.
- Object with the discriminator **absent** → one `Violation` (it is `required`);
  the deserializer never falls back to trial-matching branches.
- `null` against a nullable union (`Widget | number[] | null`) → accepted as the
  null state (required+nullable) / omitted-or-null per the [[nullability]]
  serialize table (optional); `null` against a non-nullable union → one
  `Violation`.
- A branch constraint is enforced **only for the branch the token selected**:
  `"ab"` against `{oneOf:[{type:string,minLength:3},{type:integer,minimum:1}]}`
  → one length `Violation`; `0` against the same union → one `minimum`
  `Violation` (never the string branch's).
- An array branch's own bounds: `[]` against
  `{type:array,minItems:1,uniqueItems:true}` → one `minItems` `Violation`;
  `[1.5,1.5]` → one duplicate-items `Violation`.
- A `const`/`enum` branch: an off-set string → one `Violation` naming the
  admissible values, while the sibling numeric branch stays unbounded.
- Serialize of an in-memory member violating its branch's own constraints →
  rejected before emit (**P12**), aggregated with any failing sibling field.
- A union-typed array element / map member: each value routed independently,
  with a failing one reported under its index or key (`shapes[1]`,
  `choices.primary`) and the rest still checked (**P11**).
- Serialize of an array of union members: each member's own branch constraints
  re-run before emit (**P12**).
- An **array-of-models** branch, both directions: parse maps each element
  through the referenced model's converter and re-paths its violations under the
  member's index; serialize does the mirror image, so the emitted wire holds
  JSON objects — never the in-memory element values — and an element violating
  its model's constraints is reported at `member[i].field` rather than passing
  through unchecked.
- A failing union combined with a failing sibling field → **both** reported in
  one shot (**P11**).

## Interactions

- **[[nullability]]**: owns the two-branch `oneOf:[{T},{null}]` pattern — the
  degenerate type-token case (the `null` token is the selector) — and the
  per-language nullable encoding/serialize tables. A `null` branch among 3+
  kinds is a **nullable union**: supported here, reusing those same tables over
  the union type (the `null` branch marks the field nullable, the non-null
  branches form the sum type).
- **[[const]] / [[enum]]**: the `const`-tag discriminator's basis — the
  validation-bearing selector for object branches, reusing [[const]]'s exact
  value-equality (already enforced both directions, **P12**) and its
  closed-value-set rejection of unknowns (**P13.1**); a single-value [[enum]] is
  an equivalent tag. Same-kind *scalar* branches are an [[enum]] rather than a
  `oneOf`. Also: [[properties]]'s synthesized-type naming rule — reused verbatim
  here for the union type name.
- **[[type]]**: every branch must declare one recognized `type` (or `$ref` to a
  typed def); it supplies the JSON kind that is the selector. A branch's own
  [[type]] siblings (formats, numeric/string constraints) validate normally once
  the branch is selected. The union **node** declares no `type` of its own; one
  written there is a load reject (above), and [[type]]'s leaf-`type` and
  object-without-shape guards must stay armed for a node that carries `oneOf` —
  a sibling the union rejects must not be able to switch them off.
- **[[ref]]**: a branch may be a `$ref` to a named typed definition; the
  resolved type supplies the kind, and (in Go/Java) that named type implements
  the union marker / interface directly rather than being wrapped.
- **[[additionalProperties]]**: object openness (**P13**) is *why* same-kind
  object branches need an explicit `const` tag — extra keys never disqualify a
  structural match. The tag closes selection *across* branches while each branch
  stays open *within* (unknown members preserved), so the two concerns coexist:
  closed discriminator, open payload. It also supplies the **free-form object**
  — the one inline branch shape that needs no name at all (it declares nothing
  to emit) — and the per-language member representation its Go/Java wrapper
  reuses.
- **[[required]]**: the discriminator property **must** be `required` in every
  object branch — otherwise a payload could omit it and be unselectable.
  Separately, whether the whole union-typed member is present is its own
  `required` question; an absent optional union raises no `oneOf` violation
  (**P8**).
- **[[allOf]]**: the *other* admitted boolean-logic applicator, but by a
  different mechanism — intersection **collapses to one type at load** (a
  merge/flatten), so it disappears downstream, whereas `oneOf` is retained and
  emits a union. An `allOf` branch that is itself a `oneOf` is rejected
  ([[allOf]]): an intersection with a union does not collapse.
- **[[anyOf]] / [[dependentSchemas]] / [[not]] / [[if-then-else]]**: the
  boolean-logic / conditional applicators that stay **rejected** per **P6**.
  `oneOf` is admitted because "exactly one" is a *closed sum type* with a
  decidable selector, which inclusive-or ([[anyOf]]), negation ([[not]]), and
  runtime shape-forking ([[if-then-else]] / [[dependentSchemas]]) are not.

## Ecosystem variance

| Source dialect            | Action                                                                                                                                                                                                                                                           |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12       | Native. Type-token-separable unions (incl. nullable unions), plus object tagged unions via a shared required-`const` discriminator. `integer`+`number` overlap rejected.                                                                                         |
| OpenAPI 3.1               | Adopts 2020-12 — `oneOf` identical; the const-tag form works unchanged. The 3.1 `discriminator: {propertyName, mapping}` object is **deferred** (will be accepted as optional sugar over the `const`-tag form, consistency-checked against the branch `const`s). |
| OpenAPI 3.0 / Swagger 2.0 | `oneOf` exists (3.0) with the same exactly-one semantics; the standalone `discriminator` is the same deferred sugar. Swagger 2.0 has no `oneOf`.                                                                                                                 |
| draft-4..7                | `oneOf` present since draft-4 with identical semantics — no rewrite; the subset rules (selector required, overlaps rejected) apply unchanged.                                                                                                                    |

## See also

- [[nullability]] — owns the two-branch `oneOf:[{T},{null}]` pattern (the
  degenerate type-token case) and the per-language nullable encoding a nullable
  union reuses over the union type.
- [[const]] / [[enum]] — the `const`-tag discriminator's validation-bearing
  selector (and closed-value-set rejection), and the source of the
  synthesized-type naming rule reused for the union type.
- [[type]] — supplies each branch's JSON kind, the outer selector.
- [[ref]] — `$ref` branches; the resolved type implements the Go/Java union
  marker directly.
- [[additionalProperties]] — object openness (**P13**) is why object branches
  need an explicit `const` tag; branches stay open within. Also the free-form
  object (the one inline branch shape that needs no name) and the member
  representation its Go/Java wrapper reuses.
- [[required]] — the discriminator must be required in each branch; also the
  union member's own presence, distinct from branch selection.
- [[allOf]] — the other admitted applicator; intersection collapses to one type
  at load (merge), while `oneOf` is retained as a union.
- [[dependentSchemas]] — a rejected applicator; shares the P6 rationale that
  `oneOf` (as a closed sum type) is the exception to.
- [[PRINCIPLES.md]] — **P1** (polyglot sum type), **P6** (strict subset),
  **P7/P7.1** (reject ambiguity), **P2** (idiomatic per-language output),
  **P13** (open objects).
