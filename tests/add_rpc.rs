// The `add-rpc` CLI surface lives behind the `advanced` feature.
#![cfg(feature = "advanced")]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use nexgen::add_rpc_to_string;
use nexgen::language::Language;
use nexgen::spec::{ApiSpec, TypeSpec};

const PRIMARY_EXAMPLE_PATH: &str = "advanced/samples/inputs/workflow-service.wit";

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn descriptor_path(root: &std::path::Path) -> PathBuf {
    root.join("advanced/samples/descriptors/temporal_api.bin")
}

fn linked_inputs_path(root: &std::path::Path) -> PathBuf {
    root.join("advanced/samples/inputs/deps")
}

fn linked_inputs(root: &std::path::Path) -> Vec<PathBuf> {
    vec![linked_inputs_path(root)]
}

fn merge_inputs(root: &std::path::Path, input_path: PathBuf) -> Vec<PathBuf> {
    vec![input_path, linked_inputs_path(root)]
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("nexgen-{name}-{unique}"))
}

fn write_temp_wit(name: &str, contents: &str) -> PathBuf {
    let temp_dir = unique_temp_dir(name);
    fs::create_dir_all(&temp_dir).unwrap();
    let path = temp_dir.join("input.wit");
    fs::write(&path, contents).unwrap();
    if contents.contains("dotnet/WorkflowServiceSupport.cs") {
        let dotnet_dir = temp_dir.join("dotnet");
        fs::create_dir_all(&dotnet_dir).unwrap();
        fs::copy(
            project_root().join("advanced/samples/inputs/dotnet/WorkflowServiceSupport.cs"),
            dotnet_dir.join("WorkflowServiceSupport.cs"),
        )
        .unwrap();
    }
    path
}

fn parse(language: Language, wit: &str, path: &str) -> ApiSpec {
    let root = project_root();
    let input_path = write_temp_wit(path, wit);
    nexgen::parser::load_api_spec_from_wit_for_language_with_inputs(
        language,
        &merge_inputs(&root, input_path),
    )
    .unwrap()
}

#[test]
fn cli_add_rpc_generates_standalone_wit_for_signal_with_start() {
    let root = project_root();
    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "add-rpc",
            "--descriptors",
            descriptor_path(&root).to_str().unwrap(),
            "--rpc",
            "SignalWithStartExecution",
            "--input",
            linked_inputs_path(&root).to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let rendered = String::from_utf8(output.stdout).unwrap();

    assert!(rendered.contains("package temporal:nexus@1.0.0;"));
    assert!(rendered.contains("/// @nexus.endpoint \"__REPLACE_ME__\""));
    assert!(rendered.contains("record signal-with-start-workflow-execution-request {"));
    assert!(rendered.contains("workflow-type: option<workflow-type>,"));
    assert!(rendered.contains("input: option<payloads>,"));
    assert!(rendered.contains("signal-name: string,"));
    assert!(rendered.contains("signal-input: option<payloads>,"));
    assert!(rendered.contains(
        "signal-with-start-workflow-execution: func(\n    request: signal-with-start-workflow-execution-request,\n  ) -> signal-with-start-workflow-execution-response;"
    ));
    assert!(!rendered.contains("@nexus.output-transform"));
    assert!(!rendered.contains("workflow: workflow-function,"));
    assert!(!rendered.contains("signal: signal-function,"));
}

