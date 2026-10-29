// Drives the `nexgen` binary over the WIT/proto CLI surface (`--descriptors`,
// `--native-api`, `--support-package`), all behind the `advanced` feature.
#![cfg(feature = "advanced")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nexgen::{GenerateRequest, generate_to_file};

mod common;
use common::wit_input_path;

const PROTO_GENERIC_EXAMPLE_ID: &str = "proto-generic";
static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A property whose union has one inline structured object branch (named
/// `<Union>Object`) and one scalar branch.
/// Unions in positions with no property of their own: an array element (inline
/// and `$ref`), a map member (inline), plus a nullable element for contrast.
const ELEMENT_UNION_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  segments:
    type: array
    items:
      oneOf:
        - { type: string }
        - { type: integer }
  choices:
    type: array
    items: { $ref: "#/$defs/Choice" }
  entries: { $ref: "#/$defs/Entries" }
  slots:
    type: array
    items:
      oneOf:
        - { type: string }
        - { type: "null" }
$defs:
  Choice:
    oneOf:
      - { type: string }
      - { type: boolean }
  Entries:
    type: object
    additionalProperties:
      oneOf:
        - { type: string }
        - { type: integer }
"##;

const INLINE_OBJECT_BRANCH_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  payload:
    oneOf:
      - type: object
        required: [text]
        properties:
          text: { type: string, minLength: 1 }
      - { type: string }
"#;

/// A union whose **non-object** branches each declare constraints of their own:
/// once the wire token selects a branch, the value is held to everything that
/// branch declares — including a closed value set — in both directions.
const BRANCH_CONSTRAINT_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string, minLength: 3, pattern: "^[a-z]+$" }
      - { type: integer, minimum: 1 }
  listOrName:
    oneOf:
      - { type: array, items: { type: number }, minItems: 1, uniqueItems: true }
      - { type: string, enum: [auto, manual] }
"#;

/// Array branches of inline unions must take the same recursive conversion path
/// as an ordinary array property. Each item kind below has a visibly different
/// in-memory and wire representation.
const UNION_ARRAY_TRANSFORM_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  events:
    oneOf:
      - type: array
        minItems: 1
        items: { type: string, format: date-time }
      - { type: string }
  blobs:
    oneOf:
      - type: array
        items: { type: string, contentEncoding: base64 }
      - { type: boolean }
  children:
    oneOf:
      - type: array
        items: { $ref: "#/$defs/Child" }
      - { type: integer }
$defs:
  Child:
    type: object
    additionalProperties: false
    required: [name]
    properties:
      name: { type: string, minLength: 2 }
"##;

const UNION_ARRAY_ASSERTION_ONLY_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
title: AssertionOnly
type: object
properties:
  contacts:
    oneOf:
      - { type: array, items: { type: string, format: email } }
      - { type: boolean }
"#;

/// `number` finiteness is an implicit wire constraint at every materialized
/// position, including branches, nested elements, and typed-map members.
const NON_FINITE_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  scalar: { type: number }
  choice:
    oneOf:
      - { type: number }
      - { type: string }
  nested:
    type: array
    items:
      type: array
      items: { type: number }
  metrics: { $ref: "#/$defs/Metrics" }
$defs:
  Metrics:
    type: object
    additionalProperties: { type: number }
"##;

/// Runtime support discovery must recurse through arrays, union branches, and
/// typed maps instead of looking only at direct model properties.
const NESTED_RUNTIME_SUPPORT_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  dates:
    type: array
    items: { type: string, format: date }
  encoded:
    oneOf:
      - type: array
        items: { type: string, contentEncoding: base64url }
      - { type: boolean }
  timestampMap: { $ref: "#/$defs/TimestampMap" }
$defs:
  TimestampMap:
    type: object
    additionalProperties: { type: string, format: date-time }
"##;

const NATIVE_TEMPORAL_SERIALIZE_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  occurredAt: { type: string, format: date-time }
  history:
    type: array
    items: { type: string, format: date-time }
  businessDay: { type: string, format: date }
required: [occurredAt, history, businessDay]
"#;

const NULLABLE_CONSTRAINED_ARRAY_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  slots:
    type: array
    items:
      oneOf:
        - { type: string, minLength: 2 }
        - { type: "null" }
required: [slots]
"#;

const TYPESCRIPT_PROPERTY_NAMES_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: { type: integer }
propertyNames:
  type: string
  minLength: 6
  maxLength: 32
  pattern: "@example\\.com$"
  format: email
  enum: [a@example.com, b@example.com]
"#;

