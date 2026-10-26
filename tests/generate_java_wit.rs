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
    assert!(operations.contains(" first(io.temporal.workflow.FirstOptions options)"));
    assert!(operations.contains(" second(io.temporal.workflow.SecondOptions options)"));
    assert!(operations.contains("static io.temporal.workflow.Request executeFirst("));
    assert!(!operations.contains("public static io.temporal.workflow.Request executeFirst("));
    let options = fs::read_to_string(output.join("FirstOptions.java")).unwrap();
    assert!(options.contains("setValue(String value)"));
    assert!(options.contains("java.util.Objects.requireNonNull(value, \"value\")"));
    assert!(!operations.contains("Functions.Proc1<"));
}

#[test]
fn dynamic_endpoint_and_required_values_live_in_options_without_name_collisions() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("service.wit");
    fs::write(
        &input,
        r#"
package example:routing;
world system { export router; }
interface router {
  record request { endpoint: string, endpoint2: string, value: string, }
  dispatch: func(request: request) -> request;
}
"#,
    )
    .unwrap();
    let output = temp.path().join("generated");
    generate_to_file(&GenerateRequest {
        config: NexgenConfig {
            mode: GenerationMode::NativeApi,
            system_nexus: false,
        },
        language: Language::Java,
        input_paths: vec![input],
        descriptor_paths: vec![],
        support_package: Some("example.Support".into()),
        java_package_name: Some("example.generated".into()),
        output_path: output.clone(),
        format: false,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let operations = fs::read_to_string(output.join("RouterOperations.java")).unwrap();
    let options = fs::read_to_string(output.join("DispatchOptions.java")).unwrap();
    assert!(operations.contains("dispatch(example.generated.DispatchOptions options)"));
    assert!(operations.contains("String endpoint = options.getEndpoint3()"));
    assert!(operations.contains("requestBuilder.setEndpoint(options.getEndpoint())"));
    assert!(operations.contains("requestBuilder.setValue(options.getValue())"));
    for field in ["endpoint", "endpoint2", "endpoint3", "value"] {
        assert!(options.contains(&format!(
            "java.util.Objects.requireNonNull({field}, \"{field}\")"
        )));
        assert!(options.contains(&format!("private String {field} = null")));
    }
}

#[test]
fn operation_api_keeps_only_functions_and_their_inputs_outside_required_options() {
    let temp = generate("workflow-service", GenerationMode::NativeApi, true);
    let output = temp.path().join("generated");
    let operations = fs::read_to_string(output.join("WorkflowOperations.java")).unwrap();
    assert!(operations.contains(
        "signalWithStartWorkflow(String workflow, Object[] args, String signal, Object[] signalArgs, test.generated.SignalWithStartWorkflowOptions options)"
    ));
    assert!(operations.contains(
        "String signal, Object[] signalArgs, test.generated.SignalWithStartWorkflowOptions options)"
    ));
    assert!(!operations.contains(
        "public static io.temporal.workflow.ExternalWorkflowStub executeSignalWithStartWorkflow("
    ));
    assert!(operations.contains("builder.setStaticSummary(options.getStaticSummary())"));
    assert!(operations.contains("Functions.Func<WorkflowResult> workflow"));
    assert!(operations.contains("Functions.Func6<WorkflowArg1, WorkflowArg2, WorkflowArg3, WorkflowArg4, WorkflowArg5, WorkflowArg6, WorkflowResult> workflow"));
    assert!(operations.contains("Functions.Proc6<WorkflowArg1, WorkflowArg2, WorkflowArg3, WorkflowArg4, WorkflowArg5, WorkflowArg6> workflow"));
    assert!(operations.contains("Functions.Proc1<SignalArg1> signal, SignalArg1 signalArg1"));
    assert!(!operations.contains("Functions.Func7<"));
    assert!(!operations.contains("Functions.Proc2<SignalArg"));
    assert!(!operations.contains("FunctionCall"));
    assert!(!operations.contains("Class<WorkflowInterface>"));
    assert!(!output.join("FunctionCall.java").exists());
    assert!(operations.contains("support.TemporalModelConverters.captureWorkflowCall("));
    assert!(operations.contains("support.TemporalModelConverters.captureSignalCall("));
    assert!(operations.contains(
        "ProbeResult workflowMetadata = support.TemporalModelConverters.captureWorkflowCall("
    ));
    assert!(operations.contains("requestBuilder.setRetryPolicy(support.TemporalModelConverters.mergeMethodRetry(workflowMetadata, options.getRetryPolicy()))"));
    assert!(operations.contains("requestBuilder.setCronSchedule(support.TemporalModelConverters.mergeCronSchedule(workflowMetadata, options.getCronSchedule()))"));
    assert!(operations.contains("java.util.Objects.requireNonNull(options, \"options\")"));
    assert!(!operations.contains("String id,"));
    assert!(!operations.contains("String taskQueue,"));
    let untyped = operations
        .split("ProbeResult workflowMetadata")
        .next()
        .unwrap();
    assert!(!untyped.contains("mergeMethodRetry("));
    assert!(!untyped.contains("mergeCronSchedule("));
    assert!(
        operations.contains("requestBuilder.setArgs(java.util.Arrays.asList(capturedArguments))")
    );
    assert!(
        operations
            .contains("requestBuilder.setSignalArgs(java.util.Arrays.asList(capturedArguments))")
    );
    assert!(!operations.contains("must be supplied through the bound call, not options"));
    assert!(operations.contains("requestBuilder.setArgs(java.util.Arrays.asList(java.util.Objects.requireNonNull(args, \"args\")))"));
    assert!(operations.contains("requestBuilder.setSignalArgs(java.util.Arrays.asList(java.util.Objects.requireNonNull(signalArgs, \"signalArgs\")))"));
    assert!(!operations.contains("requestBuilder.setSignalInput("));
    let options = fs::read_to_string(output.join("SignalWithStartWorkflowOptions.java")).unwrap();
    for expected in [
        "setStaticSummary(String value)",
        "setStaticDetails(String value)",
        "setId(String value)",
        "setTaskQueue(String value)",
        "java.util.Objects.requireNonNull(taskQueue, \"taskQueue\")",
        "private String id = null",
        "toBuilder()",
        "WorkflowIdReusePolicy.forNumber(1)",
    ] {
        assert!(
            options.contains(expected),
            "missing option behavior: {expected}"
        );
    }
    for hidden in [
        "java.util.Objects.requireNonNull(id, \"id\")",
        "getArgs()",
        "getSignalArgs()",
        "setArgs(",
        "setSignalArgs(",
        "getWorkflow()",
        "getDefaultInstance()",
        "getSignal()",
        "getNamespace()",
        "getRequestId()",
        "getHeaders()",
        "getUserMetadata()",
        "TransferConverter",
        "JsonCreator",
    ] {
        assert!(
            !options.contains(hidden),
            "request-only field leaked into options: {hidden}"
        );
    }
    let definitions = generate("workflow-service", GenerationMode::DefinitionsOnly, true);
    assert!(
        !definitions
            .path()
            .join("generated/SignalWithStartWorkflowOptions.java")
            .exists()
    );
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
fn bound_calls_follow_idl_names_without_operation_specific_policy() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("dispatch.wit");
    let source = fs::read_to_string(root.join("advanced/samples/inputs/workflow-service.wit"))
        .unwrap()
        .replace("signal-with-start-workflow", "dispatch")
        .replace("workflow: workflow-function", "origin: workflow-function")
        .replace("function=\"workflow\"", "function=\"origin\"")
        .replace("signal: signal-function", "destination: signal-function");
    fs::write(&input, source).unwrap();
    let output = temp.path().join("generated");
    generate_to_file(&GenerateRequest {
        config: NexgenConfig {
            mode: GenerationMode::NativeApi,
            system_nexus: false,
        },
        language: Language::Java,
        input_paths: vec![input, root.join("advanced/samples/inputs/deps")],
        descriptor_paths: vec![root.join("advanced/samples/descriptors/temporal_api.bin")],
        support_package: Some("example.Support".into()),
        java_package_name: Some("example.generated".into()),
        output_path: output.clone(),
        format: false,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let operations = fs::read_to_string(output.join("WorkflowOperations.java")).unwrap();
    assert!(operations.contains(" dispatch("));
    assert!(
        operations
            .contains("Functions.Func1<OriginArg1, WorkflowResult> origin, OriginArg1 originArg1")
    );
    assert!(
        operations.contains(
            "Functions.Proc1<DestinationArg1> destination, DestinationArg1 destinationArg1"
        )
    );
    assert!(operations.contains("requestBuilder.setOrigin(capturedName)"));
    assert!(operations.contains("requestBuilder.setDestination(capturedName)"));
    assert!(operations.contains("mergeMethodRetry(originMetadata,"));
    assert!(operations.contains("mergeCronSchedule(originMetadata,"));
    assert!(!operations.contains("WithStartWorkflowOperation"));
    assert!(!operations.contains("signalArg1"));
}

#[test]
fn function_merge_annotations_are_selected_only_for_java() {
    use nexgen::parser::load_api_spec_from_wit_for_language_with_inputs;
    use nexgen::spec::TypeDeclSpec;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let paths = vec![
        root.join("advanced/samples/inputs/workflow-service.wit"),
        root.join("advanced/samples/inputs/deps"),
    ];
    for language in [
        Language::Java,
        Language::Python,
        Language::TypeScript,
        Language::Go,
        Language::Dotnet,
    ] {
        let spec = load_api_spec_from_wit_for_language_with_inputs(language, &paths).unwrap();
        let request = spec
            .types
            .values()
            .find_map(|entry| match &entry.declaration {
                TypeDeclSpec::Record(record) if record.fields.contains_key("retry_policy") => {
                    Some(record)
                }
                _ => None,
            })
            .unwrap();
        let rule = &request.fields["retry_policy"].merge_from_function;
        assert_eq!(rule.is_some(), language == Language::Java);
        if let Some(rule) = rule {
            assert_eq!(rule.function, "workflow_type");
            assert_eq!(rule.helper, "mergeMethodRetry");
        }
        assert_eq!(
            request.fields["workflow_type"]
                .function
                .as_ref()
                .unwrap()
                .call_metadata_type
                .is_some(),
            language == Language::Java
        );
    }
}

#[test]
fn invalid_function_merge_directives_report_actionable_errors() {
    use nexgen::parser::load_api_spec_from_wit_for_language_with_inputs;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source =
        fs::read_to_string(root.join("advanced/samples/inputs/workflow-service.wit")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("service.wit");
    for (from, to, expected) in [
        (
            "function=\"workflow\"",
            "function=\"missing\"",
            "unknown function field",
        ),
        (
            "function=\"workflow\"",
            "function=\"id\"",
            "is not a function field",
        ),
        (
            "function=\"workflow\"",
            "function=\"\"",
            "requires `function`",
        ),
        (
            "java=\"mergeMethodRetry\"",
            "java=\"bad helper()\"",
            "valid `java` support helper",
        ),
        (
            "java=\"mergeMethodRetry\"",
            "java=\"mergeMethodRetry\" python=\"merge_retry\"",
            "supports Java only",
        ),
    ] {
        fs::write(&input, source.replace(from, to)).unwrap();
        let error = load_api_spec_from_wit_for_language_with_inputs(
            Language::Java,
            &[input.clone(), root.join("advanced/samples/inputs/deps")],
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("merge-from-function") && error.contains(expected),
            "{error}"
        );
    }
    let minimal = r#"
package example:merges;
world system { export service; }
interface service {
  record request {
    /// @nexus.function args-field="input" java-result="Void" java-call-extractor="probe"
    target: string,
    input: option<list<string>>,
    /// @nexus.merge-from-function function="target" java="mergeValue"
    value: option<string>,
  }
  run: func(request: request);
}
"#;
    fs::write(&input, minimal).unwrap();
    let error = load_api_spec_from_wit_for_language_with_inputs(Language::Java, &[input])
        .unwrap_err()
        .to_string();
    assert!(error.contains("call metadata type"), "{error}");
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