#[test]
fn add_rpc_matches_signal_with_start_proto_shape_but_not_handwritten_refinements() {
    let root = project_root();
    let descriptors = descriptor_path(&root);
    let generated = add_rpc_to_string(
        &[descriptors],
        "SignalWithStartExecution",
        &linked_inputs(&root),
    )
    .unwrap();

    let generated_python = parse(Language::Python, &generated, "generated-add-rpc.wit");
    let handwritten_python = nexgen::parser::load_api_spec_from_wit_for_language_with_inputs(
        Language::Python,
        &[root.join(PRIMARY_EXAMPLE_PATH), linked_inputs_path(&root)],
    )
    .unwrap();
    let generated_typescript = parse(Language::TypeScript, &generated, "generated-add-rpc.wit");
    let handwritten_typescript = nexgen::parser::load_api_spec_from_wit_for_language_with_inputs(
        Language::TypeScript,
        &[root.join(PRIMARY_EXAMPLE_PATH), linked_inputs_path(&root)],
    )
    .unwrap();

    let generated_python_service = &generated_python.services[0];
    let handwritten_python_service = &handwritten_python.services[0];
    assert_eq!(
        generated_python_service.name,
        handwritten_python_service.name
    );
    assert_ne!(
        generated_python_service.operations[0].name,
        handwritten_python_service.operations[0].name
    );
    assert_eq!(
        generated_python_service.operations[0].wire_name,
        handwritten_python_service.operations[0].wire_name
    );
    assert_eq!(
        generated_python_service.operations[0]
            .input_type()
            .and_then(TypeSpec::reference),
        handwritten_python_service.operations[0]
            .input_type()
            .and_then(TypeSpec::reference)
    );
    assert_eq!(
        generated_python_service.operations[0]
            .output_type()
            .and_then(|output| output.reference()),
        handwritten_python_service.operations[0]
            .output_type()
            .and_then(|output| output.reference())
    );
    assert!(
        generated_python_service.operations[0]
            .output_transform
            .is_none()
    );
    assert!(
        handwritten_python_service.operations[0]
            .output_transform
            .is_some()
    );

    let generated_python_request = &generated_python
        .record_for_proto("temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest")
        .unwrap();
    let handwritten_python_request = &handwritten_python
        .record_for_proto("temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest")
        .unwrap();
    assert_eq!(
        generated_python_request.field_name_override("workflow_type"),
        Some("workflow-type")
    );
    assert_eq!(
        generated_python_request.field_name_override("signal_name"),
        Some("signal-name")
    );
    assert_eq!(
        generated_python_request.field_name_override("input"),
        Some("input")
    );
    assert_eq!(
        generated_python_request.field_name_override("signal_input"),
        Some("signal-input")
    );
    assert!(generated_python_request.function("workflow_type").is_none());
    assert!(generated_python_request.function("signal_name").is_none());
    assert_ne!(
        generated_python_request.field_name_override("workflow_type"),
        handwritten_python_request.field_name_override("workflow_type")
    );
    assert_ne!(
        generated_python_request.field_name_override("signal_name"),
        handwritten_python_request.field_name_override("signal_name")
    );
    assert_ne!(
        generated_python_request.function("workflow_type"),
        handwritten_python_request.function("workflow_type")
    );

    let generated_typescript_request = &generated_typescript
        .record_for_proto("temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest")
        .unwrap();
    let handwritten_typescript_request = &handwritten_typescript
        .record_for_proto("temporal.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest")
        .unwrap();
    assert_eq!(
        generated_typescript_request.field_name_override("workflow_type"),
        Some("workflow-type")
    );
    assert_eq!(
        generated_typescript_request.field_name_override("signal_name"),
        Some("signal-name")
    );
    assert_eq!(
        generated_typescript_request.field_name_override("input"),
        Some("input")
    );
    assert_eq!(
        generated_typescript_request.field_name_override("signal_input"),
        Some("signal-input")
    );
    assert!(
        generated_typescript_request
            .function("workflow_type")
            .is_none()
    );
    assert!(
        generated_typescript_request
            .function("signal_name")
            .is_none()
    );
    assert_ne!(
        generated_typescript_request.field_name_override("workflow_type"),
        handwritten_typescript_request.field_name_override("workflow_type")
    );
    assert_ne!(
        generated_typescript_request.field_name_override("signal_name"),
        handwritten_typescript_request.field_name_override("signal_name")
    );
    assert_ne!(
        generated_typescript_request.function("signal_name"),
        handwritten_typescript_request.function("signal_name")
    );

    assert!(generated.contains("user-metadata: option<user-metadata>,"));
    assert!(generated.contains("header: option<header>,"));
    assert!(generated.contains("links: list<link>,"));
    assert!(!generated.contains("@nexus.source"));
}

