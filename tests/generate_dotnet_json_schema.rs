//! C# generator tests for JSON Schema input that need no .NET toolchain.
//!
//! These pin the emitted shape and the load diagnostics with text assertions.
//! The tests that compile the output and run it through Temporal's default
//! payload converter live with the other toolchain tests in
//! `validation-tests/tests/support/dotnet_json_schema.rs`.

// The `dotnet` generate target lives behind the `advanced` feature.
#![cfg(feature = "advanced")]

mod common;

#[path = "../validation-tests/tests/support/csharp_json_case.rs"]
mod csharp_json_case;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nexgen::generator::GenerationMode;
use nexgen::language::Language;
use nexgen::nexgen_config::NexgenConfig;
use nexgen::{GenerateRequest, generate_to_file};

use csharp_json_case::{
    JsonCase, Output, assert_has, assert_lacks, read_cs_files, unique_output_path,
};

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// ---------------------------------------------------------------------------
// Checked-in samples
// ---------------------------------------------------------------------------

/// Generates a `samples/schemas` example the way `cargo build-examples` does,
/// minus the sample namespace, so the checks below see the derived defaults.
fn generate_sample(example: &str, mode: GenerationMode) -> (PathBuf, Output) {
    let output_path = unique_output_path(&format!("dotnet-json-sample-{example}"));
    generate_to_file(&GenerateRequest {
        config: NexgenConfig {
            mode,
            ..Default::default()
        },
        language: Language::Dotnet,
        input_paths: vec![common::json_input_path(&project_root(), example)],
        support_package: None,
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();
    let files = Output(read_cs_files(&output_path));
    (output_path, files)
}

#[test]
fn csharp_json_examples_render_expected_language_features() {
    for mode in [GenerationMode::DefinitionsOnly, GenerationMode::NativeApi] {
        let (dir, showcase) = generate_sample("showcase", mode);
        let models = showcase.file("Models.cs");
        assert_has(
            models,
            &[
                // Scalar defaults keep presence and read through `OrDefault`.
                "public string? Greeting { get; init; }",
                "public string GreetingOrDefault => Greeting ?? \"hello\";",
                "public bool DebugOrDefault => Debug ?? false;",
                // `deprecated` and `title`.
                "[global::System.Obsolete]",
                "Retry budget",
                // `x-cs-name` renames the member and keeps the wire key.
                "public string? LegacyIdCs { get; init; }",
                "\"legacyId\"",
                // `x-cs-const-name` / `x-cs-enum-names` rename value constants.
                "RevisionCs",
                "ActiveCs",
                // `x-cs-name` on a model renames the type.
                "public sealed record ContactCs",
                // Free-form members keep unknown members verbatim.
                "public sealed record Extras",
                "global::System.Text.Json.JsonElement",
                // Inline tagged-union branches named by `x-cs-name`.
                "public interface Note",
                "public sealed record TextNote : Note",
                "public sealed record LinkNote : Note",
                // An inline object branch is named after its owner and member.
                "public sealed record ShowcaseDetailObject : Showcase.DetailUnion",
                "must have at most 4 properties",
            ],
        );
        let services = showcase.file("Services.cs");
        assert_has(
            services,
            &[
                "[NexusService(\"example.showcase.v1.ShowcaseService\")]",
                "internal interface IShowcaseServiceCs",
                "[NexusOperation(\"GetShowcase\")]",
                "GetShowcaseCs(",
            ],
        );
        let client = "public class ShowcaseServiceCsClient";
        if mode == GenerationMode::NativeApi {
            assert_has(services, &[client, "GetShowcaseCsAsync("]);
        } else {
            assert_lacks(services, &[client, "Temporalio.Workflows"]);
        }
        fs::remove_dir_all(dir).unwrap();

        let (dir, temporal) = generate_sample("temporal", mode);
        assert_has(
            temporal.file("Models.cs"),
            &["global::System.DateTimeOffset", "global::System.TimeSpan"],
        );
        assert!(temporal.has_file("TemporalSupport.cs"));
        assert!(!temporal.has_file("Services.cs"));
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn csharp_json_property_names_without_a_key_predicate_are_refused() {
    for property_names in ["{ type: string }", "{ type: string, format: date-time }"] {
        let case = JsonCase::new("property-names-refused");
        case.write(
            "keys.yaml",
            &format!(
                "type: object\nadditionalProperties: {{ type: string }}\npropertyNames: {property_names}\n"
            ),
        );
        assert_has(&case.generate_err(&["keys.yaml"]), &["propertyNames"]);
    }
}

#[test]
fn csharp_json_bare_ref_alias_cycle_is_rejected() {
    let case = JsonCase::new("alias-cycle");
    case.write(
        "cycle.yaml",
        "title: Cycle\n$ref: \"#/$defs/A\"\n$defs:\n  A: { $ref: \"#/$defs/Z\" }\n  Z: { $ref: \"#/$defs/A\" }\n",
    );
    assert_has(&case.generate_err(&["cycle.yaml"]), &["cycle"]);
}

#[test]
fn csharp_cli_generates_json_schema_without_a_support_package() {
    let root = project_root();
    let temp = unique_output_path("dotnet-json-cli");
    let run = |args: &[&str], output: &Path| {
        let result = Command::new(env!("CARGO_BIN_EXE_nexgen"))
            .arg("dotnet")
            .args(args)
            .arg("--output")
            .arg(output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        read_cs_files(output)
    };
    let chat = common::json_input_path(&root, "chat");
    let chat = chat.to_str().unwrap();

    // No `--support-package` is needed: the runtime is generated alongside.
    let derived = run(&[chat], &temp.join("derived"));
    assert!(derived.contains_key(Path::new("Violation.cs")));
    // A single input with a service derives its root namespace from it.
    assert_has(
        &derived[Path::new("Models.cs")],
        &["namespace Nexgen.ChatService\n"],
    );

    let named = run(&[chat, "--namespace", "Acme.Chat"], &temp.join("named"));
    assert_has(&named[Path::new("Models.cs")], &["namespace Acme.Chat\n"]);
    assert_has(
        &named[Path::new("Violation.cs")],
        &["namespace Acme.Chat\n"],
    );

    // Without a service, the default root namespace.
    let models_only = temp.join("models.yaml");
    fs::create_dir_all(&temp).unwrap();
    fs::write(
        &models_only,
        "type: object\nproperties:\n  a: { type: string }\n",
    )
    .unwrap();
    let generated = run(&[models_only.to_str().unwrap()], &temp.join("generated"));
    assert_has(
        &generated[Path::new("Models.cs")],
        &["namespace Nexgen.Generated\n"],
    );
    fs::remove_dir_all(temp).unwrap();
}

// ---------------------------------------------------------------------------
// Annotations and keywords outside the subset
// ---------------------------------------------------------------------------

/// A keyword the shared loader refuses (P6 / P7.1) fails a C# generation
/// before anything is emitted, so C# never approximates it.
#[test]
fn csharp_json_rejects_unsupported_keywords() {
    let cases: &[(&str, &str)] = &[
        (
            "type: object\nproperties:\n  doc: { type: string, contentMediaType: application/json }\n",
            "`contentMediaType` is not supported",
        ),
        (
            "type: object\nproperties:\n  doc: { type: string, contentSchema: { type: object } }\n",
            "`contentSchema` is not supported",
        ),
        (
            "type: object\nproperties:\n  pair: { type: array, prefixItems: [{ type: string }, { type: integer }] }\n",
            "tuple arrays (`prefixItems`) are not supported",
        ),
        (
            "type: object\nproperties:\n  tags: { type: array, items: { type: string }, unevaluatedItems: false }\n",
            "`unevaluatedItems` is not supported",
        ),
        (
            "type: object\npatternProperties:\n  \"^x-\": { type: string }\n",
            "`patternProperties` is not supported",
        ),
        (
            "type: object\nproperties:\n  card: { type: string }\ndependentSchemas:\n  card: { required: [billing] }\n",
            "`dependentSchemas` is not supported",
        ),
        (
            "type: object\nproperties:\n  a: { type: string }\nunevaluatedProperties: false\n",
            "`unevaluatedProperties` is not supported",
        ),
    ];
    for (schema, expected) in cases {
        let case = JsonCase::new("unsupported-keyword");
        case.write("model.yaml", schema);
        assert_has(&case.generate_err(&["model.yaml"]), &[expected]);
    }
}
