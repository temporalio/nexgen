# `allOf`

Source: JSON Schema 2020-12, Core (Applicator vocabulary), §10.2.1.1 "Keywords
for Applying Subschemas With Boolean Logic → allOf".

The **intersection / schema-composition** keyword: an instance is valid iff it
validates against **every** subschema. Unlike the other boolean-logic
applicators ([[oneOf]], [[anyOf]], [[not]], [[if-then-else]]), `allOf` describes
constraints that all apply to **one value of one kind** — so it is not a runtime
fork at all. The generator admits it as a **load-time merge**: the branches are
flattened into a **single materialized schema** that is then lowered by the
ordinary keyword specs, exactly as if the author had hand-written the combined
schema. There is no retained `allOf` construct past the loader, no runtime
"check all branches" loop, and no new emitted type — `allOf` is a normalization
step, invisible downstream. It earns its place (against **P6**'s example
rejection of the applicators) precisely because intersection
*collapses to a single type*; where two branches cannot be collapsed (disjoint
kinds, contradictory constraints), the merge is **rejected loudly** (**P7.1**),
never approximated.

## Spec summary

Verbatim (2020-12 core, Applicator, §10.2.1.1):

> This keyword's value MUST be a non-empty array. Each item of the array MUST be
> a valid JSON Schema.

> An instance validates successfully against this keyword if it validates
> successfully against all schemas defined by this keyword's value.

Distilled:
- The value is a non-empty array of subschemas — the **branches** — and the
  instance must satisfy **all** of them. Intersection, not choice: the
  constraints *accumulate* onto the same value.
- Because every branch constrains the same value, the conjunction is itself
  expressible as one schema whenever the branches are *compatible*. Merging
  `{minimum: 3}` and `{minimum: 4}` yields `{minimum: 4}`; merging
  `{type: string}` and `{type: number}` yields nothing satisfiable. The
  generator computes that merge at load and works only with the result.
- This is the keyword the numeric specs already anticipated: same-axis bounds on
  a *single* node are rejected as redundant ([[maximum]], [[minimum]]), but the
  same pair arriving from *different* `allOf` branches is legitimate and must be
  **tightened**, not rejected.

## Support decision

**Support:** yes, as a **load-time merge** that materializes a single schema.
The generator accepts an `allOf` iff its branches are **mergeable** into one
coherent, satisfiable schema; it rejects an `allOf` whose branches contradict or
cannot be represented as a single type.

Concretely, `allOf` is a rewrite that runs after a raw branch-grammar gate and
before the ordinary post-merge keyword lowering. The raw gate owns exactly the
checks a merge could **silently discard** and that are decidable on one branch
alone: the keyword allowlist, raw value shapes (`required` and
`dependentRequired` grammar, raw property counts, a same-axis bound pair
redundant *within one branch*), and per-branch malformations no merge can
repair. A same-axis pair arriving from *different* branches is not one of these
— it is the tightening the merge exists to do.

**Naming and satisfiability checks are *not* in that gate.** A **P15**
identifier collision, or the encodability of a value constant, is a property of
the *merged* result — a branch may declare `enum: [user, USER]` that the merge
intersects away — so running such a check per authored branch rejects schemas
whose merged form is clean. They run on the merged schema only (step 3).

The merge logic itself is narrow:

1. **Flatten** — recursively inline nested `allOf`, resolve `$ref` branches
   (below), and drop identity branches (`true`/`{}`).
2. **Merge** — fold the branches key-by-key using the per-keyword rules in
   *Merge algorithm* below: same-axis constraints **tighten**, value sets
   **intersect**, object/array subschemas **merge recursively**.
3. **Hand back** — feed the merged schema to the ordinary loader. Every
   satisfiability, shape, and naming check is **delegated** to the keyword spec
   that owns the surviving keyword: an empty numeric interval is [[maximum]]'s
   reject, `minLength > maxLength` is [[minLength]]'s, a synthesized-name
   collision is **P15**. `allOf` does not re-implement them — it only produces
   the schema they judge.

So the merge step owns exactly two decisions the downstream loaders can't make
on their own: **tightening** a same-axis pair that legitimately came from two
branches, and **rejecting the unmergeable** — a pair with no single-schema
representation. Everything else falls out of the normal pipeline.

