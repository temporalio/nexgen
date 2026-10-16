# Input files (cross-cutting design note)

Not a JSON Schema keyword. Specifies the **document-level** concerns of a
generator input file: what a single input file *is*, the two file modes the
loader recognizes, and the root-property rules (`nexusrpc` / `$schema`) that are
decided **before** any schema processing. Like [[generated-file-layout]] and
[[pipeline]], this is a loader-level note, not a per-keyword spec, so it adapts
the `features/<keyword>.md` skeleton rather than following it verbatim.

The reference semantics that turn a *set* of these files into one package live
in [[ref]] (input set / closure) and [[generated-file-layout]] (module
computation); this doc owns only the *single-file* document concerns those build
on.

## Two file modes

Every input file is interpreted in exactly one of two modes, decided **before**
any schema processing, by the presence of a root `nexusrpc` property:

- **Nexus document** — the root carries `nexusrpc: "<version>"`. This is the
  *only* thing that enables the [[services]] extension keyword. In this mode the
  document root is a **document envelope** — its recognized members are
  `nexusrpc`, `services`, `$defs`, and optionally `$schema` / `description`. It
  is **not itself a type** and emits **no file-root type**. Any schema-shaped
  root keyword (`type`, `properties`, `additionalProperties`, …) →
  **load reject** with a fix-it ("move the type into `$defs`"). A root keyword
  that is a recognized *schema* keyword but is not an envelope member rejects on
  **its own** grounds instead — `$vocabulary` and `$id` because they are
  rejected anywhere, anything else because the envelope's member list is closed.
  None of them may be reported as a schema-shaped root: "move the type into
  `$defs`" is not their remedy, and for `$vocabulary` and `$id` it names a
  position where they are equally illegal, so following it produces a second,
  different error.
- **Pure JSON Schema** — no `nexusrpc` property. The file is an ordinary type
  schema exactly as the rest of these specs describe: its root **is a type**,
  named from the file basename ([[ref]] type-name derivation). `services` is
  **not** a recognized keyword in this mode.
  - **Definitions-only exception.** If the root carries **only** `$defs`
    (optionally alongside `description` / `$schema`) and **no** other keyword,
    it is a definitions bucket, not a type: the root is the trivial true-schema
    (validates anything), so emitting a file-root type would mean a meaningless
    `any`-typed top-level type named from the basename. In this case the
    generator emits **no file-root type** — the file contributes only its
    **non-empty** `$defs` types, exactly as a Nexus document's envelope
    contributes only its `$defs`. A root shape such as `type: object`,
    `properties`, or `allOf` leaves definitions-only mode and is emitted subject
    to the root-shape rules. A bare `$ref` is governed by [[ref]]'s root-alias
    status; a scalar `enum` root is not an admitted root shape.

The mode is a property of the file, not of the run: a single generator input set
may mix Nexus documents and pure JSON Schema files freely. Both modes contribute
types to the one output package ([[generated-file-layout]]), and a Nexus
document's `$defs` are ordinary types reachable by `$ref` from any file in the
closure ([[ref]]).

## Root properties

