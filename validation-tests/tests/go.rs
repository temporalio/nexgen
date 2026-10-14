//! Generated Go compilation and runtime integration tests.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nexgen::{GenerateRequest, generate_to_file};

mod support;
use support::common;

#[path = "support/json_schema/selected_tests.rs"]
mod json_schema_tests;

const JSON_SCHEMA_TARGET: support::json_schema::toolchain::Target =
    support::json_schema::toolchain::Target::Go;

use common::{json_input_path, write_bare_ref_alias_closure};

#[test]
fn go_hostile_documentation_is_wrapped_and_syntax_safe() {
    support::doc_rendering::go_hostile_documentation_is_wrapped_and_syntax_safe();
}

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

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

const RECURSIVE_POSITION_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [count, matrix, dates, blobs, children]
properties:
  count: { type: integer }
  score: { type: number }
  matrix:
    type: array
    items:
      type: array
      items: { type: number }
  dates:
    type: array
    items: { type: string, format: date }
  blobs: { $ref: "#/$defs/BlobMap" }
  children:
    type: array
    items: { $ref: "#/$defs/Child" }
$defs:
  BlobMap:
    type: object
    additionalProperties: { type: string, contentEncoding: base64 }
  Child:
    type: object
    required: [value]
    properties:
      value: { type: number }
"##;

const GO_WAVE3_MATRIX_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
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

fn generate_to_string_with_inputs(
    language: nexgen::language::Language,
    input_paths: &[PathBuf],
    descriptor_paths: &[PathBuf],
) -> Result<String, Box<dyn std::error::Error>> {
    let temp_dir = unique_output_path("go-rendered");
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            mode: nexgen::generator::GenerationMode::NativeApi,
            ..Default::default()
        },
        language,
        input_paths: input_paths.to_vec(),
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: descriptor_paths.to_vec(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })?;
    let rendered = if output_path.is_file() {
        fs::read_to_string(&output_path)?
    } else {
        read_go_output_files(&output_path)
            .into_iter()
            .map(|(path, contents)| format!("### {}\n{contents}", path.display()))
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    fs::remove_dir_all(temp_dir)?;
    Ok(rendered)
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn descriptor_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/descriptors/temporal_api.bin")
}

fn go_package_name(example_id: &str) -> String {
    example_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_lowercase()
}

fn read_go_output_files(dir: &Path) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, files);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("go") {
                if path
                    .file_name()
                    .and_then(|file_name| file_name.to_str())
                    .is_some_and(|file_name| file_name.ends_with("_test.go"))
                {
                    continue;
                }
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
    let path = std::env::temp_dir().join(format!("nexgen-{label}-{unique}-{counter}"));
    if label.starts_with("go-json-") {
        fs::create_dir_all(&path).unwrap();
        let sample_root = project_root().join("samples/go");
        fs::copy(sample_root.join("go.mod"), path.join("go.mod")).unwrap();
        fs::copy(sample_root.join("go.sum"), path.join("go.sum")).unwrap();
    }
    path
}

#[test]
fn go_sourced_map_field_converts_to_proto() {
    let root = project_root();
    let temp_dir = unique_output_path("go-sourced-map-input");
    fs::create_dir_all(&temp_dir).unwrap();
    let wit_path = temp_dir.join("sourced-map.wit");
    fs::write(
        &wit_path,
        r#"package temporal:sourced-map@1.0.0;

world system {
  export namespace-service;
}

/// @nexus.endpoint "namespace-service"
interface namespace-service {
  /// String-shaped placeholder for omitted fields.
  type placeholder = string;

  /// @nexus.proto "temporal.api.namespace.v1.NamespaceInfo"
  record namespace-info {
    name: option<string>,
    /// @nexus.source go="NamespaceData(ctx)"
    data: option<map<string, string>>,
    /// @nexus.omit
    state: placeholder,
    /// @nexus.omit
    description: placeholder,
    /// @nexus.omit
    owner-email: placeholder,
    /// @nexus.omit
    id: placeholder,
    /// @nexus.omit
    capabilities: placeholder,
    /// @nexus.omit
    limits: placeholder,
    /// @nexus.omit
    supports-schedules: placeholder,
  }

  describe-namespace: func(request: namespace-info);
}
"#,
    )
    .unwrap();

    let rendered = generate_to_string_with_inputs(
        nexgen::language::Language::Go,
        &[wit_path.clone()],
        &[descriptor_path(&root)],
    )
    .unwrap();

    // The sourced map is bound to a field-unique local, evaluated once, and
    // copied into a properly typed proto map.
    assert!(rendered.contains("sourcedData := support.NamespaceData(ctx)"));
    assert!(rendered.contains("if len(sourcedData) > 0 {"));
    assert!(rendered.contains("message.Data = make(map[string]string, len(sourcedData))"));
    assert!(rendered.contains("for k, v := range sourcedData {"));
    assert!(rendered.contains("message.Data[k] = v"));
    assert!(rendered.contains("support \"example.com/nexgen/support\""));

    // With no other converter to register the package, a sourced field alone
    // must import the support package and qualify its helper invocation.
    let module_root = temp_dir.join("module");
    fs::create_dir_all(module_root.join("support")).unwrap();
    for file in ["go.mod", "go.sum"] {
        fs::copy(
            root.join("advanced/samples/go").join(file),
            module_root.join(file),
        )
        .unwrap();
    }
    fs::write(
        module_root.join("support/support.go"),
        "package support\nimport \"go.temporal.io/sdk/workflow\"\nfunc NamespaceData(ctx workflow.Context) map[string]string { return map[string]string{\"namespace\": \"test\"} }\n",
    )
    .unwrap();
    let output_path = module_root.join("generated");
    generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            mode: nexgen::generator::GenerationMode::NativeApi,
            ..Default::default()
        },
        language: nexgen::language::Language::Go,
        input_paths: vec![wit_path],
        support_package: Some("go.temporal.io/sdk/advanced/samples/go/support".into()),
        descriptor_paths: vec![descriptor_path(&root)],
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let generated = read_go_output_files(&output_path);
    assert!(generated
        .values()
        .any(|file| file.contains("support \"go.temporal.io/sdk/advanced/samples/go/support\"")));
    assert!(
        generated
            .values()
            .any(|file| file.contains("support.NamespaceData(ctx)"))
    );
    let build = Command::new("go")
        .args(["test", "./..."])
        .current_dir(&module_root)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_json_examples_render_expected_language_features() {
    let root = project_root();
    for example_id in ["chat", "kb", "showcase", "temporal"] {
        for (mode, native_api) in [("definitions", false), ("api", true)] {
            let temp_dir = unique_output_path(&format!("go-json-{example_id}-{mode}"));
            fs::create_dir_all(&temp_dir).unwrap();
            fs::write(temp_dir.join("go.mod"), "module examples/go\n\ngo 1.24.0\n").unwrap();
            let output_path = temp_dir
                .join("json_schema")
                .join(mode)
                .join(go_package_name(example_id));
            generate_formatted_go_json_output(&root, example_id, &output_path, native_api);

            let actual = read_go_output_files(&output_path);

            let generated_file = PathBuf::from(format!("{example_id}.go"));
            let generated = actual
                .get(&generated_file)
                .unwrap_or_else(|| panic!("{} should be generated", generated_file.display()));
            let definitions = actual
                .get(&PathBuf::from("definitions.go"))
                .expect("JSON generation emits package definitions");
            if native_api {
                match example_id {
                    "chat" => {
                        assert!(generated.contains("var ChatService = struct"));
                        assert!(generated.contains("type ChatServiceClient struct"));
                        assert!(generated.contains("func NewChatServiceClient(endpoint string)"));
                        assert!(generated.contains(
                            "nexus.OperationReference[SendMessageInput, SendMessageOutput]"
                        ));
                        assert!(generated.contains(
                            "func (c *ChatServiceClient) Ping(ctx workflow.Context) workflow.Future"
                        ));
                    }
                    "kb" => {
                        assert!(generated.contains("var KnowledgeBaseService = struct"));
                        assert!(generated.contains("type KnowledgeBaseServiceClient struct"));
                        assert!(
                            generated
                                .contains("func NewKnowledgeBaseServiceClient(endpoint string)")
                        );
                        assert!(generated.contains("nexus.OperationReference[GetPageInput, Page]"));
                        assert!(
                            generated.contains("nexus.OperationReference[Block, PutBlockOutput]")
                        );
                        assert!(generated.contains(
                            "func (c *KnowledgeBaseServiceClient) GetPage(ctx workflow.Context, request GetPageInput) workflow.Future"
                        ));
                    }
                    "showcase" => {
                        assert!(generated.contains("type Showcase struct"));
                        assert!(generated.contains("func (m Showcase) RetriesOrDefault() int64"));
                        // Scalar defaults of every kind materialize on read.
                        assert!(
                            generated.contains("func (m Showcase) GreetingOrDefault() string {")
                        );
                        assert!(generated.contains("return \"hello\""));
                        assert!(generated.contains("func (m Showcase) DebugOrDefault() bool {"));
                        // `title` → name-led doc summary; `deprecated` → godoc marker.
                        assert!(generated.contains("// Retries Retry budget"));
                        assert!(generated.contains("// Deprecated: This field is deprecated."));
                        // `x-go-name` override (Stage 4): the emitted identifier
                        // is the derived name plus a `Go` suffix; the wire name
                        // (json tag) is pinned.
                        assert!(
                            generated.contains("LegacyIdGo *string `json:\"legacyId,omitempty\"`")
                        );
                        assert!(
                            generated
                                .contains("marshalField(out, \"legacyId\", *m.LegacyIdGo, &errs)")
                        );
                        // Value-constant overrides (Go-specific keys): the single
                        // `const` and the `active` enum value emit renamed
                        // constants while the wire values stay `1` / "active".
                        assert!(generated.contains("RevisionGo ShowcaseRevision = 1"));
                        assert!(generated.contains("ActiveGo ShowcaseStatus = \"active\""));
                        // An inline free-form object branch: the union declares
                        // the variant struct, whose sole member carries the wire
                        // members verbatim (a named free-form model gets the same
                        // shape).
                        assert!(generated.contains("type ShowcasePayloadObject struct"));
                        assert!(
                            generated
                                .contains("func (ShowcasePayloadObject) isShowcasePayload() {}")
                        );
                        assert!(generated.contains("type Extras struct"));
                        // A tagged union whose branches are written inline: each
                        // branch names itself with `x-go-name` and is emitted as a
                        // full model implementing the union interface.
                        assert!(generated.contains("type TextNote struct"));
                        assert!(generated.contains("func (TextNote) isNote() {}"));
                        assert!(generated.contains("func (LinkNote) isNote() {}"));
                        // The lone inline object branch of a property union
                        // derives its name from the union it belongs to.
                        assert!(generated.contains("type ShowcaseDetailObject struct"));
                        assert!(
                            generated.contains("func (ShowcaseDetailObject) isShowcaseDetail() {}")
                        );
                        assert!(
                            generated.contains("AdditionalProperties map[string]json.RawMessage")
                        );
                        // Type-level override: `Contact` emits as `ContactGo`
                        // at its declaration and at every `$ref`.
                        assert!(generated.contains("type ContactGo struct"));
                        assert!(
                            generated.contains("Contact *ContactGo `json:\"contact,omitempty\"`")
                        );
                        // showcase carries a minimal service/operation so the
                        // generator's service glue gets round-trip coverage too.
                        // The service/operation `x-go-name` overrides rename the
                        // emitted identifiers while the wire names are pinned.
                        assert!(generated.contains("var ShowcaseServiceGo = struct"));
                        assert!(generated.contains("type ShowcaseServiceGoClient struct"));
                        assert!(
                            generated.contains("func NewShowcaseServiceGoClient(endpoint string)")
                        );
                        assert!(
                            generated
                                .contains("nexus.OperationReference[GetShowcaseInput, Showcase]")
                        );
                        assert!(generated.contains(
                            "func (c *ShowcaseServiceGoClient) GetShowcaseGo(ctx workflow.Context, request GetShowcaseInput) workflow.Future"
                        ));
                        // The wire service/operation names are unaffected by the
                        // code-identifier overrides.
                        assert!(generated.contains("\"example.showcase.v1.ShowcaseService\""));
                        assert!(generated.contains("(\"GetShowcase\")"));
                    }
                    // temporal is a pure JSON Schema file materializing the four
                    // temporal formats into native Go types.
                    "temporal" => {
                        assert!(generated.contains("type Temporal struct"));
                        assert!(generated.contains("CreatedAt time.Time"));
                        assert!(generated.contains("Timeout time.Duration"));
                        assert!(
                            definitions.contains("func formatDuration(d time.Duration) string")
                        );
                        assert!(!generated.contains("Service = struct"));
                        assert!(!generated.contains("ServiceClient"));
                    }
                    _ => unreachable!(),
                }
            } else {
                // Definitions mode emits the Nexus service/operation reference
                // struct (so callers drive the SDK's Nexus client directly)
                // but never the workflow client — that surface is NativeApi
                // only, hence no `ServiceClient` type and no workflow import.
                assert!(!generated.contains("ServiceClient"));
                assert!(!generated.contains("go.temporal.io/sdk/workflow"));
                match example_id {
                    "chat" => {
                        assert!(generated.contains("var ChatService = struct"));
                        assert!(generated.contains("github.com/nexus-rpc/sdk-go/nexus"));
                        assert!(generated.contains(
                            "nexus.OperationReference[SendMessageInput, SendMessageOutput]"
                        ));
                        assert!(generated.contains("\"example.chat.v1.ChatService\""));
                    }
                    "kb" => {
                        assert!(generated.contains("var KnowledgeBaseService = struct"));
                        assert!(generated.contains("github.com/nexus-rpc/sdk-go/nexus"));
                        assert!(generated.contains("nexus.OperationReference[GetPageInput, Page]"));
                        assert!(
                            generated.contains("nexus.OperationReference[Block, PutBlockOutput]")
                        );
                        assert!(generated.contains("\"example.kb.v1.KnowledgeBaseService\""));
                    }
                    "showcase" => {
                        // The `x-go-name` overrides still rename the emitted
                        // identifiers while the wire names stay pinned.
                        assert!(generated.contains("var ShowcaseServiceGo = struct"));
                        assert!(generated.contains("github.com/nexus-rpc/sdk-go/nexus"));
                        assert!(
                            generated
                                .contains("nexus.OperationReference[GetShowcaseInput, Showcase]")
                        );
                        assert!(generated.contains("\"example.showcase.v1.ShowcaseService\""));
                        assert!(generated.contains("(\"GetShowcase\")"));
                    }
                    "temporal" => {
                        assert!(!generated.contains("Service = struct"));
                        assert!(!generated.contains("github.com/nexus-rpc/sdk-go/nexus"));
                    }
                    _ => unreachable!(),
                }
            }

            fs::remove_dir_all(temp_dir).unwrap();
        }
    }
}

