#![cfg(feature = "advanced")]

use std::fs;
use std::path::{Path, PathBuf};

use nexgen::descriptors::DescriptorIndex;
use nexgen::generator::generate_source;
use nexgen::language::Language;
use nexgen::parser::load_api_spec_from_wit_for_language_with_inputs;

fn descriptor_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("advanced/samples/descriptors/temporal_api.bin")
}

fn write_fixture(
    root: &Path,
    operation: bool,
    omit_oneof: bool,
    export_interface: bool,
) -> PathBuf {
    let operation = if operation {
        "  pause: func(request: pause-activity-request);\n"
    } else {
        ""
    };
    let omit = if omit_oneof {
        "    /// @nexus.omit\n"
    } else {
        ""
    };
    let export = if export_interface {
        "  export api;\n"
    } else {
        ""
    };
    let source = format!(
        r#"package test:oneof@1.0.0;

world system {{
{export}
}}

interface api {{
  /// @nexus.proto "temporal.api.workflowservice.v1.PauseActivityRequest.activity"
  variant activity-selection {{
    id(string),
    %type(string),
  }}

  /// @nexus.proto "temporal.api.common.v1.WorkflowExecution"
  record workflow-execution {{
    workflow-id: string,
    run-id: string,
  }}

  /// @nexus.proto "temporal.api.workflowservice.v1.PauseActivityRequest"
  record pause-activity-request {{
    namespace: string,
    execution: option<workflow-execution>,
    identity: string,
{omit}    activity: option<activity-selection>,
    reason: string,
    request-id: string,
  }}

{operation}}}
"#
    );
    let path = root.join("oneof.wit");
    fs::write(&path, source).unwrap();
    path
}

fn generate(language: Language, path: &Path) -> nexgen::error::Result<String> {
    let spec = load_api_spec_from_wit_for_language_with_inputs(language, &[path.to_path_buf()])?;
    let descriptors = DescriptorIndex::load(&descriptor_path())?;
    let support_package = match language {
        Language::Dotnet => "Nexgen.Support",
        Language::Go => "example.com/nexgen/support",
        Language::TypeScript => "./support",
        Language::Python => "temporal_support",
        Language::Java => "",
        _ => "",
    };
    generate_source(language, spec, &descriptors, support_package)
}

#[test]
fn typescript_allows_oneofs_in_operation_models() {
    let temp = tempfile::tempdir().unwrap();
    let path = write_fixture(temp.path(), true, false, true);
    // The proto-oneof sample covers model-only conversion at runtime. This
    // fixture additionally checks that a oneof-bearing operation is accepted.
    generate(Language::TypeScript, &path).unwrap();
}

#[test]
fn typescript_allows_omitted_or_unreachable_oneofs() {
    let temp = tempfile::tempdir().unwrap();
    let omitted = write_fixture(temp.path(), true, true, true);
    generate(Language::TypeScript, &omitted).unwrap();

    let unreachable = write_fixture(temp.path(), false, false, false);
    generate(Language::TypeScript, &unreachable).unwrap();
}

#[test]
fn typescript_emits_a_complete_conversion_pair() {
    let temp = tempfile::tempdir().unwrap();
    let path = write_fixture(temp.path(), true, true, true);
    let output = generate(Language::TypeScript, &path).unwrap();
    assert!(output.contains("function pauseActivityRequestFromProto("));
    assert!(output.contains("function pauseActivityRequestToProto("));
    assert!(!output.contains("export function pauseActivityRequestFromProto("));
    assert!(!output.contains("export function pauseActivityRequestToProto("));
}
