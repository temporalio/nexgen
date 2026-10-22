//! Proto-backed enums retain a native model reference, just like records.
#![cfg(feature = "advanced")]

use std::fs;
use std::path::{Path, PathBuf};

use nexgen::error::Error;
use nexgen::language::Language;
use nexgen::{GenerateRequest, generate_to_file};

const WIT: &str = r#"package temporal:nexus@1.0.0;

world system {
  export alerts;
}

/// @nexus.endpoint "alerts"
interface alerts {
  /// @nexus.proto "temporal.api.enums.v1.Severity" typescript-import="@temporalio/proto"
  enum alert-level {
    severity-unspecified,
    severity-high,
    severity-medium,
    severity-low,
  }

  /// @nexus.proto "temporal.api.version.v1.Alert" typescript-import="@temporalio/proto"
  record alert {
    message: string,
    severity: option<alert-level>,
  }

  raise: func(request: alert) -> alert;
}
"#;

fn generate(language: Language, wit: &str) -> String {
    try_generate(language, wit).unwrap()
}

fn try_generate(language: Language, wit: &str) -> Result<String, Error> {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.wit");
    fs::write(&input, wit).unwrap();
    let output = dir.path().join("output");
    generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            mode: nexgen::generator::GenerationMode::NativeApi,
            ..Default::default()
        },
        language,
        input_paths: vec![input],
        support_package: Some(
            match language {
                Language::Go => "example.com/support",
                Language::Dotnet => "Nexgen.Support",
                _ => "temporal_support",
            }
            .to_string(),
        ),
        descriptor_paths: vec![
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("advanced/samples/descriptors/temporal_api.bin"),
        ],
        output_path: output.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })?;
    fn read(path: &Path, result: &mut String) {
        if path.is_dir() {
            let mut entries = fs::read_dir(path)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            entries.sort();
            for entry in entries {
                read(&entry, result);
            }
        } else {
            result.push_str(&fs::read_to_string(path).unwrap());
        }
    }
    let mut result = String::new();
    read(&output, &mut result);
    Ok(result)
}

#[test]
fn python_operation_reaches_the_authored_enum_model() {
    let output = generate(Language::Python, WIT);
    for expected in [
        "class AlertLevel(enum.IntEnum):",
        "severity: AlertLevel | None = None",
        "severity=AlertLevel(value.severity)",
        "common_pb2.Severity.ValueType(value.severity)",
    ] {
        assert!(output.contains(expected), "missing {expected}:\n{output}");
    }
}

#[test]
fn typescript_operation_reaches_the_authored_enum_model() {
    let output = generate(Language::TypeScript, WIT);
    for expected in [
        "export enum AlertLevel {",
        "severity?: AlertLevel;",
        "proto.severity as number as AlertLevel",
        "as number as temporal.api.enums.v1.Severity",
    ] {
        assert!(output.contains(expected), "missing {expected}:\n{output}");
    }
    assert!(!output.contains("as any"));
}

#[test]
fn dotnet_converts_the_authored_enum_model() {
    let output = generate(Language::Dotnet, WIT);
    for expected in [
        "public enum AlertLevel",
        "public AlertLevel? Severity { get; init; }",
        "null : (AlertLevel)wire.Severity",
        "proto.Severity = (Temporalio.Api.Enums.V1.Severity)severity;",
    ] {
        assert!(output.contains(expected), "missing {expected}:\n{output}");
    }
}

#[test]
fn go_converts_the_authored_enum_model() {
    let output = generate(Language::Go, WIT);
    for expected in [
        "type AlertLevel int32",
        "Severity *AlertLevel",
        "AlertLevel(int32(",
    ] {
        assert!(output.contains(expected), "missing {expected}:\n{output}");
    }
}

#[test]
fn defaults_use_the_authored_enum_member() {
    let wit = WIT.replace(
        "severity: option<alert-level>,",
        "/// @nexus.default \"severity-low\"\n    severity: alert-level,",
    );
    for (language, expected) in [
        (Language::Python, "AlertLevel.SeverityLow"),
        (Language::TypeScript, "AlertLevel.SeverityLow"),
        (Language::Dotnet, "(Temporalio.Api.Enums.V1.Severity)3"),
    ] {
        let output = generate(language, &wit);
        assert!(output.contains(expected), "missing {expected}:\n{output}");
    }
}

#[test]
fn model_only_exports_also_render_the_enum() {
    let wit = WIT.replace("  raise: func(request: alert) -> alert;\n", "");
    for (language, expected) in [
        (Language::Python, "class AlertLevel(enum.IntEnum):"),
        (Language::TypeScript, "export enum AlertLevel {"),
        (Language::Dotnet, "public enum AlertLevel"),
        (Language::Go, "type AlertLevel int32"),
    ] {
        let output = generate(language, &wit);
        assert!(output.contains(expected), "missing {expected}:\n{output}");
    }
}
