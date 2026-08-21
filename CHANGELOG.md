# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- WIT signal-with-start request models now carry Temporal headers while keeping
  them out of generated convenience operation APIs.
- .NET proto-backed models now use Temporal SDK transfer-type converters. The
  generated output requires `Temporalio` 1.18.0 or newer; generic proto-backed
  .NET models report an explicit unsupported-conversion error until the SDK
  supports generic converter registration.
- Python JSON Schema packages now export `ValidationError` and `Violation`
  from their root `__init__.py`; callers no longer need to import the private
  `_definitions` module.
- Added grouped protobuf `oneof` authoring and bidirectional Python conversion,
  including required and optional oneofs, scaffolding through `add-rpc` and
  `add-message`, and explicit diagnostics for unsupported target backends.
- Python proto-backed generic records may use Temporal `Payload` and `Payloads`
  fields (including oneof members) as runtime value carriers. Decoding preserves
  concrete runtime type arguments through nested models and `Payload` values.
- JSON Schema: An object `oneOf` branch may now be written **inline**, whatever
  its shape. A structured branch (declared `properties`, or a typed
  `additionalProperties`) is named — `<Union>Object` for a lone branch, or the
  branch's own `x-<lang>-name` — and emitted as the same named model an authored
  `$defs` definition of that shape produces, so it validates, exports, and
  round-trips identically in all four languages. Two or more inline object
  branches must each carry the target's `x-<lang>-name`: every branch would
  otherwise derive the same name, and a discriminator `const` is a wire value,
  not an identifier.
- JSON Schema: A `oneOf` sum type may now be written in an **element
  position** — an array's `items` (at any depth) or an object's typed
  `additionalProperties`. It is named after its position (`<Enclosing>Item` /
  `<Enclosing>Value`, or its own `x-<lang>-name`), moved into `$defs`, and the
  position rewritten to a `$ref`, so the element type is an ordinary named union
  in all four languages — including any inline object branch it carries, which is
  named in turn. Previously such a union was silently collapsed to one branch
  (Go, Java) or distributed over the array (TypeScript).
- JSON Schema: The free-form object (`type: object` with
  `additionalProperties: true` and no `properties`) is now supported as a `oneOf`
  branch. Its members are carried verbatim in every language: Go and Java
  synthesize the wrapper the union needs (`<Union>Object`), TypeScript and Python
  inline it as `Record<string, unknown>` / `dict[str, Any]`.

### Changed

- JSON Schema conformance is now enforced end to end: the loader rejects unknown
  schema and Nexus-envelope members (including `endpoint` and OpenAPI
  `discriminator`), discovers transitive local `$ref` files and nested RFC 6901
  `$defs` pointers, and normalizes `allOf` annotations and constraints before
  generation. Generated Go, TypeScript, Python, and Java APIs now keep mixed
  declared/typed catch-all objects fully typed, apply complete `contains` and
  `propertyNames` scalar matchers in both directions, enforce wire-string
  constraints around temporal/byte materialization, keep closed/default values
  native, and mark deprecated types, fields, services, and operations
  idiomatically. Generated documentation also preserves paragraphs, wraps at 88
  columns, and escapes target comment syntax. These changes add no CLI flags and
  do not change the JSON wire format.

- JSON Schema numbers round-trip by mathematical JSON value rather than token
  spelling: whitespace, object-member order, and spellings such as `5`, `5.0`,
  and `5e0` are not identity-bearing. Generated Java keeps idiomatic `double`
  serialization rather than applying a Java-only spelling normalization.

- Generated source headers now include the version of the `nexgen` binary that
  produced them.
- Protobuf-backed models now consistently generate conversions in both
  directions whenever they are reachable. Go and TypeScript emit previously
  suppressed complementary helpers, operation-free exported models receive the
  same validation as operation-used models, and Java now reports its existing
  lack of protobuf model support instead of silently dropping protobuf
  operation types.