fn go_descriptor_fixture(signal_directives: &str, args_field: &str) -> String {
    format!(
        r#"package test:go-descriptors@1.0.0;

world test-world {{
  export descriptor-service;
}}

interface functions {{
  type placeholder = string;
  /// @nexus.type go="any"
  type payload = placeholder;
  type payloads = list<payload>;
  /// @nexus.type go="go.temporal.io/sdk/workflow:workflow.Context"
  type context = placeholder;

  /// @nexus.function-args varargs=true param="args"
  workflow-call: func(ctx: context, args: payloads) -> string;
  /// @nexus.function
  ///   primary=true
  ///   signature="workflow-call"
  ///   args-field="input"
  ///   alternate-type="string"
  ///   result-type-parameter="WorkflowResult"
  type workflow-function = placeholder;

  /// @nexus.function-args varargs=true
  signal-call: func(signal-args: payloads) -> string;
  /// @nexus.function
  ///   signature="signal-call"
  ///   args-field="{args_field}"
  ///   alternate-type="string"
{signal_directives}
  type signal-function = placeholder;
}}

/// @nexus.endpoint "descriptor-endpoint"
interface descriptor-service {{
  use functions.{{workflow-function, signal-function, payloads}};
  record execute-request {{
    workflow: workflow-function,
    signal: signal-function,
  }}
  execute: func(request: execute-request);
}}
"#
    )
}

fn render_go_descriptor_fixture(wit: &str) -> Result<String, Box<dyn std::error::Error>> {
    let temp_dir = unique_output_path("go-descriptor-input");
    fs::create_dir_all(&temp_dir)?;
    let input = temp_dir.join("descriptor-service.wit");
    fs::write(&input, wit)?;
    let result = generate_to_string_with_inputs(nexgen::language::Language::Go, &[input], &[]);
    fs::remove_dir_all(temp_dir)?;
    result
}

fn compile_go_descriptor_fixture(rendered: &str) {
    let module_root = unique_output_path("go-descriptor-compile");
    let output_path = module_root.join("output");
    fs::create_dir_all(&output_path).unwrap();
    for file in ["go.mod", "go.sum"] {
        fs::copy(
            project_root().join("advanced/samples/go").join(file),
            module_root.join(file),
        )
        .unwrap();
    }
    for section in rendered.split("### ").skip(1) {
        let (file, source) = section.split_once('\n').unwrap();
        fs::write(
            output_path.join(file),
            source.replace(
                "example.com/nexgen/support",
                "go.temporal.io/sdk/advanced/samples/go/support",
            ),
        )
        .unwrap();
    }
    fs::create_dir_all(module_root.join("support")).unwrap();
    fs::write(
        module_root.join("support/support.go"),
        r#"package support
import "go.temporal.io/sdk/workflow"
func DescriptorName(ctx workflow.Context, value any) string { return "descriptor" }
"#,
    )
    .unwrap();
    let build = Command::new("go")
        .args(["test", "./..."])
        .current_dir(&module_root)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "generated fixture failed to compile:\nstdout: {}\nstderr: {}\n{rendered}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    fs::remove_dir_all(module_root).unwrap();
}

#[test]
fn go_explicit_descriptors_control_signal_api_and_packing_independent_of_wire_name() {
    for args_field in ["signal-input", "notification-payloads"] {
        for (args_type, public_args, packing) in [
            ("any", "signalArg any", "[]any{signalArg}"),
            ("[]string", "signalArg []string", "[]any{signalArg}"),
        ] {
            let directives =
                format!("  ///   go-value-type=\"string\"\n  ///   go-args-type=\"{args_type}\"");
            let rendered =
                render_go_descriptor_fixture(&go_descriptor_fixture(&directives, args_field))
                    .unwrap();
            assert!(
                rendered.contains(&format!(
                    "\tsignal string,\n\t{public_args},\n\tworkflow any,\n\targs ...any,\n"
                )),
                "{args_field}/{args_type}:\n{rendered}"
            );
            assert!(
                rendered.contains(&format!("\t\tSignalArgs: {packing},")),
                "{rendered}"
            );
            assert!(!rendered.contains("SignalF interface"), "{rendered}");
            compile_go_descriptor_fixture(&rendered);
        }
    }
}

#[test]
fn go_signature_signal_input_uses_callable_fallback_and_secondary_generic_in_typed_wrapper() {
    let rendered =
        render_go_descriptor_fixture(&go_descriptor_fixture("", "signal-input")).unwrap();
    assert!(
        rendered.contains("SignalF interface{ ~string | func(...any) string }"),
        "{rendered}"
    );
    assert!(
        rendered
            .contains("\tsignal SignalF,\n\tsignalArgs []any,\n\tworkflow any,\n\targs ...any,"),
        "{rendered}"
    );
    let typed = rendered.split("func ExecuteTyped[").nth(1).unwrap();
    assert!(typed.contains("SignalF interface{ ~string | func(...any) string }"));
    assert!(typed.contains("\tsignal SignalF,\n\tsignalArgs []any,"));
    assert!(typed.contains("\tworkflow func(workflow.Context, WorkflowArg) WorkflowResult,"));
    assert!(typed.contains("reflect.ValueOf(signal)"));
    assert!(typed.contains("\t\tSignal: signalName,"));
    assert!(typed.contains("\t\tSignalArgs: signalArgs,"));
    assert!(typed.contains("\t\tArgs: []any{arg},"));
    compile_go_descriptor_fixture(&rendered);
}