Grounding ([[PRINCIPLES.md]]): **P1** — a merged single schema round-trips
identically across all four targets because there is no combinator left to
represent per-language; **P6** — this is the coherent-representation bar the
applicator rejection sets: intersection *can* be represented as one type, so it
is admitted as a merge, whereas [[oneOf]]'s sibling applicators (`anyOf`
inclusive-or, `not` negation, `if` runtime fork) cannot collapse and stay
rejected; **P7.1** — an unmergeable pair errors at load with a fix-it, never a
silent approximation; **P2** — the output is whatever the merged schema lowers
to, i.e. ordinary hand-written-feeling types, with no `allOf` residue.

### `$ref` branches and the implicit-`allOf` sugar

A branch may be a `$ref` to a named typed definition ([[ref]]). The merge
**resolves the ref and folds the target's schema in** — the referenced type's
constraints are copied into the merged result. This is the classic "extend a
base type" composition (`allOf: [{$ref: Base}, {extra}]`).

The 2020-12 **`$ref`-with-siblings** form is the same operation spelled as
sugar: `{$ref: X, minLength: 3}` is an implicit `allOf` of the referenced schema
and the sibling keywords. Because `allOf` merges, the sugar is
**merged identically** — the loader rewrites `{$ref: X, …siblings}` to
`allOf: [{$ref: X}, {…siblings}]` and folds it, and the explicit and implicit
spellings then behave the same. Two sibling classes are the exception: they are
not conjuncts, so the node stays a plain reference and no merge happens.

- an **`x-<lang>-name`** renames the member the reference is bound to and
  asserts nothing about the value;
- the **non-conjunct annotations** `$comment`, `examples` and [[deprecated]] —
  the last at **either value**, since it marks the member rather than asserting
  anything about the referenced value — are dropped **before** the fold
  decision, so they can neither clone the target into a new type nor add a
  **P15** identifier.

Every other sibling folds, and the merged node is therefore a
**new standalone declaration** whose synthesized name enters **P15** — including
a [[title]] or [[description]] sibling, which is not inert in this position
because it survives into the merged declaration. [[ref]] states the obligation
that entailment puts on the resulting collision diagnostic.

**Keywords sibling to `allOf` fold in the same way** — the `$ref` case is just
its most common instance. A node may carry keywords *alongside* its `allOf`:
`{allOf: [A, B], type: "object", description: "D"}`. Per JSON Schema every
keyword in a node applies conjunctively, so those siblings are one more conjunct
— the node is equivalent to `allOf: [A, B, {…siblings}]`, with the node's
**own** keywords folded as the **final** branch. `$ref`, `allOf`, and plain
siblings compose into one canonical fold: `{$ref: R, allOf: [A], …S}` →
`allOf: [{$ref: R}, A, {…S}]`. The node's own keywords go **last** for a reason
— under the metadata **last-wins** rule (below) the local declaration then
overrides any [[title]]/[[description]]/[[default]] pulled in from a branch or a
`$ref` target, which is the intuitive "use-site wins" precedence. Constraints
are unaffected by the position (intersection is order-independent).

Ref-branch specifics:
- Resolution reuses [[ref]] entirely: named-targets-only, local-file-only, no
  `$id`, no HTTP. An unresolvable branch is [[ref]]'s reject.
- A `$ref` **cycle** that an `allOf` merge must inline is unflattenable and is
  rejected at merge time. This is distinct from [[ref]]'s no-finite-instance
  satisfiability reject: an optional recursive value may be satisfiable while
  still being impossible to flatten finitely, so [[ref]]'s pass cannot stand in
  for this one. The reject is **position-independent** — it fires wherever the
  back-edge sits, including under a `properties`, an `items`, a schema-valued
  `additionalProperties`, a [[oneOf]] branch or a nested `$defs` entry, and not
  only on a definition that merges itself directly. A bare recursive `$ref`
  back-edge is unaffected and keeps working; what is rejected is a back-edge
  that is *itself* a merged node. Exhausting the stack, or recursing without
  bound, is never an acceptable outcome: the loader owes a diagnostic naming the
  reference and the cycle path.
