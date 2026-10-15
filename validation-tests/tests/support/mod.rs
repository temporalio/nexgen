//! Helpers shared by generated-code integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub mod common;
pub mod doc_rendering;
pub mod json_schema;

/// Build the CLI only when a test needs to invoke it. Cargo's artifact report
/// supplies its location, including when CARGO_TARGET_DIR is customized.
pub fn nexgen_binary() -> PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let output = Command::new(cargo)
                .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
                .args([
                    "build",
                    "--package",
                    "nexgen",
                    "--all-features",
                    "--bin",
                    "nexgen",
                    "--message-format=json",
                ])
                .output()
                .expect("build nexgen for CLI integration tests");
            let messages: Vec<serde_json::Value> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            let diagnostics: String = messages
                .iter()
                .filter_map(|message| message["message"]["rendered"].as_str())
                .collect();
            assert!(
                output.status.success(),
                "failed to build nexgen:\n{diagnostics}{}",
                String::from_utf8_lossy(&output.stderr)
            );
            messages
                .iter()
                .find_map(|message| {
                    (message["reason"] == "compiler-artifact"
                        && message["target"]["name"] == "nexgen")
                        .then(|| message["executable"].as_str().map(PathBuf::from))
                        .flatten()
                })
                .expect("Cargo must report the nexgen executable")
        })
        .clone()
}