const TYPESCRIPT_MATERIALIZED_CLOSED_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  fixedBlob:
    type: string
    contentEncoding: base64
    const: aGk=
  businessDay:
    type: string
    format: date
    enum: [2024-01-01, 2024-01-02]
  fallbackBlob:
    oneOf:
      - { type: string, contentEncoding: base64 }
      - { type: "null" }
    default: aGk=
"#;

const TYPESCRIPT_DEPRECATION_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Jobs:
    deprecated: true
    x-ts-name: jobApi
    operations:
      accept:
        deprecated: true
        x-ts-name: acceptJob
        input: { $ref: "#/$defs/LegacyJob" }
$defs:
  LegacyJob:
    type: object
    deprecated: true
    properties:
      oldId: { type: string, deprecated: true }
"##;

/// A service whose two operations are each one-sided: one declares only an
/// `input`, the other only an `output`.
const ONE_SIDED_OPERATION_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Jobs:
    fqn: example.jobs.v1.Jobs
    operations:
      accept:
        input: { $ref: "#/$defs/Job" }
      produce:
        output: { $ref: "#/$defs/Job" }
$defs:
  Job:
    type: object
    properties:
      id: { type: string }
"##;

/// An operation whose output type carries an `x-ts-name` override.
const OPERATION_IO_TS_NAME_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Pages:
    fqn: example.pages.v1.Pages
    operations:
      get:
        input: { $ref: "#/$defs/GetInput" }
        output: { $ref: "#/$defs/Page" }
$defs:
  GetInput:
    type: object
    properties:
      id: { type: string }
  Page:
    type: object
    x-ts-name: RenamedPage
    properties:
      title: { type: string }
"##;

/// The entry file of a two-file closure. `get`'s output is the model the *other*
/// file declares, and `FindOutput.page` `$ref`s it from a property, so both
/// cross-module reference shapes are covered.
const CROSS_MODULE_ENTRY_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Pages:
    fqn: example.pages.v1.Pages
    operations:
      get:
        input: { $ref: "#/$defs/GetInput" }
        output: { $ref: "content/page.json" }
      find:
        input: { $ref: "#/$defs/GetInput" }
        output: { $ref: "#/$defs/FindOutput" }
$defs:
  GetInput:
    type: object
    additionalProperties: false
    properties:
      id: { type: string }
  FindOutput:
    type: object
    additionalProperties: false
    properties:
      page: { $ref: "content/page.json" }
"##;

/// The referenced file. Its model carries the name override the *consuming*
/// module has to resolve through.
const CROSS_MODULE_PAGE_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
x-ts-name: RenamedPage
properties:
  title: { type: string }
"##;

/// Writes the two-file cross-module closure into `dir` and returns the input
/// directory to generate from.
fn write_cross_module_closure(dir: &Path) -> PathBuf {
    let input_dir = dir.join("input");
    fs::create_dir_all(input_dir.join("content")).unwrap();
    fs::write(
        input_dir.join("kb.nexusrpc.yaml"),
        CROSS_MODULE_ENTRY_SCHEMA,
    )
    .unwrap();
    fs::write(
        input_dir.join("content/page.json"),
        CROSS_MODULE_PAGE_SCHEMA,
    )
    .unwrap();
    input_dir
}

/// A property carrying a per-language name override alongside a `default`, a
/// `const`, and an inline object — one schema covering all three synthesized-name
/// families at once.
const MEMBER_DERIVED_NAME_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  retryCount:
    type: integer
    default: 3
    x-ts-name: attempts
    x-go-name: Attempts
  kind:
    type: string
    const: widget
    x-ts-name: category
    x-go-name: Category
  address:
    type: object
    x-ts-name: location
    x-go-name: Location
    properties:
      street: { type: string }
"#;

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn descriptor_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/descriptors/temporal_api.bin")
}

fn linked_inputs_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/inputs/deps")
}

fn read_typescript_output_files(dir: &Path) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, files);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("ts") {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read_to_string(&path).unwrap(),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(dir, dir, &mut files);
    files
}

fn unique_output_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("nexgen-{label}-{unique}-{counter}"))
}

