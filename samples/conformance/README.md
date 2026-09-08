# JSON Schema conformance manifest

`json-schema.json` is the language-neutral statement of P1: *a value one target
accepts round-trips through any other unchanged.* It is **executed**, not just
described — `tests/json_schema_conformance_manifest.rs` generates every case
into Go, Java, Python and TypeScript, pushes every declared wire value through
the generated code of all four, and requires the verdicts to agree with the
manifest and with each other.

## Layout

    json-schema.json   the manifest
    schemas/           schemas written for conformance, including cross-file inputs
    runners/           the four generic runners the Rust driver drives

The generator derives a root model's name from the schema **file stem**, so
`schemas/numeric-bounds.yaml` produces `NumericBounds` in every target. Keep
conformance schemas to lowerCamel ASCII property names: the runners' native
mutation paths map a JSON property to a member by convention, and that
convention is only total for those names.

## A case

| field | meaning |
|---|---|
| `id` | stable, lowercase kebab-case |
| `intent` | what the case pins, in prose |
| `schemas` | ordered repository-relative schema paths; multiple entries exercise cross-file `$ref` generation |
| `model` | the generated model every wire value is driven through |
| `expected_load` | `accepted`, or `rejected` with the `diagnostic` substring and load-phase `covers` claims |
| `accepted_wire_values` | `fixture` or `wire_json`, optional native `mutations`, optional `expected_wire`, and round-trip/serialize `covers` claims |
| `parse_failures` | `wire_json` + `expected_paths` + parse-phase `covers` claims |
| `serialize_failures` | `from_wire`/`from_fixture` + native `mutations` + `expected_paths` + serialize-phase `covers` claims |
| `permitted_presence_nullability_collapse` | the **closed** list of members allowed to change presence on the way out |
| `expected_divergence` | the findings a case still reproduces (see below) |
| `consumers` | optional; a hand-written per-language test that also covers the case |

Failures compare **paths**, never reason text: P11 leaves the wording
target-idiomatic. A **load** verdict may legitimately differ per target — P15
scopes identifier validity to the emitted language, so a member named `validate`
is rejected for Go and accepted for the other three
(`features/properties.md:132-144`). What may never differ is the accepted and
rejected **wire** value set. `tests/json_schema_probe_matrix.rs` declares the
former with a `ScopedLoad` row rather than treating it as a finding. Round-trip comparison is member-by-member — presence (absent /
explicit null / present) is exact, and numbers compare by mathematical value with
`-0` kept distinct from `0`.

Rejected manifest cases are generated for every base target and must produce
the declared diagnostic in all four. The probe matrix remains the owner of
legitimate language-specific identifier rejections.

### Coverage declarations

Manifest version 3 makes coverage mechanical. Each `covers` value is a
`feature.phase` key from the fixed matrix in
`tests/json_schema_conformance_manifest.rs`. A claim in the wrong section, an
unknown or stale key, a duplicate owner, or a missing requirement fails the
structural test. The same ledger includes data-driven corpus anchors such as
`pattern.parse`, `format.round_trip`, `duration.*`, and `uri-reference.*`.

Annotation-only keywords (`title`, `description`, `examples`, `$comment`, and
deprecation) cannot affect the wire, so they stay in generator and
compile/import coverage rather than receiving artificial runtime claims.

### Serialize failures

A serialize-side rejection needs a native value the parser would never produce,
so a case declares one by parsing `from_wire` and then mutating the model. The
mutation vocabulary is deliberately tiny and implemented identically by all four
runners:

    {"path": "count",         "set_integer": "9007199254740992"}
    {"path": "ratio",         "set_number": "inf" | "-inf" | "nan" | "1e308"}
    {"path": "name",          "set_string": "..."}
    {"path": "maybe",         "set_null": true}
    {"path": "numbers",       "duplicate_element": 0}
    {"path": "numbers",       "remove_array_element": 0}
    {"path": "typed",         "put_map_entry": {"key": "x", "value": 1}}
    {"path": "typed",         "remove_map_entry": "x"}
    {"path": "optional",      "set_absent": true}
    {"path": "payload",       "set_bytes": [104, 105]}
    {"path": "span",          "set_duration": {"seconds": 90, "nanoseconds": 0}}

A path is `a.b[0][1]`: dot-separated members, each optionally indexed.

### `permitted_presence_nullability_collapse` is closed

Any member **not** listed must round-trip with its presence intact in every
target. Any member that *is* listed must actually collapse, in every target it
names — a stale entry fails the driver just as loudly as a missing one, because
a stale entry would hide the day a target stopped collapsing.

### `expected_divergence`

A case that a target still gets wrong carries:

```json
"expected_divergence": {
  "findings": ["13#2"],
  "status": "open",
  "note": "why, in one sentence",
  "matches": ["java rejected at"]
}
```

`status` is `open` (a defect someone owns) or `structural` (a permanent
limitation of the target — nobody should go looking for a fix). Both keep their
`matches` live; the difference is only how the driver reports them, so a
`structural` entry never becomes a stale to-do and still fails if the target
starts agreeing.

`matches` classifies which driver findings are *this* known divergence. Every
observed finding must match one, and every entry must match at least one
finding. So a pinned case cannot quietly absorb a new bug, and the marker cannot
outlive the fix: the driver fails when the case starts passing.

Use a `new:<slug>` finding id for a divergence this driver measured first.

## Runners

`runners/` holds one generic runner per target. They are copied into a scratch
workspace by `tests/toolchain/mod.rs`; nothing is generated into the committed
samples, which are golden snapshots.

| runner | how it reaches the model |
|---|---|
| `runner.go` | a generated `registry.go` maps a case to a `reflect.Type` (Go cannot look a type up by name), then reflection |
| `Runner.java` | `Class.forName` + Jackson, configured like Temporal's default converter |
| `runner.py` | `importlib` + the registered `TransferTypeConverter` |
| `runner.test.ts` | a generated `registry.ts` of lazy `import()`s plus Temporal SDK `TypeInfo` conversion, run under vitest (whose transform resolves the generator's extension-less imports) |

`smoke.py` and `smoke.test.ts` are the import-only variants used by
`tests/json_schema_probe_matrix.rs`.

`tests/json_schema_corpus_runtime.rs` runs the pure matcher data. Pattern
coverage executes all 102 gate-accepted rows in the four base runtimes and
keeps the 38 rejected rows anchored to the shared loader gate. Format coverage
adds `typescript-date` and `typescript-temporal` to the four defaults, for six
profiles total. Clock rows declare a common wire and explicit legacy-`Date`
overrides; Python's sub-microsecond `09#8` loss remains a live reported
divergence. Duration and URI-reference each have accept/reject and round-trip
corpora.

.NET is not part of this P1 runtime matrix; its validation contract remains a
separate generator project.

Adding a case needs no runner change. Adding a *mutation kind* needs all four.