#[test]
fn go_primary_variadic_group_is_last_and_secondary_groups_keep_authored_order() {
    let wit = go_descriptor_fixture("", "signal-input")
        .replace(
            "  type signal-function = placeholder;",
            r#"  type signal-function = placeholder;
  /// @nexus.function-args varargs=true
  other-call: func(other-args: payloads) -> string;
  /// @nexus.function
  ///   signature="other-call"
  ///   args-field="other-payloads"
  ///   alternate-type="string"
  type other-function = placeholder;"#,
        )
        .replace(
            "workflow-function, signal-function, payloads",
            "workflow-function, signal-function, other-function, payloads",
        )
        .replace(
            "    signal: signal-function,",
            "    signal: signal-function,\n    other: other-function,",
        );
    let rendered = render_go_descriptor_fixture(&wit).unwrap();
    assert!(
        rendered.contains(
            "\tsignal SignalF,\n\tsignalArgs []any,\n\tother OtherF,\n\totherArgs []any,\n\tworkflow any,\n\targs ...any,"
        ),
        "{rendered}"
    );
    let typed = rendered.split("func ExecuteTyped[").nth(1).unwrap();
    assert!(typed.contains("OtherF interface{ ~string | func(...any) string }"));
    assert!(
        typed.contains(
            "\tsignal SignalF,\n\tsignalArgs []any,\n\tother OtherF,\n\totherArgs []any,\n\tworkflow func(workflow.Context, WorkflowArg) WorkflowResult,"
        ),
        "{typed}"
    );
    assert!(typed.contains("reflect.ValueOf(other)"));
    assert!(typed.contains("\t\tOtherArgs: otherArgs,"));
    compile_go_descriptor_fixture(&rendered);

    // Swap only the secondary groups. Their relative ordering must follow the
    // authored request, rather than a special case for signal-input.
    let swapped = wit.replace(
        "    signal: signal-function,\n    other: other-function,",
        "    other: other-function,\n    signal: signal-function,",
    );
    let rendered = render_go_descriptor_fixture(&swapped).unwrap();
    assert!(
        rendered.contains(
            "\tother OtherF,\n\totherArgs []any,\n\tsignal SignalF,\n\tsignalArgs []any,\n\tworkflow any,\n\targs ...any,"
        ),
        "{rendered}"
    );
    compile_go_descriptor_fixture(&rendered);
}

#[test]
fn go_final_secondary_signature_args_are_variadic_without_primary_signature_varargs() {
    let baseline = go_descriptor_fixture("", "signal-input");
    let fixed = baseline.replace(
            "  /// @nexus.function-args varargs=true param=\"args\"\n  workflow-call: func(ctx: context, args: payloads) -> string;",
            "  workflow-call: func(name: string) -> string;",
        );
    let descriptor = baseline.replace(
        "  ///   primary=true",
        "  ///   primary=true\n  ///   go-value-type=\"any\"\n  ///   go-args-type=\"[]string\"\n  ///   go-name-extractor=\"DescriptorName\"",
    );
    for (name, wit, primary_params) in [
        ("fixed", fixed, "\tworkflow WorkflowF,\n\tname string,"),
        ("descriptor", descriptor, "\tworkflow any,\n\targ []string,"),
    ] {
        let rendered = render_go_descriptor_fixture(&wit).unwrap();
        assert!(
            rendered.contains(&format!(
                "{primary_params}\n\tsignal SignalF,\n\tsignalArgs ...any,"
            )),
            "{name}:\n{rendered}"
        );
        assert!(!rendered.contains("func ExecuteTyped["), "{rendered}");
        assert!(rendered.contains("\t\tSignalArgs: signalArgs,"));
        compile_go_descriptor_fixture(&rendered);
    }
}

#[test]
fn go_rejects_model_parameters_colliding_with_wrapper_generics_during_planning() {
    let fixture = r#"package test:generic@1.0.0;
world system { export service; }
interface functions {
  type placeholder = string;
  /// @nexus.type go="any"
  type payload = placeholder;
  /// @nexus.type go="go.temporal.io/sdk/workflow:workflow.Context"
  type workflow-context = placeholder;
  /// @nexus.function-args varargs=true param="input"
  workflow-call: func(ctx: workflow-context, input: list<payload>) -> placeholder;
  /// @nexus.function
  ///   primary=true
  ///   signature="workflow-call"
  ///   args-field="input"
  ///   alternate-type="string"
  ///   result-type-parameter="WorkflowResult"
  type workflow-function = placeholder;
}
/// @nexus.endpoint "test"
interface service {
  use functions.{placeholder, workflow-function};
  /// @nexus.type-parameter
  type workflow-arg = placeholder;
  record request {
    context: workflow-arg,
    workflow: workflow-function,
  }
  run: func(request: request);
}
"#;
    for (authored, reserved, wrapper) in [
        ("workflow-arg", "WorkflowArg", "Typed"),
        ("workflow-result", "WorkflowResult", "Typed"),
        ("workflow-f", "WorkflowF", ""),
    ] {
        let mut wit = fixture.replace("workflow-arg", authored);
        if reserved == "WorkflowF" {
            wit = wit.replace("primary=true", "primary=false");
        }
        let error = render_go_descriptor_fixture(&wit).unwrap_err();
        match error.downcast_ref::<nexgen::error::Error>().unwrap() {
            nexgen::error::Error::UnsupportedGoType { context, reason } => {
                assert!(context.to_lowercase().contains("run"), "{error}");
                assert!(context.contains(wrapper), "{error}");
                assert!(
                    reason.contains(&format!("model type parameter `{reserved}`")),
                    "{error}"
                );
                assert!(
                    reason.contains(&format!("generated reserved name `{reserved}`")),
                    "{error}"
                );
            }
            other => panic!("expected early UnsupportedGoType, got {other:?}"),
        }
    }
    // Noncolliding model names keep the generated names, and the whole fixture
    // compiles: rejection is about the wrapper namespace, not generic models.
    let valid = fixture.replace("workflow-arg", "context-t");
    let rendered = render_go_descriptor_fixture(&valid).unwrap();
    assert!(
        rendered.contains("func RunTyped[ContextT any, WorkflowArg any, WorkflowResult any]"),
        "{rendered}"
    );
    compile_go_descriptor_fixture(&rendered);
}

#[test]
fn go_callable_descriptor_invokes_extractor_and_composite_types_import_and_compile() {
    for (value_type, args_type, public_type, import) in [
        ("func(...any) string", "any", "func(...any) string", None),
        (
            "[]time.Duration",
            "any",
            "[]time.Duration",
            Some("\"time\""),
        ),
        (
            "func(context.Context) any",
            "any",
            "func(context.Context) any",
            Some("\"context\""),
        ),
    ] {
        let directives = format!(
            "  ///   go-value-type=\"{value_type}\"\n  ///   go-args-type=\"{args_type}\"\n  ///   go-name-extractor=\"DescriptorName\""
        );
        let rendered = render_go_descriptor_fixture(&go_descriptor_fixture(
            &directives,
            "notification-payloads",
        ))
        .unwrap();
        assert!(
            rendered.contains(&format!("\tsignal {public_type},")),
            "{rendered}"
        );
        assert!(
            rendered.contains("signalName := support.DescriptorName(ctx, signal)"),
            "{rendered}"
        );
        if let Some(import) = import {
            assert!(rendered.contains(import), "{rendered}");
        }
        compile_go_descriptor_fixture(&rendered);
    }
}

fn generate_formatted_go_json_output(
    root: &Path,
    example_id: &str,
    output_path: &Path,
    native_api: bool,
) {
    let mut command = Command::new(support::nexgen_binary());
    command.args([
        "go",
        json_input_path(root, example_id).to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
        "--support-package",
        "example.com/nexgen/support",
    ]);
    if native_api {
        command.arg("--native-api");
    }
    let status = command.status().unwrap();
    assert!(status.success());

    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
}

