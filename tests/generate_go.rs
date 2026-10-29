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
use common::{json_input_path, wit_input_path};

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

fn generate_to_string_with_inputs(
    language: nexgen::language::Language,
    input_paths: &[PathBuf],
    descriptor_paths: &[PathBuf],
) -> Result<String, Box<dyn std::error::Error>> {
    let temp_dir = unique_output_path("go-rendered");
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            system_nexus: false,
            targets: nexgen::nexgen_config::GenerationTargets::WORKFLOW,
        },
        language,
        input_paths: input_paths.to_vec(),
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: descriptor_paths.to_vec(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
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
fn cli_accepts_unused_go_support_package() {
    let root = project_root();
    let temp_dir = unique_output_path("go-support-file-input");
    fs::create_dir_all(&temp_dir).unwrap();
    let output_path = temp_dir.join("output");

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "go",
            wit_input_path(&root, "user-service").to_str().unwrap(),
            "--support-package",
            "example.com/nexgen/support",
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
    let input = fs::read_to_string(wit_input_path(&root, "user-service"))
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
            "--targets=workflow",
            "--output",
            output_path.to_str().unwrap(),
            "--support-package",
            "example.com/nexgen/support",
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
    let input = fs::read_to_string(wit_input_path(&root, "user-service"))
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
            "--support-package",
            "example.com/nexgen/support",
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
            wit_input_path(&root, "user-service").to_str().unwrap(),
            "--output",
            "/",
            "--support-package",
            "example.com/nexgen/support",
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
            wit_input_path(&root, "user-service").to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
            "--support-package",
            "example.com/nexgen/support",
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
            wit_input_path(&root, "user-service").to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
            "--support-package",
            "example.com/nexgen/support",
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
fn go_json_package_name_derives_from_output_directory_name() {
    // The Go package name for a JSON-schema input must come from the output
    // directory's basename (Go convention: package name = directory name),
    // not from an unrelated signal like the input's service name.
    let root = project_root();
    let temp_dir = unique_output_path("go-json-package-name-fallback");
    let output_path = temp_dir.join("widgets");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![json_input_path(&root, "chat")],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();

    let generated = fs::read_to_string(output_path.join("widgets.go")).unwrap();
    assert!(generated.starts_with(concat!(
        "// Code generated by nexgen v",
        env!("CARGO_PKG_VERSION"),
        ". DO NOT EDIT.\n\npackage widgets\n"
    )));
    fs::remove_dir_all(temp_dir).unwrap();
}

// Keep these fixtures independent of the Temporal sample directives: changing
// a wire field's spelling must not change Go's public callable policy.
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

#[test]
fn go_rejects_descriptors_on_fixed_argument_functions_with_go_diagnostic() {
    let wit = go_descriptor_fixture(
        "  ///   go-value-type=\"string\"\n  ///   go-args-type=\"any\"",
        "signal-input",
    )
    .replace(
        "  /// @nexus.function-args varargs=true\n  signal-call: func(signal-args: payloads) -> string;",
        "  signal-call: func(signal-arg: string) -> string;",
    );
    let error = render_go_descriptor_fixture(&wit).unwrap_err();
    let error = error
        .downcast_ref::<nexgen::error::Error>()
        .expect("descriptor rejection should preserve the generator error type");
    match error {
        nexgen::error::Error::UnsupportedGoType { context, reason } => {
            assert!(context.to_lowercase().contains("signal"), "{error}");
            assert!(reason.contains("fixed"), "{error}");
            assert!(reason.contains("descriptor"), "{error}");
        }
        other => panic!("expected UnsupportedGoType, got {other:?}"),
    }
}

fn render_go_ordinary_serialization_context_fixture(
    declarations: &str,
    operations: &str,
) -> Result<BTreeMap<PathBuf, String>, Box<dyn std::error::Error>> {
    let temp_dir = unique_output_path("go-serialization-context-conversions");
    fs::create_dir_all(&temp_dir).unwrap();
    let wit_path = temp_dir.join("conversions.wit");
    fs::write(
        &wit_path,
        format!(
            r#"package temporal:context-conversions@1.0.0;
world system {{ export sample-service; }}
/// @nexus.endpoint "sample-service"
interface sample-service {{
  {declarations}
  {operations}
}}
"#
        ),
    )
    .unwrap();
    let output_path = temp_dir.join("output");
    let result = generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            system_nexus: false,
            targets: nexgen::nexgen_config::GenerationTargets::WORKFLOW,
        },
        language: nexgen::language::Language::Go,
        input_paths: vec![wit_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: vec![descriptor_path(&project_root())],
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    });
    let files = result.map(|_| read_go_output_files(&output_path));
    fs::remove_dir_all(temp_dir).unwrap();
    Ok(files?)
}

