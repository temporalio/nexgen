# `$ref`

Source: JSON Schema 2020-12, Core vocabulary, §8.2.3.1 (`$ref`) and §8.2.4
(`$defs`).

The reference keyword. `$ref` lets one schema reuse another by URI reference;
`$defs` is the standard container for named reusable subschemas. Together they
are the structural backbone of any non-trivial schema set, and they are what let
the generator emit **named, reused types** instead of duplicating shapes. This
spec also owns the cross-file story: how a set of input files becomes one
generated package per language (the layout itself lives in
[[generated-file-layout]]).

## Spec summary

- `$ref` is a URI-reference resolved against the current base URI. Its fragment
  is a JSON Pointer (`#/$defs/Foo`) or a plain-name `$anchor`.
- In 2020-12, `$ref` **may carry sibling keywords** (unlike draft-07, where
  siblings were ignored); siblings combine with the referenced schema as an
  implicit `allOf`.
- `$defs` is an object of named subschemas. It has no validation effect on its
  own — entries are reached only via `$ref`.
- `$id` declares a base URI for a schema resource and re-bases the resolution of
  relative `$ref`s beneath it; it may also appear nested to embed a separate
  schema resource.

## Support decision

**Support: partial** — `$ref` to **named targets only**, **local files only**,
**no `$id`**; sibling keywords are accepted and merged as an implicit `allOf`
([[allOf]]).

`$defs` is **supported** as the canonical (and only) place a `$ref` may target
by name.

Rationale (citing [[PRINCIPLES.md]]):
- **External refs are local-file-only**: file refs resolve to YAML/JSON on disk
  relative to the referring file; HTTP/URI refs are rejected for
  reproducibility.
- **P14 (one file per input; merge recursion, not files)**: each input file
  becomes one generated module in a single flat package; reference cycles hoist
  the cyclic types into a shared module rather than merging whole files (see
  Recursion below and [[generated-file-layout]]).
- **P7 / P7.1 (strict schema, reject loudly)**: every accepted `$ref` must
  resolve to a **nameable, top-level generated type** — codegen has no name for
  a pointer into the middle of a schema.
- **`$ref` siblings are an implicit `allOf`**, and `allOf` is a supported
  load-time merge ([[allOf]]): a sibling-bearing `$ref` is rewritten to
  `allOf:[{$ref:X},{…siblings}]` and **merged** into a single schema. `$id`
  re-basing opens a URI-resolution surface we otherwise avoid (**P6**).
- **`x-<lang>-name` is the one exception**, because it is not a conjunct: it
  names the **member** the reference is bound to ([[properties]] Stage 4) and
  asserts nothing about the value, so merging it would *clone* the referenced
  target into the use site instead of referencing it. A `$ref` carrying nothing
  but name overrides therefore stays a plain reference with the member renamed —
  and without that, a member whose type is a `$ref` could not be renamed at all,
  leaving a member named `class` unfixable in Python and Java.

### Accepted ref forms

After JSON Pointer unescaping per RFC 6901 (`~1` → `/`, `~0` → `~`):

| Form                            | Meaning                                                                              |
| ------------------------------- | ------------------------------------------------------------------------------------ |
| `#`                             | the referring file's **root** schema                                                 |
| `#/$defs/<Name>`                | a named definition (nested `$defs` chains that terminate at a named def are allowed) |
| `<relative-path>`               | another file's **root** schema                                                       |
| `<relative-path>#/$defs/<Name>` | a named def in another file                                                          |

### Loader behavior (rejected at load time)

Each rejection carries a diagnostic naming the schema location and a fix-it:

- **Pointer into a non-`$defs` node** (`#/properties/x/items`, `#/items`, …) →
  reject. Fix-it: "extract the target into `$defs` and reference that." (Every
  ref must resolve to a nameable type — P7.)
- **Sibling keys alongside `$ref`** — accepted: the implicit-`allOf` sugar is
  rewritten to an explicit `allOf` and merged ([[allOf]]). The merged result is
  subject to `allOf`'s reject rules (e.g. a sibling that contradicts the
  target), and unresolvable/cyclic targets still reject here. Two sibling
  classes are **not** merged and leave the reference intact: an `x-<lang>-name`,
  which renames the member (above); and the **non-conjunct annotations**
  `$comment`, `examples` and [[deprecated]] — the last at **either value**,
  since it marks the member rather than asserting anything about the referenced
  value — which are dropped **before** the loader decides whether the reference
  needs an implicit merge, so they can neither clone the target into a new type
  nor add a **P15** identifier. See [[allOf]] for the same rule stated from the
  merge side, and [[deprecated]] for the member-marking rule.