#[test]
fn add_rpc_can_extend_an_existing_wit_file() {
    let root = project_root();
    let descriptors = descriptor_path(&root);
    let input = root.join(PRIMARY_EXAMPLE_PATH);
    let generated = add_rpc_to_string(
        &[descriptors],
        "SignalWorkflowExecution",
        &merge_inputs(&root, input),
    )
    .unwrap();

    assert!(generated.contains("signal-with-start-workflow: func("));
    assert!(generated.contains("signal-workflow-execution: func("));
    assert!(generated.contains("record signal-workflow-execution-request {"));
    assert!(generated.contains("record signal-workflow-execution-response {"));
    assert!(generated.contains("signal-name: string,"));
    assert!(generated.contains("input: option<payloads>,"));
    assert!(generated.contains("workflow-execution: option<workflow-execution>,"));
    assert!(generated.contains(
        "/// @nexus.endpoint \"temporal-system\" go=\"__temporal_system\" python=\"__temporal_system\""
    ));
    assert!(!generated.contains("/// @nexus.endpoint \"__REPLACE_ME__\""));
    assert!(!generated.contains("package temporal:nexus@1.0.0;\n\npackage temporal:nexus@1.0.0;"));

    let parsed = parse(Language::Python, &generated, "extended-input.wit");
    let service = &parsed.services[0];
    assert!(service.operation("SignalWithStartWorkflow").is_some());
    assert!(service.operation("SignalWorkflowExecution").is_some());
}

#[test]
fn add_rpc_can_update_existing_signal_with_start_operation() {
    let root = project_root();
    let descriptors = descriptor_path(&root);
    let input = root.join(PRIMARY_EXAMPLE_PATH);
    let generated = add_rpc_to_string(
        &[descriptors],
        "SignalWithStartExecution",
        &merge_inputs(&root, input),
    )
    .unwrap();

    assert!(generated.contains("signal-with-start-workflow: func("));
    assert_eq!(
        generated
            .matches("signal-with-start-workflow: func(")
            .count(),
        1
    );
    assert!(generated.contains("workflow: workflow-function,"));
    assert!(generated.contains("signal: signal-function,"));
    assert!(!generated.contains("input: option<payloads>,"));
    assert!(!generated.contains("signal-input: option<payloads>,"));
    assert!(generated.contains("time-skipping-config: placeholder,"));
    assert!(generated.contains("signal-link: placeholder,"));
    assert!(!generated.contains("record link-workflow-event"));
    assert!(!generated.contains("record link-batch-job"));
    assert!(!generated.contains("record link-activity"));
    assert!(!generated.contains("record link-nexus-operation"));
    assert!(!generated.contains("record link-workflow"));
    assert!(!generated.contains("record link {"));
    assert!(generated.contains(
        "/// @nexus.endpoint \"temporal-system\" go=\"__temporal_system\" python=\"__temporal_system\""
    ));
    assert!(!generated.contains("/// @nexus.endpoint \"__REPLACE_ME__\""));

    let parsed = parse(Language::Python, &generated, "updated-existing-input.wit");
    let service = &parsed.services[0];
    let operation = service.operation("SignalWithStartWorkflow").unwrap();
    assert_eq!(operation.wire_name, "SignalWithStartWorkflowExecution");
}

