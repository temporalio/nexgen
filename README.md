# nexgen

Generate code for Go, Java, Python, and TypeScript from **NexusRPC definition
files** describing your services and the types they exchange. Each type becomes a
typed model with a runtime validator, wired to the **Nexus service bindings** —
one contract, shared across every language. Types are modeled with **JSON Schema
2020-12**.

> [!WARNING]
> Under active development. The supported JSON Schema subset, CLI options, and
> emitted code may change incompatibly before a stable release.

## What it does

Point `nexgen` at a definition file and it emits, per language, a typed model for
every type in your schema plus one **shared validator** behind it. The same
validator runs on both sides of the wire — when a value is parsed and when it is
serialized — so a payload can never enter or leave your service in a shape the
contract forbids.

Constraint failures don't throw one-at-a-time. They aggregate into a single
native error that carries every violation at once, with each reason naming the
concrete bound and the offending value. A Nexus handler maps that error straight
to `BAD_REQUEST`.

The supported subset is deliberately strict. Anything ambiguous, or anything
that can't be lowered cleanly and identically into all four languages, is
**rejected at generation time** with a fix-it diagnostic that tells you how to
express it instead. The generator would rather reject loudly than emit something
subtly wrong in one language.

## Installation

Download the archive for your platform from the
[latest GitHub release](https://github.com/temporalio/nexgen/releases/latest):

| Platform | Archive target |
| --- | --- |
| Linux (x86-64) | `x86_64-unknown-linux-gnu` |
| Linux (Arm64) | `aarch64-unknown-linux-gnu` |
| macOS (Intel) | `x86_64-apple-darwin` |
| macOS (Apple silicon) | `aarch64-apple-darwin` |
| Windows (x86-64) | `x86_64-pc-windows-msvc` |
| Windows (Arm64) | `aarch64-pc-windows-msvc` |

Extract the archive, then move `nexgen` (`nexgen.exe` on Windows) to a directory
on your `PATH`.

## Usage

```
nexgen <lang> [OPTIONS] <INPUT>...

Arguments:
  <lang>       Target language: go | java | python | typescript  (ts = typescript)
  <INPUT>...   One or more JSON Schema definition files or directories
               (.json / .yaml / .yml)

Options:
      --output <DIR>              Directory to write generated code into
      --package-name <PKG>        Java only (required): base package for the
                                  generated types, e.g. com.example.chat. Its last
                                  dot-separated segment must match the --output
                                  directory's name.
      --date-time-types <REPR>    TypeScript only: in-memory type for materialized
                                  temporal `format` fields (date-time/date/time/
                                  duration) — string | date | temporal [default: string]
  -h, --help                      Print help
  -V, --version                   Print version
```

Generate TypeScript bindings from one schema:

```bash
nexgen ts samples/schemas/showcase.nexusrpc.yaml --output ./gen
```

The `--output` directory is created if missing and written into if it already
exists — nothing there is deleted, so hand-written files can live alongside the
generated ones. Generated files are overwritten in place, which also means a
file from an earlier run whose definition has since been renamed or removed
stays behind until you delete it.

Generate Go, TypeScript, and Python:

```bash
for lang in go typescript python; do
  nexgen "$lang" samples/schemas/showcase.nexusrpc.yaml --output "./gen/$lang"
done
```

Java requires `--package-name`, whose last dot-separated segment must match the
`--output` directory's name (here both end in `showcase`):

```bash
nexgen java samples/schemas/showcase.nexusrpc.yaml \
  --output ./gen/com/example/showcase --package-name com.example.showcase
```

Generated Java source uses JSpecify's `@NullMarked` and `@Nullable`
annotations. Add JSpecify 1.0.0 to the compile classpath; it is not needed at
runtime. For Gradle:

```groovy
dependencies {
    compileOnly 'org.jspecify:jspecify:1.0.0'
    testCompileOnly 'org.jspecify:jspecify:1.0.0'
}
```

For Maven, use a `provided` dependency:

```xml
<dependency>
  <groupId>org.jspecify</groupId>
  <artifactId>jspecify</artifactId>
  <version>1.0.0</version>
  <scope>provided</scope>
</dependency>
```

Choose how TypeScript represents temporal `format` fields (date-time, date, time,
duration) in memory:

```bash
nexgen ts samples/schemas/temporal.yaml --output ./gen --date-time-types temporal
```

- **`string`** (default) — every temporal field stays an RFC 3339 / ISO 8601
  string, exactly as it appears on the wire. No runtime dependency, runs on any
  Node or browser, and round-trips losslessly (the original offset and sub-second
  precision are preserved). The cost: you parse and compare the strings yourself,
  and the type system won't distinguish a timestamp from any other `string`.
- **`date`** — `date-time` fields become a native JS `Date`; `date`, `time`, and
  `duration` stay strings, because a `Date` can't represent a date-only,
  time-only, or duration value. Convenient for instant math via `.getTime()`, but
  **lossy**: a `Date` is a UTC instant, so the original offset is folded away
  (`2021-06-15T12:30:45.123456+02:00` → `2021-06-15T10:30:45.123Z`) and precision
  is capped at milliseconds. Reach for it only when you already work in `Date`
  and don't care about the offset.
- **`temporal`** — full-fidelity typed values from the TC39 Temporal API:
  `date-time` → `Temporal.ZonedDateTime`, `date` → `Temporal.PlainDate`,
  `duration` → `Temporal.Duration` (`time` stays a string). Preserves offset and
  sub-second precision and gives you real calendar/duration arithmetic, at the
  cost of requiring the `Temporal` global — present in recent runtimes, otherwise
  a polyfill.

See what the output looks like without generating it yourself: the checked-in
results live under `samples/{go,typescript,python,java}/`.

## Definition files

A definition file comes in two flavors.

**Pure JSON Schema** — the root *is* a type. Reusable types live in `$defs` and
are referenced with `$ref`. Good for a single model or a small closure of types:

```yaml
$schema: https://json-schema.org/draft/2020-12/schema
title: Room
type: object
additionalProperties: false
properties:
  roomId: { type: string }
  members:
    type: array
    items: { type: string }
required: [roomId]
```

**Nexus document** — opt in with a root `nexusrpc: "1.0.0"` marker, which enables
a `services` section. Now the root is an envelope: services and operations live
at the top, and your types live in `$defs`:

```yaml
nexusrpc: "1.0.0"
$schema: https://json-schema.org/draft/2020-12/schema
services:
  ChatService:
    fqn: example.chat.v1.ChatService
    operations:
      sendMessage:
        input: { $ref: "#/$defs/SendMessageInput" }
        output: { $ref: "#/$defs/SendMessageOutput" }
$defs:
  SendMessageInput:
    type: object
    additionalProperties: false
    properties:
      roomId: { type: string }
      message: { type: string }
    required: [roomId, message]
  SendMessageOutput:
    type: object
    properties:
      messageId: { type: string }
    required: [messageId]
```

An operation's `input` and `output` are each optional. When present, each must be
an object type (this keeps schemas forward-compatible — a scalar today can't grow
fields later). An omitted side simply means no input, or no output, for that
direction.

The [`samples/schemas/`](samples/schemas/) directory holds example definitions:
the pure-schema [`temporal.yaml`](samples/schemas/temporal.yaml), the Nexus
document [`chat.nexusrpc.yaml`](samples/schemas/chat.nexusrpc.yaml), the
feature-diverse [`showcase.nexusrpc.yaml`](samples/schemas/showcase.nexusrpc.yaml),
and the multi-file closure under [`kb/`](samples/schemas/kb/).

## Naming & overrides

Emitted names are derived **deterministically** from your schema. The generator
never mangles or auto-disambiguates: if two types or members would collide in a
target language, generation **fails at load time** with a fix-it rather than
silently renaming one. The same input always produces byte-identical output.

When the derived name isn't what you want — or to resolve a collision — use the
per-language `x-<lang>-*` extension keywords. They attach to services,
operations, types, and properties:

```yaml
nexusrpc: "1.0.0"
services:
  ChatService:
    x-go-name: Chat              # the Go package is already `chat` — avoid stutter
    fqn: example.chat.v1.ChatService
    operations:
      getRoomById:
        x-go-name: GetRoomByID   # keep the `ID` initialism uppercase in Go
        input: { $ref: "#/$defs/GetRoomRequest" }
        output: { $ref: "#/$defs/Room" }
$defs:
  Message:
    type: object
    properties:
      from:
        type: string
        x-py-name: sender        # `from` is a Python keyword — it can't be a field
      state:
        enum: ["open", "n/a"]
        # `n/a` isn't a valid identifier, so name the Go/Java constants explicitly
        x-go-enum-names: { "open": Open, "n/a": Unavailable }
        x-java-enum-names: { "open": OPEN, "n/a": UNAVAILABLE }
      schemaVersion:
        const: 2
        x-go-const-name: SchemaVersionV2   # a bare `2` has no usable identifier
        x-java-const-name: SCHEMA_VERSION_V2
```

`x-<lang>-name` sets the emitted identifier — one keyword per target: `x-go-name`,
`x-ts-name`, `x-py-name` (note: the token is `py`, not `python`), and
`x-java-name`. For the languages that emit *named* constants, `x-go-const-name` /
`x-java-const-name` name the identifier for a `const`, and `x-go-enum-names` /
`x-java-enum-names` name each `enum` member (keyed by its wire value). TypeScript
and Python have no equivalent, because there a constant or enum member *is* its
literal value.

## Supported JSON Schema features

Support levels: **Full** (works as specified), **Partial** (a documented subset),
**No** (deliberately rejected with a fix-it), **Planned** (accepted or deferred,
not yet lowered).

### Core & objects

| Feature | Support | Notes |
|---|---|---|
| `properties` | Full | Typed struct fields. |
| `required` | Full | Presence enforced at runtime. |
| `type` | Partial | Single-string form only; the array form is rejected. |
| `default` | Full | Scalar; off-the-wire, materialized on read, never echoed. |
| `additionalProperties` | Partial | Open (default), closed (`false`), or typed map (typed value). |
| `minProperties` / `maxProperties` | Full | Runtime member count. |
| `propertyNames` | Partial | Only on a map-shaped object. |
| `dependentRequired` | Full | Runtime conditional-requirement assertion. |

### Strings & numbers

| Feature | Support | Notes |
|---|---|---|
| `minLength` / `maxLength` | Full | Runtime code-point count. |
| `minimum` / `maximum` | Full | Runtime comparison. |
| `exclusiveMinimum` / `exclusiveMaximum` | Full | Runtime comparison. |
| `const` | Partial | Scalar values only; composite deferred. |
| `enum` | Partial | Scalar values only; composite deferred. |
| `format` | Partial | Curated portable subset, each asserted at runtime. |
| `pattern` | Partial | Portable RE2-safe subset only. |
| `multipleOf` | Partial | Positive integer divisor only. |
| `contentEncoding` | Partial | `base64` / `base64url` only, materialized to native bytes. |

### Arrays

| Feature | Support | Notes |
|---|---|---|
| `items` | Full | Homogeneous list (`type: array` + single `items`). |
| `minItems` / `maxItems` | Full | Runtime element count. |
| `minContains` / `maxContains` | Full | Runtime match-count bounds. |
| `uniqueItems` | Partial | Scalar element types only. |
| `contains` | Partial | Scalar matcher over scalar elements only. |

### Composition & nullability

| Feature | Support | Notes |
|---|---|---|
| `allOf` | Full | Load-time merge into one schema; only contradictory/unrepresentable branches reject. |
| nullability | Full | The recognized `oneOf: [{type: T}, {type: "null"}]` pattern. |
| `oneOf` | Partial | Branches must be separable by a decidable selector (≥2 branches). |
| `$ref` / `$defs` | Partial | Named targets in local files only; no `$id`, no remote. |
| `anyOf` | No | No coherent typed lowering in the strict subset. |
| `not` | No | Negation has no coherent typed lowering. |
| `if` / `then` / `else` | No | Conditional shape has no coherent lowering. |
| `dependentSchemas` | No | Branches object shape on a runtime condition. |
| `prefixItems` | No | Positional tuples don't lower coherently across targets. |
| `unevaluatedProperties` / `unevaluatedItems` | No | No coherent lowering in the strict subset. |
| `contentMediaType` / `contentSchema` | No | No coherent lowering / nowhere to emit. |

### Annotations & Nexus

| Feature | Support | Notes |
|---|---|---|
| `title` / `description` | Full | Emitted as doc-comment summary/body. |
| `deprecated` | Full | Native deprecation marker + doc note. |
| `$comment` | Full | Accepted, silently dropped. |
| `services` / `operations` | Full | First-class (Nexus documents). |
| `examples` | Planned | Accepted and ignored; pending the doc-comment feature. |
| `readOnly` | Planned | Deferred; needs request/response split-types. |
| `writeOnly` | Planned | Deferred; a single type can't hold a per-direction field. |
| `patternProperties` | Planned | Rejected in v1; a narrow single-pattern form is tracked. |

## Examples

Sample inputs live under [`samples/schemas/`](samples/schemas/) and their
generated output — in every language — under
[`samples/`](samples/)`{go,typescript,python,java}/`. Two are worth starting with:

- [`showcase.nexusrpc.yaml`](samples/schemas/showcase.nexusrpc.yaml) — a tour
  exercising most of the supported subset in one file.
- [`kb/`](samples/schemas/kb/) — a multi-file Nexus closure, showing how types
  split across files resolve through `$ref`.
