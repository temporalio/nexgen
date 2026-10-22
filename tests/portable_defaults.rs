#![cfg(feature = "advanced")]

use std::fs;
use std::path::Path;

use nexgen::generator::GenerationMode;
use nexgen::language::Language;
use nexgen::{GenerateRequest, generate_to_file};

const NATIVE: &str = r#"package test:defaults;
world system { export service; }
/// @nexus.endpoint "defaults"
interface service {
  enum mode { automatic, manual }
  record request {
    /// @nexus.default true
    enabled: bool,
    /// @nexus.default 3
    count: s32,
    /// @nexus.default 1.25
    ratio: f64,
    /// @nexus.default "portable"
    label: string,
    /// @nexus.default "manual"
    mode: mode,
    /// @nexus.default "portable" go="workflow.GetInfo(ctx).TaskQueueName"
    queue: string,
  }
  execute: func(request: request);
}
"#;

const PROTO: &str = r#"package test:defaults;
world system { export service; }
/// @nexus.endpoint "defaults"
interface service {
  type placeholder = string;
  /// @nexus.proto "temporal.api.common.v1.RetryPolicy" typescript-import="@temporalio/proto"
  record retry-policy {
    /// @nexus.omit
    initial-interval: placeholder,
    /// @nexus.default 1.25
    backoff-coefficient: f64,
    /// @nexus.omit
    maximum-interval: placeholder,
    /// @nexus.default 3
    maximum-attempts: s32,
    /// @nexus.omit
    non-retryable-error-types: placeholder,
  }
  /// @nexus.proto "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionResponse" typescript-import="@temporalio/proto"
  record signal-result {
    /// @nexus.default "portable"
    run-id: string,
    /// @nexus.default true
    started: bool,
    /// @nexus.omit
    first-execution-run-id: placeholder,
    /// @nexus.omit
    signal-link: placeholder,
  }
  retry: func(request: retry-policy);
  signal: func(request: signal-result);
}
"#;

fn read_generated(path: &Path) -> String {
    if path.is_file() {
        return fs::read_to_string(path).unwrap();
    }
    let mut entries = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();
    entries
        .iter()
        .map(|entry| read_generated(entry))
        .collect::<Vec<_>>()
        .join("\n")
}

fn generate(language: Language, wit: &str, proto: bool) -> String {
    try_generate(language, wit, proto).unwrap()
}

fn try_generate(language: Language, wit: &str, proto: bool) -> nexgen::error::Result<String> {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("defaults.wit");
    fs::write(&input, wit).unwrap();
    let output = dir.path().join("output");
    generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            mode: GenerationMode::NativeApi,
            system_nexus: false,
        },
        language,
        input_paths: vec![input],
        support_package: Some(
            match language {
                Language::Go => "go.temporal.io/sdk/advanced/samples/go/support",
                Language::Python => "temporal_support",
                Language::TypeScript => "./support",
                Language::Dotnet => "Nexgen.Support",
                _ => unreachable!(),
            }
            .into(),
        ),
        descriptor_paths: if proto {
            vec![
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("advanced/samples/descriptors/temporal_api.bin"),
            ]
        } else {
            Vec::new()
        },
        output_path: output.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })?;
    Ok(read_generated(&output))
}

fn contains_all(rendered: &str, expected: &[&str]) {
    for expected in expected {
        assert!(
            rendered.contains(expected),
            "missing {expected:?} in:\n{rendered}"
        );
    }
}

#[test]
fn portable_defaults_validate_api_ranges_and_external_enum_cases() {
    let out_of_range = NATIVE
        .replace("default 3", "default 2147483648")
        .replace("count: s32", "count: u32");
    assert!(
        try_generate(Language::Go, &out_of_range, false)
            .unwrap_err()
            .to_string()
            .contains("API range")
    );
    let external_enum = NATIVE
        .replace("enum mode { automatic, manual }", "/// @nexus.proto \"temporal.api.enums.v1.WorkflowIdReusePolicy\" typescript-import=\"@temporalio/proto\"\n  type mode = string;")
        .replace("default \"manual\"", "default \"missing-case\"");
    for language in [
        Language::Go,
        Language::Python,
        Language::TypeScript,
        Language::Dotnet,
    ] {
        assert!(
            try_generate(language, &external_enum, true)
                .unwrap_err()
                .to_string()
                .contains("unknown protobuf enum case")
        );
        let escaped = NATIVE.replace(
            "default \"portable\"\n    label",
            "default \"\\u0000\"\n    label",
        );
        let rendered = generate(language, &escaped, false);
        assert!(rendered.contains("\"\\u0000\""));
        assert!(!rendered.contains("\\u{0}"));
    }
}

#[test]
fn go_portable_defaults_use_the_expression_initialization_path() {
    let rendered = generate(Language::Go, NATIVE, false);
    contains_all(
        &rendered,
        &[
            "if request.Enabled == nil {",
            "var value bool = true",
            "var value int32 = 3",
            "var value float64 = 1.25",
            "var value string = \"portable\"",
            "var value Mode = 1",
            "var value string = workflow.GetInfo(ctx).TaskQueueName",
            "Enabled *bool",
            "Count *int32",
            "Label *string",
        ],
    );
}

#[test]
fn portable_defaults_render_for_python_typescript_and_dotnet() {
    contains_all(
        &generate(Language::Python, NATIVE, false),
        &[
            "enabled: bool = True",
            "count: int = 3",
            "ratio: float = 1.25",
            "label: str = \"portable\"",
            "mode: Mode = Mode.Manual",
            "queue: str = \"portable\"",
        ],
    );
    contains_all(
        &generate(Language::TypeScript, NATIVE, false),
        &[
            "enabled: request.enabled ?? true",
            "count: request.count ?? 3",
            "ratio: request.ratio ?? 1.25",
            "label: request.label ?? \"portable\"",
            "mode: request.mode ?? 1",
            "queue: request.queue ?? \"portable\"",
        ],
    );
    contains_all(
        &generate(Language::Dotnet, NATIVE, false),
        &[
            "Enabled { get; init; } = true",
            "Count { get; init; } = (int)3",
            "Ratio { get; init; } = (double)1.25",
            "Label { get; init; } = \"portable\"",
            "Mode { get; init; } = (Mode)1",
            "Queue { get; init; } = \"portable\"",
            "options?.Enabled ?? true",
        ],
    );
}

#[test]
fn proto_defaults_use_api_values_and_normal_wire_conversion() {
    contains_all(
        &generate(Language::Go, PROTO, true),
        &[
            "var value int32 = 3",
            "var value bool = true",
            "var value string = \"portable\"",
        ],
    );
    contains_all(
        &generate(Language::Python, PROTO, true),
        &[
            "maximum_attempts: int = 3",
            "backoff_coefficient: float = 1.25",
            "started: bool = True",
            "run_id: str = \"portable\"",
        ],
    );
    contains_all(
        &generate(Language::TypeScript, PROTO, true),
        &[
            "model.started == null ? true : model.started",
            "model.maximumAttempts == null ? 3 : model.maximumAttempts",
            "model.runId == null ? \"portable\" : model.runId",
        ],
    );
    contains_all(
        &generate(Language::Dotnet, PROTO, true),
        &[
            "Started { get; init; } = true",
            "RunId { get; init; } = \"portable\"",
            "proto.Started = true;",
        ],
    );
}