- **`$id` anywhere** → reject. Fix-it: "remove `$id`; refs resolve by file path
  + JSON pointer." (local-file-only — no URI resolution.) The reject is
  **position-independent**: a root `$id`, a nested one, one on any [[allOf]]
  branch including a non-first conjunct, and one on the implicit conjunct a
  `$ref`-with-siblings forms all reject alike. A merge must not drop it as
  unrecognized on the branch it did not happen to read first.
- **HTTP/URI ref**, `$anchor` fragment, `$dynamicRef`, `$dynamicAnchor` → reject
  (local-file-only / P6). These anchor and dynamic-reference keywords are owned
  by this spec — see Anchors & dynamic references below.
- **Unresolvable target** (missing file, missing `$defs` entry) → reject. (`..`
  segments are resolved, not rejected — see Resolution.)
- **`$defs` outside a model position** → reject. A `$defs` bucket belongs on a
  document root, a `$defs` entry, or a nested `$defs` chain — the positions
  whose entries become named models. Under an ordinary subschema (a property, an
  `items`, a [[oneOf]] branch, an `additionalProperties` schema, a [[contains]]
  matcher) it declares nothing any `$ref` can reach, so accepting it silently
  discards whatever it holds — including keywords the subset rejects outright
  and an invalid `type`, all of which reject one level up. Nor is such a bucket
  inert: a cross-file `$ref` inside it still joins the input closure and renames
  every emitted module. Fix-it: "move the definition to the document's `$defs`."
- **Absolute-path `$ref`**, and a `$ref` that raises the input root above the
  **invocation root** → reject (see Resolution, where both carry a diagnostic
  with the authored schema location and remedy).

### Anchors & dynamic references — rejected

The 2020-12 anchoring keywords all name a target by some route **other** than
the `$defs` key + JSON Pointer this spec is built on, and each opens a
resolution surface the strict subset avoids (**P6**). They live here, alongside
`$id`, because they are reference-mechanism variants — not standalone
applicators — and share one rationale:

- **`$anchor`** declares a plain-name fragment (`{"$anchor":"foo"}`, referenced
  as `#foo`). It is a *second* way to name a subschema that is not the `$defs`
  key our type-name derivation reads (see Type-name derivation), and in practice
  travels with `$id` re-basing. Accepting it would fork naming into two
  mechanisms with no added expressiveness → **reject**. Fix-it: "put the target
  in `$defs` and reference it by `#/$defs/<Name>`."
- **`$dynamicRef` / `$dynamicAnchor`** resolve *at validation time* against the
  dynamic scope — the same `$dynamicRef` binds to whichever `$dynamicAnchor` is
  outermost on the current evaluation path, so it denotes
  **different schemas for different instances**. There is no single static
  target to lower to a named type at codegen time (**P6/P7** — no decidable
  typed lowering) → **reject**. Fix-it: "use a static `#/$defs/<Name>`
  reference; the generated types are resolved once, not per-instance."

## Resolution & the input set

- A relative path resolves against the **referring file's directory** (standard
  base-URI behavior, minus URIs) and is **normalized to an absolute path** —
  `..` segments are *resolved*, not rejected. `a/b/x.json` containing
  `$ref: "../y.json"` resolves to `a/y.json`.
- The **input set** is the transitive closure of local refs reachable from the
  entry file(s)/directory the CLI/API is given, each resolved to its absolute
  path. Every reachable file becomes a per-input output module.
- The closure follows a `$ref` **only where a `$ref` is a schema keyword.** A
  dead `$defs` entry counts — it is generated API surface, so its dependencies
  must be in the set — but a `$ref`-shaped key inside a *value* is data, not a
  reference: an object with a `$ref` member appearing in [[examples]], a
  [[default]], a [[const]] or an [[enum]] contributes **no** closure edge.
  Treating it as one enlarges the input set, raises the input root and renames
  every emitted module, which is the opposite of the inertness those keywords
  promise.
