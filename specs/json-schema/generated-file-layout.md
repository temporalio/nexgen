# Generated file layout (cross-cutting design note)

Not a JSON Schema keyword. Specifies how a set of input schema files
becomes generated source in each language: the package structure, file
names, where shared runtime lives, and how reference cycles and name
collisions are handled at the file level. Driven by **P14** (one module
per input file; merge recursion, not files) and local-file-only external
refs; the reference semantics that feed it live in [[ref]].

## Output mirrors the input directory tree (Python / TypeScript / Java)

Each input schema file `<subpath>/<name>.<ext>` becomes a **per-input
module directory** `<subpath>/<name>/` under the output package root,
preserving the input directory structure verbatim — **directories are not
flattened**. Inside each per-input directory:

- `models.<lang>` — the domain types declared in that file (its root type
  and every `$defs`).
- `services.<lang>` — the Nexus service bindings declared in that file,
  emitted **only if it declares any** (see [[services]]).

Schema-independent runtime intended for cross-model reuse — `Violation`, any
target-owned payload-validation helper, and each target's shared
numeric or (de)serialization helpers — is defined **once** at the package root
(the shared `definitions` file, below), never duplicated per module. A target
may still inline a leaf check when it has no shared runtime primitive for it;
TypeScript's safe-integer predicate is one such check.

Nesting works because each of these languages maps an input directory onto
its native module unit:

- **Python** — a subpackage directory carrying an `__init__.py`.
- **TypeScript** — a directory with an `index.ts` barrel.
- **Java** — a package mirroring the directory, one `.java` per public
  class.

Cross-directory mutual references are supported: Python hoists the cyclic
types into `_recursive.py` (Recursion, below), TypeScript relies on
cycle-safe `import type` and call-time live bindings, and Java on object
references. Because each input file's types live in its own module, type names
need to be unique only within that module for Java. Python and TypeScript root
barrels re-aggregate module exports, so their names must be unique across the
whole run (see Collisions and [[ref]]).

## Go flattens to one package

**Go is the exception.** Every input file in a generator run collapses
into **one flat package** — nested input directories are not mirrored. A
Go package cannot participate in an import cycle, so if two directories
became two packages a **cross-directory mutual reference** would be an
illegal cyclic import, and Go has no cross-package hoist like Python's
`_recursive`. Flattening to a single package makes every reference —
including a cross-directory cycle — a **within-package** reference, which
the Go compiler resolves natively. It is also what lets the
schema-independent boilerplate be defined **once** in that package instead
of per file.

Within the one package, each input file emits a `<module>.go` whose name
is its flattened path (Module paths, below), plus a shared `definitions.go`.
Because everything shares one namespace, type and module names cannot collide
across files — the loader, the generation planner, or file assembly rejects such
collisions, depending on the family (see Collisions).

## The single-input special case

When the closure is **exactly one input file**, there is no directory tree
to mirror: the file's output lands **directly at the package root** rather
than in a per-input subdirectory. A single `chat.yaml` → package `chat/`
holding `models.py`, `services.py`, the shared `_definitions.py`, and the
`__init__.py` aggregator side by side. No per-input subdirectory, and no
`_recursive` (a cross-file cycle is impossible with one file). The
shared-runtime file is still separate in every language, Go included: Go always
emits the one input's `<package>.go` alongside its own `definitions.go`, the
same shared-runtime layout it uses for multi-input closures. The
models/services split itself is a Python/TypeScript property; Go places both in
one file and Java emits one file per class.

## Files per language

### Multi-input (≥2 input files in the closure)

Per input file `<subpath>/<name>`:

