// The `dotnet` generate target lives behind the `advanced` feature.
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

const WORKFLOW_SERVICE_EXAMPLE_ID: &str = "workflow-service";

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

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

fn read_dotnet_output_files(dir: &Path) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, files);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("cs") {
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

fn render_output_files(files: BTreeMap<PathBuf, String>) -> String {
    files
        .into_iter()
        .map(|(path, contents)| format!("### {}\n{contents}", path.display()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn generate_dotnet_to_string(input_paths: &[PathBuf], descriptor_paths: &[PathBuf]) -> String {
    render_output_files(generate_dotnet_files(input_paths, descriptor_paths))
}

fn generate_dotnet_files(
    input_paths: &[PathBuf],
    descriptor_paths: &[PathBuf],
) -> BTreeMap<PathBuf, String> {
    let temp_dir = unique_output_path("dotnet-rendered");
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            mode: nexgen::generator::GenerationMode::NativeApi,
            ..Default::default()
        },
        language: nexgen::language::Language::Dotnet,
        input_paths: input_paths.to_vec(),
        descriptor_paths: descriptor_paths.to_vec(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        support_package: Some("Nexgen.Support".to_string()),
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let files = if output_path.is_file() {
        BTreeMap::from([(
            PathBuf::from("output.cs"),
            fs::read_to_string(&output_path).unwrap(),
        )])
    } else {
        read_dotnet_output_files(&output_path)
    };
    fs::remove_dir_all(temp_dir).unwrap();
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
fn cli_uses_dotnet_support_package_without_copying_files() {
    let root = project_root();
    let temp_dir = unique_output_path("dotnet-support-package");
    fs::create_dir_all(&temp_dir).unwrap();
    let output_path = temp_dir.join("output");

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "dotnet",
            wit_input_path(&root, "user-service").to_str().unwrap(),
            "--support-package",
            "Custom",
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
    assert!(!output_path.join("Support/CustomSupport.cs").exists());
    assert!(output_path.join("Models.cs").exists());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn dotnet_qualifies_helpers_from_supplied_support_namespace() {
    let root = project_root();
    let output = unique_output_path("dotnet-custom-support-namespace");
    let status = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .arg("dotnet")
        .args(example_input_paths(&root, WORKFLOW_SERVICE_EXAMPLE_ID))
        .args([
            "--descriptors",
            descriptor_path(&root).to_str().unwrap(),
            "--support-package",
            "Custom.Support",
            "--output",
            output.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let files = read_dotnet_output_files(&output);
    let models = files.get(&PathBuf::from("Models.cs")).unwrap();
    assert!(models.contains("using Custom.Support;"));
    assert!(models.contains("Custom.Support.ProtoExtensions."));
    assert!(!files.keys().any(|path| path.starts_with("Support")));
    fs::remove_dir_all(output).unwrap();
}

#[test]
fn dotnet_uses_annotated_namespace_and_operations_class() {
    let temp_dir = unique_output_path("dotnet-annotated-namespace");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("main.wit");
    fs::write(
        &input_path,
        r#"
package example:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "temporal-system"
/// @nexus.namespace dotnet="Temporalio.Workflows"
/// @nexus.operations-class dotnet="Workflow"
interface workflow-service {
  record signal-request {
    id: string,
  }

  record signal-response {
    run-id: option<string>,
  }

  /// @nexus.operation name="SignalWithStartWorkflowExecution"
  signal-with-start-workflow: func(request: signal-request) -> signal-response;
}
"#,
    )
    .unwrap();

    let input_paths = vec![input_path];
    let rendered = generate_dotnet_to_string(&input_paths, &[]);

    assert!(rendered.contains("namespace Temporalio.Workflows\n{"));
    assert!(rendered.contains("public static partial class Workflow"));
    assert!(!rendered.contains("WorkflowServiceOperations"));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn dotnet_imports_converters_only_where_unqualified_types_need_them() {
    let root = project_root();
    for example in ["notification-service", "proto-oneof", "workflow-service"] {
        let files = generate_dotnet_files(
            &example_input_paths(&root, example),
            &[descriptor_path(&root)],
        );
        let models = &files[&PathBuf::from("Models.cs")];
        assert!(models.contains("Temporalio.Converters.TemporalTransferTypeConverter"));
        assert!(!models.contains("using Temporalio.Converters;"));
        if let Some(services) = files.get(&PathBuf::from("Services.cs")) {
            assert!(services.contains("ISerializationContext"));
            assert!(services.contains("using Temporalio.Converters;"));
        }
    }
}

#[test]
fn dotnet_model_only_interfaces_use_their_namespace_directive() {
    let temp_dir = unique_output_path("dotnet-model-only-namespace");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("models.wit");
    fs::write(
        &input_path,
        r#"package test:models@1.0.0;

world system {
  export notification-models;
}

/// @nexus.namespace dotnet="Acme.Notifications"
interface notification-models {
  record notification {
    message: string,
  }
}
"#,
    )
    .unwrap();

    let files = generate_dotnet_files(&[input_path], &[]);
    fs::remove_dir_all(temp_dir).unwrap();
    let models = files
        .get(&PathBuf::from("Models.cs"))
        .expect("model-only output should include Models.cs");

    assert!(models.contains("namespace Acme.Notifications\n{"));
    assert!(models.contains("public record Notification"));
}
