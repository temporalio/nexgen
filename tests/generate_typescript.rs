// Drives the `nexgen` binary over the WIT/proto CLI surface (`--descriptors`,
// `--native-api`, `--support-file`), all behind the `advanced` feature.
#![cfg(feature = "advanced")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nexgen::generator::generate_source;
use nexgen::spec::SupportFragmentSpec;
use nexgen::{GenerateRequest, SupportFiles, generate_to_file};

mod common;
use common::{json_input_path, write_bare_ref_alias_closure};

const PRIMARY_EXAMPLE_ID: &str = "workflow-service";
const START_WORKFLOW_EXAMPLE_ID: &str = "start-workflow";
const TYPE_ROUNDTRIP_EXAMPLE_ID: &str = "type-roundtrip";
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

fn example_input_paths(root: &Path, example_id: &str) -> Vec<PathBuf> {
    let input = input_path(root, example_id);
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

fn input_path(root: &Path, example_id: &str) -> PathBuf {
    let flat_path = root
        .join("advanced/samples/inputs")
        .join(format!("{example_id}.wit"));
    if flat_path.is_file() {
        flat_path
    } else {
        root.join("advanced/samples/inputs")
            .join(example_id)
            .join("main.wit")
    }
}

fn typescript_output_path(root: &Path, example_id: &str) -> PathBuf {
    typescript_root(root).join("wit").join(example_id)
}

fn samples_typescript_root(root: &Path) -> PathBuf {
    root.join("samples/typescript")
}

fn typescript_json_definitions_output_path(root: &Path, example_id: &str) -> PathBuf {
    // Definitions are the beginner-facing samples, flattened directly under
    // `samples/typescript/<example>`.
    samples_typescript_root(root).join(example_id)
}

fn typescript_json_api_output_path(root: &Path, example_id: &str) -> PathBuf {
    typescript_root(root)
        .join("json_schema")
        .join("api")
        .join(example_id)
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

fn render_output_files(files: BTreeMap<PathBuf, String>) -> String {
    files
        .into_iter()
        .map(|(path, contents)| format!("### {}\n{contents}", path.display()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn generate_typescript_to_string(input_paths: &[PathBuf], descriptor_paths: &[PathBuf]) -> String {
    let temp_dir = unique_output_path("typescript-rendered");
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            mode: nexgen::generator::GenerationMode::NativeApi,
            ..Default::default()
        },
        language: nexgen::language::Language::TypeScript,
        input_paths: input_paths.to_vec(),
        support_paths: Vec::new(),
        descriptor_paths: descriptor_paths.to_vec(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = if output_path.is_file() {
        fs::read_to_string(&output_path).unwrap()
    } else {
        render_output_files(read_typescript_output_files(&output_path))
    };
    fs::remove_dir_all(temp_dir).unwrap();
    rendered
}

fn generate_formatted_typescript_output(root: &Path, example_id: &str, output_path: &Path) {
    ensure_typescript_dependencies(&typescript_root(root));

    let mut command = Command::new(env!("CARGO_BIN_EXE_nexgen"));
    command
        .arg("typescript")
        .args(example_input_paths(root, example_id))
        .args([
            "--descriptors",
            descriptor_path(root).to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
            "--native-api",
        ]);
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

    let status = Command::new(env!("CARGO_BIN_EXE_nexgen"))
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
fn cli_generates_typescript_support_file_from_parameter() {
    let root = project_root();
    let temp_dir = unique_output_path("typescript-support-file-input");
    fs::create_dir_all(&temp_dir).unwrap();
    let support_path = temp_dir.join("custom-support.ts");
    let output_path = temp_dir.join("output");
    fs::write(
        &support_path,
        "export function customSupportHook(): string {\n  return \"custom\";\n}\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "typescript",
            input_path(&root, "user-service").to_str().unwrap(),
            "--support-file",
            support_path.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::read_to_string(output_path.join("support.ts"))
            .unwrap()
            .contains("export function customSupportHook()")
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_rejects_support_namespace() {
    let root = project_root();
    let spec = nexgen::parser::load_api_spec_from_wit_for_language_with_inputs(
        nexgen::language::Language::TypeScript,
        &example_input_paths(&root, PRIMARY_EXAMPLE_ID),
    )
    .unwrap();
    let descriptors = nexgen::descriptors::DescriptorIndex::load(&descriptor_path(&root)).unwrap();
    let err = generate_source(
        nexgen::language::Language::TypeScript,
        spec.clone(),
        &descriptors,
        &SupportFiles {
            fragments: vec![SupportFragmentSpec {
                path: "support.ts".to_string(),
                contents: String::new(),
                namespace: Some("example.support".to_string()),
            }],
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("support namespace"));
}

#[test]
fn typescript_generated_operation_path_collision_names_both_operations() {
    let root = project_root();
    let temp_dir = unique_output_path("typescript-operation-path-collision");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("operations-collision.wit");
    fs::write(
        &input_path,
        r#"
package temporal:collision@1.0.0;

world system {
  export first-service;
  export second-service;
}

/// @nexus.endpoint "first"
interface first-service {
  record first-request { value: string }
  collide: func(request: first-request);
}

/// @nexus.endpoint "second"
interface second-service {
  record second-request { value: string }
  collide: func(request: second-request);
}
"#,
    )
    .unwrap();
    let spec = nexgen::parser::load_api_spec_from_wit_for_language_with_inputs(
        nexgen::language::Language::TypeScript,
        &[input_path],
    )
    .unwrap();
    let descriptors = nexgen::descriptors::DescriptorIndex::load(&descriptor_path(&root)).unwrap();
    let error = generate_source(
        nexgen::language::Language::TypeScript,
        spec,
        &descriptors,
        &SupportFiles::default(),
    )
    .unwrap_err();
    let nexgen::error::Error::GeneratedFileSourceConflict {
        path,
        first_source,
        second_source,
        remedy,
    } = error
    else {
        panic!("expected generated-file source conflict, got {error}");
    };
    assert_eq!(path, PathBuf::from("operations/collide.ts"));
    assert_eq!(first_source, "TypeScript operation `FirstService.Collide`");
    assert_eq!(
        second_source,
        "TypeScript operation `SecondService.Collide`"
    );
    assert!(remedy.contains("operation `Collide` in service `FirstService`"));
    assert!(remedy.contains("operation `Collide` in service `SecondService`"));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn typescript_renders_required_fields_and_custom_message_types() {
    let root = project_root();
    let rendered = generate_typescript_to_string(
        &example_input_paths(&root, PRIMARY_EXAMPLE_ID),
        &[descriptor_path(&root)],
    );

    assert!(!rendered.contains("type _RequestWithFunctionField<"));
    assert!(!rendered.contains("type _RequestWithArgumentsField<"));
    assert!(!rendered.contains("type SignalWithStartWorkflowRequestBase = {"));
    assert!(rendered.contains("export type SignalWithStartWorkflowRequest<"));
    assert!(rendered.contains("export type ReplaceSignalWithStartWorkflowRequest<Base, New>"));
    assert!(rendered.contains(
        "WorkflowFn extends (...args: any[]) => Promise<any> = (...args: any[]) => Promise<any>,"
    ));
    assert!(rendered.contains(
        "SignalValue extends workflow.SignalDefinition<any[]> = workflow.SignalDefinition<any[]>"
    ));
    assert!(rendered.contains("> = ReplaceSignalWithStartWorkflowRequest<"));
    assert!(
        rendered.contains(
            "SignalValue extends workflow.SignalDefinition<infer Args, any> ? Args : never"
        )
    );
    assert!(rendered.contains("SignalArgs extends any[] = SignalValue extends"));
    assert!(rendered.contains("signalArgs: SignalArgs | Readonly<SignalArgs>;"));
    assert!(rendered.contains("signalArgs?: SignalArgs | Readonly<SignalArgs>;"));
    assert!(
        rendered
            .contains("Workflow type name or workflow function identifying the workflow to start.")
    );
    assert!(rendered.contains("workflow: string;"));
    assert!(rendered.contains("Arguments for workflow."));
    assert!(rendered.contains("args?: ReadonlyArray<unknown>;"));
    assert!(rendered.contains("Arguments for signal."));
    assert!(rendered.contains("signalArgs?: ReadonlyArray<unknown>;"));
    assert!(rendered.contains("* Unique identifier for the workflow execution. Must be nonempty."));
    assert!(!rendered.contains("@property workflow"));
    assert!(rendered.contains("* @returns A workflow handle to the started workflow."));
    assert!(rendered.contains("id: string;"));
    assert!(rendered.contains("taskQueue: string;"));
    assert!(rendered.contains("runTimeout?: common.Duration;"));
    assert!(rendered.contains("idReusePolicy?: common.WorkflowIdReusePolicy;"));
    assert!(rendered.contains("idConflictPolicy?: common.WorkflowIdConflictPolicy;"));
    assert!(!rendered.contains("identity?: string;"));
    assert!(rendered.contains("memo?: Record<string, unknown>;"));
    assert!(rendered.contains("searchAttributes?: common.TypedSearchAttributes;"));
    assert!(!rendered.contains("common.TypedSearchAttributes | common.SearchAttributes"));
    assert!(rendered.contains("versioningOverride?: common.VersioningOverride;"));
    assert!(rendered.contains("priority?: common.Priority;"));
    assert!(rendered.contains("signal: string;"));
    assert!(rendered.contains("staticSummary?: string;"));
    assert!(rendered.contains("staticDetails?: string;"));
    assert!(!rendered.contains("userMetadata?: UserMetadata;"));
    assert!(rendered.contains("### support.ts"));
    assert!(rendered.contains("### index.ts"));
    let index_rendered = rendered
        .split("### index.ts")
        .nth(1)
        .expect("rendered output should include index.ts");
    assert!(!index_rendered.contains("export * from './support.ts';"));
    assert!(rendered.contains("export function retryPolicyFromProto("));
    assert!(!index_rendered.contains("export const SignalWithStartWorkflowRequest = {"));
    assert!(!index_rendered.contains("const UserMetadata = {"));
    assert!(index_rendered.contains("function userMetadataFromProto("));
    assert!(index_rendered.contains("function signalWithStartWorkflowRequestToProto<"));
    assert!(rendered.contains("workflowType: workflowTypeToProto("));
    assert!(rendered.contains("workflowFunctionName("));
    assert!(rendered.contains("input: requestArgsToPayloads(model.args),"));
    assert!(rendered.contains("signalInput: requestArgsToPayloads(model.signalArgs),"));
    assert!(!rendered.contains("_RequestArgsToPayloads"));
    let signal_request_to_proto = rendered
        .split("function signalWithStartWorkflowRequestToProto<")
        .nth(1)
        .and_then(|body| {
            body.split("export async function signalWithStartWorkflow")
                .next()
        })
        .expect("signal-with-start request renderer should be present");
    let workflow_type_index = signal_request_to_proto
        .find("workflowType: workflowTypeToProto(")
        .expect("workflow type should be serialized");
    let input_index = signal_request_to_proto
        .find("input: requestArgsToPayloads(model.args),")
        .expect("workflow args should be serialized");
    let workflow_id_index = signal_request_to_proto
        .find("workflowId: requiredField(")
        .expect("workflow id should be serialized");
    let task_queue_index = signal_request_to_proto
        .find("taskQueue: taskQueueToProto(")
        .expect("task queue should be serialized");
    let signal_name_index = signal_request_to_proto
        .find("signalName:")
        .expect("signal name should be serialized");
    assert!(workflow_type_index < input_index);
    assert!(input_index < workflow_id_index);
    assert!(workflow_id_index < task_queue_index);
    assert!(task_queue_index < signal_name_index);
    assert!(rendered.contains("signalName: signalFunctionName("));
    assert!(!rendered.contains("signalName: ((value) =>"));
    assert!(rendered.contains("workflowType: workflowTypeToProto("));
    assert!(rendered.contains("taskQueue: taskQueueToProto("));
    assert!(rendered.contains(
        "workflowRunTimeout: model.runTimeout == null ? undefined : durationToProto(model.runTimeout),"
    ));
    assert!(rendered.contains("common.WorkflowIdReusePolicy.ALLOW_DUPLICATE"));
    assert!(rendered.contains("workflowIdReusePolicy: workflowIdReusePolicyToProto("));
    assert!(!rendered.contains("workflowIdReusePolicyToProto(0"));
    assert!(rendered.contains("workflowIdConflictPolicy:"));
    assert!(!rendered.contains("workflowIdConflictPolicyToProto(0"));
    assert!(rendered.contains("model.idConflictPolicy == null"));
    assert!(rendered.contains("workflowIdConflictPolicyToProto(model.idConflictPolicy)"));
    assert!(rendered.contains("memo: model.memo == null ? undefined : memoToProto(model.memo),"));
    assert!(rendered.contains(
        "searchAttributes: model.searchAttributes == null ? undefined : searchAttributesToProto(model.searchAttributes),"
    ));
    assert!(rendered.contains(
        "priority: model.priority == null ? undefined : priorityToProto(model.priority),"
    ));
    assert!(rendered.contains("model.staticSummary == null && model.staticDetails == null"));
    assert!(rendered.contains("summary: model.staticSummary == null"));
    assert!(rendered.contains("configuredPayloadConverter().toPayload(model.staticSummary)"));
    assert!(rendered.contains("common.toPayloads(configuredPayloadConverter(), ...args)"));
    assert!(!rendered.contains("common.defaultPayloadConverter"));
    assert!(!rendered.contains("payloadToProto(payload: unknown"));
    assert!(!rendered.contains("function isPayload("));
    assert!(rendered.contains(
        "versioningOverride: model.versioningOverride == null ? undefined : versioningOverrideToProto(model.versioningOverride),"
    ));
    assert!(rendered.contains("export function taskQueueFromProto("));
    assert!(rendered.contains("export function taskQueueToProto("));
    assert!(rendered.contains(
        "): temporal.api.workflowservice.v1.ISignalWithStartWorkflowExecutionRequest | undefined {"
    ));
    assert!(rendered.contains("const result = await handle.result();"));
    assert!(rendered.contains("return workflow.getExternalWorkflowHandle("));
    assert!(rendered.contains("request.id"));
    assert!(rendered.contains("result.runId ?? undefined"));
    assert!(rendered.contains("export async function signalWithStartWorkflow<"));
    assert!(rendered.contains(
        "export { signalWithStartWorkflow } from './operations/signal-with-start-workflow';"
    ));
    assert!(rendered.contains("export const operationRegistry = ["));
    assert!(rendered.contains("service: \"temporal.api.workflowservice.v1.WorkflowService\""));
    assert!(rendered.contains("operation: \"SignalWithStartWorkflowExecution\""));
    assert!(rendered.contains(
        "inputType: \"temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest\""
    ));
    assert!(rendered.contains(
        "outputType: \"temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionResponse\""
    ));
    assert!(rendered.contains("export type { SignalWithStartWorkflowRequest } from './models';"));
    assert!(!rendered.contains("export { WorkflowService } from './services';"));
    assert!(!rendered.contains("export type { SignalWithStartWorkflowResponse"));
    assert!(rendered.contains("headers?: never"));
    assert!(rendered.contains("request: SignalWithStartWorkflowInput<WorkflowFn, SignalValue>,"));
    assert!(rendered.contains("const client = workflow.createNexusServiceClient({"));
    assert!(!rendered.contains("export class WorkflowServiceClient"));
    assert!(rendered.contains("): Promise<workflow.ExternalWorkflowHandle> {"));
    assert!(!rendered.contains("SignalWithStartWorkflowRequest = {\n  fromProto("));
    assert!(!rendered.contains("export interface SignalWithStartWorkflowRequest {"));
    assert!(!rendered.contains("export interface RetryPolicy"));
    assert!(!rendered.contains("export interface WorkflowType"));
    assert!(!rendered.contains("export interface TaskQueue"));
    assert!(!rendered.contains("export interface Duration"));
    assert!(!rendered.contains("export interface Memo"));
    assert!(!rendered.contains("export interface SearchAttributes"));
    assert!(!rendered.contains("export interface Priority"));
    assert!(!rendered.contains("export interface VersioningOverride"));
    assert!(!rendered.contains("export enum WorkflowIdReusePolicy"));
    assert!(!rendered.contains("export enum WorkflowIdConflictPolicy"));
    assert!(!rendered.contains("signalWithStartWorkflowExecution("));
    assert!(!rendered.contains("from './temporal_model_converters.ts'"));

    let start_workflow_rendered = generate_typescript_to_string(
        &example_input_paths(&root, START_WORKFLOW_EXAMPLE_ID),
        &[descriptor_path(&root)],
    );
    assert!(
        start_workflow_rendered
            .contains("export type CancelWorkflowResponse = Record<string, never>;")
    );
    assert!(!start_workflow_rendered.contains("export interface CancelWorkflowResponse {}"));

    let type_roundtrip_rendered = generate_typescript_to_string(
        &example_input_paths(&root, TYPE_ROUNDTRIP_EXAMPLE_ID),
        &[descriptor_path(&root)],
    );
    assert!(type_roundtrip_rendered.contains("retryPolicy: common.RetryPolicy;"));
    assert!(!type_roundtrip_rendered.contains("retryPolicyOperation"));
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: nexgen::generator::TsDateTimeTypes::Date,
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
            support_paths: Vec::new(),
            descriptor_paths: Vec::new(),
            output_path: output_path.clone(),
            format: false,
            java_package_name: None,
            ts_date_time_types: repr,
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: nexgen::nexgen_config::NexgenConfig {
            mode: nexgen::generator::GenerationMode::NativeApi,
            ..Default::default()
        },
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
fn typescript_json_bare_ref_root_alias_typechecks_and_uses_target_converter() {
    let temp_dir = unique_typescript_runtime_dir("ts-json-bare-ref-alias");
    let input = write_bare_ref_alias_closure(temp_dir.path());
    let output_path = temp_dir.path().join("output");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join("output"),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join(output),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
fn typescript_json_emits_complete_matchers_and_typed_mixed_extras() {
    let temp_dir = unique_typescript_runtime_dir("ts-json-wave2-conformance");
    let input_path = temp_dir.path().join("probe.yaml");
    fs::write(&input_path, TYPESCRIPT_CONFORMANCE_SCHEMA).unwrap();
    let output_path = temp_dir.path().join("probe");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: nexgen::generator::TsDateTimeTypes::Temporal,
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: nexgen::generator::TsDateTimeTypes::Temporal,
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
fn typescript_json_wave3_pairwise_runtime_matrix() {
    let temp_dir = unique_typescript_runtime_dir("ts-json-wave3-matrix");
    let input_path = temp_dir.path().join("audit.yaml");
    fs::write(&input_path, TYPESCRIPT_WAVE3_MATRIX_SCHEMA).unwrap();
    let output_path = temp_dir.path().join("audit");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::TypeScript,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: nexgen::generator::TsDateTimeTypes::Temporal,
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

/// Every shape whose generated TypeScript was previously unparseable, mis-pathed
/// or reference-compared: a closed **empty** object
/// ([[additionalProperties]] "Closed empty object"), a nested `uniqueItems`
/// level ([[items]] §"Nested arrays" — the inner loop must not shadow the
/// enclosing index), `uniqueItems` over a materialized element (compared by
/// canonical wire string, not by object identity), a typeless `contains`
/// matcher (which needs the element type's guard so a mistyped element
/// aggregates instead of throwing), the serialize-side integer cap
/// ([[type]] §Serialize-side), and the code-point length scan.
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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

/// Decision **D2**: a nullable *scalar* element (`items: {oneOf: [T, null]}`) is
/// accepted for `uniqueItems` and `contains`. The wrapper carries no `type`, so
/// reading it leaves the matcher unguarded — and `null <= 5` is `true` in JS, so
/// an unguarded numeric matcher would count `null` as a match. Semantics per
/// `uniqueItems.md:189-191` (two `null`s are a duplicate) and `contains.md`
/// Interactions → [[nullability]] (a `null` element never satisfies a scalar
/// matcher).
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
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

const TYPESCRIPT_FRACTIONAL_SECOND_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [createdAt]
properties:
  createdAt: { type: string, format: date-time }
"##;

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
            support_paths: Vec::new(),
            descriptor_paths: Vec::new(),
            output_path: output_path.clone(),
            format: false,
            java_package_name: None,
            ts_date_time_types: *repr,
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
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
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let models = fs::read_to_string(output_path.join("models.ts")).unwrap();
    assert!(models.contains("Java emits Long here"), "{models}");
    assert!(models.contains("\"Long\""), "{models}");
    assert!(!models.contains("from 'long'"), "{models}");
    fs::remove_dir_all(temp_dir).unwrap();
}
