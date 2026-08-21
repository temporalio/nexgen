// Drives the `nexgen` binary over the WIT/proto CLI surface (`--descriptors`,
// `--native-api`, `--support-file`), all behind the `advanced` feature.
#![cfg(feature = "advanced")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nexgen::{GenerateRequest, generate_to_file};

mod common;
use common::json_input_path;

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A property whose union has one inline structured object branch (named
/// `<Union>Object`) and one scalar branch.
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

fn generate_to_string_with_inputs(
    language: nexgen::language::Language,
    input_paths: &[PathBuf],
    descriptor_paths: &[PathBuf],
) -> Result<String, Box<dyn std::error::Error>> {
    let temp_dir = unique_output_path("go-rendered");
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        language,
        input_paths: input_paths.to_vec(),
        support_paths: Vec::new(),
        descriptor_paths: descriptor_paths.to_vec(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: true,
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

fn go_root(root: &Path) -> PathBuf {
    root.join("advanced/samples/go")
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

fn go_package_name(example_id: &str) -> String {
    example_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_lowercase()
}

fn go_output_path(root: &Path, example_id: &str) -> PathBuf {
    go_root(root).join(go_package_name(example_id))
}

fn go_example_ids(root: &Path) -> Vec<String> {
    let go_root = go_root(root);
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
            if go_root.join(go_package_name(&example_id)).is_dir() {
                Some(example_id)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids
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

fn generate_formatted_go_output(root: &Path, example_id: &str, output_path: &Path) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexgen"));
    command
        .arg("go")
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

    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
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

#[test]
fn cli_generates_go_support_file_from_parameter() {
    let root = project_root();
    let temp_dir = unique_output_path("go-support-file-input");
    fs::create_dir_all(&temp_dir).unwrap();
    let support_path = temp_dir.join("custom_support.go");
    let output_path = temp_dir.join("output");
    fs::write(
        &support_path,
        "package placeholder\n\nfunc CustomSupportHook() string {\n\treturn \"custom\"\n}\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "go",
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
    // The explicit support file is emitted even though the WIT-direct
    // user-service package performs no proto conversion, and its package
    // declaration is rewritten to the generated package name, which is the
    // output directory's basename (Go convention: package name = directory
    // name).
    let support_contents = fs::read_to_string(output_path.join("support.go")).unwrap();
    assert!(support_contents.starts_with("package output\n"));
    assert!(support_contents.contains("func CustomSupportHook() string"));
    assert!(output_path.join("userservice.go").is_file());
    assert!(!output_path.join("api.go").exists());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn cli_generates_go_with_package_self_imports_removed() {
    let root = project_root();
    let temp_dir = unique_output_path("go-namespace");
    // The output directory's basename must match the last path segment of
    // `@nexus.namespace go="..."` -- the generated package name always comes
    // from the output directory, and this generated code is declared to live
    // inside the real `go.temporal.io/sdk/workflow` package.
    let output_path = temp_dir.join("workflow");
    fs::create_dir_all(&temp_dir).unwrap();
    let temp_input_path = temp_dir.join("user-service.wit");
    let input = fs::read_to_string(input_path(&root, "user-service"))
        .unwrap()
        .replace(
            "interface user-service {",
            "/// @nexus.namespace go=\"go.temporal.io/sdk/workflow\"\ninterface user-service {",
        );
    fs::write(&temp_input_path, input).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "go",
            temp_input_path.to_str().unwrap(),
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
    let api = fs::read_to_string(output_path.join("userservice.go")).unwrap();
    assert!(api.contains("package workflow\n"));
    assert!(!api.contains("\"go.temporal.io/sdk/workflow\""));
    assert!(!api.contains("type OperationFuture interface {"));
    assert!(api.contains("func getUser(ctx Context, request getUserRequest) Future"));
    assert!(!api.contains("const ServiceName"));
    assert!(!api.contains("const Endpoint"));
    assert!(!api.contains("const GetUserOp"));
    assert!(api.contains("NexusOperationOptions{}"));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn cli_rejects_go_output_directory_mismatched_with_namespace() {
    let root = project_root();
    let temp_dir = unique_output_path("go-namespace-mismatch");
    // Deliberately mismatched: the namespace says `workflow`, but the output
    // directory is named `output`.
    let output_path = temp_dir.join("output");
    fs::create_dir_all(&temp_dir).unwrap();
    let temp_input_path = temp_dir.join("user-service.wit");
    let input = fs::read_to_string(input_path(&root, "user-service"))
        .unwrap()
        .replace(
            "interface user-service {",
            "/// @nexus.namespace go=\"go.temporal.io/sdk/workflow\"\ninterface user-service {",
        );
    fs::write(&temp_input_path, input).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "go",
            temp_input_path.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("go.temporal.io/sdk/workflow"), "{stderr}");
    assert!(stderr.contains("package `workflow`"), "{stderr}");
    assert!(stderr.contains("package `output`"), "{stderr}");
    assert!(!output_path.exists());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn cli_rejects_output_at_filesystem_root() {
    // An output path with no basename is never a real output directory, so
    // generating into `/` is rejected up front. This check is
    // language-agnostic, exercised here via `generate go`.
    let root = project_root();
    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "go",
            input_path(&root, "user-service").to_str().unwrap(),
            "--output",
            "/",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("filesystem root"), "{stderr}");
}

#[test]
fn cli_preserves_existing_output_directory_contents() {
    // Generating into an existing directory writes into it instead of
    // replacing it: hand-written files sitting alongside the generated ones
    // survive, and so does a file a previous run generated for a definition
    // that no longer exists. This behavior is language-agnostic, exercised
    // here via `generate go`.
    let root = project_root();
    let temp_dir = unique_output_path("go-preserve-output");
    let output_path = temp_dir.join("output");
    fs::create_dir_all(output_path.join("notes")).unwrap();
    let handwritten = "package output\n\nfunc Handwritten() string {\n\treturn \"kept\"\n}\n";
    fs::write(output_path.join("handwritten.go"), handwritten).unwrap();
    fs::write(output_path.join("notes/keep.txt"), "keep me\n").unwrap();
    let stale = "package output\n\n// Left over from an earlier run.\n";
    fs::write(output_path.join("stale.go"), stale).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "go",
            input_path(&root, "user-service").to_str().unwrap(),
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
    assert!(output_path.join("userservice.go").is_file());
    assert_eq!(
        fs::read_to_string(output_path.join("handwritten.go")).unwrap(),
        handwritten
    );
    assert_eq!(
        fs::read_to_string(output_path.join("notes/keep.txt")).unwrap(),
        "keep me\n"
    );
    assert_eq!(
        fs::read_to_string(output_path.join("stale.go")).unwrap(),
        stale
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn cli_overwrites_previously_generated_files_in_place() {
    // Preserving the output directory must not turn into refusing to update
    // the files the generator owns: a path it writes is replaced wholesale.
    let root = project_root();
    let temp_dir = unique_output_path("go-overwrite-output");
    let output_path = temp_dir.join("output");
    fs::create_dir_all(&output_path).unwrap();
    fs::write(
        output_path.join("userservice.go"),
        "package output\n\n// Stale generated contents.\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "go",
            input_path(&root, "user-service").to_str().unwrap(),
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
    let contents = fs::read_to_string(output_path.join("userservice.go")).unwrap();
    assert!(!contents.contains("Stale generated contents"), "{contents}");
    assert!(contents.contains("Code generated by nexgen"), "{contents}");
    fs::remove_dir_all(temp_dir).unwrap();
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
    /// @nexus.source go="NamespaceData()"
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
        &[wit_path],
        &[descriptor_path(&root)],
    )
    .unwrap();

    // The sourced map is bound to a field-unique local, evaluated once, and
    // copied into a properly typed proto map.
    assert!(rendered.contains("sourcedData := NamespaceData()"));
    assert!(rendered.contains("if len(sourcedData) > 0 {"));
    assert!(rendered.contains("message.Data = make(map[string]string, len(sourcedData))"));
    assert!(rendered.contains("for k, v := range sourcedData {"));
    assert!(rendered.contains("message.Data[k] = v"));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_proto_enum_field_infers_descriptor_go_package() {
    let root = project_root();
    let temp_dir = unique_output_path("go-unconvertible-input");
    fs::create_dir_all(&temp_dir).unwrap();
    let wit_path = temp_dir.join("unconvertible.wit");
    // The `namespace-state` alias is type-replaced for Python only. Go should
    // still infer the proto enum import and conversion from descriptor
    // `go_package` metadata.
    fs::write(
        &wit_path,
        r#"package temporal:unconvertible@1.0.0;

world system {
  export namespace-service;
}

/// @nexus.endpoint "namespace-service"
interface namespace-service {
  /// String-shaped placeholder for omitted and replaced fields.
  type placeholder = string;

  /// @nexus.proto "temporal.api.enums.v1.NamespaceState"
  /// @nexus.type python="int"
  type namespace-state = placeholder;

  /// @nexus.proto "temporal.api.namespace.v1.NamespaceInfo"
  record namespace-info {
    name: option<string>,
    state: option<namespace-state>,
    /// @nexus.omit
    description: placeholder,
    /// @nexus.omit
    owner-email: placeholder,
    /// @nexus.omit
    data: placeholder,
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
        &[wit_path],
        &[descriptor_path(&root)],
    )
    .unwrap();

    assert!(rendered.contains("namespace \"go.temporal.io/api/namespace/v1\""));
    assert!(rendered.contains("enums \"go.temporal.io/api/enums/v1\""));
    assert!(rendered.contains("type NamespaceState int32"));
    assert!(rendered.contains("State *NamespaceState"));
    assert!(rendered.contains("message.State = enums.NamespaceState((*m.State))"));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_output_transform_returns_transformed_type() {
    let temp_dir = unique_output_path("go-output-transform");
    fs::create_dir_all(&temp_dir).unwrap();
    let wit_path = temp_dir.join("go-output-transform.wit");
    fs::write(
        &wit_path,
        r#"package temporal:go-output-transform@1.0.0;

world system {
  export sample-service;
}

/// @nexus.endpoint "sample-service"
interface sample-service {
  record transform-request {
    id: string,
  }

  record transform-response {
    value: string,
  }

  /// @nexus.output-transform
  ///   go-type="example.com/nexgen/handles:handles.ValueHandle"
  ///   go="handles.NewValueHandle(request.Id, result.Value)"
  get-handle: func(request: transform-request) -> transform-response;
}
"#,
    )
    .unwrap();

    let rendered =
        generate_to_string_with_inputs(nexgen::language::Language::Go, &[wit_path], &[]).unwrap();

    assert!(rendered.contains("\"example.com/nexgen/handles\""));
    assert!(rendered.contains(
        "func getHandle(ctx workflow.Context, request transformRequest) workflow.Future {"
    ));
    assert!(rendered.contains("result, resultSettable := workflow.NewFuture(ctx)"));
    assert!(rendered.contains("workflow.Go(ctx, func(ctx workflow.Context) {"));
    assert!(rendered.contains("resultSettable.Set(value, nil)"));
    assert!(!rendered.contains("nexgenOperationFuture"));
    assert!(rendered.contains("\tvar result transformResponse\n"));
    assert!(rendered.contains("\t\tif err := fut.Get(ctx, &result); err != nil {\n"));
    assert!(
        rendered.contains("\t\tvalue, err := handles.NewValueHandle(request.Id, result.Value)\n")
    );
    assert!(rendered.contains("\t\tresultSettable.Set(value, nil)\n"));
    assert!(
        rendered.contains(
            "func GetHandle(ctx workflow.Context, opts GetHandleOptions) workflow.Future {"
        )
    );
    assert!(!rendered.contains("NexusOperationFuture"));
    assert!(!rendered.contains("GetNexusOperationExecution"));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_examples_generation_matches_checked_in_output() {
    let root = project_root();
    for example_id in go_example_ids(&root) {
        let temp_root = unique_output_path(&format!("go-{example_id}"));
        fs::create_dir_all(&temp_root).unwrap();
        fs::write(
            temp_root.join("go.mod"),
            "module examples/go\n\ngo 1.24.0\n",
        )
        .unwrap();
        let output_path = temp_root.join(go_package_name(&example_id));
        generate_formatted_go_output(&root, &example_id, &output_path);
        let rendered = read_go_output_files(&output_path);
        let expected = read_go_output_files(&go_output_path(&root, &example_id));
        assert_eq!(rendered, expected, "snapshot mismatch for {example_id}");
        fs::remove_dir_all(temp_root).unwrap();
    }
}

#[test]
fn go_json_generation_matches_checked_in_output() {
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

            let expected = read_go_output_files(&go_json_output_path(&root, mode, example_id));
            let actual = read_go_output_files(&output_path);
            assert_eq!(
                actual, expected,
                "go JSON {example_id} {mode} output changed"
            );

            let generated_file = PathBuf::from(format!("{example_id}.go"));
            let generated = actual
                .get(&generated_file)
                .unwrap_or_else(|| panic!("{} should be generated", generated_file.display()));
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
                        assert!(generated.contains("func formatDuration(d time.Duration) string"));
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

#[test]
fn go_json_package_name_derives_from_output_directory_name() {
    // The Go package name for a JSON-schema input must come from the output
    // directory's basename (Go convention: package name = directory name),
    // not from an unrelated signal like the input's service name.
    let root = project_root();
    let temp_dir = unique_output_path("go-json-package-name-fallback");
    let output_path = temp_dir.join("widgets");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![json_input_path(&root, "chat")],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: true,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let generated = fs::read_to_string(output_path.join("widgets.go")).unwrap();
    assert!(generated.starts_with(concat!(
        "// Code generated by nexgen v",
        env!("CARGO_PKG_VERSION"),
        ". DO NOT EDIT.\npackage widgets\n"
    )));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_function_fields_accept_strings_or_exact_function_pointers() {
    let root = project_root();
    let rendered = generate_to_string_with_inputs(
        nexgen::language::Language::Go,
        &example_input_paths(&root, "function-execution"),
        &[descriptor_path(&root)],
    )
    .unwrap();

    let service_rendered = rendered
        .split("### functionexecution.go")
        .nth(1)
        .expect("functionexecution.go should be rendered");

    assert!(service_rendered.contains("\"reflect\""));
    assert!(service_rendered.contains("\"runtime\""));
    assert!(service_rendered.contains("\"strings\""));
    assert!(!service_rendered.contains("\"errors\""));
    assert!(!service_rendered.contains("func nexgenFunctionName[F any](value F) string"));
    assert!(service_rendered.contains("functionName = strings.TrimSuffix(shortName, \"-fm\")"));
    assert!(service_rendered.contains("rv := reflect.ValueOf(function)"));
    assert!(service_rendered.contains("switch rv := reflect.ValueOf(function); rv.Kind()"));
    assert!(service_rendered.contains("functionName = strings.TrimSuffix(shortName, \"-fm\")"));
    assert!(!service_rendered.contains("func nexgenWorkflowDataConverter"));
    assert!(!service_rendered.contains("func getWorkflowDataConverter"));
    assert!(!service_rendered.contains("type nexgenOperationFuture struct"));
    assert!(!service_rendered.contains("type OperationFuture interface"));
    // Internal request structs remain wire-shaped without public API docs.
    assert!(rendered.contains("type executeFunctionRequest struct {\n\tFunction string"));
    assert!(rendered.contains("type executeNamedFunctionRequest struct {\n\tFunction string"));
    assert!(!rendered.contains("type executeFunctionRequest struct {\n\t// Required."));

    // Required public function fields accept string names only when the WIT
    // directive declares an alternate type.
    assert!(rendered.contains(
        "func ExecuteFunction(\n\tctx workflow.Context,\n\topts ExecuteFunctionOptions,\n\tfunction func(string, bool) string,\n"
    ));
    assert!(rendered.contains(
        "functionName := \"\"\n\t{\n\t\trv := reflect.ValueOf(function)\n\t\tfullName := runtime.FuncForPC(rv.Pointer()).Name()"
    ));
    assert!(rendered.contains(
        "func ExecuteCountedFunction(\n\tctx workflow.Context,\n\topts ExecuteCountedFunctionOptions,\n\tfunction func(string, int32) string,\n"
    ));
    assert!(rendered.contains(
        "func ExecuteNamedFunction[FunctionF interface{ ~string | func(string, bool) string }]("
    ));
    assert!(rendered.contains(
        "func ExecuteVarargsFunction(\n\tctx workflow.Context,\n\topts ExecuteVarargsFunctionOptions,\n\tfunction func(...string) string,\n"
    ));
    assert!(rendered.contains(
        "func ExecuteNamedVarargsFunction[FunctionF interface{ ~string | func(...string) string }]("
    ));
    assert!(rendered.contains("\tfunction FunctionF,\n"));
    assert!(rendered.contains("\t\tFunction: functionName,\n"));
    assert!(!rendered.contains("func ExecuteFunctionWithArgs"));
    assert!(!rendered.contains("func ExecuteCountedFunctionWithArgs"));
    assert!(!rendered.contains("func ExecuteNamedFunctionWithArgs"));

    // Function-adjacent args stay positional and do not enter the options struct.
    assert!(rendered.contains("type ExecuteVarargsFunctionOptions struct {\n}"));
    assert!(rendered.contains(
        "func ExecuteVarargsFunction(\n\tctx workflow.Context,\n\topts ExecuteVarargsFunctionOptions,\n\tfunction func(...string) string,\n"
    ));
    assert!(rendered.contains("\topts ExecuteVarargsFunctionOptions,\n"));
    assert!(rendered.contains("\tfunction func(...string) string,\n"));
    assert!(rendered.contains("\targs ...string,\n"));
    assert!(rendered.contains("\t\tArgs: args,\n"));

    // Primary base varargs collapse into the default wrapper; args are
    // positional only, never options fields.
    assert!(!rendered.contains("func ExecuteVarargsFunctionWithArgs"));
    assert!(!rendered.contains("func ExecuteNamedVarargsFunctionWithArgs"));
    assert!(!rendered.contains("cannot specify both positional arguments and args"));
    assert!(!rendered.contains("opts.Args"));
    assert!(rendered.contains("\t\tArgs: args,\n"));
    assert!(rendered.contains(
        "// Input name: The name argument for the function.\n// Input enabled: The enabled argument for the function.\nfunc ExecuteFunction"
    ));
    assert!(
        rendered
            .contains("// Input args: Arguments for the function.\nfunc ExecuteVarargsFunction")
    );

    let user_rendered = generate_to_string_with_inputs(
        nexgen::language::Language::Go,
        &[input_path(&root, "user-service")],
        &[],
    )
    .unwrap();
    assert!(!user_rendered.contains("nexgenFunctionName"));
    assert!(!user_rendered.contains("getWorkflowDataConverter"));
    assert!(!user_rendered.contains("\"reflect\""));
    assert!(!user_rendered.contains("\"runtime\""));
    assert!(!user_rendered.contains("\"strings\""));
    assert!(!user_rendered.contains("\"errors\""));
}

#[test]
fn go_function_name_locals_do_not_collide_with_authored_parameters() {
    let temp_dir = unique_output_path("go-function-local-collisions");
    fs::create_dir_all(&temp_dir).unwrap();
    let wit_path = temp_dir.join("collision-service.wit");
    fs::write(
        &wit_path,
        r#"
package test:function-local-collisions@1.0.0;

world test-world {
  export collision-service;
}

interface functions {
  type placeholder = string;
  function-call: func(name: string, enabled: bool) -> string;

  /// @nexus.function
  ///   signature="function-call"
  type executable-function = placeholder;
}

/// @nexus.endpoint "collision-endpoint"
interface collision-service {
  use functions.{executable-function};

  record execute-request {
    function: executable-function,
    function-name: string,
    rv: string,
    full-name: string,
    elements: string,
    short-name: string,
  }

  execute: func(request: execute-request);
}
"#,
    )
    .unwrap();

    let rendered =
        generate_to_string_with_inputs(nexgen::language::Language::Go, &[wit_path], &[]).unwrap();

    // The result local is allocated around the authored `functionName`
    // parameter. Extraction-only locals can keep simple names because their
    // block scope safely shadows the authored parameters.
    assert!(rendered.contains("\tfunctionName2 := \"\"\n\t{"));
    assert!(rendered.contains("\t\trv := reflect.ValueOf(function)"));
    assert!(rendered.contains("\t\tfullName := runtime.FuncForPC(rv.Pointer()).Name()"));
    assert!(rendered.contains("\t\telements := strings.Split(fullName, \".\")"));
    assert!(rendered.contains("\t\tshortName := elements[len(elements)-1]"));
    assert!(rendered.contains("\t\tfunctionName2 = strings.TrimSuffix(shortName, \"-fm\")"));
    assert!(rendered.contains("\t\tFunction: functionName2,"));

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_temporal_function_constraints_use_workflow_context_prefix() {
    let root = project_root();
    let rendered = generate_to_string_with_inputs(
        nexgen::language::Language::Go,
        &example_input_paths(&root, "workflow-service"),
        &[descriptor_path(&root)],
    )
    .unwrap();

    assert!(rendered.contains("func SignalWithStartWorkflow("));
    assert!(
        rendered
            .contains("func SignalWithStartWorkflowTyped[WorkflowArg any, WorkflowResult any](")
    );
    assert!(rendered.contains("\tworkflow func(workflow.Context, WorkflowArg) WorkflowResult,"));
    assert!(!rendered.contains("func SignalWithStartWorkflowWithArgs("));
    assert!(!rendered.contains("SignalF interface"));
    assert!(rendered.contains("\topts SignalWithStartWorkflowOptions,\n"));
    assert!(
        rendered.contains("\tsignal string,\n\tsignalArg any,\n\tworkflow any,\n\targs ...any,\n")
    );
    assert!(rendered.contains("\targs ...any,\n"));
    assert!(rendered.contains("switch rv := reflect.ValueOf(workflow); rv.Kind()"));
    assert_eq!(
        rendered
            .matches("switch rv := reflect.ValueOf(workflow); rv.Kind()")
            .count(),
        1
    );
    assert!(!rendered.contains("switch rv := reflect.ValueOf(signal); rv.Kind()"));
    assert!(rendered.contains("\t\tWorkflow: workflowName,\n"));
    assert!(rendered.contains("\t\tSignal: signal,\n"));
    assert!(rendered.contains("\t\tArgs: args,\n"));
    assert!(rendered.contains("\t\tSignalArgs: []any{signalArg},\n"));
}

#[test]
fn go_type_showcase_generates_expected_types() {
    let root = project_root();
    let rendered = generate_to_string_with_inputs(
        nexgen::language::Language::Go,
        &example_input_paths(&root, "type-showcase"),
        &[descriptor_path(&root)],
    )
    .unwrap();

    // Service and operation names are inlined at call sites, not exported as constants.
    assert!(!rendered.contains("const ServiceName"));
    assert!(!rendered.contains("const Endpoint"));
    assert!(!rendered.contains("const GetUserOp"));
    assert!(!rendered.contains("const UpdateEmailOp"));
    assert!(!rendered.contains("const RenameOp"));
    assert!(!rendered.contains("const SetProfileOp"));
    assert!(!rendered.contains("const DeactivateOp"));

    // Enums
    assert!(rendered.contains("type UserStatus int32"));
    assert!(rendered.contains("UserStatusActive"));
    assert!(rendered.contains("UserStatusSuspended"));
    assert!(rendered.contains("UserStatusDeleted"));

    // Flags
    assert!(rendered.contains("type UserCapability int32"));
    assert!(rendered.contains("UserCapabilityReadProfile"));
    assert!(rendered.contains("1 << 0"));
    assert!(rendered.contains("1 << 1"));
    assert!(rendered.contains("1 << 2"));

    // Variants -- sealed interface pattern
    assert!(rendered.contains("type NotificationTarget interface {"));
    assert!(rendered.contains("isNotificationTarget()"));
    // Case structs with payload
    assert!(rendered.contains("type NotificationTargetEmail struct {"));
    assert!(rendered.contains("Value string"));
    assert!(rendered.contains("func (NotificationTargetEmail) isNotificationTarget() {}"));
    assert!(rendered.contains("type NotificationTargetSms struct {"));
    assert!(rendered.contains("func (NotificationTargetSms) isNotificationTarget() {}"));
    // Payload-less case struct
    assert!(rendered.contains("type NotificationTargetNone struct{}"));
    assert!(rendered.contains("func (NotificationTargetNone) isNotificationTarget() {}"));

    // Records with required/optional fields
    assert!(rendered.contains("type getUserRequest struct"));
    assert!(!rendered.contains("type GetUserRequest struct"));
    assert!(rendered.contains("\t// Required.\n\tUserId string"));
    // Optional scalar fields are rendered as pointers so absence is
    // representable as nil (distinct from a present zero value).
    assert!(rendered.contains("ConsistencyToken *string"));

    assert!(rendered.contains("type PostalAddress struct"));
    assert!(rendered.contains("\t// Required.\n\tStreet string"));
    assert!(rendered.contains("\t// Required.\n\tCity string"));
    assert!(rendered.contains("\t// Required.\n\tCountry string"));
    assert!(rendered.contains("Coordinates *Tuple2[float64, float64]"));
    assert!(!rendered.contains("type Coordinates struct"));

    assert!(rendered.contains("type UserProfile struct"));
    assert!(rendered.contains("\t// Required.\n\tTags []string"));
    assert!(rendered.contains("\t// Required.\n\tMetadata map[string]string"));
    assert!(rendered.contains("\t// Required.\n\tCapabilities UserCapability"));
    assert!(rendered.contains("\t// Required.\n\tSyncState Result[string, string]"));
    assert!(!rendered.contains("type SyncState struct"));
    // Variant interface field
    assert!(rendered.contains("\t// Required.\n\tNotificationTarget NotificationTarget"));
    // Optional struct field keeps pointer and is not marked required.
    assert!(rendered.contains("Address *PostalAddress"));
    assert!(!rendered.contains("\t// Required.\n\tAddress *PostalAddress"));

    assert!(rendered.contains("type deactivateRequest struct"));
    assert!(!rendered.contains("type DeactivateRequest struct"));
    assert!(rendered.contains("\t// Required.\n\tUserId string"));
    // Optional scalar -- pointer so absence is representable as nil.
    assert!(rendered.contains("Reason *string"));

    // Tuples and results inside containers instantiate shared generic helper
    // types instead of field-named structs.
    assert!(rendered.contains("type SyncReport struct"));
    assert!(rendered.contains("\t// Required.\n\tRoute []Tuple2[float64, float64]"));
    assert!(rendered.contains("\t// Required.\n\tAttempts []Result[string, string]"));
    assert!(rendered.contains("\t// Required.\n\tRegionStatus map[string]Result[string, string]"));
    assert!(rendered.contains("type Tuple2[T1, T2 any] struct {"));
    assert!(rendered.contains("\t// Required.\n\tFirst T1"));
    assert!(rendered.contains("\t// Required.\n\tSecond T2"));
    assert!(rendered.contains("type Result[T, E any] struct {"));
    assert!(rendered.contains("Result T"));
    assert!(rendered.contains("Error E"));

    // Resource struct
    assert!(rendered.contains("type User struct"));
    assert!(rendered.contains("\t// Required.\n\tDisplayName string"));
    assert!(rendered.contains("\t// Required.\n\tStatus UserStatus"));
    assert!(rendered.contains("\t// Required.\n\tProfile UserProfile"));
    assert!(rendered.contains("func NewUser("));

    // Resource methods
    assert!(rendered.contains(
        "func (u *User) UpdateEmail(ctx workflow.Context, email string) workflow.Future"
    ));
    assert!(rendered.contains("updateEmailRequest{UserId: u.UserId, Email: email}"));
    assert!(rendered.contains(
        "func (u *User) Rename(ctx workflow.Context, displayName string) workflow.Future"
    ));
    assert!(rendered.contains("renameRequest{UserId: u.UserId, DisplayName: displayName}"));
    // Void resource method -- optional primitive param is value-shaped publicly
    // and converted to a pointer for the internal request.
    assert!(rendered.contains(
        "func (u *User) Deactivate(ctx workflow.Context, reason string) workflow.Future"
    ));
    assert!(rendered.contains("var reasonPtr *string"));
    assert!(rendered.contains("deactivateRequest{UserId: u.UserId, Reason: reasonPtr}"));

    // Unexported operation wrapper functions
    assert!(
        rendered
            .contains("func getUser(ctx workflow.Context, request getUserRequest) workflow.Future")
    );
    assert!(rendered.contains(
        "func updateEmail(ctx workflow.Context, request updateEmailRequest) workflow.Future"
    ));
    assert!(rendered.contains("workflow.NewNexusClient(\"type-showcase\", \"TypeShowcase\")"));
    assert!(rendered.contains(
        "c.ExecuteOperation(ctx, \"GetUser\", request, workflow.NexusOperationOptions{})"
    ));
    // Void operation
    assert!(rendered.contains(
        "func deactivate(ctx workflow.Context, request deactivateRequest) workflow.Future"
    ));
    assert!(rendered.contains("\treturn fut\n"));

    // Exported convenience wrappers -- required non-function fields live in options.
    assert!(rendered.contains(
        "func UpdateEmail(ctx workflow.Context, opts UpdateEmailOptions) workflow.Future"
    ));
    // The request struct is always constructed across multiple lines.
    assert!(
        rendered
            .contains("updateEmailRequest{\n\t\tUserId: opts.UserId,\n\t\tEmail: opts.Email,\n\t}")
    );
    // Required and optional public fields share the options struct; optional
    // primitive fields default-pun to nil internally.
    assert!(rendered.contains("type GetUserOptions struct"));
    assert!(rendered.contains("UserId string"));
    assert!(rendered.contains("ConsistencyToken string"));
    assert!(
        rendered
            .contains("func GetUser(ctx workflow.Context, opts GetUserOptions) workflow.Future")
    );
    assert!(rendered.contains("var consistencyToken *string"));
    assert!(rendered.contains(
        "getUserRequest{\n\t\tUserId: opts.UserId,\n\t\tConsistencyToken: consistencyToken,\n\t}"
    ));
    // Void convenience wrapper with options
    assert!(rendered.contains("type DeactivateOptions struct"));
    assert!(rendered.contains("Reason string"));
    assert!(
        rendered.contains(
            "func Deactivate(ctx workflow.Context, opts DeactivateOptions) workflow.Future"
        )
    );
}

#[test]
fn go_type_roundtrip_generates_proto_conversions() {
    let root = project_root();
    let rendered = generate_to_string_with_inputs(
        nexgen::language::Language::Go,
        &example_input_paths(&root, "type-roundtrip"),
        &[descriptor_path(&root)],
    )
    .unwrap();

    // Aliased proto imports derived from the descriptors' `go_package` option.
    assert!(rendered.contains("activity \"go.temporal.io/api/activity/v1\""));
    assert!(rendered.contains("common \"go.temporal.io/api/common/v1\""));

    // Optional override-typed fields are rendered as pointers; the required
    // retry-policy field stays a value. (Assertions use the un-gofmt'd output,
    // so fields are single-space separated.)
    assert!(rendered.contains("TaskQueue *string"));
    assert!(rendered.contains("\t// Required.\n\tRetryPolicy temporal.RetryPolicy"));
    assert!(rendered.contains("ScheduleToCloseTimeout *time.Duration"));
    assert!(rendered.contains("Priority *temporal.Priority"));

    // Generated model gets a context-aware ToProto method targeting the proto
    // message type and returning conversion errors.
    assert!(rendered
        .contains("func (m ActivityOptions) toProto(ctx workflow.Context) (*activity.ActivityOptions, error) {"));
    assert!(rendered.contains("message := &activity.ActivityOptions{}"));
    assert!(
        rendered.contains("type ActivityOptions struct {\n\t// Optional.\n\tTaskQueue *string")
    );
    assert!(rendered.contains("\t// Optional.\n\tScheduleToCloseTimeout *time.Duration"));
    assert!(rendered.contains("\t// Optional.\n\tPriority *temporal.Priority"));
    // Optional override fields pass the pointer straight to the nil-safe
    // converter; the required field passes its address.
    assert!(rendered.contains("converted, err := retryPolicyToProto(ctx, &m.RetryPolicy)"));
    assert!(rendered.contains("converted, err := taskQueueToProto(ctx, m.TaskQueue)"));
    assert!(rendered.contains("converted, err := priorityToProto(ctx, m.Priority)"));
    assert!(rendered.contains("converted, err := durationToProto(ctx, m.ScheduleToCloseTimeout)"));
    assert!(rendered.contains("return message, nil"));

    // Generated model gets a context-aware FromProto constructor. Optional
    // override fields assign the converter's pointer result directly; the
    // required field is dereferenced with a nil guard.
    assert!(rendered.contains(
        "func activityOptionsFromProto(ctx workflow.Context, proto *activity.ActivityOptions) (ActivityOptions, error) {"
    ));
    assert!(rendered.contains("converted, err := taskQueueFromProto(ctx, proto.GetTaskQueue())"));
    assert!(
        rendered.contains("converted, err := retryPolicyFromProto(ctx, proto.GetRetryPolicy())")
    );
    assert!(rendered.contains("value.RetryPolicy = *converted"));
    // Operation functions convert the request to proto before the SDK call and
    // decode the proto response afterwards.
    assert!(rendered.contains("requestProto, err := request.toProto(ctx)"));
    assert!(rendered.contains(
        "fut := c.ExecuteOperation(ctx, \"ActivityOptionsOperation\", requestProto, workflow.NexusOperationOptions{})"
    ));
    assert!(rendered.contains("var result activity.ActivityOptions"));
    assert!(rendered.contains("value, err := activityOptionsFromProto(ctx, &result)"));

    // The hand-written support fragment is emitted alongside the generated
    // service file with the pointer-in/pointer-out converter contract.
    assert!(rendered.contains("### support.go"));
    assert!(
        rendered.contains("func retryPolicyToProto(_ workflow.Context, p *temporal.RetryPolicy) (*common.RetryPolicy, error) {")
    );
    assert!(
        rendered
            .contains("func retryPolicyFromProto(_ workflow.Context, p *common.RetryPolicy) (*temporal.RetryPolicy, error) {")
    );
}

#[test]
fn go_proto_resource_return_converts_request_and_constructs_resource() {
    let root = project_root();
    let rendered = generate_to_string_with_inputs(
        nexgen::language::Language::Go,
        &example_input_paths(&root, "start-workflow"),
        &[descriptor_path(&root)],
    )
    .unwrap();

    assert!(rendered.contains("\trequestProto, err := request.toProto(ctx)\n"));
    assert!(rendered.contains(
        "\tif err != nil {\n\t\tresult, resultSettable := workflow.NewFuture(ctx)\n\t\tresultSettable.SetError(err)\n\t\treturn result\n\t}\n"
    ));
    assert!(rendered.contains(
        "fut := c.ExecuteOperation(ctx, \"StartWorkflow\", requestProto, workflow.NexusOperationOptions{})"
    ));
    assert!(rendered.contains("\tvar result workflowservice.StartWorkflowExecutionResponse\n"));
    assert!(rendered.contains(
        "value := NewStartedWorkflow(requestProto.GetNamespace(), request.WorkflowId, result.GetRunId())"
    ));
    assert!(rendered.contains(
        "type StartedWorkflow struct {\n\t// Required.\n\tNamespace string\n\t// Required.\n\tWorkflowId string\n\t// Optional.\n\tRunId *string\n}"
    ));
    assert!(rendered.contains(
        "func NewStartedWorkflow(namespace string, workflowId string, runId string) StartedWorkflow"
    ));
    assert!(rendered.contains(
        "fut := c.ExecuteOperation(ctx, \"RestartWorkflow\", requestProto, workflow.NexusOperationOptions{})"
    ));
    assert!(rendered.contains("func StartWorkflow("));
    assert!(rendered.contains(
        "type StartWorkflowOptions struct {\n\t// Required.\n\tWorkflow string\n\t// Required.\n\tWorkflowId string\n\t// Required.\n\tTaskQueue string\n\t// Optional.\n\tWorkflowStartDelay time.Duration\n}"
    ));
    assert!(rendered.contains(
        "func StartWorkflow(ctx workflow.Context, opts StartWorkflowOptions) workflow.Future"
    ));
    assert!(rendered.contains("func RestartWorkflow("));
    assert!(rendered.contains(
        "func RestartWorkflow(ctx workflow.Context, opts RestartWorkflowOptions) workflow.Future"
    ));
    assert!(rendered.contains(
        "func (u *StartedWorkflow) RestartWorkflow(ctx workflow.Context, workflow string, taskQueue string) workflow.Future"
    ));
    assert!(rendered.contains(
        "return restartWorkflow(ctx, startWorkflowRequest{WorkflowId: u.WorkflowId, Workflow: workflow, TaskQueue: taskQueue})"
    ));
    assert!(rendered.contains("Workflow: opts.Workflow"));
    assert!(!rendered.contains("opts.Args"));
}

#[test]
fn go_resource_return_binding_handles_optional_proto_scalars() {
    let root = project_root();
    let temp_dir = unique_output_path("go-resource-return-scalars");
    fs::create_dir_all(&temp_dir).unwrap();
    let wit_path = temp_dir.join("resource-return-scalars.wit");
    fs::write(
        &wit_path,
        r#"package temporal:resource-return-scalars@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "temporal-system"
interface workflow-service {
  type placeholder = string;

  resource signal-result {
    constructor(namespace: string, started: option<bool>);
  }

  /// @nexus.proto "temporal.api.workflowservice.v1.RequestCancelWorkflowExecutionRequest"
  record cancel-request {
    /// @nexus.source go="workflowNamespace()"
    namespace: string,
    /// @nexus.omit
    workflow-execution: placeholder,
    /// @nexus.omit
    reason: placeholder,
    /// @nexus.omit
    identity: placeholder,
    /// @nexus.omit
    request-id: placeholder,
    /// @nexus.omit
    first-execution-run-id: placeholder,
    /// @nexus.omit
    links: placeholder,
  }

  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionResponse"
  type signal-result-response = own<signal-result>;

  signal-with-start: func(request: cancel-request) -> signal-result-response;
}
"#,
    )
    .unwrap();

    let rendered = generate_to_string_with_inputs(
        nexgen::language::Language::Go,
        &[wit_path],
        &[descriptor_path(&root)],
    )
    .unwrap();

    assert!(
        rendered
            .contains("value := NewSignalResult(requestProto.GetNamespace(), result.GetStarted())")
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_flatten_in_api_embeds_options_value() {
    let root = project_root();
    let rendered = generate_to_string_with_inputs(
        nexgen::language::Language::Go,
        &example_input_paths(&root, "workflow-service"),
        &[descriptor_path(&root)],
    )
    .unwrap();

    assert!(rendered.contains("type SignalWithStartWorkflowOptions struct {"));
    assert!(rendered.contains("\tUserMetadata\n}\n\n// Signal a workflow"));
    assert!(!rendered.contains("\tUserMetadata *UserMetadata\n}\n\n// Signal a workflow"));
    assert!(rendered.contains("\t\tUserMetadata: &opts.UserMetadata,\n"));
    assert!(rendered.contains("\tStaticSummary string"));
    assert!(rendered.contains("\tStaticDetails string"));
    assert!(rendered.contains("// Input signal: Signal name to send with the start request."));
    assert!(
        rendered
            .contains("// Input workflow: Workflow function identifying the workflow to start.")
    );
}

#[test]
fn go_doc_directives_render_godoc_comments() {
    let temp_dir = unique_output_path("go-doc-directives-input");
    fs::create_dir_all(&temp_dir).unwrap();
    let wit_path = temp_dir.join("doc-service.wit");
    fs::write(
        &wit_path,
        r#"package temporal:doc-demo@1.0.0;

world system {
  export doc-service;
}

/// @nexus.endpoint "doc-service"
interface doc-service {
  record greet-request {
    /// @nexus.doc "Name of the person to greet."
    name: string,
    /// @nexus.doc "Default greeting doc." go="Go-specific greeting doc."
    greeting: option<string>,
    /// @nexus.doc python="Python-only field doc."
    locale: option<string>,
    /// @nexus.doc "A very long field doc that has to be wrapped because it exceeds the generated comment line width by a comfortable margin for testing."
    salutation: option<string>,
  }

  record greet-response {
    message: string,
  }

  /// @nexus.doc
  ///   "Greets the given person."
  ///   returns="The rendered greeting."
  greet: func(request: greet-request) -> greet-response;
}
"#,
    )
    .unwrap();

    let rendered =
        generate_to_string_with_inputs(nexgen::language::Language::Go, &[wit_path], &[]).unwrap();

    // Internal wire structs have no field docs; exported request types retain
    // their public API docs.
    assert!(rendered.contains("type greetRequest struct {\n\tName string"));
    assert!(!rendered.contains("type greetRequest struct {\n\t// Required."));
    assert!(rendered.contains("\t// Required. Name of the person to greet.\n\tName string"));

    // Required fields without any doc text get a bare `// Required.` comment.
    assert!(rendered.contains("\t// Required.\n\tMessage string"));

    // The `go=` override wins over the default text on the public options
    // type; the default-only doc falls through.
    assert!(rendered.contains("\t// Optional.\n\t// Go-specific greeting doc.\n\tGreeting string"));
    assert!(!rendered.contains("Default greeting doc."));

    // Per-language docs without a default or `go=` key are omitted from Go.
    assert!(!rendered.contains("Python-only field doc."));

    // Long docs wrap across comment lines.
    assert!(rendered.contains(
        "\t// Optional.\n\t// A very long field doc that has to be wrapped because it exceeds the generated\n\t// comment line width by a comfortable margin for testing.\n\tSalutation string"
    ));

    // Operation docs render on the exported convenience wrapper, with the
    // `returns=` text in a separate paragraph.
    assert!(rendered.contains(
        "// Greets the given person.\n//\n// Returns: The rendered greeting.\nfunc Greet("
    ));

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_rejects_inputs_flattening_to_the_same_module_file() {
    // Two distinct input files -- `full_name.json` at the root and
    // `full/name.json` in a subdirectory -- flatten to the same Go module file
    // `full_name.go`. Go collapses the whole closure into one flat package, so
    // the emit layer rejects the collision (specs/json-schema/generated-file-layout.md).
    let temp_dir = unique_output_path("go-json-flatten-collision");
    fs::create_dir_all(temp_dir.join("full")).unwrap();
    let schema = |title: &str| {
        format!(
            "{{\n  \"title\": \"{title}\",\n  \"type\": \"object\",\n  \"properties\": {{ \"id\": {{ \"type\": \"string\" }} }}\n}}\n"
        )
    };
    fs::write(temp_dir.join("full_name.json"), schema("FlatThing")).unwrap();
    fs::write(
        temp_dir.join("full").join("name.json"),
        schema("NestedThing"),
    )
    .unwrap();
    let output_path = temp_dir.join("output");

    let result = generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![temp_dir.clone()],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path,
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    });

    let error = result
        .expect_err("inputs flattening to the same Go module file should be rejected")
        .to_string();
    assert!(error.contains("full_name.go"), "{error}");
    assert!(
        error.contains("conflicts with another generated file"),
        "{error}"
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_rejects_reserved_generated_name_collision() {
    // An input file whose module segment is `definitions` would collide with the
    // schema-independent runtime file `definitions.go`. The loader rejects it up
    // front as a reserved module name (before the Go emit layer would raise a
    // `GeneratedFileConflict`). Two inputs are used so the reserved file carries a
    // real module segment rather than flattening to the root `api.go`.
    let temp_dir = unique_output_path("go-json-reserved-name");
    fs::create_dir_all(&temp_dir).unwrap();
    let schema =
        "{\n  \"type\": \"object\",\n  \"properties\": { \"id\": { \"type\": \"string\" } }\n}\n";
    fs::write(temp_dir.join("definitions.json"), schema).unwrap();
    fs::write(temp_dir.join("other.json"), schema).unwrap();
    let output_path = temp_dir.join("output");

    let result = generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![temp_dir.clone()],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path,
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    });

    let error = result
        .expect_err("an input mapping to the reserved `definitions` module should be rejected")
        .to_string();
    assert!(error.contains("reserved module name"), "{error}");
    assert!(error.contains("definitions"), "{error}");
    fs::remove_dir_all(temp_dir).unwrap();
}

fn go_json_output_path(root: &Path, mode: &str, example_id: &str) -> PathBuf {
    // Definitions are the beginner-facing samples (samples/go/<pkg>); native-api
    // output is snapshot-only under the advanced project.
    match mode {
        "definitions" => root.join("samples/go").join(go_package_name(example_id)),
        _ => go_root(root)
            .join("json_schema")
            .join(mode)
            .join(go_package_name(example_id)),
    }
}

fn generate_formatted_go_json_output(
    root: &Path,
    example_id: &str,
    output_path: &Path,
    native_api: bool,
) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexgen"));
    command.args([
        "go",
        json_input_path(root, example_id).to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
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

/// An inline **structured** object `oneOf` branch on a property: the branch is
/// named `<Union>Object` and emitted as a full model (struct, `Validate`,
/// (de)serialize) that implements the union's marker method, so the union's
/// object token has a concrete type to decode into.
/// See `specs/json-schema/features/oneOf.md` ("Object branches").
#[test]
fn go_json_names_inline_object_union_branch() {
    let temp_dir = unique_output_path("go-json-inline-branch");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("detail.yaml");
    fs::write(&input_path, INLINE_OBJECT_BRANCH_SCHEMA).unwrap();
    let output_path = temp_dir.join("detail");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("detail.go")).unwrap();

    assert!(rendered.contains("Payload DetailPayload `json:\"payload,omitempty\"`"));
    assert!(rendered.contains("type DetailPayloadObject struct {"));
    assert!(rendered.contains("Text string `json:\"text\"`"));
    assert!(rendered.contains("func (DetailPayloadObject) isDetailPayload() {}"));
    // The branch decodes through its own model, so its constraints apply.
    assert!(rendered.contains("var v DetailPayloadObject"));
    assert!(rendered.contains("func (m DetailPayloadObject) Validate() error {"));
    assert!(rendered.contains("func (m *DetailPayloadObject) UnmarshalJSON(data []byte) error {"));
    fs::remove_dir_all(temp_dir).unwrap();
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
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
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
    assert!(rendered.contains("if value, ok := unmarshalEntriesValue(v, k, &errs); ok {"));
    // Element nullability stays the element's own concern.
    assert!(rendered.contains("Slots []*string `json:\"slots,omitempty\"`"));

    fs::write(
        output_path.join("bag_test.go"),
        r#"package bag

import (
	"encoding/json"
	"errors"
	"testing"
)

func TestNilUnionPositionsReturnValidationErrors(t *testing.T) {
	var segment *BagSegmentsItemString
	var entry *EntriesValueString
	value := Bag{
		Segments: []BagSegmentsItem{segment},
		Entries: &Entries{AdditionalProperties: map[string]EntriesValue{"bad": entry}},
	}
	_, err := json.Marshal(value)
	var validation *ValidationError
	if !errors.As(err, &validation) {
		t.Fatalf("expected ValidationError, got %v", err)
	}
	got := map[string]bool{}
	for _, violation := range validation.Violations {
		got[violation.Path] = true
	}
	for _, path := range []string{"segments[0]", "entries.bad"} {
		if !got[path] {
			t.Errorf("missing violation path %q in %#v", path, validation.Violations)
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
        .env("GO111MODULE", "off")
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
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
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
    assert!(rendered.contains("decodeBase64(k, s, blobMapValueContentEncoding, &errs)"));
    assert!(rendered.contains("mergeNested(&errs, p0, v0.Validate())"));

    fs::write(
        output_path.join("recursive_test.go"),
        r#"package recursive

import (
	"encoding/json"
	"errors"
	"math"
	"testing"
	"time"
)

func paths(err error) map[string]bool {
	var validation *ValidationError
	if !errors.As(err, &validation) {
		return nil
	}
	out := map[string]bool{}
	for _, violation := range validation.Violations {
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
	var validation *ValidationError
	if !errors.As(err, &validation) {
		t.Fatalf("expected ValidationError, got %v", err)
	}
	if len(validation.Violations) != 4 {
		t.Fatalf("expected exactly four validation violations, got %#v", validation.Violations)
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
        .env("GO111MODULE", "off")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Every constraint a **non-object** branch declares is carried into the
/// wrapper's `Validate`, which the dispatcher calls on the way in and
/// `MarshalJSON` re-runs before emit (P12). A `pattern` on a branch compiles to a
/// package-level regex var keyed by the wrapper type, and a closed value set is
/// checked against the wire literals (the wrapper has no field to hang value
/// constants off). See `specs/json-schema/features/oneOf.md` ("Validator mapping").
#[test]
fn go_json_validates_non_object_union_branch_constraints() {
    let temp_dir = unique_output_path("go-json-branch-constraints");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("bc.yaml");
    fs::write(&input_path, BRANCH_CONSTRAINT_SCHEMA).unwrap();
    let output_path = temp_dir.join("bc");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("bc.go")).unwrap();

    // The string branch: length, then the pattern through its own compiled var.
    assert!(rendered.contains("var bcValueStringPattern = regexp.MustCompile(\"^[a-z]+$\")"));
    assert!(rendered.contains("must have length >= 3, got %d"));
    assert!(rendered.contains("if !bcValueStringPattern.MatchString(string(v)) {"));
    // The integer branch's own bound.
    assert!(rendered.contains("if int64(v) < 1 {"));
    assert!(rendered.contains("must be >= 1, got %v"));
    // The array branch's bounds, over the wrapper's underlying slice.
    assert!(rendered.contains("if n := len([]float64(v)); n < 1 {"));
    assert!(rendered.contains("duplicate items: element at index %d equals index %d"));
    // A closed value set on a branch is a membership check over the wire values.
    assert!(rendered.contains("switch string(v) {"));
    assert!(rendered.contains("case \"auto\", \"manual\":"));
    // The dispatcher runs the selected branch's `Validate`, and so does the
    // model's own `Validate` (which `MarshalJSON` calls before emit).
    assert!(rendered.contains("v := BcValueString(s)\n\t\tmergeNested(errs, path, v.Validate())"));
    assert!(rendered.contains("mergeNested(&errs, \"value\", m.Value.Validate())"));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A description that ends a sentence with a package-like word must not pull in
/// an import: an unused import is a Go compile error.
#[test]
fn go_json_ignores_package_names_in_doc_comments() {
    let temp_dir = unique_output_path("go-json-comment-import");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("note.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  label:
    description: Processed one at a time.
    type: string
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("note");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("note.go")).unwrap();

    assert!(rendered.contains("one at a time."));
    assert!(!rendered.contains("\t\"time\"\n"));
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
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("output.go")).unwrap();

    assert!(rendered.contains("utf8.RuneCountInString(e) >= 5"));
    assert!(rendered.contains("regexp.MustCompile(\"^api\\\\.\").MatchString(e)"));

    let format_status = Command::new("gofmt")
        .args(["-w", output_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(format_status.success());
    let test_status = Command::new("go")
        .args(["test", "./..."])
        .env("GO111MODULE", "off")
        .current_dir(&output_path)
        .status()
        .unwrap();
    assert!(test_status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

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
x-go-name: RenamedPage
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

/// An `x-go-name` override on a model in *another* input file moves every
/// reference the consuming module emits. Go collapses the whole closure into one
/// flat package, so there is no import to fix — but the operation generic and the
/// cross-module `$ref` field still name the type, and the override is declared in
/// the referenced file, so only the tree-wide name manifest can resolve it
/// (P14/P15).
#[test]
fn go_json_cross_module_go_name_override_moves_every_reference() {
    let temp_dir = unique_output_path("go-json-cross-module-override");
    let input_dir = write_cross_module_closure(&temp_dir);
    let output_path = temp_dir.join("output");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![input_dir],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let declaring = fs::read_to_string(output_path.join("content_page.go")).unwrap();
    assert!(declaring.contains("type RenamedPage struct {"));

    let consuming = fs::read_to_string(output_path.join("kb.go")).unwrap();
    for expected in [
        "Get nexus.OperationReference[GetInput, RenamedPage]",
        "Get: nexus.NewOperationReference[GetInput, RenamedPage](\"Get\")",
        "Page *RenamedPage `json:\"page,omitempty\"`",
        "var tmp RenamedPage",
    ] {
        assert!(consuming.contains(expected), "{expected}\n{consuming}");
    }
    // Nothing names the pre-override identifier.
    for stale in ["[GetInput, Page]", "*Page ", "var tmp Page\n"] {
        assert!(!consuming.contains(stale), "{stale}\n{consuming}");
    }
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A property carrying a per-language name override alongside a `const` and an
/// inline object: the closed-value type is member-derived and moves with the
/// override, while the hoisted shape is position-derived and does not.
const MEMBER_DERIVED_NAME_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  retryCount:
    type: integer
    default: 3
    x-go-name: Attempts
  kind:
    type: string
    const: widget
    x-go-name: Category
  address:
    type: object
    x-go-name: Location
    properties:
      street: { type: string }
"#;

/// A name synthesized from a member follows that member's `x-go-name` — the
/// `<Field>OrDefault()` accessor and the `<Type><Member>` closed-value type (plus
/// its value constants). A shape named after its *position* does not move.
/// See `specs/json-schema/PRINCIPLES.md` §15, `specs/json-schema/features/const.md`.
#[test]
fn go_json_override_moves_member_derived_names_only() {
    let temp_dir = unique_output_path("go-json-member-derived-names");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("probe.yaml");
    fs::write(&input_path, MEMBER_DERIVED_NAME_SCHEMA).unwrap();
    let output_path = temp_dir.join("probe");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = read_go_output_files(&output_path)
        .into_values()
        .collect::<Vec<_>>()
        .join("\n");

    // Member-derived: accessor, closed-value type, and its value constant.
    assert!(rendered.contains("func (m Probe) AttemptsOrDefault() int64 {"));
    assert!(rendered.contains("type ProbeCategory string"));
    assert!(rendered.contains("const ProbeCategoryWidget ProbeCategory = \"widget\""));
    assert!(!rendered.contains("ProbeKind"));
    // Position-derived: the hoisted shape keeps the position's name.
    assert!(rendered.contains("Location *ProbeAddress `json:\"address,omitempty\"`"));
    assert!(rendered.contains("type ProbeAddress struct {"));
    assert!(!rendered.contains("ProbeLocation"));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Go flattens every module into one package, so its collision scope is the whole
/// input closure, not the module. Two modules declaring the same type name are a
/// redeclaration in that single package — uncompilable Go, previously emitted
/// without a diagnostic because the pass scoped collisions per module.
/// See `specs/json-schema/PRINCIPLES.md` §15 and
/// `specs/json-schema/generated-file-layout.md`.
#[test]
fn go_json_rejects_same_type_name_in_two_modules() {
    let temp_dir = unique_output_path("go-json-flat-package-collision");
    let input_dir = temp_dir.join("input");
    fs::create_dir_all(input_dir.join("a")).unwrap();
    fs::create_dir_all(input_dir.join("b")).unwrap();
    let page = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"title":{"type":"string"}}}"#;
    fs::write(input_dir.join("a/page.json"), page).unwrap();
    fs::write(input_dir.join("b/page.json"), page).unwrap();
    fs::write(
        input_dir.join("root.nexusrpc.yaml"),
        r##"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Svc:
    fqn: example.v1.Svc
    operations:
      one:
        input: { $ref: "a/page.json" }
        output:
          type: object
          additionalProperties: false
          properties: { ok: { type: boolean } }
          required: [ok]
      two:
        input: { $ref: "b/page.json" }
"##,
    )
    .unwrap();

    let error = generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![input_dir.clone()],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join("out"),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .expect_err("two modules declaring `Page` collide in Go's flat package")
    .to_string();
    // The diagnostic names both modules — the bare type name appears twice and
    // would otherwise read as one declaration seen twice.
    assert!(error.contains("collision"), "{error}");
    assert!(error.contains("a/page#Page"), "{error}");
    assert!(error.contains("b/page#Page"), "{error}");

    // The same closure is fine in a language whose modules are separate scopes.
    // Java gives each module its own sub-package (`…pkg.a.page`, `…pkg.b.page`)
    // and emits no aggregating barrel, so the two `Page` classes stay distinct.
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Java,
        input_paths: vec![input_dir],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join("pkg"),
        format: false,
        generate_native_api: false,
        java_package_name: Some("com.example.pkg".to_string()),
        ts_date_time_types: Default::default(),
    })
    .expect("separate Java packages keep `Page` apart");
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A service enters the collision pass in the module that declares it. Keying the
/// insert on the root module meant that in multi-input mode services never entered
/// the pass at all, so this clash — which rejects when the same file is the only
/// input — was silently accepted.
#[test]
fn go_json_rejects_service_clashing_with_a_model_in_multi_input() {
    let temp_dir = unique_output_path("go-json-multi-input-service-clash");
    let input_dir = temp_dir.join("input");
    fs::create_dir_all(input_dir.join("sub")).unwrap();
    fs::write(
        input_dir.join("sub/api.nexusrpc.yaml"),
        r##"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Thing:
    fqn: example.v1.Thing
    operations:
      doIt:
        input: { $ref: "#/$defs/Thing" }
$defs:
  Thing:
    type: object
    properties:
      id: { type: string }
"##,
    )
    .unwrap();
    fs::write(
        input_dir.join("root.nexusrpc.yaml"),
        r##"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
$defs:
  Root:
    type: object
    properties:
      x: { type: string }
"##,
    )
    .unwrap();

    let error = generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![input_dir],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join("out"),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .expect_err("a service and a model in one module both map to `Thing`")
    .to_string();
    assert!(
        error.contains("collision") && error.contains("Thing"),
        "{error}"
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Go flattens every module into one package, so re-emitting a `$ref`d type into
/// the referencing service's module put two `type Page struct` in that package —
/// `Page redeclared in this block`, confirmed with the Go compiler. It happened
/// whenever the service module declared no types of its own, because reachability
/// pruning read "this module owns nothing" as "this front end does not scope by
/// module" and kept every referenced declaration.
#[test]
fn go_json_service_module_without_own_types_does_not_redeclare_refs() {
    let temp_dir = unique_output_path("go-json-service-only-module");
    let input_dir = temp_dir.join("input");
    fs::create_dir_all(input_dir.join("a")).unwrap();
    fs::write(
        input_dir.join("a/page.json"),
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"properties":{"title":{"type":"string"}},"required":["title"]}"#,
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
"#,
    )
    .unwrap();

    let output_path = temp_dir.join("out");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![input_dir],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let rendered = read_go_output_files(&output_path)
        .into_values()
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        rendered.matches("type Page struct {").count(),
        1,
        "`Page` must be declared once in the flat package\n{rendered}"
    );
    fs::remove_dir_all(temp_dir).unwrap();
}
