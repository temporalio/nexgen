#![cfg(feature = "advanced")]

use std::fs;
use std::path::Path;

use nexgen::language::Language;

#[path = "../validation-tests/tests/support/variant_docs.rs"]
mod variant_docs;
use variant_docs::{VARIANTS, experimental_variants, generate, generate_proto_variants};

#[test]
fn python_variant_docs_select_overrides_and_preserve_tagged_tuples() {
    let output = generate(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        Language::Python,
        VARIANTS,
        &[],
    );
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
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn typescript_variant_docs_select_overrides_and_escape_comments() {
    let output = generate(
        Path::new(env!("CARGO_MANIFEST_DIR")),
        Language::TypeScript,
        VARIANTS,
        &[],
    );
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
    let output = generate_proto_variants(root);
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
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn experimental_variants_render_warnings_without_marking_cases() {
    let source = experimental_variants();
    for language in [Language::Python, Language::TypeScript] {
        let output = generate(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            language,
            &source,
            &[],
        );
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