- JSON Schema: An `x-<lang>-name` alongside a `$ref` is no longer merged as an
  implicit-`allOf` conjunct, which cloned the referenced target into the use site.
  It names the _member_ the reference is bound to and leaves the reference intact
  — the one sibling keyword treated this way, because it asserts nothing about the
  value, and the only way to rename a member whose type is a `$ref` (a member
  named `class` was otherwise unfixable in Python and Java).
- TypeScript: JSON Schema models now export `TransferTypeConverter` instances
  (`fromTransferType`/`toTransferType`) instead of mapper classes, and generated
  operations reference them through `inputType`/`outputType`. Converter names
  follow resolved model names, participate in collision checks, and require the
  nexus-rpc type-info API.
- Generating into an existing `--output` directory no longer deletes it first.
  The directory is written into instead, so pre-existing files and
  subdirectories are preserved; generated files are still overwritten in place.
  A file left over from an earlier run whose definition has since been renamed
  or removed now stays behind until it is deleted. The `build-examples` and
  `build-json-examples` maintenance commands, which own the directories they
  write, delete each example's output directory before regenerating it, so the
  checked-in samples stay free of stale files.
- Java: A `oneOf` union now carries its `JsonNode` dispatcher as a static
  `fromNode` on the union interface in **both** positions — a named `$defs` union
  and a union written inline on a property (whose interface is nested in the
  declaring class). The enclosing deserializer reads the member with one
  delegating call instead of an inlined token chain, and a union member
  serializes by runtime class: object branches through their POJO's serializer,
  scalar/array wrappers through Jackson's `@JsonValue` on `getValue()`.
- JSON Schema: A **materializing** keyword on a non-object branch of a `oneOf`
  sum type — a temporal `format` (`date-time`/`date`/`time`/`duration`) or a
  `contentEncoding` — is now **rejected at load** with a located diagnostic. The
  synthesized `<Union><Kind>` wrapper has no native construct to hold, so the
  branch materialized in Python while Go, TypeScript, and Java carried an
  unvalidated `string`. The remedy is a plain `string` branch (still fully
  validated) or an object branch carrying the value as a property, where
  materialization already works. Asserted string formats (`uuid`, `email`,
  `hostname`, `uri`, `ipv4`, `ipv6`) are unaffected, and the nullability
  `oneOf:[{T},{null}]` is not a sum type, so a materialized nullable field keeps
  working.

### Deprecated

### Breaking Changes

- Python: JSON Schema output now uses slotted, keyword-only dataclasses instead
  of Pydantic and works with the default Temporal converter, removing the
  Pydantic dependency and contrib converter wiring. Generated transfer converters
  preserve wire names, carry unknown fields in `additional_properties` instead of
  `model_extra`, aggregate structured validation errors, collapse absent and
  explicit-null optional-and-nullable values to `None`, and surface schema
  defaults through mutable properties whose deleter restores unset state.
- Java: A map-shaped model (a pure typed map — `additionalProperties` with no
  declared `properties`) now names its catch-all member `additionalProperties`,
  matching the struct-shaped POJOs and the other languages (Go
  `AdditionalProperties`, TypeScript `additionalProperties`) as
  `additionalProperties.md` specifies. The generated accessor is
  `getAdditionalProperties()` (was `getValues()`); the constructor keeps its
  single positional map parameter, so only getter call sites need updating. The
  wire form is unchanged.
- Renamed the project to `nexgen`. The crate is published as `nexgen` (was
  `nex-gen`), generated .NET code uses the `Nexgen.*` namespaces and the
  `NexgenClient`/`RequireNexgenClient` members (were `NexGen.*` and
  `NexGenClient`/`RequireNexGenClient`), the TypeScript definitions namespace is
  `__nexgenDefinitions` (was `__nexGenDefinitions`), and the samples honor
  `NEXGEN_BIN` (was `NEX_GEN_BIN`). Generated-file headers and the
  `[GeneratedCode]` attribute now read `nexgen`.

