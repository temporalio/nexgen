#![cfg(feature = "advanced")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use nexgen::language::Language;
use nexgen::{GenerateRequest, generate_to_file};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const VARIANTS: &str = r#"
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

fn generate(language: Language, source: &str, dependencies: &[PathBuf]) -> PathBuf {
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
        descriptor_paths: vec![
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("advanced/samples/descriptors/temporal_api.bin"),
        ],
        output_path: output.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    output
}

fn assert_python_parses(output: &Path) {
    assert!(
        Command::new("python3")
            .args([
                "-c",
                "import ast,pathlib,sys\nfor p in pathlib.Path(sys.argv[1]).rglob('*.py'): ast.parse(p.read_text(), feature_version=(3, 10))",
                output.to_str().unwrap(),
            ])
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn python_variant_docs_select_overrides_and_preserve_tagged_tuples() {
    let output = generate(Language::Python, VARIANTS, &[]);
    let models = fs::read_to_string(output.join("models.py")).unwrap();
    assert!(models.contains("    # Python payload.\n    tuple[typing.Literal[\"text\"], str]"));
    assert!(models.contains("    # Default empty case.\n    | tuple[typing.Literal[\"none\"]]"));
    assert!(models.contains("    | tuple[typing.Literal[\"undocumented\"]]"));
    assert!(models.contains(")\n\"\"\"Python variant.\"\"\""));
    assert!(
        models.contains("Single = (\n    # Single case.\n    tuple[typing.Literal[\"only\"]]\n)")
    );
    assert!(models.contains("# Python-only case."));
    assert!(models.contains("\"\"\"Default only variant.\"\"\""));
    assert!(!models.contains("Default variant."));
    assert!(!models.contains("TypeScript payload"));
    assert_python_parses(&output);
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn typescript_variant_docs_select_overrides_and_escape_comments() {
    let output = generate(Language::TypeScript, VARIANTS, &[]);
    let models = fs::read_to_string(output.join("models.ts")).unwrap();
    assert!(models.contains("/**\n * TypeScript variant.\n */\nexport type Choice"));
    assert!(models.contains(
        "  /**\n   * TypeScript payload * / safe.\n   */\n  | { tag: \"text\"; value: string }"
    ));
    assert!(models.contains("   * Default empty case.\n   */\n  | { tag: \"none\" }"));
    assert!(models.contains(" * Default only variant.\n */\nexport type Fallback"));
    assert!(models.contains("   * Single case."));
    assert!(!models.contains("Python-only case."));
    assert!(!models.contains("Default variant."));
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn python_proto_variant_docs_render_on_alias_and_case_classes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source =
        fs::read_to_string(root.join("advanced/samples/inputs/notification-service.wit")).unwrap();
    let output = generate(
        Language::Python,
        &source,
        &[root.join("advanced/samples/inputs/deps")],
    );
    let models = fs::read_to_string(output.join("models.py")).unwrap();
    assert!(models.contains(
        "class OnCompleteRequest(typing.Generic[OutputT, SourceContextT]):\n    \"\"\"A notification that reports that a source operation completed.\n\n    .. warning::\n        This API is experimental and subject to change.\n    \"\"\""
    ));
    assert!(models.contains(
        "class OnCompleteResponse:\n    \"\"\"An empty response that acknowledges a completion notification.\n\n    .. warning::\n        This API is experimental and subject to change.\n    \"\"\""
    ));
    assert!(models.contains(
        "class OnCompleteRequestResultSuccess(typing.Generic[OutputT]):\n    \"\"\"The source operation completed successfully with this output.\"\"\""
    ));
    assert!(models.contains(
        "class OnCompleteRequestResultFailure:\n    \"\"\"The source operation failed, including timeout, cancellation, and application\n    errors.\n    \"\"\""
    ));
    assert!(models.contains(
        ")\n\"\"\"The successful output or failure of the source operation.\n\n.. warning::\n    This API is experimental and subject to change.\n\"\"\""
    ));
    // Only the two records and variant alias carry warnings, not case classes.
    assert_eq!(models.matches(".. warning::").count(), 3);
    assert_python_parses(&output);
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn experimental_variants_render_warnings_without_marking_cases() {
    let source = VARIANTS
        .replace(
            "  variant choice {",
            "  /// @nexus.experimental\n  variant choice {",
        )
        .replace(
            "  variant single {",
            "  /// @nexus.experimental\n  variant single {",
        );
    for language in [Language::Python, Language::TypeScript] {
        let output = generate(language, &source, &[]);
        let models = fs::read_to_string(output.join(match language {
            Language::Python => "models.py",
            Language::TypeScript => "models.ts",
            _ => unreachable!(),
        }))
        .unwrap();
        if language == Language::Python {
            assert!(models.contains(
                "\"\"\"Python variant.\n\n.. warning::\n    This API is experimental and subject to change.\n\"\"\""
            ));
            assert_eq!(models.matches(".. warning::").count(), 2);
            assert_python_parses(&output);
        } else {
            assert!(models.contains(
                " * TypeScript variant.\n *\n * @experimental This API is experimental and subject to change.\n */\nexport type Choice"
            ));
            assert_eq!(models.matches("@experimental").count(), 2);
            assert!(models.contains(
                "/**\n * @experimental This API is experimental and subject to change.\n */\nexport type Single"
            ));
        }
        fs::remove_dir_all(output.parent().unwrap()).unwrap();
    }
}