fn assert_no_generated_context_plumbing(files: &BTreeMap<PathBuf, String>) {
    assert!(!files.contains_key(Path::new("serialization_context.go")));
    assert!(!files.contains_key(Path::new("registry.go")));
    let source = &files[Path::new("sampleservice.go")];
    for forbidden in [
        "nexgenWithOperationSerializationContext",
        "nexgenPayloadContext",
        "nexgenSerializationContextKey",
        "WithDataConverter",
        "WithValue",
        "WithRootDataConverterSerializationContext",
        ".SerializationContext(request)",
    ] {
        assert!(!source.contains(forbidden), "sampleservice.go: {forbidden}");
    }
}

const GO_CONTEXT_OVERRIDE_TYPES: &str = r#"
  type placeholder = string;
  /// @nexus.proto "temporal.api.common.v1.RetryPolicy"
  /// @nexus.type go="go.temporal.io/sdk/temporal.RetryPolicy"
  type retry-policy = placeholder;
"#;

// TODO: Remove this test once full Go signal-with-start support is available.
#[test]
fn go_serialization_context_ordinary_endpoint_converts_external_input_in_wrapper() {
    let files = render_go_ordinary_serialization_context_fixture(
        GO_CONTEXT_OVERRIDE_TYPES,
        r#"/// @nexus.serialization-context go="requestContext"
        roundtrip: func(request: retry-policy) -> retry-policy;"#,
    )
    .unwrap();
    let operation = &files[Path::new("sampleservice.go")];
    let main = operation;
    let conversion = operation
        .find("requestProto, err := support.RetryPolicyToProto(&request)")
        .unwrap();
    let error = operation[conversion..]
        .find("resultSettable.SetError(err)")
        .unwrap()
        + conversion;
    let execute = operation
        .find("c.ExecuteOperation(ctx, \"Roundtrip\", requestProto,")
        .unwrap();
    assert!(conversion < error && error < execute, "{operation}");
    assert!(operation.contains("workflow.NewNexusClient(\"sample-service\", \"SampleService\")"));
    assert_eq!(operation.matches("support.RetryPolicyToProto(").count(), 1);
    assert!(operation.contains("value, err := support.RetryPolicyFromProto(&result)"));
    assert!(!operation.contains("requestContext("));
    assert_eq!(main.matches("\"go.temporal.io/sdk/temporal\"").count(), 1);
    assert!(!main.contains("// --- Registry ---"));
    assert!(!main.contains("\"reflect\""));
    assert!(!main.contains("\"go.temporal.io/sdk/converter\""));
    assert!(!main.contains("go.temporal.io/sdk/internal"));
    assert_no_generated_context_plumbing(&files);
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
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![temp_dir.clone()],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path,
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    });

    let error =
        result.expect_err("inputs flattening to the same Go module file should be rejected");
    let nexgen::error::Error::GeneratedFileSourceConflict {
        path,
        first_source,
        second_source,
        remedy,
    } = error
    else {
        panic!("expected generated-file source conflict, got {error}");
    };
    assert_eq!(path, PathBuf::from("full_name.go"));
    assert!(
        first_source.contains("full/name.json") || first_source.contains("full_name.json"),
        "{first_source}"
    );
    assert!(
        second_source.contains("full/name.json") || second_source.contains("full_name.json"),
        "{second_source}"
    );
    assert_ne!(first_source, second_source);
    assert!(remedy.contains("full/name.json"), "{remedy}");
    assert!(remedy.contains("full_name.json"), "{remedy}");
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_rejects_reserved_generated_name_collision() {
    // An input file whose module segment is `definitions` would collide with the
    // schema-independent runtime file `definitions.go`. The loader rejects it up
    // front as a reserved module name (before the Go emit layer would raise a
    // generated-file source conflict). Two inputs are used so the reserved file
    // carries a real module segment rather than flattening to the root `api.go`.
    let temp_dir = unique_output_path("go-json-reserved-name");
    fs::create_dir_all(&temp_dir).unwrap();
    let schema =
        "{\n  \"type\": \"object\",\n  \"properties\": { \"id\": { \"type\": \"string\" } }\n}\n";
    fs::write(temp_dir.join("definitions.json"), schema).unwrap();
    fs::write(temp_dir.join("other.json"), schema).unwrap();
    let output_path = temp_dir.join("output");

    let result = generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![temp_dir.clone()],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path,
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    });

    let error = result
        .expect_err("an input mapping to the reserved `definitions` module should be rejected")
        .to_string();
    assert!(error.contains("reserved module name"), "{error}");
    assert!(error.contains("definitions"), "{error}");
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_rejects_single_input_output_named_definitions_with_a_remedy() {
    let temp_dir = unique_output_path("go-json-output-name-collision");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("other.json");
    fs::write(
        &input_path,
        r#"{"type":"object","properties":{"id":{"type":"string"}}}"#,
    )
    .unwrap();

    let request = |output_name: &str| GenerateRequest {
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path.clone()],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join(output_name),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    };

    for output_name in ["definitions", "Definitions", "definitions-"] {
        let error = generate_to_file(&request(output_name))
            .expect_err("the model file must not overwrite the generated runtime")
            .to_string();
        assert!(error.contains("definitions.go"), "{output_name}: {error}");
        assert!(error.contains("output directory"), "{output_name}: {error}");
        assert!(
            error.contains("generated Go validation runtime"),
            "{output_name}: {error}"
        );
        assert!(error.contains("--output"), "{output_name}: {error}");
        assert!(
            error.contains("name generates a different Go file path"),
            "{output_name}: {error}"
        );
    }

    generate_to_file(&request("api")).expect("a non-conflicting derived package name should load");
    assert!(temp_dir.join("api/api.go").is_file());
    assert!(temp_dir.join("api/definitions.go").is_file());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_multi_input_module_named_support_has_no_support_file_collision() {
    let temp_dir = unique_output_path("go-json-module-support-collision");
    fs::create_dir_all(&temp_dir).unwrap();
    fs::write(
        temp_dir.join("support.json"),
        r#"{"title":"SupportModel","type":"object","properties":{"id":{"type":"string"}}}"#,
    )
    .unwrap();
    fs::write(
        temp_dir.join("other.json"),
        r#"{"title":"OtherModel","type":"object","properties":{"id":{"type":"string"}}}"#,
    )
    .unwrap();
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![temp_dir.clone()],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join("output"),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .expect("support.json may now generate support.go without a copied support file");
    assert!(temp_dir.join("output/support.go").is_file());
    assert!(temp_dir.join("output/other.go").is_file());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_json_output_named_support_has_no_support_file_collision() {
    let temp_dir = unique_output_path("go-json-output-support-collision");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("model.json");
    fs::write(
        &input_path,
        r#"{"title":"Model","type":"object","properties":{"id":{"type":"string"}}}"#,
    )
    .unwrap();
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join("support"),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .expect("support output name is no longer reserved for copied fragments");
    assert!(temp_dir.join("support/support.go").is_file());
    assert!(temp_dir.join("support/definitions.go").is_file());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_wit_service_named_support_has_no_support_file_collision() {
    let temp_dir = unique_output_path("go-wit-service-support-collision");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("support.wit");
    fs::write(
        &input_path,
        r#"
package temporal:collision@1.0.0;

world system {
  export support;
}

/// @nexus.endpoint "support"
interface support {
  record request { value: string }
  run: func(request: request);
}
"#,
    )
    .unwrap();
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path.clone()],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join("output"),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .expect("the service may now own support.go");
    assert!(temp_dir.join("output/support.go").is_file());
    fs::remove_dir_all(temp_dir).unwrap();
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
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
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
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("bc.go")).unwrap();
    let definitions = fs::read_to_string(output_path.join("definitions.go")).unwrap();

    // The string branch: length, then the pattern through its own compiled var.
    assert!(definitions.contains(
        "var _nexgenJsonSchemaPattern5e5b612d7a5d2b24 = regexp.MustCompile(\"^[a-z]+$\")"
    ));
    assert!(rendered.contains("must have length >= 3, got %d"));
    assert!(
        rendered.contains("if !_nexgenJsonSchemaPattern5e5b612d7a5d2b24.MatchString(string(v)) {")
    );
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
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("note.go")).unwrap();

    assert!(rendered.contains("one at a time."));
    assert!(!rendered.contains("\t\"time\"\n"));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_json_services_render_deprecation_and_one_sided_io() {
    let temp_dir = unique_output_path("go-json-deprecated-services");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("service.nexusrpc.yaml");
    fs::write(
        &input_path,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  DeprecatedService:
    fqn: example.v1.DeprecatedService
    deprecated: true
    operations:
      submit:
        deprecated: true
        input:
          type: object
          additionalProperties: false
          required: [value]
          properties: { value: { type: string } }
      status:
        output:
          type: object
          additionalProperties: false
          required: [ok]
          properties: { ok: { type: boolean } }
"#,
    )
    .unwrap();
    let output_path = temp_dir.join("service");
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
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("service.go")).unwrap();
    assert!(rendered.contains("// Deprecated: This service is deprecated.\nvar DeprecatedService"));
    assert!(rendered.contains("// Deprecated: This operation is deprecated.\n\tSubmit"));
    assert!(rendered.contains("nexus.OperationReference[SubmitInput, nexus.NoValue]"));
    assert!(rendered.contains("nexus.OperationReference[nexus.NoValue, StatusOutput]"));
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
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_dir],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
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
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_path],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
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
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_dir.clone()],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join("out"),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
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
        config: Default::default(),
        language: nexgen::language::Language::Java,
        input_paths: vec![input_dir],
        support_package: None,
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join("pkg"),
        format: false,
        java_package_name: Some("com.example.pkg".to_string()),
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .expect("separate Java packages keep `Page` apart");
    let service = fs::read_to_string(temp_dir.join("pkg/root/Svc.java")).unwrap();
    assert!(
        service.contains("one(com.example.pkg.a.page.Page input);")
            && service.contains("two(com.example.pkg.b.page.Page input);"),
        "ambiguous foreign model names must be package-qualified:\n{service}"
    );
    assert!(!service.contains("import com.example.pkg.a.page.Page;"));
    assert!(!service.contains("import com.example.pkg.b.page.Page;"));
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
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_dir],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join("out"),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
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
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_dir],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
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
    // Owning no models of its own must not turn the module's service into the
    // WIT operation-function surface: a JSON service is a `var <Service>` of
    // `nexus.OperationReference`s and never depends on the Temporal SDK.
    assert!(rendered.contains("var Svc = struct {"), "{rendered}");
    assert!(
        rendered.contains("nexus.NewOperationReference[Page, nexus.NoValue](\"One\")"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("go.temporal.io/sdk/workflow"),
        "definitions-only output must not depend on the Temporal SDK\n{rendered}"
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A cross-file `$ref` union branch must resolve to its target's schema. Go
/// flattens the generate closure into one package, but reachability pruning
/// removes another file's declarations from this module's plan, so the branch
/// used to classify as nothing at all and the union collapsed to
/// `type Shape struct{}`. A cross-module `$ref` *to* a named union must resolve
/// to the sealed interface, not to `*Shape`.
#[test]
fn go_json_cross_file_union_branches_resolve() {
    let temp_dir = unique_output_path("go-json-cross-file-union");
    let input_dir = temp_dir.join("input");
    fs::create_dir_all(&input_dir).unwrap();
    fs::write(
        input_dir.join("shapes.nexusrpc.yaml"),
        r#"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
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
      side: { type: number }
"#,
    )
    .unwrap();
    fs::write(
        input_dir.join("holder.nexusrpc.yaml"),
        r#"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
$defs:
  Shape:
    oneOf:
      - { $ref: "shapes.nexusrpc.yaml#/$defs/Circle" }
      - { $ref: "shapes.nexusrpc.yaml#/$defs/Square" }
"#,
    )
    .unwrap();
    fs::write(
        input_dir.join("drawing.nexusrpc.yaml"),
        r#"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
$defs:
  Drawing:
    type: object
    properties:
      main: { $ref: "holder.nexusrpc.yaml#/$defs/Shape" }
"#,
    )
    .unwrap();

    let output_path = temp_dir.join("out");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Go,
        input_paths: vec![input_dir],
        support_package: Some("example.com/nexgen/support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let rendered = read_go_output_files(&output_path)
        .into_values()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        rendered.contains("// Shape is one of: Circle, Square."),
        "{rendered}"
    );
    assert!(
        !rendered.contains("type Shape struct {"),
        "a named cross-file union must not collapse to an empty struct\n{rendered}"
    );
    assert_eq!(
        rendered.matches("type Shape interface {").count(),
        1,
        "the union interface is declared once, by the file that owns it\n{rendered}"
    );
    // The referencing module gets the interface, not a pointer to it.
    assert!(
        rendered.contains("Main Shape `json:\"main,omitempty\"`"),
        "{rendered}"
    );
    assert!(!rendered.contains("*Shape"), "{rendered}");
    // The discriminator is matched on the JSON value, not on the wire lexeme.
    assert!(
        rendered.contains("case jsonScalarEquals(discRaw, \"\\\"circle\\\"\"):"),
        "{rendered}"
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn go_json_semantic_helper_names_participate_in_p15() {
    let temp_dir = unique_output_path("go-json-semantic-helper-p15");
    fs::create_dir_all(&temp_dir).unwrap();
    for (case, helper, constraint) in [
        ("format", "_nexgenJsonSchemaEmailFormat", "format: email"),
        (
            "encoding",
            "_nexgenJsonSchemaBase64ContentEncoding",
            "contentEncoding: base64",
        ),
        (
            "pattern",
            "_nexgenJsonSchemaPattern5e5b612d7a5d2b24",
            "pattern: '^[a-z]+$'",
        ),
    ] {
        let input_path = temp_dir.join(format!("{case}.yaml"));
        fs::write(
            &input_path,
            format!(
                "$schema: https://json-schema.org/draft/2020-12/schema\ntitle: Collision\nx-go-name: {helper}\ntype: object\nproperties:\n  value: {{ type: string, {constraint} }}\n"
            ),
        )
        .unwrap();
        let error = generate_to_file(&GenerateRequest {
            language: nexgen::language::Language::Go,
            input_paths: vec![input_path],
            support_package: Some("example.com/nexgen/support".into()),
            descriptor_paths: Vec::new(),
            output_path: temp_dir.join(format!("out-{case}")),
            format: false,
            config: Default::default(),
            java_package_name: None,
            ts_date_time_types: Default::default(),
            dotnet_namespace: None,
        })
        .expect_err("the authored type collides with a package semantic helper")
        .to_string();
        assert!(error.contains(helper), "{case}: {error}");
        assert!(error.contains("x-go-name"), "{case}: {error}");
    }
    fs::remove_dir_all(temp_dir).unwrap();
}