### Fixed

- Java now rejects numeric JSON tokens outside the finite binary64 domain (for
  example `1e400`) with an aggregated, fully pathed violation in ordinary
  properties, union branches, nested arrays, and typed-map members.
- JSON Schema `minItems`, `maxItems`, `uniqueItems`, and `contains` now inspect
  the original wire array in every target even when one or more elements fail
  `items`. Failed conversions no longer fabricate count or duplicate results;
  indexed violations precede sibling array-keyword violations at every depth.

- JSON Schema converters now apply scalar, reference, union, nested-array,
  temporal, and content-encoding handling recursively inside array elements and
  typed-map members. Go reports indexed/keyed violations instead of collapsing
  them to the collection, TypeScript no longer passes a `oneOf` array branch
  through verbatim, and required temporal/base64 runtime support is discovered
  at every nesting depth.
- JSON Schema `number` values now reject `NaN` and positive/negative infinity
  with aggregated, fully pathed validation errors before serialization in every
  target. Go also accepts every valid integer-valued JSON number spelling
  (`1`, `1.0`, `1e2`, `1.5e1`) while continuing to reject fractional and
  over-cap values.
- JSON Schema temporal dates and date-times now use `0001` as their shared
  minimum year. Year `0000` is rejected by schema-literal validation and by the
  generated Go, TypeScript, Python, and Java runtime predicates.
- JSON Schema: Cross-input emission and naming now follow each target's actual
  scope and `x-<lang>-name` overrides. Foreign types are imported rather than
  duplicated, empty TypeScript model modules are omitted, member-derived names
  stay aligned, and root/`$defs`/synthesized collisions fail at load time.
- TypeScript: String array elements now enforce their own constraints and report
  type errors at the indexed element path.
- Python: Closed-value checks now use tuple membership, array-element errors name
  the expected type, converter locals cannot be shadowed by properties, and all
  synthesized module names participate in collision checks. `_definitions` is
  reserved for the generated runtime module.
- JSON Schema: A **non-object `oneOf` branch's own constraints** were dropped in
  three of four languages: only Go carried them, in the synthesized
  `<Union><Kind>` variant's `Validate`. TypeScript cast the narrowed value
  through unchecked, Python emitted a bare `str | SpecInt` with no constraint
  metadata, and Java's wrapper classes held the value without validating it. A
  branch is now held to everything it declares — string lengths, `pattern`, an
  asserted `format`, numeric bounds and `multipleOf`, `minItems`/`maxItems`/
  `uniqueItems`/`contains`, a `const`/`enum` value set — in all four languages and
  both directions, under the union's own violation path. Go additionally dropped a
  branch's `pattern`/`format` while emitting an (empty) `Validate` for it.
- TypeScript: A `const`/`enum` on a non-object `oneOf` branch generated code that
  **did not compile** (`tsc` TS2322): the branch narrowed the member type to its
  literal set while the parse path assigned the wider primitive into it. The
  branch's member type and its narrowed assignment now agree.
- TypeScript: A nested violation carrying no path of its own — a union branch's
  own constraint, an element-level check — was reported with a dangling separator
  (`segments[0].`). The prefix is now the whole path, matching Go and Java.
- JSON Schema: `uniqueItems` and `contains` were dropped on an array-typed **typed
  map member** in Python. Both now run in the member's converter through the
  runtime's `_check_unique_items` / `_check_contains`, with the same reasons the
  property position emits (and the same mechanism now serves a `oneOf` branch).