- The **input root** is the absolute path of the directory common to all
  resolved input files (their longest common-ancestor directory), computed
  **after** `..` normalization. Module names derive relative to the root (see
  [[generated-file-layout]]); because they are relative to the common ancestor
  they never contain `..`.
- **The root is bounded below by the invocation root** — the common-ancestor
  directory of the entry paths the CLI/API was given (the directory itself when
  one directory is given; the containing directory when one file is). A `$ref`
  that would raise the input root **above** the invocation root is a load
  reject, naming the reference and the directory it escaped to. Without that
  bound a reference can lift the root to an ancestor of the checkout, at which
  point the emitted module tree, Java package names and Go file names encode the
  **absolute filesystem location of the working copy** — so two developers
  generating the same schema get different package names, and a committed
  artifact can never match a colleague's regeneration. An upward `$ref` that
  stays at or below the invocation root is fine and raises the input root as
  usual, so the fix-it is to widen the invocation — name the ancestor directory,
  or name the escaping target as a second entry path — which makes the root the
  author's choice rather than a consequence of where the checkout sits. The
  diagnostic names the `$ref` that escaped, never the entry file that did not
  change, and never a directory outside the working copy (**P7.2**).
- **Reproducible** means exactly that: for a given invocation root, the emitted
  module names, package names and file names are a function of the input files'
  paths *relative to it* and of nothing else — not of where the checkout lives,
  not of the machine. "Local-file-only" (no network fetch) is a separate and
  weaker property, and is not what this bullet promises.
- An **absolute-path `$ref`** is rejected: the accepted-form table above admits
  only `<relative-path>`, and an absolute reference both escapes the invocation
  root and is unreproducible by construction.
- **Dead `$defs`** (defined but never referenced) are still generated and
  exported — they are intended reusable API surface, not waste.

## Type-name derivation

Every accepted `$ref` resolves to a named top-level type. Names are derived as:

- **File root** → the normalized file **basename** (`user_profile.json` →
  `UserProfile`), overridable by `x-<lang>-name`. A root [[title]] is
  documentation only and never names the type. This is distinct from the
  *module file* name (the flattened full path — see [[generated-file-layout]]):
  `a/user.json` and `b/user.json` yield modules `a_user`/`b_user` but both
  derive type name `User`.
- **`$defs/<Name>`** → `<Name>` run through the shared 4-stage JSON-name →
  identifier algorithm ([[properties]] Identifier mapping).
- **Anonymous inline** subschemas promoted to types → existing synthesis rules
  ([[const]]/[[enum]]/[[properties]]; nest where the language allows, P15
  backstop).

A type's emitted name is resolved once for the **whole input closure**, so a
reference from another input file names exactly the identifier the declaring
file's own module emits — including its `x-<lang>-name` override, which the
referencing file does not restate.

**Collision.** For Go, TypeScript and Python all type names occupy
**one package-wide namespace** — Go flattens to a single package, and the
TypeScript and Python barrels re-aggregate every module into one. Java and .NET
resolve per module instead, so there the namespace is the module
([[generated-file-layout]]). A collision → **load reject, no mangling**; the
escape hatch is `x-<lang>-name` (**P15**, scope widened from per-object to
per-package). Consistent with [[properties]],
**collisions are evaluated per emitted target only** — a name set may be
accepted for a Go-only run and rejected for a Java run, because normalization
differs per language.

**The derived name is the model's identity.** The one collision that is *not*
per-target is a file-root type and a same-file `$defs` entry that derive the
**same** name (`thing.yaml` with a root type plus `$defs.Thing`): the derived
name is the identity every `$ref` resolves through and every target emits one
type for, so the two schemas would otherwise collapse into one — the loser's
shape dropped and every reference to it silently retargeted at the winner. It is
rejected for **every** target, and the fix-it is a rename of the `$defs` key or
of the file the root name derives from; an `x-<lang>-name` override does not
resolve it, because it moves one target's *emitted identifier* and leaves both
schemas on the one identity. A name **synthesized** for an inline shape
([[properties]]) is held to the same rule — a hoisted `$defs` entry whose name
equals the root type's is rejected where the shape is hoisted, with a diagnostic
naming the position it was written in. A file with no root type (a
definitions-only file or a Nexus document — see [[input-files]]) has no root
name to collide with.