| Root property | Mode         | Rule                                                                                                                                                                                                                                                                                                                                         |
| ------------- | ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `nexusrpc`    | selects mode | **Presence selects Nexus-document mode and is required to enable `services`.** Accepts **exactly** the string `"1.0.0"`. Any other value — a different version (`"1.1.0"`, `"2.0.0"`, `"0.9.0"`), a malformed string (`"1.0"`, `"v1"`), or a non-string (`1`, `1.0`) → reject (P13 — see below).                                             |
| `$schema`     | both         | **Optional.** If present, must be exactly `"https://json-schema.org/draft/2020-12/schema"` (P5). Absent → 2020-12 assumed. Any other dialect URI → reject. Applies in **both** modes (a Nexus document's `$defs` are 2020-12 too).                                                                                                           |
| `$id`         | both         | **Rejected anywhere** (root or nested) — refs resolve by file path + JSON pointer, with no URI re-basing. Owned by [[ref]].                                                                                                                                                                                                                  |
| `$vocabulary` | both         | **Rejected anywhere.** A meta-schema-only keyword (it declares which vocabularies a *dialect* requires); it has no meaning in an instance-validating type schema, and the dialect here is fixed to 2020-12 (`$schema`). Presence → reject with a fix-it ("remove `$vocabulary`; the dialect is pinned to 2020-12").                          |
| `description` | both         | Optional. In pure mode it is the file-root type's doc comment, per the usual annotation handling. A Nexus envelope emits no type to carry it — *(Status: open — an envelope `description` is validated and then dropped; either it surfaces as a module-level doc comment, or the spec declares it accepted-and-inert. Neither is decided.)* |

**An explicit `null` is a value, not an absence.** `nexusrpc: null`,
`$schema: null` and `services: null` are *present* members and are judged by the
rules above, not treated as though the key were unwritten: `nexusrpc: null` is a
non-string and rejects, `$schema: null` is not the 2020-12 URI and rejects, and
`services: null` is a present `services` member, so in a file with no `nexusrpc`
it fires the stray-`services` guard. Reading any of them as absent selects the
wrong mode for the document — for `nexusrpc` it routes a Nexus document into
pure mode, where a schema-shaped root emits exactly the file-root type the
envelope rule forbids.

### `nexusrpc` — the version marker

The `nexusrpc` version property is what lets the extension format evolve without
breaking older consumers (P13). The acceptance rule is an **exact-match pin**:
the only accepted value is the literal string `"1.0.0"`. Anything else — a
different version string, a malformed string, or a non-string — rejects loudly
rather than being coerced or range-matched.

This deliberately makes **no backward-compatibility promise** across versions.
The generator does *not* treat the marker as SemVer and does *not* accept a
range: a future `nexusrpc` release reserves the right to break compatibility
freely, so a `"1.0.0"` generator must refuse any other declared version rather
than guess that it can still read the document. The exact pin is what makes that
safe — a newer document declaring `"1.1.0"` or `"2.0.0"` is rejected with a
clear fix-it ("this generator supports `nexusrpc` `\"1.0.0\"`; document declares
…") instead of being silently mis-read against the wrong rules. Each future
generator hard-pins the exact version(s) it actually implements; widening is an
explicit, per-release decision, never an assumed range.

### `$schema` — the dialect

The generator's strict subset is built on JSON Schema 2020-12 (P5). A present
`$schema` must name exactly that dialect; an absent `$schema` is **assumed** to
be 2020-12 (the overwhelming-common case for these files). Any other dialect URI
— draft-07 (`.../draft-07/schema#`), draft-04, an OpenAPI dialect — rejects with
a fix-it naming the required URI, rather than the generator silently applying
2020-12 semantics to a document authored against a different draft. This rule is
the document-level home of the dialect decision; any standalone `$schema`
keyword spec **defers to this doc**, and any standalone `$id` keyword spec
**defers to [[ref]]** (the reject is owned there) — neither restates the rule.

The companion meta keyword **`$vocabulary`** is owned here too: it belongs only
to a *meta-schema* (a schema declaring which vocabularies its dialect requires),
never to an instance-validating type schema, and the dialect is pinned to
2020-12 regardless. It is therefore **rejected anywhere** it appears (row above)
rather than parsed — there is no custom-dialect surface for it to configure
(P5/P6).

## Stray `services` guard (P7.1)

A top-level `services` key in a file with **no** `nexusrpc` property is almost
always a forgotten version marker, so it is **rejected with a fix-it** ("add
`nexusrpc: \"1.0.0\"` to enable Nexus service generation") rather than silently
ignored. This is the only place pure-JSON-Schema mode inspects `services`, and
it runs **before** any service or operation member is looked at: a file with no
marker reports the missing marker, never a member defect inside a `services` map
that mode does not recognize. No diagnostic on such a file may describe it as a
Nexus document. The same loud-reject stance ([[PRINCIPLES.md]] P7.1) that
catches a misspelled keyword anywhere else catches a Nexus document missing its
opt-in here.

## Relationship to the input set and module layout

A single input file is one node in the larger picture; the document-level rules
above are step one of the loader (Parse), before `$ref` resolution and the
strict-subset gate (see [[pipeline]]).

- The **input set** — the transitive closure of local `$ref`s reachable from the
  entry file(s) — and the **input root** (their common-ancestor directory, which
  [[ref]] bounds by the invocation root) are computed by [[ref]] after the
  per-file mode is known.
- Each reachable file (Nexus document or pure JSON Schema) becomes one per-input
  **module**, named from its path relative to the input root
  ([[generated-file-layout]] module-name encoding). A Nexus document contributes
  its `$defs` types and its `services` bindings to that module; a pure JSON
  Schema file contributes its root type and `$defs` (or, for a definitions-only
  file, its `$defs` alone — see the definitions-only exception above).
- All types and identifiers from every file — across both modes — enter the
  single per-package namespace and the P15 collision pass (P14/P15,
  [[generated-file-layout]]).

This doc does **not** duplicate the closure or module-name algorithms; see those
specs for the detail.

## Property-testing matrix

### Accepted (positive)

| Case                               | Shape                                                                                                                         |
| ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| Pure JSON Schema, no marker        | a file with `type`/`properties` and no `nexusrpc` → root is a type                                                            |
| Pure JSON Schema, explicit dialect | as above + `$schema: ".../2020-12/schema"`                                                                                    |
| Definitions-only pure file         | a non-empty `$defs` (+ optional `description` / `$schema`), no `nexusrpc` → no file-root type, contributes `$defs` types only |
| Nexus document opt-in              | root `nexusrpc: "1.0.0"` + `services` (+ optional `$schema` 2020-12, `description`)                                           |
| Nexus document with `$defs` only   | `nexusrpc: "1.0.0"` + `$defs`, no `services` (valid envelope)                                                                 |
| Dialect assumed when absent        | no `$schema` → 2020-12 assumed in either mode                                                                                 |
| Mixed input set                    | a Nexus document and a pure JSON Schema file in the same closure                                                              |

### Rejected at load time (negative)

| Reason                                 | Example                                                                                                           |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `services` without `nexusrpc` (P7.1)   | top-level `services` + no `nexusrpc`, including `services: null` → fix-it "add `nexusrpc: \"1.0.0\"`"             |
| `nexusrpc` not exactly `"1.0.0"`       | `nexusrpc: "1.1.0"` / `"2.0.0"` / `"0.9.0"` → fix-it naming the required `"1.0.0"` and echoing the declared value |
| Malformed/non-string `nexusrpc`        | `nexusrpc: 1` / `1.0` / `"1.0"` / `"v1"` / `null`                                                                 |
| Wrong `$schema` dialect                | `$schema: ".../draft-07/schema#"` / `null` → fix-it naming the 2020-12 URI                                        |
| Schema-shaped root in a Nexus document | root `type`/`properties`/`additionalProperties` alongside `nexusrpc` → "move the type into `$defs`"               |
| `$id` present (anywhere)               | root or nested `$id` — owned by [[ref]]                                                                           |
| `$vocabulary` present                  | root `$vocabulary` — meta-schema-only keyword, no place in a type schema (P5/P6)                                  |
| No root shape and no definitions       | `{}`, a description-only document, or `$defs: {}`                                                                 |

Every reject in this table names the **file it is about** and a remedy that
works at a site the author controls (**P7.2**). Where the closure is what made a
file reachable, that means naming the `$ref` that pulled it in rather than only
the entry file the author did not touch — the module-level counterpart of this
rule, and the reserved-name and module-segment rejects it governs, are in
[[generated-file-layout]].

## Interactions

- **[[services]]** — recognized **only** in a Nexus document; this doc owns the
  `nexusrpc` opt-in, the envelope rule, and the stray-`services` guard that gate
  it. The services spec keeps only service/operation-specific content.
- **[[ref]]** — owns the `$id` reject and the input-set/closure + type-name
  derivation that build on the per-file mode decided here.
- **[[generated-file-layout]]** — owns the per-input module computation and the
  one-package collision pass that every file (both modes) feeds.
- **[[pipeline]]** — the loader stage where these document-level checks run
  (Parse), ahead of `$ref` resolution and the strict-subset gate.

## See also

- [[services]] — the Nexus extension this document mode enables;
  service/operation grammar, names, I/O, emission.
- [[ref]] — `$id` reject, input-set/closure, type-name derivation that build on
  the per-file mode.
- [[generated-file-layout]] — per-input module computation, the nested package
  tree (flat for Go), the collision pass every file feeds.
- [[pipeline]] — the loader stage these document-level checks run in.
- [[PRINCIPLES.md]] — **P5** (2020-12 base), **P6** (strict subset), **P7/P7.1**
  (strict schema, reject loudly with fix-its), **P13/P13.2** (forward
  compatibility), **P14/P15** (one module per input file; one identifier
  namespace).
