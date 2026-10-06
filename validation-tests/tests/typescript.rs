//! TypeScript toolchain integration tests.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nexgen::generator::GenerationMode;
use nexgen::language::Language;
use nexgen::nexgen_config::NexgenConfig;
use nexgen::{GenerateRequest, generate_to_file};

mod support;
use support::common;

#[path = "support/json_schema/selected_tests.rs"]
mod json_schema_tests;

const JSON_SCHEMA_TARGET: support::json_schema::toolchain::Target =
    support::json_schema::toolchain::Target::TypeScript;

use common::{json_input_path, wit_input_path, write_bare_ref_alias_closure};

#[test]
fn typescript_hostile_documentation_is_wrapped_escaped_and_typechecks() {
    support::doc_rendering::typescript_hostile_documentation_is_wrapped_escaped_and_typechecks();
}

const PRIMARY_EXAMPLE_ID: &str = "workflow-service";

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

const TYPESCRIPT_WAVE7_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [matrix, blobs, names, nums, count, label]
properties:
  empty: { $ref: "#/$defs/Empty" }
  matrix:
    type: array
    items:
      type: array
      items: { type: integer }
      uniqueItems: true
  blobs:
    type: array
    items: { type: string, contentEncoding: base64 }
    uniqueItems: true
  names:
    type: array
    items: { type: string }
    contains: { minLength: 2 }
  nums:
    type: array
    items: { type: integer }
    contains: { minimum: 5 }
  count: { type: integer }
  label: { type: string, minLength: 0, maxLength: 4, pattern: "^a/b" }
$defs:
  Empty:
    type: object
    additionalProperties: false
"##;

const TYPESCRIPT_CROSS_MODULE_SHAPES: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Circle:
    type: object
    required: [kind]
    properties:
      kind: { type: string, const: circle }
      r: { type: number }
  Square:
    type: object
    required: [kind]
    properties:
      kind: { type: string, const: square }
      s: { type: number }
"##;

const TYPESCRIPT_CROSS_MODULE_MAIN: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  shape:
    oneOf:
      - { $ref: "shapes.yaml#/$defs/Circle" }
      - { $ref: "shapes.yaml#/$defs/Square" }
      - { type: string }
"##;

const TYPESCRIPT_NULLABLE_ELEMENT_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [tags, nums, blobs]
properties:
  tags:
    type: array
    items:
      oneOf: [{ type: string }, { type: "null" }]
    uniqueItems: true
    contains: { minLength: 2 }
  nums:
    type: array
    items:
      oneOf: [{ type: number }, { type: "null" }]
    contains: { maximum: 5 }
  blobs:
    type: array
    items:
      oneOf: [{ type: string, contentEncoding: base64 }, { type: "null" }]
    uniqueItems: true
"##;

const TYPESCRIPT_FRACTIONAL_SECOND_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [createdAt]
properties:
  createdAt: { type: string, format: date-time }
"##;

/// TypeScript-specific closure for the Wave 2 matcher/materialization surface:
/// a mixed object with recursively converted typed extras, full scalar
/// `contains` matchers, the supported `propertyNames` predicates, and sibling
/// wire-string assertions around materialized temporal/bytes values.
const TYPESCRIPT_CONFORMANCE_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
minProperties: 1
properties:
  id: { type: string }
  integral:
    type: array
    items: { type: number }
    contains: { type: integer, minimum: 2, multipleOf: 2 }
  emails:
    type: array
    items: { type: string }
    contains: { type: string, minLength: 6, pattern: "@example\\.com$", format: email }
  occurredAt:
    type: string
    format: date-time
    minLength: 20
    pattern: "^2024-"
  payload:
    type: string
    contentEncoding: base64
    minLength: 4
    pattern: "^a"
additionalProperties:
  type: array
  items: { type: string, format: date }
"#;

const TYPESCRIPT_WAVE3_MATRIX_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
required:
  - requiredPlain
  - requiredNullable
  - constString
  - constInteger
  - constNumber
  - constBoolean
  - zeroConst
  - enumString
  - enumInteger
  - enumNumber
  - enumBoolean
  - bounded
  - matchedNumbers
  - matchedText
  - matchedBoolean
  - checkedArray
  - names
  - mixed
properties:
  requiredPlain: { type: string }
  requiredNullable:
    oneOf: [{ type: string }, { type: "null" }]
  optionalPlain: { type: string }
  optionalNullable:
    oneOf: [{ type: string }, { type: "null" }]
  optionalDefault: { type: string, default: fallback }
  optionalNullableDefault:
    oneOf: [{ type: string }, { type: "null" }]
    default: nullable-fallback
  constString: { type: string, const: fixed }
  constInteger: { type: integer, const: 2 }
  constNumber: { type: number, const: 1.5 }
  constBoolean: { type: boolean, const: true }
  zeroConst: { type: number, const: 0 }
  enumString: { type: string, enum: [a, b] }
  enumInteger: { type: integer, enum: [1, 2] }
  enumNumber: { type: number, enum: [1.5, 2.5] }
  enumBoolean: { type: boolean, enum: [true, false] }
  bounded: { type: number, exclusiveMaximum: 10, multipleOf: 5 }
  matchedNumbers:
    type: array
    items: { type: number }
    contains:
      type: number
      minimum: -10
      exclusiveMaximum: 10
      multipleOf: 5
      enum: [-5, 5]
  matchedText:
    type: array
    items: { type: string }
    contains:
      type: string
      minLength: 6
      maxLength: 32
      pattern: "@example\\.com$"
      format: email
      enum: [dev@example.com, ops@example.com]
  matchedBoolean:
    type: array
    items: { type: boolean }
    contains: { type: boolean, const: true }
  checkedArray:
    type: array
    items: { type: string }
    minItems: 2
    maxItems: 3
    uniqueItems: true
    contains: { type: string, const: ok }
    minContains: 1
    maxContains: 1
  names:
    type: object
    additionalProperties: { type: string }
    propertyNames:
      type: string
      minLength: 6
      maxLength: 32
      pattern: "@example\\.com$"
      format: email
      enum: [a@example.com, b@example.com]
  mixed:
    type: object
    minProperties: 2
    maxProperties: 3
    required: [id]
    properties:
      id: { type: string }
    additionalProperties:
      type: array
      items: { type: string, format: date }
"##;

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn descriptor_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/descriptors/temporal_api.bin")
}

// The descriptor set of the stream-service sample proto.
fn stream_descriptor_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/descriptors/stream_service.bin")
}

fn linked_inputs_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/inputs/deps")
}

fn example_input_paths(root: &Path, example_id: &str) -> Vec<PathBuf> {
    let input = wit_input_path(root, example_id);
    let mut paths = vec![input.clone()];
    if fs::read_to_string(&input)
        .unwrap()
        .contains("use nexus:temporal-types/")
    {
        paths.push(linked_inputs_path(root));
    }
    paths
}

fn typescript_root(root: &Path) -> PathBuf {
    root.join("advanced/samples/typescript")
}

fn typescript_output_path(root: &Path, example_id: &str) -> PathBuf {
    typescript_root(root).join("wit").join(example_id)
}

fn samples_typescript_root(root: &Path) -> PathBuf {
    root.join("samples/typescript")
}