- **The merge flattens; it does not subtype.** The base type's fields are
  *copied into* the merged type; the result is **not** a subtype of the base,
  and no inheritance/embedding is emitted (the subset has no inheritance). The
  base type still exists as its own generated type; the merged type is an
  independent, standalone type that happens to share field shapes. Editing the
  base regenerates the merged type's copied fields — an additive base change
  stays additive (**P13**).

## Merge algorithm

The merge folds branches pairwise. **Constraint** merging is associative and
order-independent (intersection is commutative), so the *validation semantics*
never depend on branch order. The one order-sensitive part is
**metadata-annotation selection** ([[title]], [[description]], [[default]]):
these carry no validation effect, so instead of rejecting a conflict the merge
keeps the **last** branch's value (**last-wins**) — a deterministic override,
not an ambiguity. The `$ref`-with-siblings rewrite places the `$ref` **first**
and the use-site siblings **last** (see [[ref]]), so an annotation written next
to a `$ref` overrides the referenced target's. Per keyword:

### Type and value sets

| Keyword                                          | Merge rule                                                                                                                                                                                                                                                                              | Reject when                                                                                                    |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `type`                                           | intersection of the declared kinds; `integer ⊂ number` (so `integer ∩ number = integer`); an absent `type` contributes no constraint                                                                                                                                                    | the kinds are **disjoint** (`string ∩ number`, `object ∩ array`) → unsatisfiable                               |
| `const`                                          | all branch `const`s must be **deep-equal**; the shared value survives; it is then checked (decidably, at load) against every other merged keyword — kind, `enum` membership, numeric range, length                                                                                      | two branches carry **different** `const`s, or the `const` violates another merged constraint                   |
| `enum`                                           | **set intersection** of the members (kept in first-seen order)                                                                                                                                                                                                                          | the intersection is **empty**                                                                                  |
| `const` + `enum`                                 | the `const` must be a member of the `enum`; result is the `const`                                                                                                                                                                                                                       | the `const` is not in the `enum`                                                                               |
| `format`                                         | identical → dedupe; when one format's accepted value set **contains** the other's, keep the narrower one (the intersection *is* that format). The containment relation is [[format]]'s to define; this spec only consumes it                                                            | neither format contains the other — overlapping is not enough, since the intersection is then no single format |
| `title` / `description` / `default`              | **last-wins**: identical values dedupe; when they differ the **last** branch's value survives (metadata, no validation effect); the `$ref`-sibling rewrite makes the use-site value override the target's. A lone value is kept.                                                        | never — a differing metadata value is an override, not a conflict                                              |
| [[deprecated]]                                   | **OR**: the merged node is deprecated if **any** branch marks it so. Not last-wins — deprecation is a warning that must not be silenced by a later branch omitting it (or writing `deprecated: false`, which asserts nothing). Beside a `$ref` it is not a conjunct at all — see above. | never                                                                                                          |
| `contentEncoding`                                | identical → dedupe                                                                                                                                                                                                                                                                      | differing encodings                                                                                            |
| `x-<lang>-name`, value-constant naming overrides | a lone or identical value survives; a name inherited from a `$ref` target is stripped so the use site does not claim the target's declaration name                                                                                                                                      | differing values, in every target, to keep the loader accept set language-independent                          |
| `$defs`                                          | a lone value survives; definitions inherited from a `$ref` target are stripped to avoid duplicate declarations                                                                                                                                                                          | differing maps                                                                                                 |

### Numeric bounds ([[minimum]] / [[maximum]] / exclusives / [[multipleOf]])

Same-axis bounds **collapse to the tighter single bound** — this is the
tightening the numeric specs deferred here, and it means the merged result never
carries a same-axis *pair* on one node, so [[maximum]]'s single-node redundancy
reject never fires on merge output.

- **Lower bound:** among all `minimum`/`exclusiveMinimum` across branches, keep
  the one with the **greatest effective floor**. Inclusive `minimum: m` admits
  `≥ m`; exclusive `exclusiveMinimum: e` admits `> e`. Keep `exclusiveMinimum`
  iff `e ≥ m`, else `minimum` (collapsing the cross pair to whichever admits the
  smaller set).