#[test]
fn add_rpc_adds_missing_field_to_existing_operation_request() {
    let root = project_root();
    let descriptors = descriptor_path(&root);
    let complete = add_rpc_to_string(
        &[descriptors.clone()],
        "SignalWorkflowExecution",
        &linked_inputs(&root),
    )
    .unwrap();
    let input = complete.replace("    input: option<payloads>,\n", "");
    let input_path = write_temp_wit("add-rpc-existing-partial", &input);

    let generated = add_rpc_to_string(
        &[descriptors.clone()],
        "SignalWorkflowExecution",
        &merge_inputs(&root, input_path.clone()),
    )
    .unwrap();

    assert!(generated.contains("signal-workflow-execution: func("));
    assert!(generated.contains("input: option<payloads>,"));
    assert_eq!(generated.matches("input: option<payloads>,").count(), 1);

    let parsed = parse(Language::Python, &generated, input_path.to_str().unwrap());
    let request = &parsed
        .record_for_proto("temporal.api.workflowservice.v1.SignalWorkflowExecutionRequest")
        .unwrap();
    assert_eq!(request.field_name_override("input"), Some("input"));

    fs::remove_dir_all(input_path.parent().unwrap()).unwrap();
}

#[test]
fn add_rpc_allows_existing_required_field_when_descriptor_field_is_optional() {
    let root = project_root();
    let descriptors = descriptor_path(&root);
    let complete = add_rpc_to_string(
        &[descriptors.clone()],
        "SignalWorkflowExecution",
        &linked_inputs(&root),
    )
    .unwrap();
    let input = complete.replace("    input: option<payloads>,\n", "    input: payloads,\n");
    let input_path = write_temp_wit("add-rpc-existing-required-tightening", &input);

    let generated = add_rpc_to_string(
        &[descriptors],
        "SignalWorkflowExecution",
        &merge_inputs(&root, input_path.clone()),
    )
    .unwrap();

    assert!(generated.contains("input: payloads,"));
    assert!(!generated.contains("input: option<payloads>,"));

    let parsed = parse(Language::Python, &generated, input_path.to_str().unwrap());
    let request = &parsed
        .record_for_proto("temporal.api.workflowservice.v1.SignalWorkflowExecutionRequest")
        .unwrap();
    assert_eq!(request.field_name_override("input"), Some("input"));

    fs::remove_dir_all(input_path.parent().unwrap()).unwrap();
}

#[test]
fn add_rpc_allows_existing_optional_field_when_descriptor_field_is_required() {
    let root = project_root();
    let descriptors = descriptor_path(&root);
    let complete = add_rpc_to_string(
        &[descriptors.clone()],
        "SignalWorkflowExecution",
        &linked_inputs(&root),
    )
    .unwrap();
    let input = complete.replace("    identity: string,\n", "    identity: option<string>,\n");
    let input_path = write_temp_wit("add-rpc-existing-optional-relaxation", &input);

    let generated = add_rpc_to_string(
        &[descriptors],
        "SignalWorkflowExecution",
        &merge_inputs(&root, input_path.clone()),
    )
    .unwrap();

    assert!(generated.contains("identity: option<string>,"));
    assert!(!generated.contains("identity: string,"));

    let parsed = parse(Language::Python, &generated, input_path.to_str().unwrap());
    let request = &parsed
        .record_for_proto("temporal.api.workflowservice.v1.SignalWorkflowExecutionRequest")
        .unwrap();
    assert_eq!(request.field_name_override("identity"), Some("identity"));

    fs::remove_dir_all(input_path.parent().unwrap()).unwrap();
}

#[test]
fn add_rpc_fails_when_existing_operation_request_conflicts_with_descriptor() {
    let root = project_root();
    let descriptors = descriptor_path(&root);
    let complete = add_rpc_to_string(
        &[descriptors.clone()],
        "SignalWorkflowExecution",
        &linked_inputs(&root),
    )
    .unwrap();
    let input = complete.replace("    signal-name: string,\n", "    signal-name: bool,\n");
    let input_path = write_temp_wit("add-rpc-existing-conflict", &input);

    let error = add_rpc_to_string(
        &[descriptors],
        "SignalWorkflowExecution",
        &merge_inputs(&root, input_path.clone()),
    )
    .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("existing WIT field is `signal-name: bool`"));
    assert!(message.contains("descriptor requires `signal-name: string`"));

    fs::remove_dir_all(input_path.parent().unwrap()).unwrap();
}
