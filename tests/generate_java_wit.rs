use std::fs;
use std::path::PathBuf;

use nexgen::generator::GenerationMode;
use nexgen::language::Language;
use nexgen::nexgen_config::NexgenConfig;
use nexgen::{GenerateRequest, generate_to_file};

fn generate(example: &str, mode: GenerationMode, system_nexus: bool) -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::tempdir().unwrap();
    generate_to_file(&GenerateRequest {
        config: NexgenConfig { mode, system_nexus },
        language: Language::Java,
        input_paths: vec![
            root.join(format!("advanced/samples/inputs/{example}.wit")),
            root.join("advanced/samples/inputs/deps"),
        ],
        descriptor_paths: vec![root.join("advanced/samples/descriptors/temporal_api.bin")],
        support_package: Some("support.TemporalModelConverters".into()),
        java_package_name: Some("test.generated".into()),
        output_path: temp.path().join("generated"),
        format: false,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    temp
}

#[test]
fn system_envelope_roots_include_payload_free_operation_responses() {
    let temp = generate("workflow-service", GenerationMode::NativeApi, true);
    let output = temp.path().join("generated");
    let roots = fs::read_to_string(output.join("system-nexus-roots.txt")).unwrap();
    assert_eq!(
        roots.lines().collect::<Vec<_>>(),
        vec![
            "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest",
            "temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionResponse",
        ]
    );
    let interceptor =
        fs::read_to_string(output.join("SystemNexusWorkflowOutboundInterceptor.java")).unwrap();
    assert!(interceptor.contains("executeSignalWithStartWorkflow"));
    assert!(interceptor.contains("nextSystemNexusInterceptor()"));
    let registry = fs::read_to_string(output.join("SystemNexusOperations.java")).unwrap();
    assert!(registry.contains("signalWithStartWorkflowSerializationContext"));
}

#[test]
fn named_operations_class_exposes_new_operations_through_an_sdk_facade() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("service.wit");
    fs::write(
        &input,
        r#"
package example:service;
world system { export example-service; }
/// @nexus.endpoint "system"
/// @nexus.operations-class java="WorkflowOperations"
interface example-service {
  record request { value: string, }
  first: func(request: request) -> request;
  second: func(request: request) -> request;
}
"#,
    )
    .unwrap();
    let output = temp.path().join("workflow");
    generate_to_file(&GenerateRequest {
        config: NexgenConfig {
            mode: GenerationMode::NativeApi,
            system_nexus: false,
        },
        language: Language::Java,
        input_paths: vec![input],
        descriptor_paths: vec![],
        support_package: Some("example.Converters".into()),
        java_package_name: Some("io.temporal.workflow".into()),
        output_path: output.clone(),
        format: false,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let operations = fs::read_to_string(output.join("WorkflowOperations.java")).unwrap();
    assert!(operations.contains("package io.temporal.workflow;"));
    assert!(operations.contains("public abstract class WorkflowOperations"));
    assert!(operations.contains("protected WorkflowOperations()"));
    assert!(operations.contains(" first("));
    assert!(operations.contains(" second("));
    assert!(!output.join("ExampleServiceOperations.java").exists());
}

#[test]
fn every_wit_fixture_generates_in_both_modes() {
    for example in [
        "function-execution",
        "generic-models",
        "multi-operation-service",
        "notification-service",
        "proto-generic",
        "proto-oneof",
        "start-workflow",
        "type-roundtrip",
        "type-showcase",
        "user-service",
        "workflow-service",
    ] {
        for mode in [GenerationMode::DefinitionsOnly, GenerationMode::NativeApi] {
            let temp = generate(example, mode, false);
            assert!(
                fs::read_dir(temp.path().join("generated"))
                    .unwrap()
                    .next()
                    .is_some(),
                "{example}: {mode:?}"
            );
            assert!(
                !temp
                    .path()
                    .join("generated/system-nexus-roots.txt")
                    .exists()
            );
        }
    }
}

#[test]
fn runtime_contains_only_features_used_by_the_generated_bindings() {
    let temp = generate("workflow-service", GenerationMode::NativeApi, true);
    let output = temp.path().join("generated");
    let runtime = fs::read_to_string(output.join("NexgenRuntime.java")).unwrap();
    assert!(runtime.contains("decodeSequence"));
    for unused in [
        "ThreadLocal",
        "withDataConverter",
        "DefaultDataConverter",
        "FunctionReference",
        "class Result",
        "interface Flag",
        "parameterized",
        "argument(",
        "getField(",
        "setField(",
        "mapValues",
        "protoMap",
    ] {
        assert!(
            !runtime.contains(unused),
            "unneeded runtime feature: {unused}"
        );
    }
    let request = fs::read_to_string(output.join("SignalWithStartWorkflowRequest.java")).unwrap();
    assert!(request.contains("proto.setWorkflowId("));
    assert!(request.contains("proto.getWorkflowId()"));
    assert!(request.contains("support.TemporalModelConverters.currentDataConverter()"));
    let json = generate("user-service", GenerationMode::NativeApi, false);
    assert!(!json.path().join("generated/NexgenRuntime.java").exists());
}