- **Upper bound:** symmetric — keep the **smallest ceiling** among
  `maximum`/`exclusiveMaximum`. Inclusive `maximum: m` admits `≤ m`; exclusive
  `exclusiveMaximum: e` admits `< e`. Keep `exclusiveMaximum` iff `e ≤ m`, else
  `maximum`.
- **`multipleOf`:** the value must be a multiple of every divisor → merge to
  their **LCM**. All supported divisors are positive integers ([[multipleOf]]),
  so the LCM is a positive integer; no new form appears.
- **Then delegate satisfiability to [[maximum]]:** an empty interval
  (`minimum > maximum`, `minimum ≥ exclusiveMaximum`, the integer "no integer in
  range" case, or no multiple of the divisor in range) is [[maximum]]'s reject
  on the merged schema — not re-checked here.

### String / array / object length and count

All are "keep the tighter":

| Family           | Lower keyword → keep      | Upper keyword → keep      |
| ---------------- | ------------------------- | ------------------------- |
| String length    | `minLength` → **max**     | `maxLength` → **min**     |
| Array length     | `minItems` → **max**      | `maxItems` → **min**      |
| Object size      | `minProperties` → **max** | `maxProperties` → **min** |
| `contains` count | `minContains` → **max**   | `maxContains` → **min**   |

- `uniqueItems`: logical **OR** — `true` if any branch sets it (the tighter
  constraint wins).
- **An omitted keyword contributes its default, not nothing**, whenever that
  default is not the neutral element of the merge direction. `minItems`,
  `minProperties` and `minLength` default to `0`, which is neutral for
  *keep the max*, and `maxContains` defaults to unbounded, neutral for
  *keep the min* — so omitting them is genuinely silent. [[minContains]] is the
  exception: its omitted form is `1`, so a branch that declares [[contains]] and
  no `minContains` contributes `minContains: 1` to the merge, and a sibling
  branch's `minContains: 0` does **not** win by default. Treating the omission
  as "no constraint" makes the merged schema weaker than the conjunction of its
  branches, which the intersection semantics forbid.
- Emptiness (`min* > max*`) is the owning spec's satisfiability reject on the
  merged schema.

### Recursive-schema keywords

Keywords whose value is itself a schema (or a map of schemas) merge by
**recursing the same `allOf` merge** on the paired subschemas — both must hold,
and both are on the same value, so they compose the same way:

- `items`: merge the two item schemas.
- `properties`: **union** of property names; a name present in **both** branches
  has its two property subschemas merged recursively.
- `patternProperties`, `propertyNames`, `additionalProperties` (when a schema):
  merge recursively per matching pattern / on the single schema.
- `required`: **union** of the required names.
- `dependentRequired`: per trigger key, **union** the dependent-name lists.

### `additionalProperties: false` across branches

The flatten **resolves the notorious `allOf` + closed-object footgun**. In raw
JSON Schema, `additionalProperties` only sees its *own* branch's `properties`,
so `allOf: [{properties: {a}, additionalProperties: false}, {properties: {b}}]`
rejects every object carrying `b`. Because the generator
**merges the property sets first**, `false` in any branch closes the merged
object against the **union** of all branches' declared properties — the
intuitive intent. This is a deliberate, documented divergence from raw allOf
validation semantics; it is coherent under **P1** (all four targets generate
from the one flattened schema and agree value-for-value) and is the only
behavior that lets base-type extension with a closed base work at all. If
**any** branch is closed, the merged object is closed to the union; if all are
open, it stays open (**P13**).

### Trivial and non-mergeable branches

- `true` / `{}` (empty schema): identity — contributes no constraint, dropped
  during flatten.
- `false`: unsatisfiable — nothing validates → **reject**.
- A branch that is itself a **combinator** (`oneOf`/`anyOf`/`not`/`if`):
  **reject**. An intersection with a union/negation/fork does not collapse to a
  single type (distributing a constraint across [[oneOf]] branches would produce
  an `anyOf` of merges — outside the subset). `anyOf`/`not`/ `if` are rejected
  everywhere (**P6**); an `allOf` branch does not reintroduce them.
- Two **distinct** `pattern`s or `contains` schemas: **reject** as unmergeable —
  each is a constraint with no single-value representation (two regexes are not
  one regex; two existential matchers are two constraints). Identical values
  dedupe.
- Two **distinct** `format`s: reject only when neither contains the other (see
  *Type and value sets*); a containment pair merges to the narrower format and
  is **not** an unmergeable pair.

## Loader behavior

- `allOf` value not a **non-empty array of valid subschemas** → reject (recurse
  into each branch to report the inner fault).
- **Empty `allOf: []`** → reject: a no-op that validates everything (pointless
  wrapper, **P7.1**; fix-it: remove it).
- **Single-branch `allOf: [X]`** → reject: it is just `X` (pointless wrapper;
  fix-it: inline the branch), mirroring the single-branch [[oneOf]] reject.
- Flatten nested `allOf`, resolve `$ref` branches ([[ref]] rules; a cycle the
  merge must inline → the merge-time unflattenable reject above), rewrite
  `$ref`-with-siblings to `allOf`, fold any keywords **sibling to `allOf`** in
  as a final branch, drop `true`/`{}` branches.
- `false` branch, or a branch that is a `oneOf`/`anyOf`/`not`/`if` combinator →
  reject.
- Fold per *Merge algorithm*. Reject the unmergeable pairs: disjoint `type`,
  disagreeing `const`, empty `enum` intersection, `const` violating a sibling
  constraint, two `format`s in no containment relation, distinct
  `pattern`/`contains`. A differing `title`/`description`/`default` is **not**
  an unmergeable pair — it is a last-wins override (see *Type and value sets*).
- **A malformed branch keeps its own keyword's diagnostic** (**P7.2**). When a
  branch's raw value is invalid for the keyword that owns it — a non-string
  `format` or `pattern`, a `multipleOf` that is zero or fractional, an array
  `type`, a keyword the subset rejects outright — the reject is the
  **owning spec's**, with its fix-it, located at the branch that carries it
  (`.allOf[i]`). A merge message must never be substituted for it, and must
  never name a keyword the author did not write: the merge is reached only for
  branches whose values are already well-formed for their own keyword. This
  holds for the implicit conjunct of a `$ref`-with-siblings as well as for an
  authored branch.
- Hand the merged schema to the ordinary loader; **all** satisfiability / shape
  / collision checks (empty interval, `min* > max*`, integer-range emptiness,
  synthesized-name **P15** collision) are the owning specs' rejects on the
  merged result, not restated here.

## Type mapping

`allOf` emits **nothing of its own.** The merged schema is lowered by the
keyword specs that own its surviving keywords — an object merge becomes an
ordinary object type ([[properties]]), a scalar merge a constrained scalar
([[type]] + the constraint families), and so on. There is no `AllOf` wrapper
type in any language.

Naming follows the ordinary rule ([[properties]] §"Synthesized type names",
reused verbatim by [[oneOf]]/[[const]]/[[enum]]):
- an `allOf` that **is** a named `$defs` entry takes the def name;
- an **anonymous** inline object merge is hoisted and named
  `<EnclosingType><Property>` in all four targets;
- an anonymous inline enum merge uses a named carrier in Go and Java, while
  TypeScript and Python inline the literal set.

Base-type extension therefore emits a flat, standalone type:

```yaml
$defs:
  Base:  { type: object, required: [id], properties: { id: {type: string} } }
  Sized: { type: object, properties: { size: {type: integer, minimum: 0} } }
  Widget:
    allOf:
      - { $ref: '#/$defs/Base' }
      - { $ref: '#/$defs/Sized' }
      - { type: object, required: [name], properties: { name: {type: string} } }
```

`Widget` merges to a single object with `{id, size, name}`,
`required: [id, name]` — copied fields, no inheritance. Every target emits it as
it would the hand-written combined object (Go struct, TS `interface`, Python
dataclass, Java POJO); `Base` and `Sized` remain their own types, unrelated to
`Widget`.

## Validator mapping

Per **P10**/**P11**/**P12**, with **nothing `allOf`-specific at runtime**: the
merged schema's constraints are validated by their own shared `Validate`
predicates in both directions, aggregated as always. Because the `allOf` is gone
after load, there is no "validate against each branch" loop, no
branch-selection, and no residual combinator — the tightened bounds, unioned
`required`, merged `properties`, and closed-object check are indistinguishable
from a hand-authored schema to every layer below the loader. Reason strings come
from the owning constraint families (`minimum 4`, `maxLength 8`, …), never a
bare `allOf`.

## Property-testing matrix

### Accepted (positive)

| Shape                                                        | Example                                                                                                                                                                                                                              |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Tighten same-axis lower bounds                               | `{allOf:[{type:integer,minimum:3},{type:integer,minimum:4}]}` → `minimum:4`                                                                                                                                                          |
| Tighten across inclusive/exclusive                           | `{allOf:[{maximum:10},{exclusiveMaximum:8}]}` → `exclusiveMaximum:8`                                                                                                                                                                 |
| Merge lower + upper into an interval                         | `{allOf:[{minimum:0},{maximum:100}]}`                                                                                                                                                                                                |
| `multipleOf` LCM                                             | `{allOf:[{multipleOf:2},{multipleOf:3}]}` → `multipleOf:6`                                                                                                                                                                           |
| String length tighten                                        | `{allOf:[{minLength:2},{minLength:5},{maxLength:20}]}`                                                                                                                                                                               |
| `enum` intersection                                          | `{allOf:[{enum:[a,b,c]},{enum:[b,c,d]}]}` → `[b,c]`                                                                                                                                                                                  |
| `const` consistent with `enum`/range                         | `{allOf:[{const:5},{minimum:0,maximum:10}]}`                                                                                                                                                                                         |
| Object field union                                           | `{allOf:[{properties:{a},required:[a]},{properties:{b},required:[b]}]}` → `{a,b}`, `required:[a,b]`                                                                                                                                  |
| Overlapping property merged recursively                      | `{allOf:[{properties:{n:{minLength:2}}},{properties:{n:{maxLength:8}}}]}`                                                                                                                                                            |
| Base-type extension via `$ref`                               | `Widget` example above                                                                                                                                                                                                               |
| `$ref`-with-siblings (implicit allOf)                        | `{$ref:'#/$defs/Base', minProperties:1}`                                                                                                                                                                                             |
| `$ref` with a non-conjunct sibling (no fold, reference kept) | `{$ref:'#/$defs/Base', x-go-name:'Renamed'}`; `{$ref:'#/$defs/Base', $comment:'note'}`; `{$ref:'#/$defs/Base', deprecated:true}`                                                                                                     |
| `format` containment pair (merge to the narrower)            | `{allOf:[{format:uri},{format:'uri-reference'}]}` → `uri`                                                                                                                                                                            |
| `allOf`-with-siblings (node keywords fold in, last)          | `{allOf:[{$ref:'#/$defs/Base'}], properties:{extra:{type:string}}, required:[extra], description:"D"}` → `allOf:[{$ref:Base},{properties:{extra},required:[extra],description:"D"}]`; siblings extend Base, local `description` wins |
| Closed base + extension (footgun-fixed)                      | `{allOf:[{properties:{a},additionalProperties:false},{properties:{b}}]}` → closed to `{a,b}`                                                                                                                                         |
| Nested `allOf` flattened                                     | `{allOf:[{allOf:[{minimum:1}]},{maximum:9}]}`                                                                                                                                                                                        |
| Identity branch dropped                                      | `{allOf:[{type:string,minLength:3},true]}`                                                                                                                                                                                           |
| Differing metadata annotation (last-wins)                    | `{allOf:[{default:1},{default:2}]}` → `2`; likewise `title`/`description` take the last branch's value (use-site sibling overrides a `$ref` target)                                                                                  |
| `deprecated` OR-merged                                       | `{allOf:[{deprecated:true},{deprecated:false}]}` → deprecated; `{allOf:[{deprecated:true},{minimum:0}]}` → deprecated                                                                                                                |

### Rejected at load time (negative)

| Reason                                               | Example                                                                                                                                             |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Disjoint `type` (unsatisfiable)                      | `{allOf:[{type:string},{type:number}]}`                                                                                                             |
| Empty numeric interval (delegated to [[maximum]])    | `{allOf:[{minimum:10},{maximum:5}]}`                                                                                                                |
| Empty `enum` intersection                            | `{allOf:[{enum:[a,b]},{enum:[c,d]}]}`                                                                                                               |
| Disagreeing `const`                                  | `{allOf:[{const:1},{const:2}]}`                                                                                                                     |
| `const` violates a sibling                           | `{allOf:[{const:5},{maximum:4}]}`                                                                                                                   |
| Value literal not representable in the merged `type` | `{allOf:[{type:number,const:2.5},{type:integer}]}` — the merge is `integer`, and `2.5` is not one; likewise a fractional `enum` member or `default` |
| `format`s with no containment relation               | `{allOf:[{format:email},{format:uri}]}` (contrast `{format:uri}` + `{format:uri-reference}`, which merges to `uri`)                                 |
| Distinct `pattern`s (no single regex)                | `{allOf:[{pattern:'^a'},{pattern:'z$'}]}`                                                                                                           |
| Distinct `contains` (two existentials)               | `{allOf:[{contains:{const:1}},{contains:{const:2}}]}`                                                                                               |
| `false` branch (unsatisfiable)                       | `{allOf:[{type:object},false]}`                                                                                                                     |
| Branch is a combinator                               | `{allOf:[{type:object},{oneOf:[…]}]}`, `…{anyOf:[…]}`, `…{not:{…}}`                                                                                 |
| Empty array (pointless)                              | `{allOf:[]}`                                                                                                                                        |
| Single-branch wrapper (pointless)                    | `{allOf:[{type:string}]}` → use the branch directly                                                                                                 |
| Unresolvable / cyclic `$ref` branch                  | `{allOf:[{$ref:'#/$defs/Missing'}]}`; a type merging itself                                                                                         |
| Merged synthesized-name collision (P15)              | two anonymous merges recasing to one Go type name (fix: `x-go-name`)                                                                                |

### Runtime fixtures (validator)

There is **no `allOf`-specific runtime behavior** — fixtures exercise the
*merged* schema through the owning families:

- A value satisfying every merged bound (`7` against merged
  `minimum:4`/`maximum:10`) → OK both directions; a value failing the tightened
  bound (`3`) → one `Violation` from [[minimum]] naming `minimum 4` (the
  *tightened* value, not either original).
- An object missing a `required` name contributed by *either* branch → one
  `Violation` from [[required]].
- An overlapping-property value failing the *merged* property schema (`"a"`
  against a field merged to `minLength:2`+`maxLength:8` → still ok; `""` → one
  `Violation`).
- An object with an unknown key against a **closed-by-merge** object → one
  `Violation` (closed to the union of declared properties); the same object
  where all branches were open → key **preserved** (**P13**).
- A merged-in base field violating its own constraint on serialize → rejected
  before emit (**P12**) — identical to a hand-written field.
- A failing merged constraint alongside a failing sibling field → **both**
  reported in one shot (**P11**).

## Interactions

- **[[maximum]] / [[minimum]] / [[exclusiveMaximum]] / [[exclusiveMinimum]] /
  [[multipleOf]]**: the numeric specs reject a same-axis bound
  *pair on one node* as redundant, but explicitly defer the **cross-branch**
  case to here — two same-axis bounds from different `allOf` branches are
  **tightened** to one, and combined-interval / integer-range / divisor
  emptiness is delegated back to [[maximum]]'s satisfiability reject on the
  merged schema.
- **[[minLength]] / [[maxLength]] / [[minItems]] / [[maxItems]] /
  [[minProperties]] / [[maxProperties]] / [[minContains]] / [[maxContains]] /
  [[uniqueItems]]**: length/count bounds keep the tighter; emptiness is the
  owning spec's reject. [[minContains]]' omitted default of `1` participates in
  the merge like an authored value, because it is the one default in the family
  that is not neutral.
- **[[const]] / [[enum]]**: closed value sets **intersect** on merge (empty →
  reject); a `const` must be consistent with a merged `enum` and every other
  merged constraint. Reuses their exact value-equality and closed-set semantics
  (**P13.1**).
- **[[properties]] / [[required]] / [[additionalProperties]] /
  [[patternProperties]] / [[propertyNames]] / [[dependentRequired]]**: object
  keywords merge structurally — property union with recursive merge of shared
  names, `required` union, and the closed-object flatten that fixes the
  raw-allOf `additionalProperties:false` footgun. The synthesized-type naming
  rule is reused verbatim for the merged type.
- **[[items]] / [[contains]]**: `items` schemas merge recursively; distinct
  `contains` matchers are unmergeable (two existential constraints) → reject.
- **[[format]]**: owns which formats **contain** which; this spec consumes that
  relation to merge a containment pair to the narrower format and to reject a
  pair with no containment.
- **[[ref]]**: a branch may `$ref` a named typed def; the target is resolved and
  **folded in** (flatten, not subtype). `$ref`-with-siblings is the
  implicit-`allOf` sugar, except for a member-only language-name override and
  the non-conjunct annotation set, which leave the reference intact. Ref
  resolution rules are reused; flattening cycles have their own merge-time
  reject, distinct from [[ref]]'s no-finite-instance check.
- **[[oneOf]]**: the sibling boolean-logic applicator that is admitted a
  *different* way — as a retained closed sum type with a decidable selector,
  because a union cannot collapse to one type. `allOf` collapses and disappears;
  `oneOf` stays and emits a union. An `allOf` branch that is itself a `oneOf`
  (or `anyOf`/`not`/`if`) is **rejected** — an intersection with a union does
  not collapse. When the union arrives as the *resolved target* of a `$ref`
  carrying a constraint sibling, the reject is the same but the diagnostic is
  [[oneOf]]'s: it names the union, the sibling the author wrote, and the branch
  to move it into — never an "`allOf` branch", a phrase naming a keyword the
  author never wrote.
- **[[nullability]]**: nullability is expressed through [[type]]/[[oneOf]], not
  `allOf`; a `{type:"null"}` branch merged with any other kind is a
  disjoint-`type` reject.
- **[[dependentSchemas]] / [[unevaluatedProperties]] / [[unevaluatedItems]]**:
  the applicators that stay **rejected** (**P6**); `allOf` is the exception only
  because intersection materializes as one schema, which these do not.

## Ecosystem variance

| Source dialect            | Action                                                                                                                                                                                                                                                                                                             |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| JSON Schema 2020-12       | Native. Merged at load; mergeable branches supported, contradictory branches rejected. `$ref`-with-siblings is the implicit-`allOf` sugar, merged identically apart from the two non-conjunct sibling classes.                                                                                                     |
| OpenAPI 3.1               | Adopts 2020-12 — `allOf` identical. The dominant OpenAPI use (base-type extension, `{allOf:[{$ref:Base},{extra}]}`) is exactly the flatten-merge case.                                                                                                                                                             |
| OpenAPI 3.0 / Swagger 2.0 | `allOf` exists with the same semantics and the same base-extension idiom; merged the same way. (3.0 `additionalProperties:false` composition inherits the same footgun-fix.)                                                                                                                                       |
| draft-4..7                | `allOf` present since draft-4 with identical semantics. Only difference: pre-2020-12 `$ref` **ignored** its siblings, so a draft-07 `{$ref, …siblings}` validated as the bare `$ref`; we merge the siblings (2020-12 semantics) — a stricter, more faithful reading, noted as the one cross-draft behavior change. |

## See also

- [[maximum]] / [[minimum]] — defer the cross-branch same-axis **tightening**
  here, and own the merged-interval satisfiability reject.
- [[multipleOf]] — divisors merge to their LCM; positive-integer only.
- [[const]] / [[enum]] — closed value sets intersect on merge; source of the
  synthesized-type naming rule reused for the merged type.
- [[properties]] / [[required]] / [[additionalProperties]] — structural object
  merge and the closed-object footgun-fix.
- [[ref]] — `$ref` branches fold in (flatten, not subtype); the implicit-`allOf`
  sibling sugar merges identically, apart from the two sibling classes that
  leave the reference intact.
- [[oneOf]] — the applicator admitted as a retained sum type; contrast with
  `allOf`, which collapses and disappears. A `oneOf` branch inside `allOf` is
  rejected.
- [[PRINCIPLES.md]] — **P1** (one merged schema, identical across targets),
  **P6** (intersection meets the coherent-representation bar the applicator
  rejection sets), **P7.1** (unmergeable pairs reject loudly), **P7.2** (the
  reject is the owning keyword's, never a merge message over a keyword the
  author did not write), **P13** (open objects; the closed-merge exception).