| Language | Per-input module | Shared runtime (once) | Recursive module | Aggregator |
|---|---|---|---|---|
| **Python** | `<subpath>/<name>/models.py` (+ `services.py` if it declares services) | `_definitions.py` (package root) | `_recursive.py` (package root) | `__init__.py` per directory — the per-input directory, every intermediate directory, and the package root |
| **TypeScript** | `<subpath>/<name>/models.ts` (+ `services.ts`) | `definitions.ts` (package root) | — | `index.ts` per directory (barrels chain upward) |
| **Go** | `<module>.go` in the one flat package (`<module>` = flattened path) | `definitions.go` (same package) | — | — (capitalized = exported) |
| **Java** | one `<ClassName>.java` per exported class plus `package-info.java` in each package containing generated classes, in a package mirroring `<subpath>/<name>/` | each runtime class its own file in the root package (`Violation.java`, `SpecNumbers.java`, …), plus root `package-info.java` | — | — (`public` = exported) |

**A module emits only the types its own input file declares.** A type reached by
`$ref` into another file belongs to the module that declares it, and is imported
from there — never re-emitted into the referencing module, which would put a
second copy of the type in the package (a redeclaration in Go's flat package, a
duplicate class file in Java, an import-versus-local-declaration conflict in
TypeScript, and a shadowed re-import in Python). A service file whose every
operation type is `$ref`d from elsewhere therefore declares nothing of its own,
and emits no models file at all — for TypeScript that also means its `index.ts`
does not re-export `./models`, since a barrel re-exporting a file with no exports
is itself an error (`TS2306`).

**`_recursive` is Python-only and is a single file at the package root**
(`<package>/_recursive.py`), **never** per-input. It holds every hoisted
cross-file SCC in the whole closure. See Recursion below.

### Single-input (exactly one input file — no cross-file refs possible)

All output lands at the package root (no per-input subdirectory, no
`_recursive`):

| Language | Output |
|---|---|
| **Python** | `models.py` (+ `services.py`), the shared `_definitions.py`, and the `__init__.py` aggregator — the same split as multi-input, flattened to the package root |
| **TypeScript** | `models.ts` (+ `services.ts`), `definitions.ts`, `index.ts` |
| **Go** | one `<package>.go` (types and services) + the shared `definitions.go` |
| **Java** | one `.java` per public class + the runtime classes and root `package-info.java`; nothing to aggregate |

Java reserves the runtime class identifiers `Violation`, `SpecNumbers`,
`TemporalSupport`, and `Base64Support` in every module scope. The temporal and
base64 helpers remain reserved even when no schema in the run requires their
class to be materialized.

Go derives `<package>` by converting the `--output` directory basename to a Go
package identifier. Because that model file shares the directory with
`definitions.go`, a single-input output whose **derived identifier** is
`definitions` is rejected with both generated-file sources and the remedy to choose an
output deriving a different identifier. This includes basenames such as
`Definitions` and `definitions-`, not only the literal `definitions`.

For Java, in both layouts, the package root is the required `--package-name`.
Its final dot-separated segment must match the output directory name. A
`package-info.java` carrying `@NullMarked` is emitted for each package that
contains generated classes; class-less intermediate directories are not Java
packages and receive no file.

## The shared `definitions` file

Holds the schema-independent runtime, defined once per package
(`_definitions.py` / `definitions.ts` / `definitions.go`; Java splits it into
one class file each). For Python/TypeScript/Java it sits at the package root;
for Go it sits in the one flat package, always as its own `definitions.go`
file regardless of how many input files that package aggregates. Python's
file is underscore-prefixed, the language's own marking for a module that is
generator-internal rather than part of the package's surface.

- Payload-validation failure construction — each target raises the SDK's
  non-retryable application failure (`ApplicationFailure` in TypeScript/Java,
  `ApplicationError` in Python/Go) with message `Payload validation failed`,
  type `PayloadValidationError`, and the language's `Violation` list as its
  first detail. Python calls
  `temporalio.converter.create_payload_validation_error` directly and requires
  Temporal SDK 1.32.0 or newer. TypeScript calls
  `createPayloadValidationError` from `@temporalio/common` and requires Temporal
  SDK 1.23.0 or newer. The generated Go helper carries a TODO to use its SDK
  factory once that factory is available in a supported release. Java inlines
  the application-failure call with the equivalent TODO at each throw site.
  Nested aggregation casts the
  locally retained first detail back to the generated violation-list type; the
  cast is cheap and performs no serialization (P11).
