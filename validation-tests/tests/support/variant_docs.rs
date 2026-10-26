//! Variant documentation fixtures shared by source-only and toolchain tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use nexgen::language::Language;
use nexgen::{GenerateRequest, generate_to_file};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub const VARIANTS: &str = r#"
package example:docs;
world system { export models; }
/// @nexus.endpoint "docs"
interface models {
  /// @nexus.doc "Default variant." python="Python variant." typescript="TypeScript variant."
  variant choice {
    /// @nexus.doc "Default payload." python="Python payload." typescript="TypeScript payload */ safe."
    text(string),
    /// @nexus.doc "Default empty case."
    none,
    undocumented,
  }
  variant single {
    /// @nexus.doc "Single case."
    only,
  }
  /// @nexus.doc "Default only variant."
  variant fallback {
    /// @nexus.doc python="Python-only case."
    only,
  }
  record request {
    choice: choice,
    single: single,
    fallback: fallback,
  }
  run: func(request: request);
}
"#;

pub fn experimental_variants() -> String {
    VARIANTS
        .replace(
            "  variant choice {",
            "  /// @nexus.experimental\n  variant choice {",
        )
        .replace(
            "  variant single {",
            "  /// @nexus.experimental\n  variant single {",
        )
}

pub fn generate(
    root: &Path,
    language: Language,
    source: &str,
    dependencies: &[PathBuf],
) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nexgen-variant-docs-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("model.wit");
    fs::write(&input, source).unwrap();
    let output = dir.join("output");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language,
        input_paths: std::iter::once(input)
            .chain(dependencies.iter().cloned())
            .collect(),
        support_package: Some("unused_support".into()),
        descriptor_paths: vec![root.join("advanced/samples/descriptors/temporal_api.bin")],
        output_path: output.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    output
}
