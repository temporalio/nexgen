// The `dotnet` generate target lives behind the `advanced` feature.
#![cfg(feature = "advanced")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

mod common;
use common::wit_input_path;

const PROTO_BACKED_EXAMPLE_ID: &str = "proto-backed-showcase";

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

fn unique_output_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("nexgen-{label}-{unique}-{counter}"))
}

#[test]
fn cli_accepts_unused_dotnet_support_package() {
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
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn cli_uses_dotnet_support_namespace() {
    let root = project_root();
    let output = unique_output_path("dotnet-custom-support-namespace");
    let status = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .arg("dotnet")
        .args(example_input_paths(&root, PROTO_BACKED_EXAMPLE_ID))
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
    let models = fs::read_to_string(output.join("Models.cs")).unwrap();
    assert!(models.contains("Custom.Support.ProtoExtensions."));
    fs::remove_dir_all(output).unwrap();
}