- JSON Schema: A typed map's members were validated against their type *token*
  only, so every constraint the member type declared was silently dropped — a
  string's `minLength`/`maxLength`/`pattern`/`format`, a number's bounds and
  `multipleOf`, an array's `minItems`/`uniqueItems`/`contains`, a `const`/`enum`
  value set. Every member is now held to everything its type declares, in both
  directions, with the member's key as the violation path. Python additionally
  validated only that a member was a _string_, leaving an object, union, or
  numeric member unchecked and unmaterialized; members now validate and
  materialize through the member type's own converter, so
  `additional_properties` holds the declared type (an `Inner`, an `int` parsed
  from `1.0`, a `datetime`, `bytes`) and re-encodes through it on the way out.
  TypeScript checked members on the way in but not on the way out, and dropped
  a nullable value's constraints in both positions (a member's *and* a
  declared field's).
- JSON Schema: A **nullable** typed-map member (`additionalProperties` as the
  nullability `oneOf`) was mishandled: Go typed the member `T` and dropped a
  `null` member from the map entirely, and Java rejected it. A null member is now
  kept as a null member — Go `map[string]*T`, Java `Map<String, @Nullable T>` —
  matching TypeScript's `Record<string, T | null>` and Python's `T | None`.
- Java: A **nested array** (`items` inside `items`) and an **array-valued typed
  map member** both bound to the placeholder violation `"unsupported nested
array"` at runtime, though `items.md` accepts them. Both now decode elementwise,
  one loop per level, with each level's index in the violation path
  (`grid[1][0]`).
- Java: A materialized temporal `format` or `contentEncoding` in an array element
  or a typed map member emitted `var` for the parsed value, which does not compile
  at the Java 8 baseline the generated code targets.
- TypeScript: A nested array's element loop reused the enclosing loop's variable
  names, so it emitted `item!.push(item)` — which does not compile — and reported
  the inner index twice in the violation path. Each level now carries its own
  element, index, and item bindings.
- TypeScript: A `pattern` or `format` on anything but a declared property — a
  typed map's member, an array element, a key-shape subschema — emitted a check
  referencing an undeclared `PATTERN_…` const, throwing `ReferenceError` at
  validation time. Every string position's regex is now declared.
- JSON Schema: An object written **inline** in a value position — a property, an
  array element at any depth, a typed `additionalProperties` member — had its
  declared shape silently discarded. Go, TypeScript, and Python typed the member
  as an opaque map (`map[string]json.RawMessage` / `Record<string, unknown>` /
  `dict[str, Any]`) and Java as `String`, so declared properties and member
  constraints never reached the generated code; Go additionally never decoded the
  member at all, leaving the value neither typed nor preserved in the catch-all.
  Every inline object is now named after its position (`<Model><Property>`,
  `<Enclosing>Item`, `<Enclosing>Value`), moved into `$defs`, and the position
  rewritten to a `$ref`, so it emits as the ordinary named model the
  `$defs` + `$ref` form produces — materialized, validated, exported, and
  round-tripped identically in all four languages. Nullability no longer changes
  the name: the object inside a `oneOf: [{object}, {"type":"null"}]` wrapper takes
  the position's name too. This covers every object shape, including a typed map
  and the free-form object, whose member-count and key-shape constraints were
  dropped along with the rest.
- JSON Schema: A union-typed array element or map member decoded through the
  whole-collection path, which cannot allocate a sealed interface: Go failed at
  runtime on `[]Union` / `map[string]Union` (`json.Unmarshal` into an interface)
  and Java on `List<Union>` / `Map<String, Union>`
  (`readTreeAsValue(node, Union.class)` on an abstract type). Each element/member
  is now routed through the union's own dispatcher, with its index or key in the
  violation path (`shapes[1]`, `choices.primary`), and the serialize side re-runs
  each element's branch constraints (P12).
- JSON Schema: A **nullable** array element (`items: {oneOf: [{T}, {null}]}`) was
  mishandled in every language: Go typed it `[]T` and silently decoded a wire
  `null` to `T`'s zero value, TypeScript emitted `T | null[]` (an unparenthesized
  union under `[]` — "a T or an array of nulls"), Python dropped the _field's_
  own `| None` because the element annotation already contained one, and Java
  rejected a null element outright. All four now follow `items.md`: `[]*T`,
  `(T | null)[]`, `list[T | None]`, `List<@Nullable T>`.