- Spec-number helpers — `parseSpecInteger` (Go), `_parse_spec_integer`
  (Python), and `SpecNumbers.specLong` (Java). TypeScript inlines the
  `Number.isSafeInteger` intrinsic at each use site rather than wrapping it in a
  shared helper.
- Shared (de)serialize scaffolding — the **P12** three-layer base: the
  temporal and content-encoding parse/format helpers, the constraint
  checks both directions call, and the helper that re-paths a nested
  violation list under its parent field (Python `_collect`, TS
  `collect`). Python additionally keeps its `_transfer_type_convertible`
  decorator shim here, so the value-type erasure each model registers
  through is written once rather than per model. The per-model conversion
  machinery stays with its type —
  Python's transfer-type converter, Java's collecting (de)serializer —
  but the shared `Violation`, payload-validation helper, and `SpecNumbers`
  runtime live here.

## Module paths

The **input root** is the absolute path of the directory common to all
resolved input files (their longest common-ancestor directory). `$ref`
paths are resolved to absolute paths first — `..` segments are normalized,
not rejected ([[ref]]) — so a `$ref` that walks upward raises the common root,
**up to the invocation root**: the common-ancestor directory of the entry paths
the run was given. Raising it above that is a load reject, because every module
path, Java package name and Go file name below would otherwise encode the
checkout's absolute location. [[ref]] owns the bound, its fix-it, and its
implementation status.

**Python / TypeScript / Java** preserve each input file's directory path
**relative to the input root** verbatim under the package root, with the
filename (minus extension) becoming the leaf per-input directory:

```
input root = /abs/schemas   (common ancestor of all resolved inputs)

/abs/schemas/kb.yaml               -> kb/            (models.py, services.py)
/abs/schemas/content/block.json    -> content/block/ (models.py)
/abs/schemas/tree/category.json    -> tree/category/ (models.py)
```

"Minus extension" strips `.json` / `.yaml` / `.yml` **and then a trailing
`.nexusrpc` infix**, so `chat.nexusrpc.yaml` is the module `chat`, not
`chat.nexusrpc` — the convention marker names the file, not the generated
package, and a `.` in a module segment would be unspellable as a Python or
Java package name anyway. The same stripping produces the file's derived
root type name. A consequence worth stating outright: `chat.yaml` and
`chat.nexusrpc.yaml` in one closure map to the **same** module path and
collide (a load reject, per *Collisions* below); a closure may name a
module once, in one spelling.

Because the directory structure is mirrored rather than collapsed, there
is no delimiter to escape and no path-encoding scheme: literal
underscores, nested directories, and repeated segments all survive
unchanged.

**Go** flattens each file's path (relative to the input root, minus
extension) into a single `<module>.go` name, replacing the directory
delimiter with `_` and **preserving literal underscores verbatim** — no
escaping. Because module names are relative to the common ancestor, they
never contain `..`:

```
/abs/schemas/full_name.json       -> full_name.go
/abs/schemas/a/b/user.json        -> a_b_user.go
/abs/schemas/billing/invoice.json -> billing_invoice.go
```

Type names are directory-independent (basename-of-root or `$defs` name —
see [[ref]]), so they do not encode the path.

### Why Go does not escape

Identifiers are `[A-Za-z0-9_]`: two structural things (the directory
separator and a literal `_`) must encode into one non-alphanumeric
character. Injective + flat + underscores-preserved is impossible without
escaping. We choose **readable + collision-reject** over an escaping
scheme: keep names clean and reject the rare collision, consistent with
the **P15** "reject loudly, never mangle, offer an override" stance used
everywhere else. Go's flattened `.go` files are internal organization
anyway — consumers import types from the package by name — so a rare
reject costs little.

## Collisions