**A sibling fold is a new declaration.** Every sibling of a `$ref` outside the
two non-conjunct classes folds ([[allOf]]), and the merged node is a standalone
type named from its **position** — the annotation text never supplies the name.
So adding a [[title]] or [[description]] beside a `$ref`, an edit that changes
neither the wire nor any validation, introduces a new package-level identifier
that can collide. Because the identifier is synthesized rather than authored,
**P7.2** applied here yields two obligations, and together they are what makes
such an edit safe to make:

- the reject must name the **position that synthesized** the identifier — the
  annotated reference — and not only the two colliding names, so the author can
  see which edit caused it;
- **every remedy it offers must resolve the reject.** "Extract the target into
  `$defs` and reference it" is no remedy when that is exactly what the author
  already wrote, and `x-<lang>-name` is one only if applying it
  **at the site the author edited** reaches the synthesized name (**P15**).
  Removing the annotation always resolves it, so a diagnostic that cannot offer
  an applicable override must offer that.

## Output layout

The full package structure (file names, the shared `definitions` file, the
Python-only `_recursive.py`, the `__init__.py`/`index.ts` aggregators, single-
vs multi-input collapse, the flattened path-to-module encoding) is specified in
**[[generated-file-layout]]**. The `$ref`-relevant summary:

- One generated package tree per language, flat for Go and mirroring the input
  tree for Python, TypeScript, and Java; one module per input file
  ([[generated-file-layout]]).
- Reference cycles that span ≥2 input files **hoist** their strongly-connected
  types into a shared module (Python `_recursive.py`; TS/Go/Java handle cycles
  natively).
- This **refines P14**: "merge on cycle" means *hoist the cyclic types*, not
  *merge whole input files*.

## Recursion & satisfiability

**Reference graph.** Nodes = named types (file roots, `$defs`, promoted
anonymous). Edge A→B whenever A's schema `$ref`s B (in a property, `items`,
`additionalProperties`, …). Compute strongly-connected components (SCCs); a
non-trivial SCC (or a self-loop) is a recursion cycle. A cycle spanning ≥2 input
files is the cross-file SCC that hoists to `_recursive.py` (Python only — see
[[generated-file-layout]]).

**Per-language emission of a cyclic back-edge:**

| Language       | Recursive field                                  | Why                                                                                                                                                                                                              |
| -------------- | ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Go**         | pointer `*T` (even when required + non-nullable) | a bare recursive `T` is an infinitely-sized struct (compile error); `[]T`/`map[string]T` already carry indirection                                                                                               |
| **Java**       | bare reference                                   | object fields are references already; recursion is free                                                                                                                                                          |
| **TypeScript** | bare reference                                   | interfaces reference themselves/each other freely                                                                                                                                                                |
| **Python**     | bare reference                                   | `from __future__ import annotations` makes every annotation a string that is never evaluated, so a dataclass references itself and its cycle peers freely; the emitted order is topological only for readability |

**Satisfiability check.** A recursion cycle has a finite instance only if
**at least one edge in it can terminate**. An edge *terminates* when it is:
- **optional** (member not in `required`) — absence ends the chain;
- **required + nullable** (the [[nullability]] `oneOf` null pattern) — `null`
  ends it;
- **collection-wrapped** (`items` / `additionalProperties` valued by the `$ref`)
  — the empty array/object ends it.