- TypeScript: An array of models or unions serialized its elements verbatim, so
  an element's in-memory `additionalProperties` bag reached the wire as a literal
  member (and an element's temporal/bytes members were never re-encoded). Each
  element now re-serializes through its own converter, as does a typed map's
  member.
- Go: A schema `description` ending a sentence with a package-like word ("one at
  a time.") added that package to the import block, and an unused import is a Go
  compile error. Package use is now read off the emitted code, not the doc
  comments.
- JSON Schema: A `oneOf` with an inline object branch generated uncompilable Go
  (a marker method on an undeclared `<Union>Object` type) and uncompilable
  TypeScript (a converter named after the anonymous `Record<string, unknown>`
  branch type); Java bound the branch to `null` without a violation.
- Java: An object branch of a union written inline on a property was silently
  dropped — the branch's class implemented nothing, and the parse arm for the
  object token was empty. The branch class now implements the nested union
  interface (`implements <DeclaringClass>.<Union>`) and parses through it.
- Java: A named `oneOf` union def with a scalar, array, or free-form-object
  branch generated uncompilable code — its `fromNode` referenced wrapper classes
  (`<Union>String`, `<Union>Array`, `<Union>Object`) that were never declared.
  The wrappers are now declared inside the union interface.
- Java: An array branch of a `oneOf` union parsed to `null` without a violation;
  its items are now parsed and validated elementwise.
- JSON Schema: A free-form object _definition_ generated an empty Go struct that
  rejected every member as an unknown field, and an empty TypeScript interface
  that dropped every member.
- JSON Schema: A typed map whose members are not strings (for example
  `additionalProperties: {type: integer}`) generated uncompilable Go — a
  `map[string]int64` member decoded as `map[string]string`, with the member
  values never parsed.
- JSON Schema: `minProperties`/`maxProperties`/`propertyNames` on a free-form
  object were dropped in Go, TypeScript, and Python; they are now enforced in
  both directions (P12).
- JSON Schema: TypeScript serialized an object member of a property-level union
  by copying the in-memory value, so the model's `additionalProperties` member
  reached the wire as a literal key and its extras were never spread back out.
  The union now serializes through the branch's converter.
- JSON Schema: TypeScript's serializer for a mixed-kind union returned the lone
  object branch unconditionally, making the scalar/array branches unreachable;
  the object branch is now guarded by the object token, matching the parse side.

### Security

## [0.2.1] - 2026-07-31

### Added

- WIT: Added `@nexus.name` directive for customizing generated field names.

## [0.2.0] - 2026-07-28

### Added

- Added the `nexgen` CLI for generating Go, Java, Python, and TypeScript
  bindings from NexusRPC definition files. Types are modeled with JSON Schema
  2020-12: each type becomes a typed model backed by a single shared validator
  that runs on both sides of the wire, so a payload can never be parsed or
  serialized in a shape the contract forbids. Constraint failures aggregate into
  one native error naming every violation, which a Nexus handler maps straight to
  `BAD_REQUEST`. The supported subset is deliberately strict — anything that
  can't be lowered cleanly and identically into all four languages is rejected at
  generation time with a fix-it diagnostic. See the [README](README.md) for the
  supported JSON Schema features, naming overrides, and usage.

### Breaking Changes

- All advanced WIT/proto-oriented functionality now lives behind an `advanced`
  Cargo feature that is off by default, so the published binary offers only the
  documented JSON Schema workflow. This gates the `dotnet` generate target, the
  WIT/proto generate flags (`--support-file`, `--descriptors`, `--format`,
  `--native-api`), and the maintenance subcommands (`build-examples`,
  `build-json-examples`, `add-rpc`, `debug-wit-dir`). Build with
  `cargo build --features advanced` to restore the previous surface.