**Go** — one unified namespace per package holds the **reserved generated
names** (the union across languages — the per-scope list is below, and it is the
same list in every target) plus one entry per input module. Any collision in
that namespace is rejected — during
loading, planning, or emission, depending on the collision family; a flattened
generated-file conflict is detected at emission. A per-input
`x-output-module` override is a
*possible* future escape hatch and is **not implemented** — nothing in the
generator reads such a keyword today, and there is no per-module escape
hatch at all (`x-<lang>-name` names types and members, not modules):

- two inputs flattening to the same module (`full/name` vs `full_name`);
- an input flattening onto a reserved generated name (a root-level
  `definitions.json`);
- a **type name** declared by two different input files — `a/page.json`
  and `b/page.json` both emitting `Page` is one redeclaration in the flat
  package, so Go's collision scope is the whole run rather than the module
  (see [[ref]] and PRINCIPLES §15); the diagnostic names both modules.
  TypeScript and Python reject the same closure, by way of their barrels
  rather than a flat package — see below;
- a generated **service binding** colliding with a model (or synthesized
  I/O) type — service `ChatService` against a `$defs/ChatService`; see
  [[services]], which shares this one namespace.

**Python / TypeScript / Java / .NET** — nesting keeps distinct input files in
distinct modules, so files no longer contend for one flat *module* name. What
remains is a small set of **reserved generated names** per scope; an input
file or directory that maps onto one → **load reject** naming that input and a
rename remedy:

- at the **package root**: the shared runtime module — **both** spellings,
  `definitions` (Go/TypeScript) and `_definitions` (Python) — plus `_recursive`
  (Python);
- at **every** directory level, root or intermediate: that directory's own
  aggregator, `__init__` and `index`. The reservation cannot be scoped to the
  root, because it is the *parent* directory's barrel that breaks: a parent
  emitting `export * from './index'` resolves it to the parent's own `index.ts`,
  so an intermediate directory named `index` drops its whole subtree off the
  package surface with **no diagnostic at the barrel** — the silent
  incorrectness P7 forbids. `from .__init__ import …` self-resolves the same
  way;

`models` and `services` are not reserved module segments. Those generated files
live *inside* a per-input leaf directory, and the file-vs-directory rule below
means that leaf can never also contain a child module; a directory named
`models` therefore does not contend with its own `models.py`.

Module-name reservation is deliberately **target-independent**: every name above
is reserved in every target — the union across languages — so an input named any
of them rejects regardless of which target is being generated. The rationale is
specific to **module names**, and does not generalize to identifiers. A module
name comes from the input file's path, not from any target's recasing, and one
input set has to yield a coherent layout in all four targets, so the reservation
is settled once over the union. Identifier validity is the opposite case and
legitimately does vary per target ([[properties]]): a member name may reject for
one target and load for the other three. What may never vary per target is the
accepted and rejected **wire** value set.

Two further module-level load rejects belong to this namespace:

- a module segment that is not a valid identifier, or that is a **reserved word
  in any of the four targets** (`class.json`, `2fa.json`) — target-independent
  for the same reason;
- an input file whose module path conflicts with a **directory** of the same
  path (`foo.yaml` beside `foo/bar.yaml`), which is why a per-input directory
  never has children — and therefore why `models` / `services` can only ever
  name a per-input directory, not a file inside one.

For **Go**, `<package>` — the flat package's name and, in the single-input case,
its `<package>.go` file name — is the basename of the output directory, so the
output directory name joins the reserved namespace: a single-input run into a
directory named `definitions` would emit that name twice and is a reject naming
the output directory, with the remedy "point `--output` at a differently named
directory".

Distinct modules do **not**, however, buy every target a distinct *type*
namespace, because two of them re-aggregate:

- **TypeScript and Python** emit a root barrel that lifts every module's
  top-level names into one namespace — `index.ts` re-exports each module with
  `export *`, `__init__.py` re-exports them by name — so a type name declared
  by two input files collides there and is rejected run-wide, exactly as in
  Go's flat package. Left to the target, TypeScript rejects the barrel
  (`TS2308`, "has already exported a member named `Page`") and Python silently
  binds whichever import runs last, dropping the other model off the package
  surface — the silent incorrectness P7 forbids. The barrel is the *mechanism*
  that makes the scope run-wide; which names enter it — declared, imported or
  synthesized — is **P15**'s, and a name synthesized beside a type has no
  narrower scope than the type itself.