If **every** edge in a cycle is mandatory-and-single-valued (required +
non-nullable + not collection-wrapped), no finite instance exists →
**load reject**. The diagnostic names the cycle path (`A → B → A`) and the three
fixes (make an edge optional, nullable, or wrap it in an array). A direct
required-non-nullable self-ref (`{"$ref":"#"}` member) is the degenerate case.
This implements the rows [[properties]] reserved ("unsatisfiable direct
self-reference"). The check is decidable and conservative — it never rejects a
satisfiable schema.

## Per-language `$ref` emission

A field `{"x": {"$ref": "#/$defs/Foo"}}` emits a field `x` of type `Foo`.
Optional/nullable wrapping layers on top per [[nullability]]; the
recursion-pointer rule above applies to cyclic edges. Imports follow
[[generated-file-layout]]:

- **Python** — `from .b import Foo`, `from ._recursive import Node`, plus
  imports of the shared `Violation` and payload-validation helper from the
  package's private `_definitions` module where validation is emitted.
- **TypeScript** — `import type { Foo } from './b'`, plus
  `import { fooTransferTypeConverter } from './b'` since the referencing type's
  converter delegates to the target's (PRINCIPLES TS §4).
- **Go** — same flat package; no import.
- **Java** — same-package references need no import in a single-input run;
  cross-file references import the target from its per-input sub-package.

**Bare-`$ref` alias.** A **schema that is exactly `{"$ref": <target>}`** — a
file root or a `$defs` entry — is an **alias** for the target: every position
that references the alias denotes the target's shape, including an operation's
`input`/`output` ([[services]]), and no shapeless type is emitted. Where the
language supports an alias declaration it is emitted as one:

| Language       | Emission                                                                                                         |
| -------------- | ---------------------------------------------------------------------------------------------------------------- |
| **Go**         | `type A = Main` (alias; fully interchangeable)                                                                   |
| **TypeScript** | `export type A = Main`                                                                                           |
| **Python**     | `A = Main` (module-level alias)                                                                                  |
| **Java**       | **no alias** — every reference to the bare-ref schema resolves directly to the target `Main`; no synthesized `A` |

The Java asymmetry is cosmetic and *safe by construction*: since `A` is nothing
but another name for `Main`, collapsing all references to `Main` yields one
interchangeable type at every site — and it is what makes the operation-I/O gate
sound: whatever proves the alias object-shaped is the target's shape, so the
operation must be typed on the target, never on a distinct shapeless `A`.
Subclassing (`A extends Main`) is **rejected** as the Java realization — it
would require dropping `final` from value types, a duplicated per-class
collecting deserializer, and would split reference sites into incompatible
`A`-typed and `Main`-typed slots (a `Main` value cannot be assigned to an `A`
field).

## Validator / serializer (P12)

`$ref` is pure delegation: validating a field typed `Foo` calls `Foo`'s own
shared `Validate` (mirror-image on both directions). Composite types recurse
into their referenced types' validators; for cyclic types the recursion is
bounded by the (finite) data. No `$ref`-specific runtime helper is emitted — the
named-type machinery already in place ([[type]], [[properties]]) does the work.

## Property-testing matrix

### Accepted (positive)

| Case                               | Form                                                                                                                       |
| ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| Same-file named def                | `{"$ref": "#/$defs/Address"}`                                                                                              |
| Whole-document root                | `{"$ref": "#"}` (terminating edge present)                                                                                 |
| Cross-file root                    | `{"$ref": "common.json"}`                                                                                                  |
| Cross-file named def               | `{"$ref": "common.json#/$defs/Money"}`                                                                                     |
| Pointer-escaped name               | `{"$ref": "#/$defs/foo~1bar"}` → def `foo/bar`                                                                             |
| Direct self-ref, optional          | `{value:{type:string}, next:{$ref:"#"}}`, `required:[value]` (linked list)                                                 |
| Self-ref via array, required       | `{value:{...}, children:{type:array, items:{$ref:"#"}}}` (tree)                                                            |
| Mutual cross-file cycle            | `a.json#/X` ↔ `b.json#/Y` with a terminating edge → hoisted to `_recursive` (Py)                                           |
| Dead `$defs`                       | a `$defs` entry never referenced → still emitted/exported                                                                  |
| Bare-`$ref` root                   | file root `{"$ref":"#/$defs/Main"}` → alias (Go/TS/Py), `Main` (Java)                                                      |
| Bare-`$ref` `$defs` entry          | `$defs.Alias: {"$ref":"#/$defs/Target"}` → alias for `Target`, never an empty model                                        |
| `$ref` with siblings               | `{"$ref":"#/$defs/X", "minProperties":1}` → implicit `allOf`, merged ([[allOf]])                                           |
| `$ref` with a non-conjunct sibling | `{"$ref":"#/$defs/X", "$comment":"note"}`, `{"$ref":"#/$defs/X", "deprecated":true}` → reference kept, no synthesized type |

### Rejected at load time (negative)

| Case                                         | Reason                                                                                                                                                            |
| -------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pointer into non-`$defs`                     | `{"$ref": "#/properties/x/items"}` — not nameable (P7)                                                                                                            |
| `$id` present                                | any position — root, nested, or an [[allOf]] conjunct — no URI resolution (local-file-only)                                                                       |
| HTTP ref                                     | `{"$ref": "https://example.com/s.json"}` — not local (local-file-only)                                                                                            |
| Absolute-path ref                            | `{"$ref": "/abs/other.json"}` — only `<relative-path>` is an accepted form                                                                                        |
| Ref escaping the invocation root             | a `$ref` that raises the input root above the entry path — the layout would encode the checkout's location                                                        |
| `$defs` outside a model position             | a `$defs` bucket under a property, `items`, `additionalProperties`, a [[oneOf]] branch or a [[contains]] matcher — unreachable by any `$ref`                      |
| `$anchor` / `$dynamicRef` / `$dynamicAnchor` | not in subset (P6) — see Anchors & dynamic references                                                                                                             |
| Unresolvable                                 | missing file or missing `$defs` entry                                                                                                                             |
| Unsatisfiable cycle                          | every edge required + non-nullable + single-valued                                                                                                                |
| Type-name collision                          | two targets → same identifier in an emitted language (per-target, P15)                                                                                            |
| Root/`$defs` name coincidence                | a file-root type and a same-file `$defs` entry — authored or synthesized for an inline shape — derive the same name: one identity, two schemas (all targets, P15) |
| Module-name collision                        | two inputs flatten to the same module name ([[generated-file-layout]])                                                                                            |

### Runtime fixtures (validator)

- A valid nested instance round-trips: parse → validate (delegated to the
  referenced type) → serialize, unchanged.
- A recursive instance (linked list / tree) of arbitrary finite depth validates;
  the terminating edge (absent / `null` / empty array) ends the chain.
- An invalid value at a referenced position pushes a `Violation` whose path
  includes the nested location, aggregated with sibling errors (P11).

## Interactions

- **[[properties]]** — a member schema may be a `$ref`; the recursion matrix
  rows there are realized here. Type-name synthesis shares the identifier
  algorithm.
- **[[nullability]]** — optional/nullable wrapping of a `$ref` field; required +
  nullable is a terminating edge.
- **[[required]]** — owns which `$ref` edges are optional (a primary source of
  cycle termination).
- **[[const]]** / **[[enum]]** — a `$defs` whose name a synthesized type reuses
  (the Go defined type / Java value class) enters the same per-package namespace
  (P15).
- **[[additionalProperties]]** — a `$ref`-valued catch-all/typed map is a
  collection-wrapped (terminating) edge.
- **[[generated-file-layout]]** — owns the package structure this spec
  references.
- **[[services]]** — an operation's `input`/`output` is a `$ref` to a `$defs`
  type or an inline schema promoted to a synthesized `<Op>Input`/`Output` type;
  both join this spec's reference graph and type-name namespace.

## Ecosystem variance

| Source                                            | Handling                                                                                                                                                                                                                                                                                                                               |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| draft-07 (`$ref` siblings ignored)                | siblings are **merged** as an implicit `allOf` per 2020-12 ([[allOf]]), not dropped — a stricter, more faithful reading (the one cross-draft behavior change), the two non-conjunct classes above excepted. A draft-07 author who intended the siblings as dead keys will see them take effect; remove them to restore the old meaning |
| `definitions` (draft-07 keyword)                  | not recognized; require `$defs`. Diagnostic suggests renaming `definitions` → `$defs`                                                                                                                                                                                                                                                  |
| `$id`-rebased refs (OpenAPI/JSON-Schema bundlers) | reject; the input must be a flat local-file tree resolvable by path + pointer                                                                                                                                                                                                                                                          |
| `$anchor` / `$dynamicRef`                         | reject (P6); not in the subset                                                                                                                                                                                                                                                                                                         |

## See also

- [[input-files]] — per-file document modes and root rules (the `$id` reject
  restated here, the `$schema` dialect, the Nexus-document envelope) that
  precede the input-set/closure computed here.
- [[generated-file-layout]] — the output package structure this spec references
  (file names, shared `definitions`, `_recursive`, aggregators, flattening,
  single-vs-multi-input).
- [[properties]] — `$ref` members + the recursion-termination rows + the shared
  identifier/collision algorithm.
- [[nullability]], [[required]] — optional/nullable wrapping and cycle
  termination.
- [[type]] — the named-type emission `$ref` delegates to.
- [[services]] — operation `input`/`output` `$ref` and inline-type promotion.
- [[PRINCIPLES.md]] — **P6** (strict subset), **P7/P7.1** (reject loudly),
  **P7.2** (a reject names the cause and a remedy that works), **P14** (one file
  per input; merge recursion, not files), local-file-only external refs, **P15**
  (one identifier namespace per scope).
