#![cfg(feature = "advanced")]

use std::fs;
use std::path::{Path, PathBuf};

use nexgen::SupportFiles;
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

fn write_generic_carrier_fixture(root: &Path) -> PathBuf {
    let source = r#"package test:generic-carrier@1.0.0;

world system {
  export api;
}

interface api {
  type placeholder = string;

  /// @nexus.type-parameter
  type input-t = placeholder;

  /// @nexus.proto "temporal.api.failure.v1.CanceledFailureInfo"
  record generic-request {
    details: option<input-t>,
    /// @nexus.omit
    identity: placeholder,
  }

}
"#;
    let path = root.join("generic-carrier.wit");
    fs::write(&path, source).unwrap();
    path
}

fn generate(language: Language, path: &Path) -> nexgen::error::Result<String> {
    let spec = load_api_spec_from_wit_for_language_with_inputs(language, &[path.to_path_buf()])?;
    let descriptors = DescriptorIndex::load(&descriptor_path())?;
    generate_source(language, spec, &descriptors, &SupportFiles::default())
}

#[test]
fn unsupported_backend_rejects_reachable_oneof_conversion() {
    let temp = tempfile::tempdir().unwrap();
    let path = write_fixture(temp.path(), true, false, true);
    let error = generate(Language::TypeScript, &path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("typescript protobuf conversion does not yet support oneof group"),
        "{error}"
    );
}

#[test]
fn unsupported_backend_rejects_operation_free_exported_oneof() {
    let temp = tempfile::tempdir().unwrap();
    let path = write_fixture(temp.path(), false, false, true);
    let error = generate(Language::TypeScript, &path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("typescript protobuf conversion does not yet support oneof group"),
        "{error}"
    );
}

#[test]
fn unsupported_backend_allows_omitted_or_unreachable_oneofs() {
    let temp = tempfile::tempdir().unwrap();
    let omitted = write_fixture(temp.path(), true, true, true);
    generate(Language::TypeScript, &omitted).unwrap();

    let unreachable = write_fixture(temp.path(), false, false, false);
    generate(Language::TypeScript, &unreachable).unwrap();
}

#[test]
fn unsupported_backend_rejects_reachable_generic_proto_carrier() {
    let temp = tempfile::tempdir().unwrap();
    let path = write_generic_carrier_fixture(temp.path());
    let error = generate(Language::TypeScript, &path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("typescript protobuf conversion does not yet support generic carrier"),
        "{error}"
    );
}

#[test]
fn dotnet_converts_generic_proto_carriers() {
    let temp = tempfile::tempdir().unwrap();
    let path = write_generic_carrier_fixture(temp.path());
    let output = generate(Language::Dotnet, &path).unwrap();
    assert!(output.contains("typeof(GenericRequest<>.TransferTypeConverter)"));
    assert!(
        output.contains("if (wire.Details != null && wire.Details.Payloads_.Count > 1)"),
        "{output}"
    );
    assert!(output.contains(
        "throw new System.InvalidOperationException($\"expected at most one payload in GenericRequest.Details, found {wire.Details.Payloads_.Count}\");"
    ));
    assert!(output.contains(
        "Details = wire.Details == null ? default : (wire.Details.Payloads_.Count == 0 ? default! : ProtoExtensions.FromPayloads<TInput>(wire.Details)[0]),"
    ));
    assert!(
        output.contains("proto.Details = ProtoExtensions.ToPayloads(new object?[] { details });")
    );
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

#[test]
fn java_rejects_an_ordinary_reachable_proto_model() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("java-proto.wit");
    fs::write(
        &path,
        r#"package test:java-proto@1.0.0;

world system {
  export api;
}

interface api {
  /// @nexus.proto "temporal.api.common.v1.WorkflowExecution"
  record workflow-execution {
    workflow-id: string,
    run-id: string,
  }
}
"#,
    )
    .unwrap();

    let error = generate(Language::Java, &path).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Java code generation does not support protobuf-backed model"),
        "{error}"
    );
}