- **Java and .NET** give each module its own sub-package/namespace
  (`com.example.api.content.page`, `Nexgen.Generated.Content.Page`) and emit
  no aggregating barrel, so two input files may each declare a `Page` and stay
  unambiguous. Their scope really is the module.

Within a single module's exported namespace the type/service/synthesized-name
collision surface is unchanged (service `ChatService` in `services.py`
against a `$defs/ChatService` in `models.py` share the per-input
directory's one exported namespace — see [[services]]; synthesized names
per [[ref]]).

Like [[properties]], **identifier** collisions are evaluated **per emitted
target only** — normalization differs per language. Module-name reservation is
the exception, and target-independent, for the reason given above.

Every reject above owes the author a working remedy: the remedy the diagnostic
names must satisfy **P7.2**, whichever stage raises the reject — load, planning
or emission — and where that remedy is a name override, **P15** governs which
name it has to move. What this section adds is the *input* each diagnostic must
identify. A
module-level reject names the input file or files it is about, not a derived
name: the same-module-path reject names **both** colliding inputs rather than
the module segment they share, and the flattened generated-file conflict names
both inputs. A reject caused by a `$ref` — a reference that raises the input
root, or pulls a colliding file into the closure — names that reference, not
only the entry file, which may not have changed at all.

Generated-file assembly retains a classified origin for every path. When two
paths collide, the diagnostic names both origins and derives a separate
path-changing action from each mutable origin. A fixed generated artifact
contributes no rename action, so a mutable/fixed collision offers only the
author-controlled change; a fixed/fixed collision is reported as an internal
generator defect naming both artifacts.

## Recursion: hoist types, not files

This is the file-level realization of **P14**. "Merge on cycle" does
**not** mean merge whole input files — it means hoist only the cyclic
types:

- Build the reference graph and its strongly-connected components
  ([[ref]]). An SCC spanning **≥2 input files** is a cross-file cycle. The
  graph is built over the resolved model names, already decoded — a `$defs` name
  carrying an RFC 6901 escape (`a~1b`) keeps its edge, and a missed edge is a
  missed cycle. Synthesized operation `<Op>Input`/`<Op>Output` types
  ([[services]]) are ordinary nodes in it and can carry a cross-file cycle like
  any declared type.
- **Python**: the cross-file SCC moves wholesale into `_recursive.py` at
  the package root, where it becomes a within-module cycle (topological
  order + a forward-ref back-edge in the annotation). It imports the
  leaf, non-cyclic types it needs from the per-input modules, and
  **nothing imports back into it from below**: the hoisted classes are
  re-exported by the **package-root `__init__.py` only**. A per-input
  module — or its `__init__.py` — importing them back would recreate
  exactly the cycle the hoist exists to break (`_recursive` imports
  `content.page.models`, whose package `__init__` would in turn import
  `_recursive`). So a hoisted type is reached as `from kb import Page`,
  never `from kb.content.page import Page`, and the cross-module import
  cycle is gone. The hoist is what makes the cycle
  tractable: a per-input module names its siblings' classes in
  **module-level `import` statements**, which is a real Python import
  cycle no annotation treatment can defuse. A cycle **within** a single
  file stays in its module.

  Two obligations follow from "wholesale", and both are load-bearing:

  - **The hoist is closed.** Every member of the SCC leaves, not only the
    endpoints of the cross-module edges — a member whose own in-edge and
    out-edge are both intra-module is still a member. And `_recursive`'s leaf
    imports must not name a module that itself references a hoisted class:
    either that leaf is hoisted too, or the reference is deferred. Otherwise
    the two modules import each other at module scope and the package cannot
    be imported at all.
  - **Every reference to a hoisted type carries an import**, including from a
    referrer left in the module the hoisted type was moved *out of*: a
    reference that is same-module in the authored schema is cross-module after
    the hoist. Because
    `from __future__ import annotations` defers annotations, a missing import
    is not an `ImportError` at load; it is a `NameError` on the first parse or
    serialize of the referring model, which no import-only or compile-only gate
    can see.
