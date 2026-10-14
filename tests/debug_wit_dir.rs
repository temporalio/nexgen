// The `debug-wit-dir` CLI surface lives behind the `advanced` feature.
#![cfg(feature = "advanced")]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn sample_input_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/inputs/workflow-service.wit")
}

fn linked_inputs_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/inputs/deps")
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir().join(format!("nexgen-{name}-{unique}"))
}

#[test]
fn cli_debug_wit_dir_writes_prepared_workspace() {
    let root = project_root();
    let output_dir = unique_temp_dir("debug-wit-dir");

    let status = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "debug-wit-dir",
            "--input",
            sample_input_path(&root).to_str().unwrap(),
            "--input",
            linked_inputs_path(&root).to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
        ])
        .status()
        .unwrap();

    assert!(status.success());
    assert!(output_dir.join("workflow-service.wit").is_file());
    assert!(
        output_dir
            .join("deps/nexus-temporal-types/model.wit")
            .is_file()
    );
    assert!(
        !output_dir
            .join("deps/nexus-temporal-types/python/temporal_model_converters.py")
            .is_file()
    );
    assert!(
        !output_dir
            .join("deps/nexus-temporal-types/typescript/temporal_model_converters.ts")
            .is_file()
    );

    let input = fs::read_to_string(output_dir.join("workflow-service.wit")).unwrap();
    assert!(input.contains("package temporal:nexus@1.0.0;"));

    let linked_type =
        fs::read_to_string(output_dir.join("deps/nexus-temporal-types/model.wit")).unwrap();
    assert!(linked_type.contains("package nexus:temporal-types@1.0.0;"));

    let _ = fs::remove_dir_all(output_dir);
}

#[test]
fn cli_debug_wit_dir_uses_only_the_given_inputs() {
    let root = project_root();
    let output_dir = unique_temp_dir("debug-wit-dir-no-defaults");

    let status = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "debug-wit-dir",
            "--input",
            root.join("advanced/samples/inputs/user-service.wit")
                .to_str()
                .unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
        ])
        .status()
        .unwrap();

    assert!(status.success());
    assert!(!output_dir.join("deps/nexus-temporal-types").exists());

    let _ = fs::remove_dir_all(output_dir);
}

#[test]
fn cli_debug_wit_dir_refuses_to_overwrite_existing_path() {
    let root = project_root();
    let output_dir = unique_temp_dir("debug-wit-dir-existing");
    fs::create_dir_all(&output_dir).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "debug-wit-dir",
            "--input",
            sample_input_path(&root).to_str().unwrap(),
            "--output",
            output_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("refusing to overwrite existing path"));

    let _ = fs::remove_dir_all(output_dir);
}