/// A union in an **element position** — an array's `items` and a map's
/// `additionalProperties` — is named by the loader and emitted as an ordinary
/// named union, and each element decodes through the union's dispatcher rather
/// than `json.Unmarshal` (which cannot allocate a sealed interface).
/// See `specs/json-schema/features/oneOf.md` ("Unions in element positions").
#[test]
fn go_json_decodes_element_position_unions() {
    let temp_dir = unique_output_path("go-json-element-union");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("bag.yaml");
    fs::write(&input_path, ELEMENT_UNION_SCHEMA).unwrap();
    let output_path = temp_dir.join("bag");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("bag.go")).unwrap();

    // The inline element union is named after its position.
    assert!(rendered.contains("type BagSegmentsItem interface {"));
    assert!(rendered.contains("Segments []BagSegmentsItem `json:\"segments,omitempty\"`"));
    // Elements decode one at a time, with the index in the violation path.
    assert!(rendered.contains("p0 := fmt.Sprintf(\"%s[%d]\", \"segments\", i0)"));
    assert!(rendered.contains("if value0, ok := unmarshalBagSegmentsItem(e0, p0, &errs); ok {"));
    // A named union in element position takes the same path.
    assert!(rendered.contains("Choices []Choice `json:\"choices,omitempty\"`"));
    assert!(rendered.contains("if value0, ok := unmarshalChoice(e0, p0, &errs); ok {"));
    // Serialize re-runs each element's own branch constraints (P12).
    assert!(rendered.contains("if isNilValue(v0) {"));
    assert!(rendered.contains("errs = append(errs, Violation{p0, \"explicit null not allowed\"})"));
    // A map member routes through the dispatcher under its key.
    assert!(rendered.contains("AdditionalProperties map[string]EntriesValue"));
    assert!(rendered.contains("if value, ok := unmarshalEntriesValue(v, path, &errs); ok {"));
    // Element nullability stays the element's own concern.
    assert!(rendered.contains("Slots []*string `json:\"slots,omitempty\"`"));

    fs::write(
        output_path.join("bag_test.go"),
        r#"package bag

import (
	"encoding/json"
	"errors"
	"testing"

	"go.temporal.io/sdk/temporal"
)

func TestNilUnionPositionsReturnValidationErrors(t *testing.T) {
	var segment *BagSegmentsItemString
	var entry *EntriesValueString
	value := Bag{
		Segments: []BagSegmentsItem{segment},
		Entries: &Entries{AdditionalProperties: map[string]EntriesValue{"bad": entry}},
	}
	_, err := json.Marshal(value)
	var validation *temporal.ApplicationError
	if !errors.As(err, &validation) {
		t.Fatalf("expected ApplicationError, got %v", err)
	}
	if validation.Type() != "PayloadValidationError" || !validation.NonRetryable() {
		t.Fatalf("unexpected application error: %#v", validation)
	}
	var violations []Violation
	if err := validation.Details(&violations); err != nil {
		t.Fatalf("decode violations: %v", err)
	}
	got := map[string]bool{}
	for _, violation := range violations {
		got[violation.Path] = true
	}
	for _, path := range []string{"segments[0]", "entries.bad"} {
		if !got[path] {
			t.Errorf("missing violation path %q in %#v", path, violations)
		}
	}
}
"#,
    )
    .unwrap();
    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_json_recursively_converts_and_validates_element_positions() {
    let temp_dir = unique_output_path("go-json-recursive-positions");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("recursive.yaml");
    fs::write(&input_path, RECURSIVE_POSITION_SCHEMA).unwrap();
    let output_path = temp_dir.join("recursive");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let rendered = fs::read_to_string(output_path.join("recursive.go")).unwrap();
    let definitions = fs::read_to_string(output_path.join("definitions.go")).unwrap();
    assert!(definitions.contains("digits += strings.Repeat(\"0\", int(scale))"));
    assert!(definitions.contains("math.IsNaN(f) || math.IsInf(f, 0)"));
    assert!(definitions.contains("if strings.HasPrefix(v.Path, \"[\")"));
    assert!(definitions.contains("if len(*errs) > 0"));
    assert!(rendered.contains("p1 := fmt.Sprintf(\"%s[%d]\", p0, i1)"));
    assert!(rendered.contains("parseNumberField(&e1, p1, true, false, &errs)"));
    assert!(rendered.contains("parseDate(p0, s0, &errs)"));
    assert!(
        rendered.contains("decodeBase64(path, s, _nexgenJsonSchemaBase64ContentEncoding, &errs)")
    );
    assert!(rendered.contains("mergeNested(&errs, p0, v0.Validate())"));

    fs::write(
        output_path.join("recursive_test.go"),
        r#"package recursive

import (
	"encoding/json"
	"math"
	"testing"
	"time"
)

func paths(err error) map[string]bool {
	violations, ok := payloadValidationErrorViolations(err)
	if !ok {
		return nil
	}
	out := map[string]bool{}
	for _, violation := range violations {
		out[violation.Path] = true
	}
	return out
}

func TestRecursivePositions(t *testing.T) {
	for _, literal := range []string{"1", "1.0", "1e2", "1.5e1", "-1.5e1", "100e-2", "0e999999999999999999999", "9007199254740991", "-9007199254740991"} {
		var value Recursive
		payload := []byte(`{"count":` + literal + `,"matrix":[],"dates":[],"blobs":{},"children":[]}`)
		if err := json.Unmarshal(payload, &value); err != nil {
			t.Fatalf("integer spelling %s rejected: %v", literal, err)
		}
	}
	for _, literal := range []string{"1.5", "1e-999999999999999999999", "1e999999999999999999999", "9007199254740990.5", "9007199254740990.6", "9007199254740991.1", "9007199254740992", "-9007199254740992"} {
		var value Recursive
		payload := []byte(`{"count":` + literal + `,"matrix":[],"dates":[],"blobs":{},"children":[]}`)
		if err := json.Unmarshal(payload, &value); err == nil {
			t.Fatalf("invalid integer spelling %s accepted", literal)
		}
	}

	score := math.Inf(1)
	value := Recursive{
		Count: 1,
		Score: &score,
		Matrix: [][]float64{{math.Inf(-1)}},
		Dates: []time.Time{time.Date(0, 1, 1, 0, 0, 0, 0, time.UTC)},
		Blobs: BlobMap{AdditionalProperties: map[string][]byte{"data": {1, 2}}},
		Children: []Child{{Value: math.NaN()}},
	}
	_, err := json.Marshal(value)
	violations, ok := payloadValidationErrorViolations(err)
	if !ok {
		t.Fatalf("expected PayloadValidationError, got %v", err)
	}
	if len(violations) != 4 {
		t.Fatalf("expected exactly four validation violations, got %#v", violations)
	}
	got := paths(err)
	for _, path := range []string{"score", "matrix[0][0]", "dates[0]", "children[0].value"} {
		if !got[path] {
			t.Errorf("missing violation path %q in %#v", path, got)
		}
	}

	value.Score = nil
	value.Matrix = [][]float64{{1}}
	value.Dates = []time.Time{time.Date(1, 1, 2, 0, 0, 0, 0, time.UTC)}
	value.Children = []Child{{Value: 2}}
	wire, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	if string(wire) != `{"blobs":{"data":"AQI="},"children":[{"value":2}],"count":1,"dates":["0001-01-02"],"matrix":[[1]]}` {
		t.Fatalf("unexpected wire: %s", wire)
	}
}
"#,
    )
    .unwrap();
    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Scalar applicators share the complete normalized matcher vocabulary.
/// `contains` must not silently drop pattern or asserted-format predicates
/// after the loader accepts them.
#[test]
fn go_json_renders_complete_contains_matchers() {
    let temp_dir = unique_output_path("go-json-scalar-matchers");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("matchers.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  hosts:
    type: array
    items: { type: string }
    contains:
      type: string
      minLength: 5
      pattern: ^api\.
      format: hostname
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("output");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("output.go")).unwrap();
    let definitions = fs::read_to_string(output_path.join("definitions.go")).unwrap();

    assert!(rendered.contains("utf8.RuneCountInString(e) >= 5"));
    // The matcher's regexes compile once at package init, never per element
    // inside the scan loop (`pattern.md`'s compile-once rule).
    assert!(
        definitions.contains(
            "var _nexgenJsonSchemaPattern5e6170695c2e = regexp.MustCompile(\"^api\\\\.\")"
        )
    );
    assert!(rendered.contains("_nexgenJsonSchemaPattern5e6170695c2e.MatchString(e)"));
    assert!(
        !rendered.contains("regexp.MustCompile(\"^api\\\\.\").MatchString("),
        "the matcher pattern must not be recompiled per element\n{rendered}"
    );

    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

/// An object may combine declared properties with a typed catch-all. The
/// generated catch-all stays native and recursively validated; it is not a
/// bucket of raw JSON. Declared wire names remain exclusively owned by their
/// declared fields on serialization.
#[test]
fn go_json_mixed_typed_additional_properties_are_native_and_collision_safe() {
    let temp_dir = unique_output_path("go-json-mixed-typed-extras");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("mixed.yaml");
    fs::write(
        &input_path,
        r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
minProperties: 2
maxProperties: 3
required: [id]
properties:
  id: { type: string }
additionalProperties:
  $ref: "#/$defs/Extra"
$defs:
  Extra:
    type: object
    additionalProperties: false
    required: [score]
    properties:
      score: { type: integer, minimum: 1 }
"##,
    )
    .unwrap();
    let output_path = temp_dir.join("mixed");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("mixed.go")).unwrap();
    assert!(rendered.contains("AdditionalProperties map[string]Extra"));

    fs::write(
        output_path.join("mixed_test.go"),
        r#"package mixed

import (
    "encoding/json"
    "strings"
    "testing"
)

func validationText(err error) string {
    if violations, ok := payloadValidationErrorViolations(err); ok {
        var text strings.Builder
        for _, violation := range violations { text.WriteString(violation.String()); text.WriteByte('\n') }
        return text.String()
    }
    return err.Error()
}

func TestMixedTypedExtras(t *testing.T) {
    var value Mixed
    if err := json.Unmarshal([]byte(`{"id":"x","bonus":{"score":2}}`), &value); err != nil {
        t.Fatal(err)
    }
    if value.AdditionalProperties["bonus"].Score != 2 {
        t.Fatalf("typed extra was not materialized: %#v", value.AdditionalProperties)
    }
    if err := json.Unmarshal([]byte(`{"id":"x","bonus":{"score":0}}`), &value); err == nil || !strings.Contains(validationText(err), "bonus.score") {
        t.Fatalf("expected indexed nested validation path, got %v", err)
    }
    value = Mixed{ID: "x", AdditionalProperties: map[string]Extra{
        "bonus": {Score: 2},
        "id": {Score: 3},
    }}
    if _, err := json.Marshal(value); err == nil || !strings.Contains(validationText(err), "id") {
        t.Fatalf("expected declared/catch-all collision, got %v", err)
    }
}
"#,
    )
    .unwrap();

    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_json_scalar_matchers_have_runtime_type_and_decimal_semantics() {
    let temp_dir = unique_output_path("go-json-matcher-runtime");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("matcher.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [integers, decimals, names]
properties:
  integers:
    type: array
    items: { type: number }
    contains: { type: integer }
  decimals:
    type: array
    items: { type: number }
    contains: { type: number, multipleOf: 2 }
  names:
    type: object
    additionalProperties: { type: string }
    propertyNames:
      type: string
      minLength: 3
      pattern: "^[a-z]+$"
      enum: [api, host]
      format: hostname
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("matcher");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    fs::write(
        output_path.join("matcher_test.go"),
        r#"package matcher

import (
    "encoding/json"
    "strings"
    "testing"
)

func decode(t *testing.T, wire string) error {
    t.Helper()
    var value Matcher
    return json.Unmarshal([]byte(wire), &value)
}

func validationText(err error) string {
    if violations, ok := payloadValidationErrorViolations(err); ok {
        var text strings.Builder
        for _, violation := range violations { text.WriteString(violation.String()); text.WriteByte('\n') }
        return text.String()
    }
    return err.Error()
}

func TestScalarMatchers(t *testing.T) {
    if err := decode(t, `{"integers":[1.5],"decimals":[0.3,2],"names":{"api":"x"}}`); err == nil || !strings.Contains(validationText(err), "integers") {
        t.Fatalf("fractional number matched integer schema: %v", err)
    }
    if err := decode(t, `{"integers":[1.5,2],"decimals":[0.3,2],"names":{"api":"x"}}`); err != nil {
        t.Fatalf("later exact multiple did not satisfy contains: %v", err)
    }
    if err := decode(t, `{"integers":[2],"decimals":[0.3,2],"names":{"Api":"x"}}`); err == nil || !strings.Contains(validationText(err), "Api") {
        t.Fatalf("property name matcher was not applied: %v", err)
    }
    // An `integer` matcher over `number` elements admits [[type]]'s integer
    // domain, cap included -- the same set Number.isSafeInteger / SpecNumbers
    // admit in the other three targets.
    if err := decode(t, `{"integers":[1e300],"decimals":[0.3,2],"names":{"api":"x"}}`); err == nil || !strings.Contains(validationText(err), "integers") {
        t.Fatalf("an over-cap integral double matched the integer matcher: %v", err)
    }
    if err := decode(t, `{"integers":[9007199254740993],"decimals":[0.3,2],"names":{"api":"x"}}`); err == nil || !strings.Contains(validationText(err), "integers") {
        t.Fatalf("a past-2^53 double matched the integer matcher: %v", err)
    }
    if err := decode(t, `{"integers":[9007199254740991],"decimals":[0.3,2],"names":{"api":"x"}}`); err != nil {
        t.Fatalf("the cap boundary did not match the integer matcher: %v", err)
    }
}
"#,
    )
    .unwrap();
    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_json_temporal_constraints_cover_original_and_canonical_wire_values() {
    let temp_dir = unique_output_path("go-json-temporal-wire-constraints");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("temporal_wire.yaml");
    fs::write(
        &input_path,
        r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
required: [day, days, byName]
properties:
  day: { type: string, format: date, minLength: 10, pattern: "^2024-" }
  days:
    type: array
    items: { type: string, format: date, pattern: "^2024-" }
  byName: { $ref: "#/$defs/DateMap" }
$defs:
  DateMap:
    type: object
    additionalProperties: { type: string, format: date, pattern: "^2024-" }
"##,
    )
    .unwrap();
    let output_path = temp_dir.join("temporal_wire");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    fs::write(
        output_path.join("temporal_wire_test.go"),
        r#"package temporal_wire

import (
    "encoding/json"
    "strings"
    "testing"
    "time"
)

func validationText(err error) string {
    if violations, ok := payloadValidationErrorViolations(err); ok {
        var text strings.Builder
        for _, violation := range violations { text.WriteString(violation.String()); text.WriteByte('\n') }
        return text.String()
    }
    return err.Error()
}

func TestTemporalWireConstraints(t *testing.T) {
    wire := `{"day":"2023-01-01","days":["2023-01-02"],"byName":{"x":"2023-01-03"}}`
    var parsed TemporalWire
    if err := json.Unmarshal([]byte(wire), &parsed); err == nil ||
        !strings.Contains(validationText(err), "day") ||
        !strings.Contains(validationText(err), "days[0]") ||
        !strings.Contains(validationText(err), "byName.x") {
        t.Fatalf("original wire constraints were not aggregated: %v", err)
    }

    old := time.Date(2023, 1, 1, 0, 0, 0, 0, time.UTC)
    value := TemporalWire{
        Day: old,
        Days: []time.Time{old},
        ByName: DateMap{AdditionalProperties: map[string]time.Time{"x": old}},
    }
    if _, err := json.Marshal(value); err == nil ||
        !strings.Contains(validationText(err), "day") ||
        !strings.Contains(validationText(err), "days[0]") ||
        !strings.Contains(validationText(err), "byName.x") {
        t.Fatalf("canonical wire constraints were not aggregated: %v", err)
    }
}
"#,
    )
    .unwrap();
    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_json_materialized_closed_values_and_defaults_stay_native() {
    let temp_dir = unique_output_path("go-json-native-closed-values");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("native.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
required: [epoch, token]
properties:
  epoch: { type: string, format: date, const: "2024-01-01" }
  token: { type: string, contentEncoding: base64, enum: ["YQ==", "Yg=="] }
  fallbackDay: { type: string, format: date, default: "2024-02-03" }
  fallbackBytes: { type: string, contentEncoding: base64, default: "Yw==" }
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("native");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    fs::write(
        output_path.join("native_test.go"),
        r#"package native

import (
    "bytes"
    "encoding/json"
    "testing"
    "time"
)

func TestNativeClosedValuesAndDefaults(t *testing.T) {
    var value Native
    if err := json.Unmarshal([]byte(`{"epoch":"2024-01-01","token":"YQ=="}`), &value); err != nil {
        t.Fatal(err)
    }
    if value.Epoch.Year() != 2024 || !bytes.Equal(value.Token, []byte("a")) {
        t.Fatalf("closed values were not materialized: %#v", value)
    }
    if got := value.FallbackDayOrDefault(); got != time.Date(2024, 2, 3, 0, 0, 0, 0, time.UTC) {
        t.Fatalf("date default was not native: %v", got)
    }
    if got := value.FallbackBytesOrDefault(); !bytes.Equal(got, []byte("c")) {
        t.Fatalf("byte default was not native: %q", got)
    }
    if err := json.Unmarshal([]byte(`{"epoch":"2024-01-02","token":"Yw=="}`), &value); err == nil {
        t.Fatal("out-of-set materialized values were accepted")
    }
}
"#,
    )
    .unwrap();
    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_json_wave3_pairwise_runtime_matrix() {
    let temp_dir = unique_output_path("go-json-wave3-matrix");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("audit.yaml");
    fs::write(&input_path, GO_WAVE3_MATRIX_SCHEMA).unwrap();
    let output_path = temp_dir.join("audit");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    fs::write(
        output_path.join("wave3_matrix_test.go"),
        r#"package audit

import (
    "encoding/json"
    "math"
    "strings"
    "testing"
    "time"
)

const base = `{
  "requiredPlain":"present","requiredNullable":null,
  "constString":"fixed","constInteger":2,"constNumber":1.5,"constBoolean":true,"zeroConst":-0,
  "enumString":"a","enumInteger":1,"enumNumber":1.5,"enumBoolean":false,
  "bounded":-1e1,"matchedNumbers":[5e0],"matchedText":["dev@example.com"],"matchedBoolean":[true],
  "checkedArray":["ok","x"],"names":{"a@example.com":"x"},
  "mixed":{"id":"m","schedule":["2024-01-01"]}
}`

func decode(wire string) (Audit, error) {
    var value Audit
    err := json.Unmarshal([]byte(wire), &value)
    return value, err
}

func replace(base, old, replacement string) string {
    return strings.Replace(base, old, replacement, 1)
}

func validationText(err error) string {
    if violations, ok := payloadValidationErrorViolations(err); ok {
        var text strings.Builder
        for _, violation := range violations { text.WriteString(violation.String()); text.WriteByte('\n') }
        return text.String()
    }
    return err.Error()
}

func TestPresenceClosedScalarsAndNumericBoundaries(t *testing.T) {
    value, err := decode(base)
    if err != nil { t.Fatal(err) }
    if value.RequiredNullable != nil || value.OptionalPlain != nil || value.OptionalNullable != nil ||
        value.OptionalDefault != nil || value.OptionalNullableDefault != nil {
        t.Fatalf("unexpected presence state: %#v", value)
    }
    if value.OptionalDefaultOrDefault() != "fallback" ||
        value.OptionalNullableDefaultOrDefault() != "nullable-fallback" {
        t.Fatal("nullable/non-nullable defaults were not materialized on read")
    }
    if !math.Signbit(float64(value.ZeroConst)) {
        t.Fatal("signed zero was not preserved in memory")
    }
    if got := value.Mixed.AdditionalProperties["schedule"]; len(got) != 1 || got[0] != time.Date(2024, 1, 1, 0, 0, 0, 0, time.UTC) {
        t.Fatalf("typed temporal extra was not materialized: %#v", got)
    }
    encoded, err := json.Marshal(value)
    if err != nil { t.Fatal(err) }
    if !strings.Contains(string(encoded), `"requiredNullable":null`) ||
        strings.Contains(string(encoded), "optionalDefault") ||
        strings.Contains(string(encoded), "optionalNullableDefault") {
        t.Fatalf("presence was not preserved on serialize: %s", encoded)
    }

    for _, wire := range []string{
        strings.Replace(base, `"requiredPlain":"present",`, "", 1),
        strings.Replace(base, `"requiredNullable":null,`, "", 1),
        replace(base, `"requiredPlain":"present"`, `"requiredPlain":null`),
        strings.Replace(base, `"requiredPlain":"present"`, `"requiredPlain":"present","optionalPlain":null`, 1),
        strings.Replace(base, `"requiredPlain":"present"`, `"requiredPlain":"present","optionalDefault":null`, 1),
    } {
        if _, err := decode(wire); err == nil { t.Fatalf("invalid presence accepted: %s", wire) }
    }
    nullable, err := decode(strings.Replace(base, `"requiredPlain":"present"`, `"requiredPlain":"present","optionalNullable":null,"optionalNullableDefault":null`, 1))
    if err != nil || nullable.OptionalNullable != nil || nullable.OptionalNullableDefault != nil {
        t.Fatalf("nullable explicit null rejected: %#v, %v", nullable, err)
    }

    for field, replacement := range map[string]string{
        `"constString":"fixed"`: `"constString":"wrong"`,
        `"constInteger":2`: `"constInteger":3`,
        `"constNumber":1.5`: `"constNumber":2.5`,
        `"constBoolean":true`: `"constBoolean":false`,
        `"enumString":"a"`: `"enumString":"c"`,
        `"enumInteger":1`: `"enumInteger":3`,
        `"enumNumber":1.5`: `"enumNumber":3.5`,
        `"enumBoolean":false`: `"enumBoolean":"no"`,
    } {
        if _, err := decode(replace(base, field, replacement)); err == nil || !strings.Contains(validationText(err), strings.Trim(strings.Split(field, ":")[0], `"`)) {
            t.Fatalf("closed scalar failure missing for %s: %v", field, err)
        }
    }
    if _, err := decode(replace(base, `"bounded":-1e1`, `"bounded":10`)); err == nil || !strings.Contains(validationText(err), "must be < 10") {
        t.Fatalf("exclusiveMaximum was not enforced: %v", err)
    }
    if _, err := decode(replace(base, `"bounded":-1e1`, `"bounded":-15`)); err != nil {
        t.Fatalf("negative multiple rejected: %v", err)
    }

    invalid := value
    invalid.ConstNumber = AuditConstNumber(2.5)
    if _, err := json.Marshal(invalid); err == nil || !strings.Contains(validationText(err), "constNumber") {
        t.Fatalf("serialize accepted invalid const: %v", err)
    }
    invalid = value
    invalid.Bounded = 10
    if _, err := json.Marshal(invalid); err == nil || !strings.Contains(validationText(err), "must be < 10") {
        t.Fatalf("serialize accepted exclusive maximum: %v", err)
    }
}

func TestMatchersArraysPropertyNamesAndTypedExtraCounts(t *testing.T) {
    for old, replacement := range map[string]string{
        `"matchedNumbers":[5e0]`: `"matchedNumbers":[1]`,
        `"matchedText":["dev@example.com"]`: `"matchedText":["bad@example.org"]`,
        `"matchedBoolean":[true]`: `"matchedBoolean":[false]`,
    } {
        if _, err := decode(replace(base, old, replacement)); err == nil || !strings.Contains(validationText(err), strings.Split(old, `"`)[1]) {
            t.Fatalf("contains failure missing for %s: %v", old, err)
        }
    }
    value, err := decode(base)
    if err != nil { t.Fatal(err) }
    invalid := value
    invalid.MatchedNumbers = []float64{1}
    if _, err := json.Marshal(invalid); err == nil || !strings.Contains(validationText(err), "matchedNumbers") { t.Fatalf("serialize contains: %v", err) }
    invalid = value
    invalid.MatchedText = []string{"bad@example.org"}
    if _, err := json.Marshal(invalid); err == nil || !strings.Contains(validationText(err), "matchedText") { t.Fatalf("serialize contains: %v", err) }
    invalid = value
    invalid.MatchedBoolean = []bool{false}
    if _, err := json.Marshal(invalid); err == nil || !strings.Contains(validationText(err), "matchedBoolean") { t.Fatalf("serialize contains: %v", err) }

    if _, err := decode(replace(base, `"checkedArray":["ok","x"]`, `"checkedArray":[]`)); err == nil ||
        !strings.Contains(validationText(err), "at least 2 items") || !strings.Contains(validationText(err), "matching") {
        t.Fatalf("array failures did not aggregate: %v", err)
    }
    crowded := replace(base, `"checkedArray":["ok","x"]`, `"checkedArray":["ok","ok","x","x"]`)
    if _, err := decode(crowded); err == nil || !strings.Contains(validationText(err), "at most 3 items") ||
        !strings.Contains(validationText(err), "duplicate") || !strings.Contains(validationText(err), "matching") {
        t.Fatalf("array upper failures did not aggregate: %v", err)
    }
    invalid = value
    invalid.CheckedArray = nil
    if _, err := json.Marshal(invalid); err == nil || !strings.Contains(validationText(err), "checkedArray") { t.Fatalf("serialize array: %v", err) }

    for key := range map[string]struct{}{ "x": {}, "bad key@example.com": {}, "c@example.com": {} } {
        names := replace(base, `"names":{"a@example.com":"x"}`, `"names":{"`+key+`":"x"}`)
        if _, err := decode(names); err == nil || !strings.Contains(validationText(err), key) {
            t.Fatalf("propertyNames failure missing for %q: %v", key, err)
        }
    }
    invalid = value
    invalid.Names.AdditionalProperties = map[string]string{"c@example.com": "x"}
    if _, err := json.Marshal(invalid); err == nil || !strings.Contains(validationText(err), "c@example.com") {
        t.Fatalf("serialize propertyNames failure missing: %v", err)
    }

    if _, err := decode(replace(base, `"mixed":{"id":"m","schedule":["2024-01-01"]}`, `"mixed":{"id":"m"}`)); err == nil || !strings.Contains(validationText(err), "at least 2 properties") {
        t.Fatalf("mixed minProperties missing: %v", err)
    }
    if _, err := decode(replace(base, `"mixed":{"id":"m","schedule":["2024-01-01"]}`, `"mixed":{"id":"m","a":["2024-01-01"],"b":["2024-01-02"],"c":["2024-01-03"]}`)); err == nil || !strings.Contains(validationText(err), "at most 3 properties") {
        t.Fatalf("mixed maxProperties missing: %v", err)
    }
    invalid = value
    invalid.Mixed.AdditionalProperties = map[string][]time.Time{}
    if _, err := json.Marshal(invalid); err == nil || !strings.Contains(validationText(err), "at least 2 properties") {
        t.Fatalf("serialize mixed minProperties missing: %v", err)
    }
    invalid, _ = decode(base)
    day := time.Date(2024, 1, 1, 0, 0, 0, 0, time.UTC)
    invalid.Mixed.AdditionalProperties = map[string][]time.Time{"a": {day}, "b": {day}, "c": {day}}
    if _, err := json.Marshal(invalid); err == nil || !strings.Contains(validationText(err), "at most 3 properties") {
        t.Fatalf("serialize mixed maxProperties missing: %v", err)
    }
    invalid, _ = decode(base)
    invalid.Mixed.AdditionalProperties = map[string][]time.Time{"id": {day}}
    if _, err := json.Marshal(invalid); err == nil || !strings.Contains(validationText(err), "id") {
        t.Fatalf("serialize mixed collision missing: %v", err)
    }
}
"#,
    )
    .unwrap();

    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_json_bare_ref_root_is_a_runtime_alias_for_operation_io() {
    let temp_dir = unique_output_path("go-json-bare-ref-alias");
    let input = write_bare_ref_alias_closure(&temp_dir);
    let output_path = temp_dir.join("aliases");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![input],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let alias = fs::read_to_string(output_path.join("alias_alternate.go")).unwrap();
    assert!(alias.contains("type Alternate = Main"), "{alias}");
    assert!(!alias.contains("type Alternate struct"), "{alias}");
    let target = fs::read_to_string(output_path.join("target_main.go")).unwrap();
    assert!(target.contains("type Mirror = Main"), "{target}");
    assert!(!target.contains("type Mirror struct"), "{target}");
    let service = fs::read_to_string(output_path.join("service.go")).unwrap();
    assert!(
        service.contains("Echo nexus.OperationReference[Alternate, Alternate]"),
        "{service}"
    );
    assert!(
        service.contains("Item Alternate `json:\"item\"`"),
        "{service}"
    );
    fs::write(
        output_path.join("alias_runtime_test.go"),
        r#"package aliases

import (
    "encoding/json"
    "testing"
)

func TestBareRefAliasRoundTrips(t *testing.T) {
    var value Alternate
    if err := json.Unmarshal([]byte(`{"value":"ok"}`), &value); err != nil {
        t.Fatal(err)
    }
    if value.Value != "ok" { t.Fatalf("value = %q", value.Value) }
    if _, err := json.Marshal(value); err != nil { t.Fatal(err) }
}
"#,
    )
    .unwrap();
    assert!(
        Command::new("go")
            .args(["test", "./..."])
            .env("GO111MODULE", "on")
            .current_dir(&output_path)
            .status()
            .unwrap()
            .success()
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A `number` field's `multipleOf` is IEEE `fmod` — the same primitive
/// TypeScript's `%`, Python's `math.fmod` and Java's `%` run — so the accepted
/// set is bit-identical across the four (`multipleOf.md:97`). Exact decimal
/// arithmetic over the shortest round-tripping spelling is a *different*
/// predicate: it accepts `1e23 % 5` (which the other three reject) and rejects
/// `1e300 % 3` (which they accept).
#[test]
fn go_json_number_multiple_of_is_ieee_fmod() {
    let temp_dir = unique_output_path("go-json-fmod");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("fmod.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  byFive: { type: number, multipleOf: 5 }
  byThree: { type: number, multipleOf: 3 }
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("fmod");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("fmod.go")).unwrap();
    assert!(
        rendered.contains("math.Mod(float64(v), 5) != 0"),
        "{rendered}"
    );
    fs::write(
        output_path.join("fmod_test.go"),
        r#"package fmod

import (
    "encoding/json"
    "testing"
)

func TestNumberMultipleOfIsFmod(t *testing.T) {
    for _, tc := range []struct {
        wire   string
        accept bool
    }{
        // fmod(1e23, 5) == 2 -- the shortest decimal spelling "1e23" is an exact
        // multiple of 5, the stored double is not.
        {`{"byFive":1e23}`, false},
        // fmod(1e300, 3) == 0 -- the stored double is an exact multiple, the
        // decimal 1e300 is not.
        {`{"byThree":1e300}`, true},
        {`{"byThree":1e22}`, false},
        {`{"byFive":10}`, true},
        {`{"byFive":-0}`, true},
        {`{"byFive":7}`, false},
    } {
        var value Fmod
        err := json.Unmarshal([]byte(tc.wire), &value)
        if (err == nil) != tc.accept {
            t.Errorf("%s: accept=%v, err=%v", tc.wire, tc.accept, err)
        }
    }
}
"#,
    )
    .unwrap();
    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

/// The `oneOf:[T, null]` nullability wrapper carries no `type` and no keywords,
/// so every shape decision and every constraint has to be read off the non-null
/// branch (`nullability.md:177-187`). Before this was fixed a nullable
/// `integer`/`number`/`boolean`/`array` property parsed through
/// `parseStringField` — the package did not compile — and the shared `Validate`
/// dropped every co-occurring constraint.
#[test]
fn go_json_nullable_non_string_properties_keep_their_shape_and_constraints() {
    let temp_dir = unique_output_path("go-json-nullable-shapes");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("nullable.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [count, tags]
properties:
  count:
    oneOf:
      - { type: integer, minimum: 1, maximum: 10 }
      - { type: "null" }
  ratio:
    oneOf:
      - { type: number, multipleOf: 2 }
      - { type: "null" }
  flag:
    oneOf:
      - { type: boolean }
      - { type: "null" }
  tags:
    oneOf:
      - { type: array, items: { type: string }, minItems: 1 }
      - { type: "null" }
  name:
    oneOf:
      - { type: string, minLength: 2, maxLength: 5, pattern: "^[a-z]+$" }
      - { type: "null" }
  kind:
    oneOf:
      - { type: string, enum: [a, b] }
      - { type: "null" }
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("nullable");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("nullable.go")).unwrap();
    assert!(
        rendered.contains("Count *int64 `json:\"count\"`"),
        "{rendered}"
    );
    assert!(rendered.contains("Ratio *float64"), "{rendered}");
    assert!(rendered.contains("Flag *bool"), "{rendered}");
    // required + nullable array is `[]T`, not `*[]T` (nullability.md:252).
    assert!(
        rendered.contains("Tags []string `json:\"tags\"`"),
        "{rendered}"
    );
    // A nullable `enum` still gets the closed defined type (P13.1).
    assert!(rendered.contains("Kind *NullableKind"), "{rendered}");
    assert!(
        !rendered.contains("parseStringField(get(\"count\")"),
        "{rendered}"
    );

    fs::write(
        output_path.join("nullable_test.go"),
        r#"package nullable

import (
    "encoding/json"
    "testing"
)

func TestNullableShapes(t *testing.T) {
    // Every nullable member accepts an explicit null.
    var allNull Nullable
    if err := json.Unmarshal([]byte(`{"count":null,"tags":null,"ratio":null,"flag":null,"name":null,"kind":null}`), &allNull); err != nil {
        t.Fatalf("explicit nulls rejected: %v", err)
    }
    encoded, err := json.Marshal(allNull)
    if err != nil {
        t.Fatalf("re-encode failed: %v", err)
    }
    if string(encoded) != `{"count":null,"tags":null}` {
        t.Errorf("required+nullable members must emit null, optional ones omit: %s", encoded)
    }

    var full Nullable
    if err := json.Unmarshal([]byte(`{"count":5,"tags":["x"],"ratio":4,"flag":true,"name":"abc","kind":"a"}`), &full); err != nil {
        t.Fatalf("valid payload rejected: %v", err)
    }

    // Every constraint on the non-null branch still applies.
    for _, wire := range []string{
        `{"count":0,"tags":["x"]}`,
        `{"count":11,"tags":["x"]}`,
        `{"count":5,"tags":[]}`,
        `{"count":5,"tags":["x"],"ratio":3}`,
        `{"count":5,"tags":["x"],"name":"A"}`,
        `{"count":5,"tags":["x"],"name":"toolong"}`,
        `{"count":5,"tags":["x"],"kind":"zzz"}`,
    } {
        var value Nullable
        if err := json.Unmarshal([]byte(wire), &value); err == nil {
            t.Errorf("%s: expected a violation", wire)
        }
    }
}
"#,
    )
    .unwrap();
    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Everything a materialized value has to survive on the way out (P12): the
/// wire grammar's calendar and offset limits, the unsigned whole-second
/// duration, and `uniqueItems`/`const` compared on the canonical **wire
/// string** in both directions (decision D10) rather than on the native
/// `time.Time`/`time.Duration`/`[]byte`.
#[test]
fn go_json_materialized_values_are_checked_and_compared_on_the_wire() {
    let temp_dir = unique_output_path("go-json-materialized-serialize");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("mat.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [blob]
properties:
  blob: { type: string, contentEncoding: base64, minLength: 4, maxLength: 8 }
  blobConst: { type: string, contentEncoding: base64, const: "aGk=" }
  when: { type: string, format: date-time }
  clock: { type: string, format: time }
  dur: { type: string, format: duration }
  times:
    type: array
    items: { type: string, format: date-time }
    uniqueItems: true
  blobs:
    type: array
    items: { type: string, contentEncoding: base64 }
    uniqueItems: true
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("mat");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("mat.go")).unwrap();
    // The wire-string length bounds run over the encoded string, never over the
    // decoded `[]byte` (`utf8.RuneCountInString` does not take one).
    assert!(
        rendered.contains("wireBlob := encodeBase64(m.Blob)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("utf8.RuneCountInString(wireBlob)"),
        "{rendered}"
    );
    // `const` compares the canonical wire string, not the decoded bytes.
    assert!(
        rendered.contains("wireBlobConst := encodeBase64(m.BlobConst)"),
        "{rendered}"
    );
    assert!(!rendered.contains("bytes.Equal(("), "{rendered}");
    // `uniqueItems` keys on the canonical wire string ([]byte is not even a
    // legal Go map key).
    assert!(rendered.contains("key := formatDateTime(e)"), "{rendered}");
    assert!(rendered.contains("key := encodeBase64(e)"), "{rendered}");

    fs::write(
        output_path.join("mat_test.go"),
        r#"package mat

import (
    "encoding/json"
    "testing"
    "time"
)

func TestMaterializedSerializeChecks(t *testing.T) {
    base := Mat{Blob: []byte("hi!!")}

    negative := -90 * time.Minute
    fractional := 500 * time.Millisecond
    for name, value := range map[string]time.Duration{"negative": negative, "sub-second": fractional} {
        model := base
        model.Dur = &value
        if _, err := json.Marshal(model); err == nil {
            t.Errorf("%s duration serialized", name)
        }
    }

    subMinuteOffset := time.Date(2021, 6, 15, 12, 30, 45, 0, time.FixedZone("", 30))
    justOutOfRangeOffset := time.Date(2021, 6, 15, 12, 30, 45, 0, time.FixedZone("", 14*60*60+60))
    negativeJustOutOfRangeOffset := time.Date(2021, 6, 15, 12, 30, 45, 0, time.FixedZone("", -(14*60*60+60)))
    formerBoundaryOffset := time.Date(2021, 6, 15, 12, 30, 45, 0, time.FixedZone("", 18*60*60))
    negativeFormerBoundaryOffset := time.Date(2021, 6, 15, 12, 30, 45, 0, time.FixedZone("", -18*60*60))
    wideOutOfRangeOffset := time.Date(2021, 6, 15, 12, 30, 45, 0, time.FixedZone("", 23*60*60+59*60))
    negativeWideOutOfRangeOffset := time.Date(2021, 6, 15, 12, 30, 45, 0, time.FixedZone("", -(23*60*60+59*60)))
    overflowYear := time.Date(10000, 1, 1, 0, 0, 0, 0, time.UTC)
    for name, value := range map[string]time.Time{"sub-minute offset": subMinuteOffset, "offset +14:01": justOutOfRangeOffset, "offset -14:01": negativeJustOutOfRangeOffset, "offset +18:00": formerBoundaryOffset, "offset -18:00": negativeFormerBoundaryOffset, "offset +23:59": wideOutOfRangeOffset, "offset -23:59": negativeWideOutOfRangeOffset, "year 10000": overflowYear} {
        model := base
        model.When = &value
        if _, err := json.Marshal(model); err == nil {
            t.Errorf("%s date-time serialized", name)
        }
    }

    for _, seconds := range []int{14 * 60 * 60, -14 * 60 * 60} {
        boundary := time.Date(2021, 6, 15, 12, 30, 45, 0, time.FixedZone("", seconds))
        boundaryModel := base
        boundaryModel.When = &boundary
        boundaryModel.Clock = &boundary
        boundaryWire, err := json.Marshal(boundaryModel)
        if err != nil {
            t.Fatalf("%d-second boundary rejected on serialize: %v", seconds, err)
        }
        var boundaryRoundTrip Mat
        if err := json.Unmarshal(boundaryWire, &boundaryRoundTrip); err != nil {
            t.Fatalf("%d-second boundary output rejected on parse: %v", seconds, err)
        }
    }

    for name, value := range map[string]time.Time{"offset +14:01": justOutOfRangeOffset, "offset -14:01": negativeJustOutOfRangeOffset, "offset +18:00": formerBoundaryOffset, "offset -18:00": negativeFormerBoundaryOffset, "offset +23:59": wideOutOfRangeOffset, "offset -23:59": negativeWideOutOfRangeOffset} {
        model := base
        model.Clock = &value
        if _, err := json.Marshal(model); err == nil {
            t.Errorf("%s time serialized", name)
        }
    }

    duplicate := time.Date(2021, 6, 15, 12, 30, 45, 0, time.UTC)
    model := base
    model.Times = []time.Time{duplicate, duplicate}
    if _, err := json.Marshal(model); err == nil {
        t.Errorf("duplicate date-times serialized")
    }
    model = base
    model.Blobs = [][]byte{[]byte("ab"), []byte("ab")}
    if _, err := json.Marshal(model); err == nil {
        t.Errorf("duplicate byte strings serialized")
    }

    model = base
    model.BlobConst = []byte("hi")
    if _, err := json.Marshal(model); err != nil {
        t.Errorf("the const value was rejected: %v", err)
    }
    model.BlobConst = []byte("no")
    if _, err := json.Marshal(model); err == nil {
        t.Errorf("a non-const value serialized")
    }

    ok := base
    canonical := time.Date(2021, 6, 15, 12, 30, 45, 0, time.UTC)
    ninety := 90 * time.Minute
    ok.When = &canonical
    ok.Dur = &ninety
    encoded, err := json.Marshal(ok)
    if err != nil {
        t.Fatalf("valid model rejected: %v", err)
    }
    if string(encoded) != `{"blob":"aGkhIQ==","dur":"PT1H30M","when":"2021-06-15T12:30:45Z"}` {
        t.Errorf("unexpected wire: %s", encoded)
    }
}
"#,
    )
    .unwrap();
    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Array-item and typed-map positions used to synthesize the same package var
/// spelling as a declared `<name>Item` / `<name>Value` property. Keep all three
/// compiled-regex families semantic and shared, so arbitrary member spellings
/// cannot make an otherwise-valid package fail to compile.
#[test]
fn go_json_regex_helpers_do_not_collide_with_declared_member_positions() {
    let temp_dir = unique_output_path("go-json-regex-collisions");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("collision.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
title: Collision
type: object
properties:
  blobs:
    type: array
    items: { type: string, contentEncoding: base64 }
  blobsItem: { type: string, contentEncoding: base64 }
  names:
    type: array
    items: { type: string, pattern: "^a+$" }
  namesItem: { type: string, pattern: "^b+$" }
  contacts:
    type: array
    items: { type: string, format: email }
  contactsItem: { type: string, format: email }
  values:
    type: object
    additionalProperties: { type: string, contentEncoding: base64url }
  valuesValue: { type: string, contentEncoding: base64url }
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("collision");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let rendered = fs::read_to_string(output_path.join("definitions.go")).unwrap();
    assert_eq!(
        rendered
            .matches("ContentEncoding = regexp.MustCompile")
            .count(),
        2
    );
    assert_eq!(rendered.matches("Format = regexp.MustCompile").count(), 1);
    assert_eq!(rendered.matches("var _nexgenJsonSchemaPattern").count(), 2);

    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let build_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(build_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Semantic predicates belong to the flattened package, not to an input leaf:
/// two files using the same format/encoding/pattern must share one declaration.
#[test]
fn go_json_semantic_helpers_emit_once_across_flattened_inputs() {
    let temp_dir = unique_output_path("go-json-package-semantic-helpers");
    let input_dir = temp_dir.join("input");
    fs::create_dir_all(&input_dir).unwrap();
    for (file, title) in [("one.yaml", "One"), ("two.yaml", "Two")] {
        fs::write(
            input_dir.join(file),
            format!(
                r#"$schema: https://json-schema.org/draft/2020-12/schema
title: {title}
type: object
properties:
  email: {{ type: string, format: email }}
  token: {{ type: string, pattern: "^[a-z]+$" }}
  bytes: {{ type: string, contentEncoding: base64 }}
"#,
            ),
        )
        .unwrap();
    }
    let output_path = temp_dir.join("out");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![input_dir],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let definitions = fs::read_to_string(output_path.join("definitions.go")).unwrap();
    assert_eq!(
        definitions
            .matches("EmailFormat = regexp.MustCompile")
            .count(),
        1
    );
    assert_eq!(
        definitions
            .matches("Base64ContentEncoding = regexp.MustCompile")
            .count(),
        1
    );
    assert_eq!(
        definitions.matches("var _nexgenJsonSchemaPattern").count(),
        1
    );
    assert!(
        !fs::read_to_string(output_path.join("one.go"))
            .unwrap()
            .contains("regexp.MustCompile")
    );
    assert!(
        !fs::read_to_string(output_path.join("two.go"))
            .unwrap()
            .contains("regexp.MustCompile")
    );

    assert!(
        Command::new("gofmt")
            .args(["-w", output_path.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("go")
            .args(["test", "./..."])
            .env("GO111MODULE", "on")
            .current_dir(&output_path)
            .status()
            .unwrap()
            .success()
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A `nil` `[]T` marshals to `null` under `encoding/json`, which is the wrong
/// wire form for a required non-nullable array — and a payload this package's
/// own decoder rejects. `MarshalJSON` emits `[]` (items.md:193-202).
#[test]
fn go_json_required_array_emits_empty_array_for_a_nil_slice() {
    let temp_dir = unique_output_path("go-json-nil-slice");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("bag.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [tags]
properties:
  tags: { type: array, items: { type: string } }
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("bag");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    fs::write(
        output_path.join("bag_test.go"),
        r#"package bag

import (
    "encoding/json"
    "testing"
)

func TestNilSliceEmitsEmptyArray(t *testing.T) {
    encoded, err := json.Marshal(Bag{})
    if err != nil {
        t.Fatalf("nil slice rejected: %v", err)
    }
    if string(encoded) != `{"tags":[]}` {
        t.Fatalf("got %s", encoded)
    }
    var back Bag
    if err := json.Unmarshal(encoded, &back); err != nil {
        t.Fatalf("own output rejected: %v", err)
    }
}
"#,
    )
    .unwrap();
    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Object-shape leftovers that each produced Go that did not compile, or a
/// `Validate()` that lied: a closed-value member with a `default` (the accessor
/// has to return the closed defined type), an untyped catch-all colliding with a
/// declared member, the object-level constraints missing from the shared
/// `Validate`.
#[test]
fn go_json_object_shapes_compile_and_validate_completely() {
    let temp_dir = unique_output_path("go-json-object-shapes");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("shapes.nexusrpc.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
$defs:
  Defaulted:
    type: object
    properties:
      status: { type: string, enum: [active, inactive], default: active }
  Contact:
    type: object
    additionalProperties: false
    minProperties: 1
    maxProperties: 3
    dependentRequired:
      email: [name]
    properties:
      name: { type: string }
      email: { type: string }
      phone: { type: string }
  Open:
    type: object
    properties:
      a: { type: string }
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("shapes");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("shapes.go")).unwrap();
    assert!(
        rendered.contains("func (m Defaulted) StatusOrDefault() DefaultedStatus {"),
        "{rendered}"
    );
    assert!(
        rendered.contains("return DefaultedStatusActive"),
        "{rendered}"
    );

    fs::write(
        output_path.join("shapes_test.go"),
        r#"package shapes

import (
    "encoding/json"
    "strings"
    "testing"
)

func validationText(err error) string {
    if violations, ok := payloadValidationErrorViolations(err); ok {
        var text strings.Builder
        for _, violation := range violations { text.WriteString(violation.String()); text.WriteByte('\n') }
        return text.String()
    }
    return err.Error()
}

func TestObjectShapes(t *testing.T) {
    if got := (Defaulted{}).StatusOrDefault(); got != DefaultedStatusActive {
        t.Errorf("default accessor returned %v", got)
    }

    // The catch-all can only hold members the declared set does not name --
    // an untyped catch-all is no different from a typed one.
    a := "x"
    collision := Open{A: &a, AdditionalProperties: map[string]json.RawMessage{"a": json.RawMessage(`"y"`)}}
    if _, err := json.Marshal(collision); err == nil || !strings.Contains(validationText(err), "collides") {
        t.Errorf("catch-all collision was not reported: %v", err)
    }

    // The object-level constraints live in the shared Validate, and are
    // reported exactly once through MarshalJSON.
    if err := (Contact{}).Validate(); err == nil || !strings.Contains(validationText(err), "at least 1 properties") {
        t.Errorf("minProperties missing from Validate: %v", err)
    }
    email := "a@b.c"
    if err := (Contact{Email: &email}).Validate(); err == nil || !strings.Contains(validationText(err), "is required when") {
        t.Errorf("dependentRequired missing from Validate: %v", err)
    }
    if _, err := json.Marshal(Contact{Email: &email}); err == nil || strings.Count(validationText(err), "is required when") != 1 {
        t.Errorf("dependentRequired was not reported exactly once: %v", err)
    }
    name := "n"
    if err := (Contact{Name: &name, Email: &email}).Validate(); err != nil {
        t.Errorf("a satisfying Contact was rejected: %v", err)
    }
}
"#,
    )
    .unwrap();
    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A `$ref` to a named union resolves through the name manifest, so an
/// `x-go-name` on the union still binds the sealed interface. Recasing the
/// reference text missed the override and fell through to `*<Union>` — a
/// pointer to an interface, which does not compile.
#[test]
fn go_json_renamed_union_reference_binds_the_interface() {
    let temp_dir = unique_output_path("go-json-renamed-union");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("shapes.nexusrpc.yaml");
    fs::write(
        &input_path,
        r##"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
$defs:
  Shape:
    x-go-name: ShapeGo
    oneOf:
      - { $ref: "#/$defs/Circle" }
      - { $ref: "#/$defs/Square" }
  Circle:
    type: object
    required: [kind]
    properties:
      kind: { type: string, const: circle }
  Square:
    type: object
    required: [kind]
    properties:
      kind: { type: string, const: square }
  Holder:
    type: object
    properties:
      shape: { $ref: "#/$defs/Shape" }
"##,
    )
    .unwrap();
    let output_path = temp_dir.join("shapes");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("shapes.go")).unwrap();
    assert!(
        rendered.contains("Shape ShapeGo `json:\"shape,omitempty\"`"),
        "{rendered}"
    );
    assert!(!rendered.contains("*ShapeGo"), "{rendered}");

    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let build_status = Command::new("go")
        .args(["build", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(build_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

/// `json.Number` is a string type, so `encoding/json` decodes the quoted token
/// `"7"` into it without complaint and the spec-number path never learns the
/// wire token was a string. TypeScript, Python and Java all reject it, so this
/// was an accept-set divergence on every numeric member in the repo.
#[test]
fn go_json_numeric_members_reject_a_quoted_token() {
    let temp_dir = unique_output_path("go-json-quoted-number");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("nums.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [count, ratio]
properties:
  count: { type: integer }
  ratio: { type: number }
  counts:
    type: array
    items: { type: number }
    contains: { type: integer }
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("nums");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    fs::write(
        output_path.join("nums_test.go"),
        r#"package nums

import (
    "encoding/json"
    "testing"
)

func TestQuotedNumericTokenRejected(t *testing.T) {
    for _, tc := range []struct {
        wire   string
        accept bool
    }{
        {`{"count":7,"ratio":1.5}`, true},
        {`{"count":"7","ratio":1.5}`, false},
        {`{"count":7,"ratio":"1.5"}`, false},
        {`{"count":7,"ratio":1.5,"counts":[1]}`, true},
        // A quoted element must not satisfy an `integer` contains matcher either.
        {`{"count":7,"ratio":1.5,"counts":["1"]}`, false},
    } {
        var value Nums
        err := json.Unmarshal([]byte(tc.wire), &value)
        if (err == nil) != tc.accept {
            t.Errorf("%s: accept=%v, err=%v", tc.wire, tc.accept, err)
        }
    }
}
"#,
    )
    .unwrap();
    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A nullable scalar element (decision D2). The Go element is a `*T`, so both
/// array-keyword loops have to look through the pointer — and `null` carries
/// its own semantics: two `null` elements are a duplicate
/// (`uniqueItems.md:188-190`) while a `null` element never matches a scalar
/// `contains` matcher (`contains.md`, Interactions → nullability).
#[test]
fn go_json_nullable_elements_are_unique_and_never_match_a_matcher() {
    let temp_dir = unique_output_path("go-json-nullable-elements");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("elements.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [words, counts]
properties:
  words:
    type: array
    uniqueItems: true
    items:
      oneOf:
        - { type: string }
        - { type: "null" }
  counts:
    type: array
    contains: { type: integer, minimum: 2 }
    items:
      oneOf:
        - { type: integer }
        - { type: "null" }
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("elements");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    fs::write(
        output_path.join("elements_test.go"),
        r#"package elements

import (
    "encoding/json"
    "strings"
    "testing"
)

func validationText(err error) string {
    if violations, ok := payloadValidationErrorViolations(err); ok {
        var text strings.Builder
        for _, violation := range violations { text.WriteString(violation.String()); text.WriteByte('\n') }
        return text.String()
    }
    return err.Error()
}

func TestNullableElements(t *testing.T) {
    for _, tc := range []struct {
        wire   string
        accept bool
    }{
        {`{"words":["a",null,"b"],"counts":[2]}`, true},
        {`{"words":[null,null],"counts":[2]}`, false},
        {`{"words":["a","a"],"counts":[2]}`, false},
        {`{"words":[],"counts":[null,3]}`, true},
        {`{"words":[],"counts":[null,1]}`, false},
        {`{"words":[],"counts":[null]}`, false},
    } {
        var value Elements
        err := json.Unmarshal([]byte(tc.wire), &value)
        if (err == nil) != tc.accept {
            t.Errorf("%s: accept=%v, err=%v", tc.wire, tc.accept, err)
        }
    }

    // Serialize reaches the same verdicts over natively built values.
    word := "a"
    two := int64(2)
    if _, err := json.Marshal(Elements{Words: []*string{nil, nil}, Counts: []*int64{&two}}); err == nil ||
        !strings.Contains(validationText(err), "duplicate") {
        t.Errorf("two nil elements are a duplicate: %v", err)
    }
    if _, err := json.Marshal(Elements{Words: []*string{nil, &word}, Counts: []*int64{&two}}); err != nil {
        t.Errorf("one nil element is not a duplicate: %v", err)
    }
    if _, err := json.Marshal(Elements{Words: []*string{}, Counts: []*int64{nil}}); err == nil ||
        !strings.Contains(validationText(err), "no element matches") {
        t.Errorf("a nil element must not match the matcher: %v", err)
    }
}
"#,
    )
    .unwrap();
    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Two synthesized names that must follow the **emitted member identifier**, not
/// the JSON name: `x-go-enum-names` is keyed by a member's wire spelling (so a
/// numeric or boolean member can be renamed at all — P15's only escape hatch for
/// a value-constant collision), and an inline union's sealed interface is
/// `<Model><Member>`, the same rule the closed-value defined type already
/// follows.
#[test]
fn go_json_synthesized_names_follow_the_emitted_member() {
    let temp_dir = unique_output_path("go-json-synthesized-names");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("names.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  tier:
    type: integer
    enum: [1, 2]
    x-go-enum-names:
      "1": TierBronzeGo
      "2": TierSilverGo
  flag:
    type: boolean
    enum: [true, false]
    x-go-enum-names:
      "true": FlagOnGo
      "false": FlagOffGo
  idOrName:
    x-go-name: Chosen
    oneOf:
      - { type: string }
      - { type: integer }
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("names");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("names.go")).unwrap();
    // Numeric and boolean members are renameable.
    assert!(
        rendered.contains("TierBronzeGo NamesTier = 1"),
        "{rendered}"
    );
    assert!(rendered.contains("FlagOnGo NamesFlag = true"), "{rendered}");
    // The union moves with `x-go-name`, wrappers and dispatcher included.
    assert!(
        rendered.contains("type NamesChosen interface {"),
        "{rendered}"
    );
    assert!(
        rendered.contains("type NamesChosenString string"),
        "{rendered}"
    );
    assert!(
        rendered.contains("func unmarshalNamesChosen("),
        "{rendered}"
    );
    assert!(!rendered.contains("NamesIdOrName"), "{rendered}");
    // The wire name is untouched by every one of them.
    assert!(
        rendered.contains("`json:\"idOrName,omitempty\"`"),
        "{rendered}"
    );

    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let build_status = Command::new("go")
        .args(["build", "./..."])
        .env("GO111MODULE", "on")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(build_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}