- **TypeScript**: no recursive file. Type references erase
  (`import type` is always cycle-safe), and the imported *values* — a
  sibling model's transfer type converter, a validator function — are ESM
  live bindings read at call time, not module-init. A converter is
  instantiated at module-init, but the sibling converters it delegates to
  are named only inside its method bodies, so a mutually-referencing pair
  initializes in either order. Every other const emitted by a **model module**
  is a self-contained leaf literal. A service module's `nexus.service(…)` const
  reads model converters at module-init, but model modules never import service
  modules, so service modules cannot join a model cycle. Thus there is no
  init-order hazard, and cyclic types stay in their per-input modules.
- **Go**: no recursive file — the single flat package makes every cycle
  (same-file or cross-directory) a within-package reference, which Go
  resolves natively. This is exactly why Go flattens (above).
- **Java**: object references handle cycles natively across packages. No
  recursive file.

**Python: annotations are lazy, union assignments are not.** Generated
modules open with `from __future__ import annotations`, so every
annotation is stored as a string and never evaluated — a dataclass field
may name a class defined later in the same module, and a *class* cycle
therefore needs no fix-up step of any kind once the SCC shares a module.
A named union is the exception, because it is an assignment rather than
an annotation: the right-hand side of
`Note: typing.TypeAlias = TextNote | LinkNote` is an ordinary expression
evaluated when the module runs, so its members must already exist and
**unions are emitted after every class they reference**. That ordering
constraint is the only intra-module one; it is orthogonal to the
cross-module hoist above.

## Exports / visibility

*Public* = every top-level named type (file roots + all `$defs`,
including dead ones; anonymous types stay nested) plus each file's service
bindings:

- **Python** — each per-input `__init__.py` re-exports its module's public
  types (and services) via `__all__` — **except** types hoisted into
  `_recursive.py`, which would reintroduce the broken cycle; each
  intermediate directory's `__init__.py` re-exports its children; the
  package-root `__init__.py` re-exports the whole tree, and is the **only**
  place the hoisted types surface, pulled from `_recursive`. The
  package root also exports the shared `Violation`; generated model modules
  call the SDK's payload-validation factory directly.
- **TypeScript** — `index.ts` per directory: per-input barrels
  `export … from './models'` (and `./services`), intermediate barrels
  `export * from './<child>'`, and the root barrel re-exports the tree plus
  the `Violation` type from `./definitions`. Runtime helpers such as `collect`
  remain internal to the generated modules; model modules call the SDK's
  payload-validation factory directly.
- **Go** — no aggregator; capitalized identifiers are exported from the one
  flat package.
- **Java** — `public` class per file; runtime classes public too.

## Service bindings

A Nexus service ([[services]]) emits into a **`services.<lang>` file in the
same per-input directory** as the models declared in that file (Python/TS),
sharing that directory's one exported namespace and the P15 collision pass
with the file's types and the synthesized operation `<Op>Input`/`<Op>Output`
types. **Go** emits the service into its flat package alongside the models
(Go has no models/services split). **Java** emits each service as its own
`<Service>.java`, like each model class. Aggregators (`index.ts` /
`__init__.py`) re-export services alongside models.

## See also

- [[input-files]] — per-file document modes (Nexus document vs pure JSON
  Schema) and root rules decided before the input-set/module computation
  here.
- [[ref]] — reference semantics, type-name derivation, recursion graph +
  satisfiability, bare-`$ref`-root alias.
- [[services]] — Nexus service/operation bindings placed by these rules.
- [[properties]] — the shared identifier/collision algorithm.
- [[nullability]] — optional/nullable wrapping (a source of cycle
  termination).
- [[PRINCIPLES.md]] — **P14** (one module per input file; merge recursion,
  not files), local-file-only external refs, **P15** (one identifier
  namespace per scope).
