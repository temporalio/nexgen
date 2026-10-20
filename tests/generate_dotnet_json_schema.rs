//! C# generator tests for JSON Schema input that need no .NET toolchain.
//!
//! These pin the emitted shape and the load diagnostics with text assertions.
//! The tests that compile the output and run it through Temporal's default
//! payload converter live with the other toolchain tests in
//! `validation-tests/tests/support/dotnet_json_schema.rs`.

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
        dotnet_namespace: Some(format!("Nexgen.JsonSchema.{example}")),
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
                "public string GreetingOrDefault => this.Greeting ?? \"hello\";",
                "public bool DebugOrDefault => this.Debug ?? false;",
                // `deprecated` carries the generic reason, and the file
                // explains the warning it suppresses for its own references.
                "[global::System.Obsolete(\"This field is deprecated.\")]",
                "// generates; keep those references from raising CS0618.\n    #pragma warning disable CS0618\n",
                // `title`.
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
        // The contract is the API without `--native-api`, so it is public;
        // with it, callers use the generated client and the contract stays
        // internal.
        let contract = if mode == GenerationMode::NativeApi {
            "internal interface IShowcaseServiceCs"
        } else {
            "public interface IShowcaseServiceCs"
        };
        assert_has(services, &[contract]);
        assert_has(
            services,
            &[
                "[NexusService(\"example.showcase.v1.ShowcaseService\")]",
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
fn csharp_json_rejects_service_clashing_with_its_io_type() {
    let case = JsonCase::new("service-clash");
    case.write(
        "sub/api.nexusrpc.yaml",
        r##"nexusrpc: "1.0.0"
services:
  Thing:
    operations:
      doIt: { input: { $ref: "#/$defs/Thing" } }
$defs:
  Thing: { type: object, properties: { id: { type: string } } }
"##,
    );
    case.write(
        "root.nexusrpc.yaml",
        "nexusrpc: \"1.0.0\"\n$defs:\n  Root: { type: object, properties: { a: { type: string } } }\n",
    );
    assert_has(
        &case.generate_err(&["."]),
        &["collision", "Thing", "x-cs-name"],
    );
}

#[test]
fn csharp_json_rejects_empty_service_descriptions() {
    let case = JsonCase::new("empty-description");
    case.write(
        "empty.nexusrpc.yaml",
        "nexusrpc: \"1.0.0\"\nservices:\n  Empty:\n    fqn: example.v1.Empty\n    description: \"\"\n    operations:\n      ping: {}\n",
    );
    assert_has(
        &case.generate_err(&["empty.nexusrpc.yaml"]),
        &["`description` must not be empty or whitespace-only"],
    );
}

#[test]
fn csharp_cli_generates_json_schema_without_a_support_package() {
    let root = project_root();
    let temp = unique_output_path("dotnet-json-cli");
    let run = |args: &[&str], output: &Path| {
        Command::new(env!("CARGO_BIN_EXE_nexgen"))
            .arg("dotnet")
            .args(args)
            .arg("--output")
            .arg(output)
            .output()
            .unwrap()
    };
    let chat = common::json_input_path(&root, "chat");
    let chat = chat.to_str().unwrap();

    // No `--support-package` is needed: the runtime is generated alongside,
    // in the root namespace `--namespace` names.
    let named = temp.join("named");
    let result = run(&[chat, "--namespace", "Acme.Chat"], &named);
    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let named = read_cs_files(&named);
    assert_has(&named[Path::new("Models.cs")], &["namespace Acme.Chat\n"]);
    assert_has(
        &named[Path::new("Violation.cs")],
        &["namespace Acme.Chat\n"],
    );

    // Like Java's `--package-name`, the root namespace is required: there is
    // no default to guess, which would move every type whenever the input set
    // changed.
    let result = run(&[chat], &temp.join("missing"));
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("requires `--namespace`"),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!temp.join("missing").exists());
    fs::remove_dir_all(temp).unwrap();
}

// ---------------------------------------------------------------------------
// Annotations and keywords outside the subset
// ---------------------------------------------------------------------------

/// A named union nests its wrappers in the interface (`Foo.FooString`), while
/// an inline union declares them beside its interface in the declaring model
/// (`Holder.ChoiceUnionString`), so the interface name is not repeated.
#[test]
fn csharp_json_union_wrapper_placement() {
    let case = JsonCase::new("union_wrappers");
    case.write(
        "holder.yaml",
        "type: object\nproperties:\n  choice:\n    oneOf:\n      - { type: string }\n      - { type: integer }\n  foo: { $ref: '#/$defs/Foo' }\n$defs:\n  Foo:\n    oneOf:\n      - { type: string }\n      - { type: boolean }\n",
    );
    let output = case.generate(&["holder.yaml"]);
    let models = output.file("Models.cs");
    assert_has(
        models,
        &[
            "        public interface ChoiceUnion\n        {\n            /// <summary>\n            /// Converts",
            "        public sealed class ChoiceUnionString : ChoiceUnion\n",
            "        public sealed class ChoiceUnionInteger : ChoiceUnion\n",
            "    public interface Foo\n    {\n        /// <summary>\n        /// The string member of <see cref=\"Foo\"/>.\n        /// </summary>\n        [global::System.CodeDom.Compiler.GeneratedCode(\"nexgen\", null)]\n        public sealed class FooString : Foo\n",
        ],
    );
    assert_lacks(
        models,
        &["            public sealed class ChoiceUnionString"],
    );
}

/// A `title` is the first line of the `<summary>`, ahead of the description,
/// and never names the type.
#[test]
fn csharp_json_title_leads_the_summary() {
    let case = JsonCase::new("title");
    case.write(
        "account.yaml",
        "title: Registered user\ndescription: Someone who signed up.\ntype: object\nproperties:\n  id: { type: string, title: Identifier }\n",
    );
    let output = case.generate(&["account.yaml"]);
    let models = output.file("Models.cs");
    assert_has(
        models,
        &[
            "    /// <summary>\n    /// Registered user\n    /// <para/>\n    /// Someone who signed up.\n    /// </summary>\n",
            "    public sealed record Account\n",
            "        /// <summary>\n        /// Identifier\n        /// </summary>\n        public string? Id { get; init; }",
        ],
    );
    assert_lacks(models, &["RegisteredUser"]);
    // Nothing is deprecated, so nothing needs the obsolete-warning pragma.
    assert_lacks(models, &["#pragma", "Obsolete"]);
}

/// An annotation the loader accepts and drops leaves the C# output exactly as
/// the schema generates without it, so none can leak into a doc comment.
#[test]
fn csharp_json_inert_annotations_change_nothing() {
    let generate = |schema: &str| {
        let case = JsonCase::new("inert-annotation");
        case.write("model.yaml", schema);
        case.generate(&["model.yaml"]).all()
    };
    let plain = generate("type: object\nproperties:\n  a: { type: string, description: Shown. }\n");
    for annotated in [
        "type: object\n$comment: \"TODO: tighten\"\nproperties:\n  a: { type: string, description: Shown., $comment: internal }\n",
        "type: object\nproperties:\n  a: { type: string, description: Shown., examples: [x, { $ref: missing.yaml }] }\n",
    ] {
        assert_eq!(generate(annotated), plain, "{annotated}");
    }
}

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
        (
            "type: object\nproperties:\n  v: { anyOf: [{ type: string }, { type: integer }] }\n",
            "`anyOf` is not supported",
        ),
        (
            "type: object\nproperties:\n  v: { type: string, not: { const: x } }\n",
            "`not` is not supported",
        ),
        (
            "type: object\nproperties:\n  kind: { type: string }\nif: { properties: { kind: { const: a } } }\nthen: { required: [kind] }\n",
            "conditional schemas (`if`/`then`/`else`) are not supported",
        ),
        (
            "type: object\nproperties:\n  id: { type: string, readOnly: true }\n",
            "`readOnly`/`writeOnly` is not supported",
        ),
        (
            "type: object\nproperties:\n  secret: { type: string, writeOnly: true }\n",
            "`readOnly`/`writeOnly` is not supported",
        ),
    ];
    for (schema, expected) in cases {
        let case = JsonCase::new("unsupported-keyword");
        case.write("model.yaml", schema);
        assert_has(&case.generate_err(&["model.yaml"]), &[expected]);
    }
}