#[test]
fn cli_uses_typescript_support_package() {
    let root = project_root();
    let temp_dir = unique_output_path("typescript-support-package");
    fs::create_dir_all(&temp_dir).unwrap();
    let output_path = temp_dir.join("output");

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "typescript",
            wit_input_path(&root, PROTO_GENERIC_EXAMPLE_ID)
                .to_str()
                .unwrap(),
            linked_inputs_path(&root).to_str().unwrap(),
            "--descriptors",
            descriptor_path(&root).to_str().unwrap(),
            "--support-package",
            "./shared/helpers",
            "--output",
            output_path.to_str().unwrap(),
            "--targets=workflow",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let models = fs::read_to_string(output_path.join("models.ts")).unwrap();
    assert!(models.contains("import * as support from './shared/helpers';"));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// An inline **structured** object `oneOf` branch on a property: the branch is
/// named `<Union>Object` and emitted as an interface with its own converter, and
/// the union's serialize side routes through it (the in-memory
/// `additionalProperties` member must not reach the wire).
/// See `specs/json-schema/features/oneOf.md` ("Object branches").
#[test]
fn typescript_json_names_inline_object_union_branch() {
    let temp_dir = unique_output_path("ts-json-inline-branch");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("detail.yaml");
    fs::write(&input_path, INLINE_OBJECT_BRANCH_SCHEMA).unwrap();
    let output_path = temp_dir.join("detail");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.ts")).unwrap();

    assert!(rendered.contains("payload?: DetailPayloadObject | string;"));
    assert!(rendered.contains("export interface DetailPayloadObject {"));
    assert!(rendered.contains(
        "export const detailPayloadObjectTransferTypeConverter = new class implements TransferTypeConverter<DetailPayloadObject> {"
    ));
    // Parse and serialize both route the object token through the branch converter.
    assert!(
        rendered.contains(
            "detailPayloadObjectTransferTypeConverter.fromTransferType(raw[\"payload\"])"
        )
    );
    assert!(rendered.contains(
        "function serializeDetailPayload(value: DetailPayloadObject | string): unknown {"
    ));
    assert!(rendered.contains("serializeDetailPayload(value.payload)"));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Every constraint a **non-object** branch declares is checked once the token
/// narrows to it, on both sides of the converter (P12). A `const`/`enum` branch
/// also narrows the member *type* to its literal set, without which the narrowed
/// assignment would not even typecheck.
/// See `specs/json-schema/features/oneOf.md` ("Validator mapping").
#[test]
fn typescript_json_validates_non_object_union_branch_constraints() {
    let temp_dir = unique_output_path("ts-json-branch-constraints");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("bc.yaml");
    fs::write(&input_path, BRANCH_CONSTRAINT_SCHEMA).unwrap();
    let output_path = temp_dir.join("bc");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.ts")).unwrap();

    // A closed value set narrows the branch type itself.
    assert!(rendered.contains("listOrName?: number[] | \"auto\" | \"manual\";"));
    // Parse: each branch's own predicates run under the union's path.
    assert!(
        rendered.contains(
            "const codePoints = __nexgenDefinitions.codePointLength((value as string), 3);"
        )
    );
    assert!(rendered.contains("if (codePoints < 3) {"));
    assert!(rendered.contains("if (!PATTERN_C182F89FDB221836.test((value as string))) {"));
    assert!(rendered.contains("if ((value as number) < 1) {"));
    assert!(rendered.contains("if (raw[\"listOrName\"].length < 1) {"));
    assert!(rendered.contains("duplicate items: element at index ${index}"));
    assert!(rendered.contains(
        "if ((listOrName as \"auto\" | \"manual\") !== \"auto\" && (listOrName as \"auto\" | \"manual\") !== \"manual\") {"
    ));
    // Serialize: the same predicates over the in-memory member, aggregated into
    // the model's own violations before the wire object is built.
    assert!(rendered.contains(
        "const codePoints = __nexgenDefinitions.codePointLength((value.value as string), 3);"
    ));
    assert!(rendered.contains("if ((value.value as number) < 1) {"));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_json_recursively_converts_union_array_branches() {
    let temp_dir = unique_output_path("ts-json-union-array-transform");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("probe.yaml");
    fs::write(&input_path, UNION_ARRAY_TRANSFORM_SCHEMA).unwrap();
    let output_path = temp_dir.join("probe");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: nexgen::generator::TsDateTimeTypes::Date,
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.ts")).unwrap();

    assert!(rendered.contains("let item: Date"), "{rendered}");
    assert!(
        rendered.contains("parseTemporalDateTime(element, `events[${index}]`, violations)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("must have at least 1 items"),
        "{rendered}"
    );
    assert!(
        rendered.contains("base64ToBytes(element, `blobs[${index}]`, violations)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("childTransferTypeConverter.fromTransferType(element)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("serializeTemporalDateTime(element)"),
        "{rendered}"
    );
    assert!(rendered.contains("bytesToBase64(element)"), "{rendered}");
    assert!(
        rendered.contains("childTransferTypeConverter.toTransferType(element)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("map((element, index)"),
        "indexed serialize mapper missing\n{rendered}"
    );
    assert!(
        rendered.contains("collect(violations, `[${index}]`, error)"),
        "indexed child failure collection missing\n{rendered}"
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_json_does_not_emit_union_serializer_for_assertion_only_format() {
    let temp_dir = unique_output_path("ts-json-union-array-assertion-only");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("probe.yaml");
    fs::write(&input_path, UNION_ARRAY_ASSERTION_ONLY_SCHEMA).unwrap();
    let output_path = temp_dir.join("probe");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.ts")).unwrap();

    assert!(
        rendered.contains("contacts?: string[] | boolean;"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("function serializeAssertionOnlyContacts"),
        "{rendered}"
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_json_rejects_non_finite_numbers_at_every_position() {
    let temp_dir = unique_output_path("ts-json-non-finite");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("probe.yaml");
    fs::write(&input_path, NON_FINITE_SCHEMA).unwrap();
    let output_path = temp_dir.join("probe");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.ts")).unwrap();

    for expected in [
        "Number.isFinite(raw[\"scalar\"])",
        "Number.isFinite((choice as number))",
        "Number.isFinite(element1)",
        "`nested[${index}][${index1}]`",
        "Number.isFinite(entry)",
        "must be a finite number",
    ] {
        assert!(rendered.contains(expected), "{expected}\n{rendered}");
    }
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_json_detects_nested_runtime_support() {
    let temp_dir = unique_output_path("ts-json-nested-runtime-support");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("probe.yaml");
    fs::write(&input_path, NESTED_RUNTIME_SUPPORT_SCHEMA).unwrap();
    let output_path = temp_dir.join("probe");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let definitions = fs::read_to_string(output_path.join("definitions.ts")).unwrap();
    let models = fs::read_to_string(output_path.join("models.ts")).unwrap();
    assert!(
        definitions.contains("import { ApplicationFailure } from \"@temporalio/common\";"),
        "{definitions}"
    );
    assert!(
        !definitions.contains("payloadValidationError"),
        "{definitions}"
    );
    assert!(
        models.contains("import { createPayloadValidationError } from \"@temporalio/common\";"),
        "{models}"
    );
    assert!(
        models.contains("throw createPayloadValidationError(violations);"),
        "{models}"
    );
    assert!(
        models.contains("import type { TransferTypeConverter } from \"nexus-rpc\";"),
        "{models}"
    );
    assert!(definitions.contains("parseTemporalDate("), "{definitions}");
    assert!(
        definitions.contains("parseTemporalDateTime("),
        "{definitions}"
    );
    assert!(definitions.contains("base64UrlToBytes("), "{definitions}");
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_json_validates_native_temporals_before_serializing() {
    for (label, repr) in [
        ("date", nexgen::generator::TsDateTimeTypes::Date),
        ("temporal", nexgen::generator::TsDateTimeTypes::Temporal),
    ] {
        let temp_dir = unique_output_path(&format!("ts-json-native-temporal-{label}"));
        fs::create_dir_all(&temp_dir).unwrap();
        let input_path = temp_dir.join("probe.yaml");
        fs::write(&input_path, NATIVE_TEMPORAL_SERIALIZE_SCHEMA).unwrap();
        let output_path = temp_dir.join("probe");

        generate_to_file(&GenerateRequest {
            config: Default::default(),
            language: nexgen::language::Language::TypeScript,
            input_paths: vec![input_path],
            support_package: Some("./support".to_string()),
            descriptor_paths: Vec::new(),
            output_path: output_path.clone(),
            format: false,
            java_package_name: None,
            ts_date_time_types: repr,
            dotnet_namespace: None,
        })
        .unwrap();
        let models = fs::read_to_string(output_path.join("models.ts")).unwrap();
        let definitions = fs::read_to_string(output_path.join("definitions.ts")).unwrap();

        assert!(
            models.contains(
                "validateTemporalDateTime(value.occurredAt, \"occurredAt\", violations);"
            ),
            "{models}"
        );
        assert!(definitions.contains("if (year < 1)"), "{definitions}");
        assert!(
            definitions
                .contains("!TEMPORAL_DATE_TIME_RE.test(wire) || !validTemporalCalendar(wire)"),
            "{definitions}"
        );
        if repr == nexgen::generator::TsDateTimeTypes::Temporal {
            assert!(
                models.contains(
                    "validateTemporalDate(value.businessDay, \"businessDay\", violations);"
                ),
                "{models}"
            );
        } else {
            assert!(
                models.contains(
                    "Number.isFinite(value.occurredAt.getTime()) ? __nexgenDefinitions.serializeTemporalDateTime(value.occurredAt) : undefined"
                ),
                "{models}"
            );
            assert!(
                models.contains(
                    "Number.isFinite(element.getTime()) ? __nexgenDefinitions.serializeTemporalDateTime(element) : undefined"
                ),
                "{models}"
            );
        }
        fs::remove_dir_all(temp_dir).unwrap();
    }
}

#[test]
fn typescript_json_guards_nullable_array_elements_during_serialize_validation() {
    let temp_dir = unique_output_path("ts-json-nullable-constrained-items");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("probe.yaml");
    fs::write(&input_path, NULLABLE_CONSTRAINED_ARRAY_SCHEMA).unwrap();
    let output_path = temp_dir.join("probe");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.ts")).unwrap();

    assert!(
        rendered.contains(
            "value.slots.forEach((element, index) => {\n          if (element !== null) {"
        ),
        "{rendered}"
    );
    assert!(
        rendered.contains("const codePoints = __nexgenDefinitions.codePointLength(element, 2);"),
        "{rendered}"
    );
    assert!(rendered.contains("if (codePoints < 2) {"), "{rendered}");
    assert!(rendered.contains("path: `slots[${index}]`"), "{rendered}");
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A union in an element position: the loader names it, so TypeScript emits an
/// ordinary union alias plus converter and runs it per element/member —
/// including on the serialize side, where an element model's catch-all bag would
/// otherwise reach the wire. A nullable element parenthesizes (`(T | null)[]`),
/// which `T | null[]` would silently misread.
/// See `specs/json-schema/features/oneOf.md` ("Unions in element positions").
#[test]
fn typescript_json_maps_element_position_unions() {
    let temp_dir = unique_output_path("ts-json-element-union");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("bag.yaml");
    fs::write(&input_path, ELEMENT_UNION_SCHEMA).unwrap();
    let output_path = temp_dir.join("bag");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.ts")).unwrap();

    assert!(rendered.contains("export type BagSegmentsItem = string | number;"));
    assert!(rendered.contains("segments?: BagSegmentsItem[];"));
    assert!(rendered.contains("bagSegmentsItemTransferTypeConverter.fromTransferType(element)"));
    assert!(rendered.contains("choiceTransferTypeConverter.fromTransferType(element)"));
    // A map member runs the member converter in both directions.
    assert!(rendered.contains("entriesValueTransferTypeConverter.fromTransferType(raw[key])"));
    assert!(rendered.contains("entriesValueTransferTypeConverter.toTransferType(entry)"));
    // Element nullability is the element's own concern, and parenthesized.
    assert!(rendered.contains("slots?: (string | null)[];"));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_json_native_barrel_exports_violation_type_only() {
    let temp_dir = unique_output_path("ts-json-native-runtime-exports");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("probe.yaml");
    fs::write(
        &input_path,
        "$schema: https://json-schema.org/draft/2020-12/schema\ntype: object\nproperties:\n  value: { type: string }\n",
    )
    .unwrap();
    let output_path = temp_dir.join("probe");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: nexgen::nexgen_config::NexgenConfig {
            targets: nexgen::nexgen_config::GenerationTargets::WORKFLOW,
            ..Default::default()
        },
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let index = fs::read_to_string(output_path.join("index.ts")).unwrap();
    let models = fs::read_to_string(output_path.join("models.ts")).unwrap();
    assert!(
        !index.contains("export { payloadValidationError } from './definitions';"),
        "{index}"
    );
    assert!(
        index.contains("export type { Violation } from './definitions';"),
        "{index}"
    );
    assert!(
        models.contains("const candidate: unknown = value;"),
        "{models}"
    );
    assert!(models.contains(".isPlainObject(candidate)"), "{models}");
    assert!(!models.contains(".isPlainObject(value)"), "{models}");
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A one-sided operation: the non-void side carries its converter as operation
/// type info, the void side carries no field at all (there is no value to
/// convert, so an empty `TypeInfo` would assert a conversion that does not
/// exist). See `specs/json-schema/services.md` ("TypeScript operation type
/// info"); the checked-in samples only cover void-on-both-sides.
#[test]
fn typescript_json_one_sided_operation_type_info() {
    let temp_dir = unique_output_path("ts-json-one-sided-type-info");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("jobs.nexusrpc.yaml");
    fs::write(&input_path, ONE_SIDED_OPERATION_SCHEMA).unwrap();
    let output_path = temp_dir.join("jobs");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("services.ts")).unwrap();

    // Input present, output omitted: `inputType` only.
    assert!(rendered.contains(
        "  >({ name: \"Accept\", inputType: { transferTypeConverter: jobTransferTypeConverter } }),"
    ));
    // The mirror: output present, input omitted.
    assert!(rendered.contains(
        "  >({ name: \"Produce\", outputType: { transferTypeConverter: jobTransferTypeConverter } }),"
    ));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// An `x-ts-name` override on an operation's I/O type moves every emitted
/// reference with the type: the operation generic, the model/converter imports,
/// and the converter named in the operation type info (the identifier is derived
/// from the *resolved* type name).
#[test]
fn typescript_json_operation_type_info_follows_ts_name_override() {
    let temp_dir = unique_output_path("ts-json-type-info-override");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("pages.nexusrpc.yaml");
    fs::write(&input_path, OPERATION_IO_TS_NAME_SCHEMA).unwrap();
    let output_path = temp_dir.join("pages");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let models = fs::read_to_string(output_path.join("models.ts")).unwrap();
    let services = fs::read_to_string(output_path.join("services.ts")).unwrap();

    assert!(models.contains("export interface RenamedPage {"));
    assert!(models.contains("export const renamedPageTransferTypeConverter = new class"));
    assert!(services.contains("import { getInputTransferTypeConverter, renamedPageTransferTypeConverter } from './models';"));
    assert!(services.contains("import type { GetInput, RenamedPage } from './models';"));
    assert!(services.contains("    RenamedPage\n"));
    assert!(
        services.contains(
            "outputType: { transferTypeConverter: renamedPageTransferTypeConverter } }),"
        )
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

/// An `x-ts-name` override on a model in *another* input file moves every
/// reference the consuming module emits: the operation generic, the type-only
/// model import, the converter value import, and the property annotation of a
/// cross-module `$ref`. The override is declared in the referenced file, so only
/// the tree-wide name manifest can resolve it (P14/P15).
#[test]
fn typescript_json_cross_module_ts_name_override_moves_every_reference() {
    let temp_dir = unique_output_path("ts-json-cross-module-override");
    let input_dir = write_cross_module_closure(&temp_dir);
    let output_path = temp_dir.join("output");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_dir],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();

    let declaring = fs::read_to_string(output_path.join("content/page/models.ts")).unwrap();
    assert!(declaring.contains("export interface RenamedPage {"));
    assert!(declaring.contains("export const renamedPageTransferTypeConverter = new class"));

    let services = fs::read_to_string(output_path.join("kb/services.ts")).unwrap();
    for expected in [
        "import { renamedPageTransferTypeConverter } from '../content/page/models';",
        "import type { RenamedPage } from '../content/page/models';",
        "    RenamedPage\n",
        "outputType: { transferTypeConverter: renamedPageTransferTypeConverter } }),",
    ] {
        assert!(services.contains(expected), "{expected}\n{services}");
    }

    let models = fs::read_to_string(output_path.join("kb/models.ts")).unwrap();
    for expected in [
        "import { renamedPageTransferTypeConverter } from '../content/page/models';",
        "import type { RenamedPage } from '../content/page/models';",
        "  page?: RenamedPage;",
        "page = renamedPageTransferTypeConverter.fromTransferType(raw[\"page\"]);",
    ] {
        assert!(models.contains(expected), "{expected}\n{models}");
    }
    // Nothing names the pre-override identifier.
    for stale in ["{ Page }", "pageTransferTypeConverter", ": Page", " Page\n"] {
        assert!(!services.contains(stale), "{stale}\n{services}");
        assert!(!models.contains(stale), "{stale}\n{models}");
    }
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_json_bare_ref_alias_cycle_rejects_without_ordering_loop() {
    let temp_dir = unique_output_path("ts-json-bare-ref-alias-cycle");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("cycle.yaml");
    fs::write(
        &input_path,
        r##"$schema: https://json-schema.org/draft/2020-12/schema
title: Cycle
$ref: "#/$defs/A"
$defs:
  A: { $ref: "#/$defs/Z" }
  Z: { $ref: "#/$defs/A" }
"##,
    )
    .unwrap();
    let error = generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join("output"),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .expect_err("alias cycle must reject");
    let message = error.to_string();
    assert!(
        message.contains("cycle") || message.contains("recursion"),
        "{message}"
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A name synthesized from a member follows that member's `x-ts-name`: the
/// `DEFAULT_<FIELD>` constant is built from the emitted identifier, not the JSON
/// key. A shape named after its *position* does not move — the hoisted inline
/// object keeps `<Model><Property>`.
/// See `specs/json-schema/PRINCIPLES.md` §15 and
/// `specs/json-schema/features/default.md`.
#[test]
fn typescript_json_override_moves_member_derived_names_only() {
    let temp_dir = unique_output_path("ts-json-member-derived-names");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("probe.yaml");
    fs::write(&input_path, MEMBER_DERIVED_NAME_SCHEMA).unwrap();
    let output_path = temp_dir.join("probe");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.ts")).unwrap();

    // Member-derived: the override moves the constant with the field.
    assert!(rendered.contains("export const DEFAULT_ATTEMPTS = 3;"));
    assert!(!rendered.contains("DEFAULT_RETRY_COUNT"));
    assert!(rendered.contains("attempts?: number;"));
    // Position-derived: the hoisted shape keeps the position's name even though
    // the declaring member is renamed.
    assert!(rendered.contains("location?: ProbeAddress;"));
    assert!(rendered.contains("export interface ProbeAddress {"));
    assert!(!rendered.contains("ProbeLocation"));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// The package barrel (`index.ts`) re-exports every module with `export *`, so two
/// modules declaring the same type name land in one namespace there. TypeScript
/// rejects that barrel outright (TS2308, "has already exported a member named
/// `Page`"), and the model's `<model>TransferTypeConverter` collides alongside the
/// type — so the generator must reject at load instead of emitting uncompilable
/// output. See `specs/json-schema/PRINCIPLES.md` §15.
#[test]
fn typescript_json_rejects_same_type_name_in_two_modules() {
    let temp_dir = unique_output_path("ts-json-barrel-collision");
    let input_dir = temp_dir.join("input");
    fs::create_dir_all(input_dir.join("a")).unwrap();
    fs::create_dir_all(input_dir.join("b")).unwrap();
    let page = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"properties":{"title":{"type":"string"}}}"#;
    fs::write(input_dir.join("a/page.json"), page).unwrap();
    fs::write(input_dir.join("b/page.json"), page).unwrap();

    let request = |output: &str| GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_dir.clone()],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join(output),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    };

    let error = generate_to_file(&request("out"))
        .expect_err("two modules declaring `Page` collide in the package barrel")
        .to_string();
    // The diagnostic names both modules — the bare type name appears twice and
    // would otherwise read as one declaration seen twice.
    assert!(error.contains("collision"), "{error}");
    assert!(error.contains("a/page#Page"), "{error}");
    assert!(error.contains("b/page#Page"), "{error}");
    assert!(error.contains("x-ts-name"), "{error}");

    // The documented escape hatch resolves it, and moves the converter with the
    // type so the barrel re-exports two distinct pairs.
    fs::write(
        input_dir.join("b/page.json"),
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","x-ts-name":"BPage","additionalProperties":false,"properties":{"title":{"type":"string"}}}"#,
    )
    .unwrap();
    let output_path = temp_dir.join("out-renamed");
    generate_to_file(&request("out-renamed")).expect("the override resolves the collision");
    let models = fs::read_to_string(output_path.join("b/page/models.ts")).unwrap();
    assert!(models.contains("export interface BPage {"), "{models}");
    assert!(
        models.contains("export const bPageTransferTypeConverter"),
        "{models}"
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Writes a closure whose service module declares no types of its own: both
/// operation types are `$ref`s into sibling files. Returns the input directory.
fn write_service_only_module_closure(temp_dir: &Path) -> PathBuf {
    let input_dir = temp_dir.join("input");
    fs::create_dir_all(input_dir.join("a")).unwrap();
    fs::create_dir_all(input_dir.join("b")).unwrap();
    fs::write(
        input_dir.join("a/page.json"),
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"properties":{"title":{"type":"string"}},"required":["title"]}"#,
    )
    .unwrap();
    fs::write(
        input_dir.join("b/note.json"),
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"properties":{"body":{"type":"string"}},"required":["body"]}"#,
    )
    .unwrap();
    fs::write(
        input_dir.join("svc.nexusrpc.yaml"),
        r#"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Svc:
    fqn: example.v1.Svc
    operations:
      one:
        input: { $ref: "a/page.json" }
        output: { $ref: "b/note.json" }
"#,
    )
    .unwrap();
    input_dir
}

/// A module emits the types it declares; a type it only `$ref`s belongs to the
/// module that declares it. Reachability pruning inferred "this front end does
/// not scope by module" from a module owning nothing, so a service file whose
/// every operation type is `$ref`d from elsewhere re-emitted all of them into
/// its own module — a second copy of each interface and converter, which
/// TypeScript rejects (`TS2440`, import conflicts with local declaration) and
/// which the package barrel then re-exports twice (`TS2308`).
#[test]
fn typescript_json_service_module_without_own_types_imports_instead_of_reemitting() {
    let temp_dir = unique_output_path("ts-json-service-only-module");
    let input_dir = write_service_only_module_closure(&temp_dir);
    let output_path = temp_dir.join("out");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_dir],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();

    // Each type and its converter are declared exactly once, in the module that
    // declares the schema.
    let files = read_typescript_output_files(&output_path)
        .into_values()
        .collect::<Vec<_>>()
        .join("\n");
    for declaration in [
        "export interface Page {",
        "export interface Note {",
        "export const pageTransferTypeConverter",
        "export const noteTransferTypeConverter",
    ] {
        assert_eq!(
            files.matches(declaration).count(),
            1,
            "expected exactly one `{declaration}`\n{files}"
        );
    }

    // The service module declares nothing, so it emits no `models.ts` at all —
    // an empty one would leave its barrel re-exporting a file with no exports,
    // which TypeScript rejects (`TS2306`, "is not a module").
    assert!(!output_path.join("svc/models.ts").exists());
    let module_index = fs::read_to_string(output_path.join("svc/index.ts")).unwrap();
    assert!(module_index.contains("export * from './services';"));
    assert!(!module_index.contains("./models"), "{module_index}");

    // It imports the types from the modules that own them.
    let services = fs::read_to_string(output_path.join("svc/services.ts")).unwrap();
    for expected in [
        "import { pageTransferTypeConverter } from '../a/page/models';",
        "import { noteTransferTypeConverter } from '../b/note/models';",
    ] {
        assert!(services.contains(expected), "{expected}\n{services}");
    }
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_json_emits_complete_property_name_matcher() {
    let temp_dir = unique_output_path("ts-json-property-names-complete");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("names.yaml");
    fs::write(&input_path, TYPESCRIPT_PROPERTY_NAMES_SCHEMA).unwrap();
    let output_path = temp_dir.join("names");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.ts")).unwrap();
    for expected in [
        "invalid property name",
        "PATTERN_",
        "a@example.com",
        "b@example.com",
        "must be a valid email",
    ] {
        assert!(rendered.contains(expected), "{expected}\n{rendered}");
    }
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_json_materializes_closed_values_and_nullable_defaults() {
    let temp_dir = unique_output_path("ts-json-materialized-closed");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("closed.yaml");
    fs::write(&input_path, TYPESCRIPT_MATERIALIZED_CLOSED_SCHEMA).unwrap();
    let output_path = temp_dir.join("closed");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: nexgen::generator::TsDateTimeTypes::Temporal,
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.ts")).unwrap();
    for expected in [
        "fixedBlob?: Uint8Array;",
        "businessDay?: Temporal.PlainDate;",
        "fallbackBlob?: Uint8Array | null;",
        "export const DEFAULT_FALLBACK_BLOB = __nexgenDefinitions.base64ToBytes(\"aGk=\", '', [])!;",
        "must equal \"aGk=\"",
        "must be one of [\"2024-01-01\", \"2024-01-02\"]",
        "base64ToBytes(raw[\"fixedBlob\"]",
        "bytesToBase64(value.fixedBlob)",
        "serializeTemporalDate(value.businessDay)",
    ] {
        assert!(rendered.contains(expected), "{expected}\n{rendered}");
    }
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_json_deprecates_types_fields_services_and_operations() {
    let temp_dir = unique_output_path("ts-json-deprecation");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("jobs.nexusrpc.yaml");
    fs::write(&input_path, TYPESCRIPT_DEPRECATION_SCHEMA).unwrap();
    let output_path = temp_dir.join("jobs");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let models = fs::read_to_string(output_path.join("models.ts")).unwrap();
    let services = fs::read_to_string(output_path.join("services.ts")).unwrap();
    assert_eq!(models.matches("@deprecated").count(), 2, "{models}");
    assert_eq!(services.matches("@deprecated").count(), 2, "{services}");
    assert!(services.contains("export const jobApi"), "{services}");
    assert!(services.contains("  acceptJob:"), "{services}");
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_json_uses_own_wire_keys_and_prototype_safe_maps() {
    let temp_dir = unique_output_path("typescript-json-hostile-wire-keys");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("wire-keys.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
title: WireKeys
type: object
required: [content-type]
properties:
  content-type: { type: string }
additionalProperties: true
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("out");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();

    let models = fs::read_to_string(output_path.join("models.ts")).unwrap();
    assert!(models.contains("export interface WireKeys {"), "{models}");
    assert!(
        models.contains("Object.prototype.hasOwnProperty.call(raw, \"content-type\")"),
        "{models}"
    );
    assert!(models.contains("raw[\"content-type\"]"), "{models}");
    assert!(models.contains("out[\"content-type\"] ="), "{models}");
    assert!(models.contains("Object.create(null)"), "{models}");
    assert!(!models.contains("raw.content-type"), "{models}");
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_json_prose_and_wire_literals_do_not_trigger_long_import() {
    let temp_dir = unique_output_path("typescript-json-description-long");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("description.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
title: Description
description: Java emits Long here, but TypeScript does not use that type.
type: object
properties:
  value: { type: integer }
  marker: { type: string, const: Long }
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("out");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();

    let models = fs::read_to_string(output_path.join("models.ts")).unwrap();
    assert!(models.contains("Java emits Long here"), "{models}");
    assert!(models.contains("\"Long\""), "{models}");
    assert!(!models.contains("from 'long'"), "{models}");
    fs::remove_dir_all(temp_dir).unwrap();
}