fn typescript_example_ids(root: &Path) -> Vec<String> {
    let mut ids = fs::read_dir(root.join("advanced/samples/inputs"))
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let example_id = if path.is_file() {
                path.file_stem()?.to_string_lossy().into_owned()
            } else if path.join("main.wit").is_file() {
                path.file_name()?.to_string_lossy().into_owned()
            } else {
                return None;
            };
            if typescript_output_path(root, &example_id).is_dir() {
                Some(example_id)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn ensure_typescript_dependencies(example_dir: &Path) {
    if example_dir.join("node_modules").exists() {
        return;
    }

    let install_status = Command::new("npm")
        .current_dir(example_dir)
        .args(["install", "--no-fund", "--no-audit"])
        .status()
        .unwrap();
    assert!(install_status.success());
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

fn assert_no_empty_else_blocks(files: &BTreeMap<PathBuf, String>) {
    for (path, contents) in files {
        let lines = contents.lines().collect::<Vec<_>>();
        for pair in lines.windows(2) {
            assert!(
                pair[0].trim() != "} else {" || pair[1].trim() != "}",
                "empty else block in {}",
                path.display()
            );
        }
    }
}

fn generate_formatted_typescript_output(root: &Path, example_id: &str, output_path: &Path) {
    ensure_typescript_dependencies(&typescript_root(root));

    let mut command = Command::new(support::nexgen_binary());
    command
        .arg("typescript")
        .args(example_input_paths(root, example_id))
        .args([
            "--descriptors",
            descriptor_path(root).to_str().unwrap(),
            "--descriptors",
            stream_descriptor_path(root).to_str().unwrap(),
            "--support-package",
            "./support",
            "--output",
            output_path.to_str().unwrap(),
            "--native-api",
        ]);
    if example_id == PRIMARY_EXAMPLE_ID {
        command.arg("--system-nexus");
    }
    let status = command.status().unwrap();
    assert!(status.success());

    let format_status = Command::new("npm")
        .current_dir(typescript_root(root))
        .args([
            "exec",
            "--",
            "prettier",
            "--write",
            "--print-width",
            "88",
            output_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(format_status.success());
}

fn generate_formatted_json_typescript_output(
    root: &Path,
    example_id: &str,
    output_path: &Path,
    generate_native_api: bool,
) {
    generate_formatted_json_typescript_output_repr(
        root,
        example_id,
        output_path,
        generate_native_api,
        None,
    );
}

/// Like `generate_formatted_json_typescript_output`, but reads the input from
/// `input_id` (so the repr variants reuse `temporal.yaml`) and threads an
/// optional `--date-time-types`.
fn generate_formatted_json_typescript_output_repr(
    root: &Path,
    input_id: &str,
    output_path: &Path,
    generate_native_api: bool,
    repr: Option<&str>,
) {
    ensure_typescript_dependencies(&typescript_root(root));

    let input_path = json_input_path(root, input_id);
    let mut args = vec![
        "typescript",
        input_path.to_str().unwrap(),
        "--support-package",
        "./support",
        "--output",
        output_path.to_str().unwrap(),
    ];
    if let Some(repr) = repr {
        args.push("--date-time-types");
        args.push(repr);
    }
    if generate_native_api {
        args.push("--native-api");
    }

    let status = Command::new(support::nexgen_binary())
        .args(args)
        .status()
        .unwrap();
    assert!(status.success());

    let format_status = Command::new("npm")
        .current_dir(typescript_root(root))
        .args([
            "exec",
            "--",
            "prettier",
            "--write",
            "--print-width",
            "88",
            output_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(format_status.success());
}

fn unique_output_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("nexgen-{label}-{unique}-{counter}"))
}

fn unique_typescript_runtime_dir(label: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("nexgen-{label}-"))
        .tempdir_in(samples_typescript_root(&project_root()))
        .unwrap()
}

#[test]
fn generated_wit_examples_format_successfully() {
    let root = project_root();
    for example_id in typescript_example_ids(&root) {
        let output_path = unique_output_path(&format!("typescript-{example_id}"));
        generate_formatted_typescript_output(&root, &example_id, &output_path);
        fs::remove_dir_all(output_path).unwrap();
    }
}

#[test]
fn typescript_json_examples_render_expected_language_features() {
    let root = project_root();
    for example_id in ["chat", "kb", "showcase", "temporal"] {
        let output_path = unique_output_path(&format!("typescript-json-{example_id}"));
        generate_formatted_json_typescript_output(&root, example_id, &output_path, false);
        let rendered = read_typescript_output_files(&output_path);
        assert_no_empty_else_blocks(&rendered);
        if example_id == "showcase" {
            let all = rendered.values().cloned().collect::<Vec<_>>().join("\n");
            // Scalar defaults → emitted DEFAULT_<FIELD> module constants.
            assert!(all.contains("export const DEFAULT_GREETING = \"hello\";"));
            assert!(all.contains("export const DEFAULT_DEBUG = false;"));
            // `deprecated` → JSDoc @deprecated tag; `title` → JSDoc summary line.
            assert!(all.contains("@deprecated"));
            assert!(all.contains("Retry budget"));
            // `x-ts-name` override (Stage 4): the interface member + (de)serialize
            // use the override while the wire key stays `legacyId`.
            assert!(all.contains("legacyIdTs?: string;"));
            assert!(all.contains("legacyIdTs = raw[\"legacyId\"];"));
            assert!(all.contains("out[\"legacyId\"] = value.legacyIdTs;"));
            // A free-form object stays an anonymous `Record` — narrowed on the
            // object token as a union branch, and the sole member of the named
            // `Extras` interface.
            assert!(all.contains("payload?: Record<string, unknown> | string;"));
            assert!(all.contains("export interface Extras {"));
            assert!(all.contains("additionalProperties: Record<string, unknown>;"));
            // A tagged union whose branches are written inline: each branch names
            // itself with `x-ts-name` and is emitted as an interface + converter.
            assert!(all.contains("export type Note = TextNote | LinkNote;"));
            assert!(all.contains("export interface TextNote {"));
            assert!(all.contains(
                "export const linkNoteTransferTypeConverter =\n  new (class implements TransferTypeConverter<LinkNote> {"
            ));
            // The lone inline object branch of a property union derives its name
            // from the union it belongs to.
            assert!(all.contains("detail?: ShowcaseDetailObject | string;"));
            assert!(all.contains("export interface ShowcaseDetailObject {"));
            assert!(all.contains("serializeShowcaseDetail(value.detail)"));
            // Each operation carries its models' converters as operation type
            // info; the `x-ts-name` override flows into the converter identifier.
            assert!(all.contains(
                "inputType: { transferTypeConverter: getShowcaseInputTransferTypeConverter },"
            ));
            assert!(
                all.contains(
                    "outputType: { transferTypeConverter: showcaseTransferTypeConverter },"
                )
            );
            assert!(all.contains("export const contactTsTransferTypeConverter ="));
        }
        if example_id == "chat" {
            let services = rendered
                .get(std::path::Path::new("services.ts"))
                .expect("chat services module");
            // A void side has no value to convert, so it carries no type info.
            assert!(services.contains("ping: nexus.operation<void, void>({ name: \"Ping\" }),"));
            assert!(services.contains(
                "inputType: { transferTypeConverter: sendMessageInputTransferTypeConverter },"
            ));
        }
        if example_id == "kb" {
            // A cross-module I/O model's converter imports as a value from the
            // module that declares it, alongside the type-only model import.
            let services = rendered
                .get(std::path::Path::new("kb/services.ts"))
                .expect("kb services module");
            assert!(services.contains(
                "import { blockTransferTypeConverter } from \"../content/block/models\";"
            ));
            assert!(
                services
                    .contains("outputType: { transferTypeConverter: pageTransferTypeConverter },")
            );
        }
        fs::remove_dir_all(output_path).unwrap();
    }
    // The `--date-time-types` date/temporal variants of the temporal example.
    for (output_id, repr) in [("temporal-date", "date"), ("temporal-temporal", "temporal")] {
        let output_path = unique_output_path(&format!("typescript-json-{output_id}"));
        generate_formatted_json_typescript_output_repr(
            &root,
            "temporal",
            &output_path,
            false,
            Some(repr),
        );
        let rendered = read_typescript_output_files(&output_path);
        assert_no_empty_else_blocks(&rendered);
        fs::remove_dir_all(output_path).unwrap();
    }
}

#[test]
fn typescript_json_api_examples_render_successfully() {
    let root = project_root();
    for example_id in ["chat", "kb", "showcase", "temporal"] {
        let output_path = unique_output_path(&format!("typescript-json-api-{example_id}"));
        generate_formatted_json_typescript_output(&root, example_id, &output_path, true);
        let rendered = read_typescript_output_files(&output_path);
        assert_no_empty_else_blocks(&rendered);
        fs::remove_dir_all(output_path).unwrap();
    }
    for (output_id, repr) in [("temporal-date", "date"), ("temporal-temporal", "temporal")] {
        let output_path = unique_output_path(&format!("typescript-json-api-{output_id}"));
        generate_formatted_json_typescript_output_repr(
            &root,
            "temporal",
            &output_path,
            true,
            Some(repr),
        );
        let rendered = read_typescript_output_files(&output_path);
        assert_no_empty_else_blocks(&rendered);
        fs::remove_dir_all(output_path).unwrap();
    }
}

#[test]
fn typescript_json_bare_ref_root_alias_typechecks_and_uses_target_converter() {
    let temp_dir = unique_typescript_runtime_dir("ts-json-bare-ref-alias");
    let input = write_bare_ref_alias_closure(temp_dir.path());
    let output_path = temp_dir.path().join("output");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input],
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

    let alias = fs::read_to_string(output_path.join("alias/alternate/models.ts")).unwrap();
    assert!(alias.contains("export type Alternate = Main;"), "{alias}");
    assert!(
        alias.contains(
            "export const alternateTransferTypeConverter: TransferTypeConverter<Alternate> = mainTransferTypeConverter;"
        ),
        "{alias}"
    );
    assert!(!alias.contains("export interface Alternate"), "{alias}");
    let target = fs::read_to_string(output_path.join("target/main/models.ts")).unwrap();
    assert!(target.contains("export type Mirror = Main;"), "{target}");
    assert!(!target.contains("export interface Mirror"), "{target}");
    let service = fs::read_to_string(output_path.join("service/services.ts")).unwrap();
    assert!(service.contains("Alternate"), "{service}");
    assert!(
        service.contains("alternateTransferTypeConverter"),
        "{service}"
    );
    let holder = fs::read_to_string(output_path.join("service/models.ts")).unwrap();
    assert!(holder.contains("item: Alternate;"), "{holder}");
    assert!(
        holder.contains("alternateTransferTypeConverter.fromTransferType"),
        "{holder}"
    );

    let runtime = output_path.join("alias.test.ts");
    fs::write(
        &runtime,
        r#"import { describe, expect, test } from "vitest";
import { alternateTransferTypeConverter } from "./alias/alternate/models";

describe("bare-ref alias", () => {
  test("uses the target wire contract", () => {
    expect(alternateTransferTypeConverter.fromTransferType({ value: "ok" })).toEqual({ value: "ok" });
    expect(() => alternateTransferTypeConverter.fromTransferType({})).toThrow();
  });
});
"#,
    )
    .unwrap();
    typecheck_generated_typescript(&output_path, "bare-ref alias");
    run_generated_typescript_test(temp_dir.path(), &runtime, "bare-ref alias");
}

#[test]
fn typescript_json_same_module_alias_chain_orders_converter_values() {
    let temp_dir = unique_typescript_runtime_dir("ts-json-same-module-alias-chain");
    let input_path = temp_dir.path().join("chain.yaml");
    fs::write(
        &input_path,
        r##"$schema: https://json-schema.org/draft/2020-12/schema
title: Chain
type: object
additionalProperties: false
required: [item, longItem]
properties:
  item: { $ref: "#/$defs/A" }
  longItem: { $ref: "#/$defs/LongA" }
$defs:
  A: { $ref: "#/$defs/Z" }
  Z: { $ref: "#/$defs/Target" }
  LongA: { $ref: "#/$defs/LongM" }
  LongM: { $ref: "#/$defs/LongZ" }
  LongZ: { $ref: "#/$defs/Target" }
  Target:
    type: object
    additionalProperties: false
    required: [value]
    properties:
      value: { type: string }
"##,
    )
    .unwrap();
    let output_path = temp_dir.path().join("output");
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
    let target = models
        .find("export const targetTransferTypeConverter")
        .expect("target converter");
    let z = models
        .find("export const zTransferTypeConverter")
        .expect("Z converter");
    let a = models
        .find("export const aTransferTypeConverter")
        .expect("A converter");
    assert!(target < z && z < a, "{models}");
    let long_z = models
        .find("export const longZTransferTypeConverter")
        .expect("LongZ converter");
    let long_m = models
        .find("export const longMTransferTypeConverter")
        .expect("LongM converter");
    let long_a = models
        .find("export const longATransferTypeConverter")
        .expect("LongA converter");
    assert!(
        target < long_z && long_z < long_m && long_m < long_a,
        "{models}"
    );

    let runtime = output_path.join("alias-chain.test.ts");
    fs::write(
        &runtime,
        r#"import { expect, test } from "vitest";
import { aTransferTypeConverter, longATransferTypeConverter } from "./models";

test("same-module alias chains delegate after initialization", () => {
  expect(aTransferTypeConverter.fromTransferType({ value: "ok" })).toEqual({ value: "ok" });
  expect(() => aTransferTypeConverter.fromTransferType({})).toThrow();
  expect(longATransferTypeConverter.fromTransferType({ value: "long" })).toEqual({ value: "long" });
});
"#,
    )
    .unwrap();
    typecheck_generated_typescript(&output_path, "same-module bare-ref alias chain");
    run_generated_typescript_test(
        temp_dir.path(),
        &runtime,
        "same-module bare-ref alias chain",
    );
}

#[test]
fn typescript_json_emits_complete_matchers_and_typed_mixed_extras() {
    let temp_dir = unique_typescript_runtime_dir("ts-json-wave2-conformance");
    let input_path = temp_dir.path().join("probe.yaml");
    fs::write(&input_path, TYPESCRIPT_CONFORMANCE_SCHEMA).unwrap();
    let output_path = temp_dir.path().join("probe");

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

    assert!(
        rendered.contains("additionalProperties: Record<string, Temporal.PlainDate[]>;"),
        "{rendered}"
    );
    assert!(
        rendered.contains("parseTemporalDate(element, `${path}[${index}]`, violations)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("serializeTemporalDate(element)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("if (PROBE_DECLARED.has(key))"),
        "{rendered}"
    );
    assert!(
        rendered.contains("collides with declared property"),
        "{rendered}"
    );
    assert!(
        rendered.contains("typeof element === 'number' && Number.isSafeInteger(element)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("PATTERN_") && rendered.contains(".test(element)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("must be a valid email") || rendered.contains(".test(element)"),
        "{rendered}"
    );
    assert!(
        rendered.contains(
            "const wire = __nexgenDefinitions.serializeTemporalDateTime(value.occurredAt)"
        ),
        "{rendered}"
    );
    assert!(
        rendered.contains("const wire") && rendered.contains("bytesToBase64(value.payload)"),
        "{rendered}"
    );

    // This deliberately does not use Vitest's default `.test.ts` suffix. The
    // full sample suite runs in parallel with this focused test and must not
    // discover a temporary file that may disappear before it imports it.
    let runtime_test = output_path.join("wave2-conformance.ts");
    fs::write(
        &runtime_test,
r#"import { expect, test } from "vitest";
import { ApplicationFailure } from "@temporalio/common";
import { probeTransferTypeConverter } from "./models.ts";
import type { Violation } from "./definitions.ts";

function violations(fn: () => unknown): Violation[] {
  try {
    fn();
  } catch (error) {
    expect(error).toBeInstanceOf(ApplicationFailure);
    const failure = error as ApplicationFailure;
    expect(failure.type).toBe("PayloadValidationError");
    expect(failure.nonRetryable).toBe(true);
    return failure.details?.[0] as Violation[];
  }
  throw new Error("expected validation failure");
}

test("mixed extras, matchers, and wire-string constraints run both ways", () => {
  const model = probeTransferTypeConverter.fromTransferType({
    id: "p1",
    integral: [1.5, 2],
    emails: ["dev@example.com"],
    occurredAt: "2024-01-01T00:00:00Z",
    payload: "aGk=",
    schedule: ["2024-01-02"],
  });
  expect(model.additionalProperties.schedule?.[0]).toBeInstanceOf(Temporal.PlainDate);
  expect(probeTransferTypeConverter.toTransferType(model)).toEqual({
    id: "p1",
    integral: [1.5, 2],
    emails: ["dev@example.com"],
    occurredAt: "2024-01-01T00:00:00Z",
    payload: "aGk=",
    schedule: ["2024-01-02"],
  });
  expect(violations(() => probeTransferTypeConverter.fromTransferType({ integral: [2.5] }))).not.toHaveLength(0);
  expect(violations(() => probeTransferTypeConverter.fromTransferType({ emails: ["dev@invalid"] }))).not.toHaveLength(0);
  expect(violations(() => probeTransferTypeConverter.fromTransferType({ payload: "Pj4+" }))[0]?.reason).toContain("pattern");
  expect(violations(() => probeTransferTypeConverter.toTransferType({
    ...model,
    additionalProperties: { id: [Temporal.PlainDate.from("2024-01-02")] },
  }))[0]?.reason).toContain("collides with declared property");
});
"#,
    )
    .unwrap();
    let sample_root = samples_typescript_root(&project_root());
    ensure_typescript_dependencies(&sample_root);
    let runtime_relative = runtime_test.strip_prefix(&sample_root).unwrap();
    let runtime_config = temp_dir.path().join("vitest.config.ts");
    fs::write(
        &runtime_config,
        format!(
            "export default {{ test: {{ include: [{}] }} }};\n",
            serde_json::to_string(runtime_relative.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    let typecheck_status = Command::new("npm")
        .current_dir(&sample_root)
        .args(["run", "typecheck"])
        .status()
        .unwrap();
    assert!(
        typecheck_status.success(),
        "focused TypeScript conformance output did not typecheck"
    );
    let runtime_status = Command::new("npm")
        .current_dir(&sample_root)
        .args([
            "exec",
            "--",
            "vitest",
            "run",
            "--config",
            runtime_config.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(
        runtime_status.success(),
        "focused TypeScript runtime test failed"
    );
}

#[test]
fn typescript_json_wave3_pairwise_runtime_matrix() {
    let temp_dir = unique_typescript_runtime_dir("ts-json-wave3-matrix");
    let input_path = temp_dir.path().join("audit.yaml");
    fs::write(&input_path, TYPESCRIPT_WAVE3_MATRIX_SCHEMA).unwrap();
    let output_path = temp_dir.path().join("audit");
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

    let runtime_test = output_path.join("wave3-matrix.ts");
    fs::write(
        &runtime_test,
        r#"import { expect, test } from "vitest";
import { ApplicationFailure } from "@temporalio/common";
import { auditTransferTypeConverter } from "./models.ts";
import type { Violation } from "./definitions.ts";

const base = {
  requiredPlain: "present",
  requiredNullable: null,
  constString: "fixed",
  constInteger: 2,
  constNumber: 1.5,
  constBoolean: true,
  zeroConst: -0,
  enumString: "a",
  enumInteger: 1,
  enumNumber: 1.5,
  enumBoolean: false,
  bounded: -1e1,
  matchedNumbers: [5e0],
  matchedText: ["dev@example.com"],
  matchedBoolean: [true],
  checkedArray: ["ok", "x"],
  names: { "a@example.com": "x" },
  mixed: { id: "m", schedule: ["2024-01-01"] },
};

function violations(fn: () => unknown): Violation[] {
  try {
    fn();
  } catch (error) {
    expect(error).toBeInstanceOf(ApplicationFailure);
    const failure = error as ApplicationFailure;
    expect(failure.type).toBe("PayloadValidationError");
    expect(failure.nonRetryable).toBe(true);
    return failure.details?.[0] as Violation[];
  }
  throw new Error("expected validation failure");
}

test("presence, closed scalars, and numeric boundaries agree both ways", () => {
  const value = auditTransferTypeConverter.fromTransferType(base);
  expect(value.requiredNullable).toBeNull();
  expect(value.optionalPlain).toBeUndefined();
  expect(value.optionalNullable).toBeUndefined();
  expect(value.optionalDefault).toBeUndefined();
  expect(value.optionalNullableDefault).toBeUndefined();
  expect(Object.is(value.zeroConst, -0)).toBe(true);
  expect(value.mixed.additionalProperties.schedule?.[0]).toBeInstanceOf(Temporal.PlainDate);

  expect(violations(() => auditTransferTypeConverter.fromTransferType({ ...base, requiredPlain: undefined })))
    .toEqual(expect.arrayContaining([{ path: "requiredPlain", reason: "expected string" }]));
  expect(violations(() => auditTransferTypeConverter.fromTransferType({ ...base, requiredNullable: undefined })))
    .toEqual(expect.arrayContaining([{ path: "requiredNullable", reason: "expected string" }]));
  expect(violations(() => auditTransferTypeConverter.fromTransferType({ ...base, optionalPlain: null }))[0]?.reason)
    .toBe("explicit null not allowed");
  expect(violations(() => auditTransferTypeConverter.fromTransferType({ ...base, optionalDefault: null }))[0]?.reason)
    .toBe("explicit null not allowed");
  expect(auditTransferTypeConverter.fromTransferType({ ...base, optionalNullable: null }).optionalNullable).toBeNull();
  expect(auditTransferTypeConverter.fromTransferType({ ...base, optionalNullableDefault: null }).optionalNullableDefault).toBeNull();

  for (const [field, replacement] of [
    ["constString", "wrong"], ["constInteger", 3], ["constNumber", 2.5],
    ["constBoolean", false], ["enumString", "c"], ["enumInteger", 3],
    ["enumNumber", 3.5], ["enumBoolean", "no"],
  ] as const) {
    expect(violations(() => auditTransferTypeConverter.fromTransferType({ ...base, [field]: replacement }))
      .some(({ path }) => path === field)).toBe(true);
  }
  expect(violations(() => auditTransferTypeConverter.fromTransferType({ ...base, bounded: 10 }))[0]?.reason).toContain("must be < 10");
  expect(auditTransferTypeConverter.fromTransferType({ ...base, bounded: -15 }).bounded).toBe(-15);

  const invalidClosed = { ...value, constNumber: 2.5 as unknown as 1.5 };
  expect(violations(() => auditTransferTypeConverter.toTransferType(invalidClosed))[0]?.path).toBe("constNumber");
  expect(violations(() => auditTransferTypeConverter.toTransferType({ ...value, bounded: 10 }))[0]?.reason).toContain("must be < 10");
});

test("contains, propertyNames, arrays, and typed-extra counts aggregate both ways", () => {
  for (const replacement of [
    { matchedNumbers: [1] },
    { matchedText: ["bad@example.org"] },
    { matchedBoolean: [false] },
  ]) {
    const field = Object.keys(replacement)[0]!;
    expect(violations(() => auditTransferTypeConverter.fromTransferType({ ...base, ...replacement }))
      .some(({ path }) => path === field)).toBe(true);
  }
  const value = auditTransferTypeConverter.fromTransferType(base);
  expect(violations(() => auditTransferTypeConverter.toTransferType({ ...value, matchedNumbers: [1] }))
    .some(({ path }) => path === "matchedNumbers")).toBe(true);
  expect(violations(() => auditTransferTypeConverter.toTransferType({ ...value, matchedText: ["bad@example.org"] }))
    .some(({ path }) => path === "matchedText")).toBe(true);
  expect(violations(() => auditTransferTypeConverter.toTransferType({ ...value, matchedBoolean: [false] }))
    .some(({ path }) => path === "matchedBoolean")).toBe(true);

  const tooSmall = violations(() => auditTransferTypeConverter.fromTransferType({ ...base, checkedArray: [] }));
  expect(tooSmall.map(({ path }) => path)).toEqual(["checkedArray", "checkedArray"]);
  const crowded = violations(() => auditTransferTypeConverter.fromTransferType({
    ...base, checkedArray: ["ok", "ok", "x", "x"],
  }));
  expect(crowded.filter(({ path }) => path === "checkedArray").length).toBeGreaterThanOrEqual(3);
  expect(violations(() => auditTransferTypeConverter.toTransferType({ ...value, checkedArray: [] }))).not.toHaveLength(0);
  expect(violations(() => auditTransferTypeConverter.toTransferType({ ...value, checkedArray: ["ok", "ok", "x", "x"] }))).not.toHaveLength(0);

  for (const key of ["x", "bad key@example.com", "c@example.com"]) {
    const segment = /^[A-Za-z_][A-Za-z0-9_]*$/u.test(key) ? `.${key}` : `["${key}"]`;
    expect(violations(() => auditTransferTypeConverter.fromTransferType({ ...base, names: { [key]: "x" } }))
      .some(({ path }) => path === `names${segment}`)).toBe(true);
  }
  expect(violations(() => auditTransferTypeConverter.toTransferType({
    ...value, names: { additionalProperties: { "c@example.com": "x" } },
  })).some(({ path }) => path.includes("c@example.com"))).toBe(true);

  expect(violations(() => auditTransferTypeConverter.fromTransferType({ ...base, mixed: { id: "m" } }))[0]?.reason).toContain("at least 2 properties");
  expect(violations(() => auditTransferTypeConverter.fromTransferType({
    ...base,
    mixed: { id: "m", a: ["2024-01-01"], b: ["2024-01-02"], c: ["2024-01-03"] },
  }))[0]?.reason).toContain("at most 3 properties");
  expect(violations(() => auditTransferTypeConverter.toTransferType({
    ...value, mixed: { ...value.mixed, additionalProperties: {} },
  }))[0]?.reason).toContain("at least 2 properties");
  expect(violations(() => auditTransferTypeConverter.toTransferType({
    ...value,
    mixed: {
      ...value.mixed,
      additionalProperties: {
        a: [Temporal.PlainDate.from("2024-01-01")],
        b: [Temporal.PlainDate.from("2024-01-02")],
        c: [Temporal.PlainDate.from("2024-01-03")],
      },
    },
  }))[0]?.reason).toContain("at most 3 properties");
  expect(violations(() => auditTransferTypeConverter.toTransferType({
    ...value,
    mixed: {
      ...value.mixed,
      additionalProperties: { id: [Temporal.PlainDate.from("2024-01-01")] },
    },
  })).some(({ path, reason }) => path.endsWith("id") && reason.includes("collides with declared property"))).toBe(true);
});
"#,
    )
    .unwrap();
    let sample_root = samples_typescript_root(&project_root());
    ensure_typescript_dependencies(&sample_root);
    let runtime_relative = runtime_test.strip_prefix(&sample_root).unwrap();
    let runtime_config = temp_dir.path().join("vitest.config.ts");
    fs::write(
        &runtime_config,
        format!(
            "export default {{ test: {{ include: [{}] }} }};\n",
            serde_json::to_string(runtime_relative.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    let typecheck_status = Command::new("npm")
        .current_dir(&sample_root)
        .args(["run", "typecheck"])
        .status()
        .unwrap();
    assert!(
        typecheck_status.success(),
        "wave 3 output did not typecheck"
    );
    let runtime_status = Command::new("npm")
        .current_dir(&sample_root)
        .args([
            "exec",
            "--",
            "vitest",
            "run",
            "--config",
            runtime_config.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(runtime_status.success(), "wave 3 runtime matrix failed");
}

/// Runs the real compiler over generated output. `npm run typecheck` is scoped
/// by the sample `tsconfig.json`'s `include` list, which no temporary output
/// directory is in — so a generated module that does not even parse can slip
/// through a text-assertion suite. This invokes `tsc` on the emitted files
/// directly, from the sample root so `nexus-rpc`/`@types/node` resolve.
fn typecheck_generated_typescript(output_path: &Path, label: &str) {
    fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
        let mut entries = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                collect(&path, files);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("ts") {
                files.push(path);
            }
        }
    }

    let sample_root = samples_typescript_root(&project_root());
    ensure_typescript_dependencies(&sample_root);
    let mut files = Vec::new();
    collect(output_path, &mut files);
    let mut command = Command::new("npm");
    command.current_dir(&sample_root).args([
        "exec",
        "--",
        "tsc",
        "--ignoreConfig",
        "--noEmit",
        "--target",
        "ES2022",
        "--lib",
        "ES2022,esnext.temporal",
        "--module",
        "ES2022",
        "--moduleResolution",
        "bundler",
        "--strict",
        "--skipLibCheck",
        "--allowImportingTsExtensions",
        "--types",
        "node",
    ]);
    command.args(files.iter().map(|path| path.to_str().unwrap()));
    let status = command.status().unwrap();
    assert!(status.success(), "{label} did not typecheck");
}

/// Runs one generated-output vitest module from the sample root.
fn run_generated_typescript_test(temp_dir: &Path, runtime_test: &Path, label: &str) {
    let sample_root = samples_typescript_root(&project_root());
    let runtime_relative = runtime_test.strip_prefix(&sample_root).unwrap();
    let runtime_config = temp_dir.join("vitest.config.ts");
    fs::write(
        &runtime_config,
        format!(
            "export default {{ test: {{ include: [{}] }} }};\n",
            serde_json::to_string(runtime_relative.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    let status = Command::new("npm")
        .current_dir(&sample_root)
        .args([
            "exec",
            "--",
            "vitest",
            "run",
            "--config",
            runtime_config.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "{label} runtime test failed");
}

#[test]
fn typescript_json_wave7_discrete_defects_typecheck_and_run() {
    let temp_dir = unique_typescript_runtime_dir("ts-json-wave7");
    let input_path = temp_dir.path().join("probe.yaml");
    fs::write(&input_path, TYPESCRIPT_WAVE7_SCHEMA).unwrap();
    let output_path = temp_dir.path().join("probe");
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
    let runtime = fs::read_to_string(output_path.join("definitions.ts")).unwrap();

    // A closed object with no declared properties rejects every key, and the
    // zero-term join must not degenerate to `if () {`.
    assert!(!rendered.contains("if () {"), "{rendered}");
    // The inner `uniqueItems` loop carries its depth so the enclosing `index`
    // (which `path` interpolates) stays visible.
    assert!(
        rendered.contains("forEach((element1, index1) => {"),
        "{rendered}"
    );
    // Materialized elements compare by their canonical wire string.
    assert!(
        rendered.contains("const wireItems = value.blobs.map((element) =>"),
        "{rendered}"
    );
    // A typeless matcher falls back to the element type's runtime guard.
    assert!(
        rendered.contains("(element) => typeof element === 'string' &&"),
        "{rendered}"
    );
    assert!(
        rendered.contains("(element) => typeof element === 'number' && Number.isSafeInteger(element) && element >= 5"),
        "{rendered}"
    );
    // The serialize-side integer cap.
    assert!(
        rendered.contains("if (!Number.isSafeInteger(value.count)) {"),
        "{rendered}"
    );
    // The shared early-exit code-point scan replaces every `[...v].length`.
    assert!(!rendered.contains("[..."), "{rendered}");
    assert!(
        runtime.contains("export function codePointLength(value: string, limit = Number.POSITIVE_INFINITY): number {"),
        "{runtime}"
    );
    // `minLength: 0` constrains nothing, so no comparison is emitted for it.
    assert!(!rendered.contains("codePoints < 0"), "{rendered}");
    // The default `pattern` form is a module-level literal.
    assert!(rendered.contains("= /^a\\/b/u;"), "{rendered}");

    typecheck_generated_typescript(&output_path, "wave 7 output");

    let runtime_test = output_path.join("wave7-discrete.ts");
    fs::write(
        &runtime_test,
        r#"import { expect, test } from "vitest";
import { ApplicationFailure } from "@temporalio/common";
import { probeTransferTypeConverter, emptyTransferTypeConverter } from "./models.ts";
import type { Violation } from "./definitions.ts";

const base = {
  matrix: [[1, 2]],
  blobs: ["AQI="],
  names: ["ab"],
  nums: [9],
  count: 1,
  label: "a/b",
};

function violations(fn: () => unknown): Violation[] {
  try {
    fn();
  } catch (error) {
    if (error instanceof ApplicationFailure && error.type === "PayloadValidationError") {
      expect(error.nonRetryable).toBe(true);
      return error.details?.[0] as Violation[];
    }
    throw error;
  }
  return [];
}

test("a closed empty object rejects every member", () => {
  expect(emptyTransferTypeConverter.fromTransferType({})).toEqual({});
  expect(violations(() => emptyTransferTypeConverter.fromTransferType({ a: 1 }))).toEqual([
    { path: "a", reason: "unknown field" },
  ]);
});

test("a nested uniqueItems violation is pathed at the outer index", () => {
  expect(violations(() => probeTransferTypeConverter.fromTransferType({ ...base, matrix: [[7, 7], [1, 2]] })))
    .toEqual([{ path: "matrix[0]", reason: "duplicate items: element at index 1 equals index 0" }]);
  const model = probeTransferTypeConverter.fromTransferType(base);
  expect(violations(() => probeTransferTypeConverter.toTransferType({ ...model, matrix: [[7, 7], [1, 2]] })))
    .toEqual([{ path: "matrix[0]", reason: "duplicate items: element at index 1 equals index 0" }]);
});

test("materialized elements compare by wire string in both directions", () => {
  expect(violations(() => probeTransferTypeConverter.fromTransferType({ ...base, blobs: ["AQI=", "AQI="] })))
    .toEqual([{ path: "blobs", reason: "duplicate items: element at index 1 equals index 0" }]);
  const model = probeTransferTypeConverter.fromTransferType(base);
  expect(
    violations(() =>
      probeTransferTypeConverter.toTransferType({
        ...model,
        blobs: [new Uint8Array([1, 2]), new Uint8Array([1, 2])],
      }),
    ),
  ).toEqual([{ path: "blobs", reason: "duplicate items: element at index 1 equals index 0" }]);
});

test("a mistyped element aggregates instead of throwing out of the matcher", () => {
  expect(violations(() => probeTransferTypeConverter.fromTransferType({ ...base, names: [5] })))
    .toEqual([
      { path: "names[0]", reason: "expected string" },
      { path: "names", reason: "no element matches the required schema" },
    ]);
  // `"9" >= 5` is `true` in JS; the element guard keeps the match count honest.
  expect(violations(() => probeTransferTypeConverter.fromTransferType({ ...base, nums: ["9"] })))
    .toEqual([
      { path: "nums[0]", reason: "expected integer" },
      { path: "nums", reason: "no element matches the required schema" },
    ]);
});

test("the integer cap is re-checked before emit", () => {
  const model = probeTransferTypeConverter.fromTransferType(base);
  expect(violations(() => probeTransferTypeConverter.toTransferType({ ...model, count: 2 ** 60 })))
    .toEqual([{ path: "count", reason: "exceeds ±(2^53-1) integer cap" }]);
  expect(probeTransferTypeConverter.toTransferType(model)).toEqual(base);
});

test("length is the code-point count, early-exited against the bound", () => {
  const model = probeTransferTypeConverter.fromTransferType({ ...base, label: "a/b\u{1F600}" });
  expect(model.label).toBe("a/b\u{1F600}");
  expect(violations(() => probeTransferTypeConverter.fromTransferType({ ...base, label: "a/bxx" })))
    .toEqual([{ path: "label", reason: "must have length <= 4, got 5" }]);
});
"#,
    )
    .unwrap();
    run_generated_typescript_test(temp_dir.path(), &runtime_test, "wave 7");
}

/// A sum-type `oneOf` branch that `$ref`s another input file. A `$ref` resolves
/// against the whole input closure ([[ref]] §"Type-name derivation"), so the
/// dispatcher must see the sibling module's shape — resolving only against the
/// current module silently collapses the union to its first local branch.
#[test]
fn typescript_json_dispatches_cross_module_ref_union_branches() {
    let temp_dir = unique_typescript_runtime_dir("ts-json-cross-module-union");
    let shapes_path = temp_dir.path().join("shapes.yaml");
    fs::write(&shapes_path, TYPESCRIPT_CROSS_MODULE_SHAPES).unwrap();
    let main_path = temp_dir.path().join("main.yaml");
    fs::write(&main_path, TYPESCRIPT_CROSS_MODULE_MAIN).unwrap();
    let output_path = temp_dir.path().join("closure");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![main_path, shapes_path],
        support_package: Some("./support".to_string()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("main/models.ts")).unwrap();
    assert!(
        rendered.contains("shape?: Circle | Square | string;"),
        "{rendered}"
    );
    assert!(
        rendered.contains("import { circleTransferTypeConverter, squareTransferTypeConverter } from '../shapes/models';"),
        "{rendered}"
    );
    assert!(
        rendered.contains("case \"circle\":") && rendered.contains("case \"square\":"),
        "{rendered}"
    );
    typecheck_generated_typescript(&output_path, "cross-module union output");

    let runtime_test = output_path.join("cross-module-union.ts");
    fs::write(
        &runtime_test,
        r#"import { expect, test } from "vitest";
import { ApplicationFailure } from "@temporalio/common";
import { mainTransferTypeConverter } from "./main/models.ts";

test("every cross-file branch round-trips", () => {
  for (const wire of [{ shape: { kind: "square", s: 2 } }, { shape: { kind: "circle", r: 1 } }, { shape: "x" }]) {
    const model = mainTransferTypeConverter.fromTransferType(wire);
    expect(mainTransferTypeConverter.toTransferType(model)).toEqual(wire);
  }
  expect(() => mainTransferTypeConverter.fromTransferType({ shape: { kind: "tri" } })).toThrow(
    ApplicationFailure,
  );
});
"#,
    )
    .unwrap();
    run_generated_typescript_test(temp_dir.path(), &runtime_test, "cross-module union");
}

#[test]
fn typescript_json_guards_nullable_elements_in_array_keywords() {
    let temp_dir = unique_typescript_runtime_dir("ts-json-nullable-elements");
    let input_path = temp_dir.path().join("bag.yaml");
    fs::write(&input_path, TYPESCRIPT_NULLABLE_ELEMENT_SCHEMA).unwrap();
    let output_path = temp_dir.path().join("bag");
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

    // The element kind comes from the wrapper's non-null branch, so the matcher
    // is guarded and `null` can never reach the bare comparison.
    assert!(
        rendered.contains("(element) => typeof element === 'string' &&"),
        "{rendered}"
    );
    assert!(
        rendered.contains(
            "(element) => typeof element === 'number' && Number.isFinite(element) && element <= 5"
        ),
        "{rendered}"
    );
    // A nullable materialized element still compares by its canonical wire form.
    assert!(
        rendered.contains(
            "const wireItems = value.blobs.map((element) => element === null ? null : __nexgenDefinitions.bytesToBase64(element));"
        ),
        "{rendered}"
    );

    typecheck_generated_typescript(&output_path, "nullable-element output");

    let runtime_test = output_path.join("nullable-elements.ts");
    fs::write(
        &runtime_test,
        r#"import { expect, test } from "vitest";
import { ApplicationFailure } from "@temporalio/common";
import { bagTransferTypeConverter } from "./models.ts";
import type { Violation } from "./definitions.ts";

const base = { tags: ["ab"], nums: [1], blobs: ["AQI="] };

function violations(fn: () => unknown): Violation[] {
  try {
    fn();
  } catch (error) {
    if (error instanceof ApplicationFailure && error.type === "PayloadValidationError") {
      expect(error.nonRetryable).toBe(true);
      return error.details?.[0] as Violation[];
    }
    throw error;
  }
  return [];
}

test("two nulls are one duplicate value", () => {
  expect(violations(() => bagTransferTypeConverter.fromTransferType({ ...base, tags: [null, null] })))
    .toContainEqual({ path: "tags", reason: "duplicate items: element at index 1 equals index 0" });
  const model = bagTransferTypeConverter.fromTransferType(base);
  expect(violations(() => bagTransferTypeConverter.toTransferType({ ...model, blobs: [null, null] })))
    .toEqual([{ path: "blobs", reason: "duplicate items: element at index 1 equals index 0" }]);
});

test("a null element never satisfies a scalar matcher", () => {
  // `null <= 5` is `true` in JS; the element guard keeps `null` out of the count.
  expect(violations(() => bagTransferTypeConverter.fromTransferType({ ...base, nums: [null] })))
    .toEqual([{ path: "nums", reason: "no element matches the required schema" }]);
  const model = bagTransferTypeConverter.fromTransferType(base);
  expect(violations(() => bagTransferTypeConverter.toTransferType({ ...model, nums: [null] })))
    .toEqual([{ path: "nums", reason: "no element matches the required schema" }]);
});

test("a nullable element round-trips alongside a matching one", () => {
  const wire = { tags: ["ab", null], nums: [1, null], blobs: [null, "AQI="] };
  const model = bagTransferTypeConverter.fromTransferType(wire);
  expect(bagTransferTypeConverter.toTransferType(model)).toEqual(wire);
});
"#,
    )
    .unwrap();
    run_generated_typescript_test(temp_dir.path(), &runtime_test, "nullable elements");
}

/// A `const` member is emitted `readonly`, but an **optional** member is assigned
/// after the result literal is built — which `readonly` forbids (TS2540), so the
/// generated module did not compile for any optional `const`, materialized or not.
#[test]
fn typescript_json_optional_const_members_typecheck() {
    let temp_dir = unique_typescript_runtime_dir("ts-json-optional-const");
    let input_path = temp_dir.path().join("pin.yaml");
    fs::write(
        &input_path,
        r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  plain: { type: string, const: "x" }
  num: { type: integer, const: 3 }
  tag: { type: string, contentEncoding: base64, const: "aGk=" }
  span: { type: string, format: duration, const: "PT1H30M" }
"##,
    )
    .unwrap();
    let output_path = temp_dir.path().join("pin");
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
    // The member stays immutable to consumers; only the staging binding is mutable.
    assert!(rendered.contains("  readonly plain?: \"x\";"), "{rendered}");
    assert!(
        rendered.contains("const out: { -readonly [K in keyof Pin]: Pin[K] } = {"),
        "{rendered}"
    );
    typecheck_generated_typescript(&output_path, "optional-const output");

    let runtime_test = output_path.join("optional-const.ts");
    fs::write(
        &runtime_test,
        r#"import { expect, test } from "vitest";
import { ApplicationFailure } from "@temporalio/common";
import { pinTransferTypeConverter } from "./models.ts";

test("an optional const member round-trips and stays absent when omitted", () => {
  expect(pinTransferTypeConverter.toTransferType(pinTransferTypeConverter.fromTransferType({}))).toEqual({});
  const wire = { plain: "x", num: 3, tag: "aGk=", span: "PT1H30M" };
  expect(pinTransferTypeConverter.toTransferType(pinTransferTypeConverter.fromTransferType(wire))).toEqual(wire);
  try {
    pinTransferTypeConverter.fromTransferType({ plain: "y" });
    throw new Error("expected validation failure");
  } catch (error) {
    expect(error).toBeInstanceOf(ApplicationFailure);
    const failure = error as ApplicationFailure;
    expect(failure.type).toBe("PayloadValidationError");
    expect((failure.details?.[0] as Array<{ reason: string }>)[0]?.reason).toContain("must equal");
  }
});
"#,
    )
    .unwrap();
    run_generated_typescript_test(temp_dir.path(), &runtime_test, "optional const");
}

/// A fractional second wider than the target's capacity is **accepted and
/// truncated**, never rejected: Go and Java truncate at nine digits, Python at
/// six, and the `string` repr keeps the wire verbatim because it has no capacity
/// to lose. `Temporal.ZonedDateTime` caps at nanoseconds exactly like
/// `java.time`, and its ISO parser *throws* past nine digits — so the `temporal`
/// repr must truncate before constructing, or it both splits the accept set and
/// escapes as a bare `RangeError` instead of an aggregated payload-validation failure
/// (P11). The truncated forms below are byte-identical to the ones the generated
/// Go emits for the same inputs (measured).
#[test]
fn typescript_json_truncates_over_capacity_fractional_seconds() {
    let expectations: &[(nexgen::generator::TsDateTimeTypes, &str)] = &[
        // repr, the re-emitted `createdAt` for `…45.123456789012Z`
        (
            nexgen::generator::TsDateTimeTypes::Temporal,
            "2021-01-15T12:30:45.123456789Z",
        ),
        (
            nexgen::generator::TsDateTimeTypes::Date,
            "2021-01-15T12:30:45.123Z",
        ),
        (
            nexgen::generator::TsDateTimeTypes::String,
            "2021-01-15T12:30:45.123456789012Z",
        ),
    ];
    for (repr, expected) in expectations {
        let label = format!("{repr:?}").to_lowercase();
        let temp_dir = unique_typescript_runtime_dir(&format!("ts-json-fraction-{label}"));
        let input_path = temp_dir.path().join("clock.yaml");
        fs::write(&input_path, TYPESCRIPT_FRACTIONAL_SECOND_SCHEMA).unwrap();
        let output_path = temp_dir.path().join("clock");
        generate_to_file(&GenerateRequest {
            config: Default::default(),
            language: nexgen::language::Language::TypeScript,
            input_paths: vec![input_path],
            support_package: Some("./support".to_string()),
            descriptor_paths: Vec::new(),
            output_path: output_path.clone(),
            format: false,
            java_package_name: None,
            ts_date_time_types: *repr,
            dotnet_namespace: None,
        })
        .unwrap();
        let runtime = fs::read_to_string(output_path.join("definitions.ts")).unwrap();
        // Only the nanosecond-capacity repr needs the scan; emitting it elsewhere
        // would be dead code.
        assert_eq!(
            runtime.contains("function truncateTemporalFraction("),
            *repr == nexgen::generator::TsDateTimeTypes::Temporal,
            "{runtime}"
        );
        if *repr == nexgen::generator::TsDateTimeTypes::Temporal {
            assert!(
                runtime.contains(
                    "const canon = truncateTemporalFraction(canonicalizeTemporalDateTime(value));"
                ),
                "{runtime}"
            );
            // A throwing constructor must surface as a Violation, never a bare
            // RangeError out of the converter (P11).
            assert!(
                runtime.contains(
                    "    return Temporal.ZonedDateTime.from(`${canon}[${zone}]`);\n  } catch {\n"
                ),
                "{runtime}"
            );
        }
        typecheck_generated_typescript(&output_path, &format!("{label} fractional output"));

        let runtime_test = output_path.join("fractional-second.ts");
        fs::write(
            &runtime_test,
            format!(
                r#"import {{ expect, test }} from "vitest";
import {{ clockTransferTypeConverter }} from "./models.ts";

function roundTrip(wire: string): unknown {{
  const model = clockTransferTypeConverter.fromTransferType({{ createdAt: wire }});
  return (clockTransferTypeConverter.toTransferType(model) as Record<string, unknown>).createdAt;
}}

test("an over-capacity fraction truncates instead of throwing", () => {{
  expect(roundTrip("2021-01-15T12:30:45.123456789012Z")).toBe({expected});
  // Widths within capacity, and an all-zero fraction, are unaffected.
  expect(roundTrip("2021-01-15T12:30:45Z")).toBe("2021-01-15T12:30:45{zero_suffix}Z");
}});
"#,
                expected = serde_json::to_string(expected).unwrap(),
                zero_suffix = if *repr == nexgen::generator::TsDateTimeTypes::Date {
                    ".000"
                } else {
                    ""
                },
            ),
        )
        .unwrap();
        run_generated_typescript_test(temp_dir.path(), &runtime_test, &format!("{label} fraction"));
    }
}

#[test]
fn renders_sample_with_versioned_header() {
    let root = project_root();
    let temp = tempfile::tempdir().unwrap();
    let temp_dir = temp.path().join("output");
    generate_to_file(&GenerateRequest {
        config: NexgenConfig {
            system_nexus: true,
            mode: GenerationMode::NativeApi,
            ..Default::default()
        },
        language: Language::TypeScript,
        input_paths: example_input_paths(&root, PRIMARY_EXAMPLE_ID),
        support_package: Some("./support".into()),
        descriptor_paths: vec![descriptor_path(&root)],
        output_path: temp_dir.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    assert!(temp_dir.is_dir());

    ensure_typescript_dependencies(&typescript_root(&root));
    let status = Command::new("npm")
        .current_dir(root.join("advanced/samples/typescript"))
        .args([
            "exec",
            "--",
            "prettier",
            "--write",
            "--print-width",
            "88",
            temp_dir.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output = read_typescript_output_files(&temp_dir);
    let manifest: toml::Table =
        toml::from_str(&fs::read_to_string(root.join("Cargo.toml")).unwrap()).unwrap();
    let header = format!(
        "nexgen v{}",
        manifest["package"]["version"].as_str().unwrap()
    );
    assert!(
        output
            .values()
            .all(|contents| { contents.contains(&header) })
    );
    assert!(!output.is_empty());
    let _ = fs::remove_dir_all(temp_dir);
}
