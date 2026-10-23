// Drives the `nexgen` binary over the WIT/proto CLI surface (`--descriptors`,
// `--native-api`, `--support-package`), all behind the `advanced` feature.
#![cfg(feature = "advanced")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nexgen::generator::generate_source;
use nexgen::{GenerateRequest, generate_to_file};

mod common;
use common::{json_input_path, wit_input_path};

const PRIMARY_EXAMPLE_ID: &str = "workflow-service";
const TYPE_ROUNDTRIP_EXAMPLE_ID: &str = "type-roundtrip";
static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A property whose union has one inline structured object branch (named
/// `<Union>Object`) and one scalar branch.
/// Unions in positions with no property of their own: an array element (inline
/// and `$ref`), a map member (inline), plus a nullable element for contrast.
const ELEMENT_UNION_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  segments:
    type: array
    items:
      oneOf:
        - { type: string }
        - { type: integer }
  choices:
    type: array
    items: { $ref: "#/$defs/Choice" }
  entries: { $ref: "#/$defs/Entries" }
  slots:
    type: array
    items:
      oneOf:
        - { type: string }
        - { type: "null" }
$defs:
  Choice:
    oneOf:
      - { type: string }
      - { type: boolean }
  Entries:
    type: object
    additionalProperties:
      oneOf:
        - { type: string }
        - { type: integer }
"##;

const INLINE_OBJECT_BRANCH_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  payload:
    oneOf:
      - type: object
        required: [text]
        properties:
          text: { type: string, minLength: 1 }
      - { type: string }
"#;

/// A union whose **non-object** branches each declare constraints of their own:
/// once the wire token selects a branch, the value is held to everything that
/// branch declares — including a closed value set — in both directions.
const BRANCH_CONSTRAINT_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string, minLength: 3, pattern: "^[a-z]+$" }
      - { type: integer, minimum: 1 }
  listOrName:
    oneOf:
      - { type: array, items: { type: number }, minItems: 1, uniqueItems: true }
      - { type: string, enum: [auto, manual] }
"#;

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn descriptor_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/descriptors/temporal_api.bin")
}

fn linked_inputs_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/inputs/deps")
}

fn example_input_paths(root: &Path, example_id: &str) -> Vec<PathBuf> {
    let input = wit_input_path(root, example_id);
    let mut paths = vec![input.clone()];
    if fs::read_to_string(&input)
        .unwrap()
        .contains("use nexus:temporal-types/")
    {
        paths.push(linked_inputs_path(root));
    }
    paths
}

fn read_python_package_files(dir: &Path) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, files);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("py") {
                if path
                    .file_name()
                    .and_then(|file_name| file_name.to_str())
                    .is_some_and(|file_name| file_name.starts_with("test_"))
                {
                    continue;
                }
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read_to_string(&path).unwrap(),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(dir, dir, &mut files);
    files
}

fn render_output_files(files: BTreeMap<PathBuf, String>) -> String {
    files
        .into_iter()
        .map(|(path, contents)| format!("### {}\n{contents}", path.display()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn generate_python_to_string(input_paths: &[PathBuf], descriptor_paths: &[PathBuf]) -> String {
    let temp_dir = unique_output_path("python-rendered");
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            mode: nexgen::generator::GenerationMode::NativeApi,
            ..Default::default()
        },
        language: nexgen::language::Language::Python,
        input_paths: input_paths.to_vec(),
        support_package: Some("temporal_support".into()),
        descriptor_paths: descriptor_paths.to_vec(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = if output_path.is_file() {
        fs::read_to_string(&output_path).unwrap()
    } else {
        render_output_files(read_python_package_files(&output_path))
    };
    fs::remove_dir_all(temp_dir).unwrap();
    rendered
}

fn generate_python_package_files(
    input_paths: &[PathBuf],
    descriptor_paths: &[PathBuf],
) -> BTreeMap<PathBuf, String> {
    let temp_dir = unique_output_path("python-package");
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Python,
        input_paths: input_paths.to_vec(),
        support_package: Some("temporal_support".into()),
        descriptor_paths: descriptor_paths.to_vec(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let files = read_python_package_files(&output_path);
    fs::remove_dir_all(temp_dir).unwrap();
    files
}

fn unique_output_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("nexgen-{label}-{unique}-{counter}"))
}

#[test]
fn cli_generates_wit_direct_example_without_descriptors() {
    let root = project_root();
    let output_path = unique_output_path("python-user-service-no-descriptors");
    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "python",
            wit_input_path(&root, "user-service").to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
            "--support-package",
            "temporal_support",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_path.join("__init__.py").is_file());
    assert!(output_path.join("models.py").is_file());
    fs::remove_dir_all(output_path).unwrap();
}

#[test]
fn cli_requires_support_package_for_wit_but_not_json_schema() {
    let root = project_root();
    let output_path = unique_output_path("python-support-package-requirement");
    let wit = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "python",
            wit_input_path(&root, "user-service").to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!wit.status.success());
    assert!(String::from_utf8_lossy(&wit.stderr).contains("--support-package"));
    assert!(!output_path.exists());

    let json = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "python",
            json_input_path(&root, "chat").to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        json.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&json.stderr)
    );
    assert!(output_path.join("models.py").is_file());
    fs::remove_dir_all(output_path).unwrap();
}

#[test]
fn cli_defaults_to_definitions_without_native_api_or_endpoint() {
    let temp_dir = unique_output_path("python-definitions-only");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("input.wit");
    let output_path = temp_dir.join("output");
    fs::write(
        &input_path,
        r#"
package temporal:example@1.0.0;

world system {
  export example-service;
}

interface example-service {
  record request {
    name: string,
  }

  record response {
    message: string,
  }

  example-operation: func(request: request) -> response;
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "python",
            input_path.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
            "--support-package",
            "temporal_support",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_path.join("__init__.py").is_file());
    assert!(output_path.join("models.py").is_file());
    assert!(output_path.join("services.py").is_file());
    assert!(!output_path.join("operations/example_operation.py").exists());

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn python_request_models_are_bidirectional_wire_models() {
    let root = project_root();
    let package = generate_python_package_files(
        &example_input_paths(&root, PRIMARY_EXAMPLE_ID),
        &[descriptor_path(&root)],
    );
    let models = package
        .get(&PathBuf::from("models.py"))
        .expect("Python package should include models.py");
    let rendered = generate_python_to_string(
        &example_input_paths(&root, PRIMARY_EXAMPLE_ID),
        &[descriptor_path(&root)],
    );

    assert!(!rendered.contains("SignalWithStartWorkflowRequest.from_proto"));
    assert!(!rendered.contains(
        "proto: temporalio.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest,\n    ) -> SignalWithStartWorkflowRequest:"
    ));
    assert!(rendered.contains("class SignalWithStartWorkflowRequest:"));
    assert!(rendered.contains(
        "@dataclasses.dataclass(slots=True, kw_only=True)\nclass SignalWithStartWorkflowRequest:\n    \"\"\"Request fields for signaling a workflow, starting it first if needed.\n\n    .. warning::\n        This API is experimental and subject to change.\n    \"\"\"\n    workflow: str | collections.abc.Callable[..., collections.abc.Awaitable[object]]\n    args: list[typing.Any] | None = None\n    id: str\n    task_queue: str\n    signal: str | collections.abc.Callable[..., None | collections.abc.Awaitable[None]]\n    signal_args: list[typing.Any] | None = None\n    execution_timeout: datetime.timedelta | None = None"
    ));
    assert!(rendered.contains("temporalio.common.WorkflowIDReusePolicy.ALLOW_DUPLICATE"));
    assert!(rendered.contains("args: list[typing.Any] | None = None"));
    assert!(rendered.contains("signal_args: list[typing.Any] | None = None"));
    assert!(!rendered.contains("args: tuple[typing.Any, ...] | None = None"));
    assert!(!rendered.contains("signal_args: tuple[typing.Any, ...] | None = None"));
    assert!(!rendered.contains("(typing.TypedDict, total=False):"));
    assert!(!rendered.contains("typing.Unpack["));
    assert!(!rendered.contains("namespace: str | None = None"));
    assert!(!rendered.contains("namespace: str | None"));
    assert!(rendered.contains("namespace: str = ("));
    assert!(rendered.contains("dataclasses.field(default_factory=_support.workflow_namespace)"));
    assert!(rendered.contains("message.namespace = value.namespace"));
    assert!(rendered.contains("result = await handle"));
    assert!(rendered.contains(
        "from temporalio.workflow import (\n        create_nexus_client,\n        get_external_workflow_handle,\n    )"
    ));
    assert!(rendered.contains("return get_external_workflow_handle("));
    assert!(rendered.contains("run_id=result.run_id"));
    assert!(rendered.contains("async def _signal_with_start_workflow("));
    assert!(rendered.contains("request: SignalWithStartWorkflowRequest"));
    assert!(rendered.contains(
        "if typing.TYPE_CHECKING:\n    from temporalio.workflow import ExternalWorkflowHandle"
    ));
    assert!(rendered.contains(") -> ExternalWorkflowHandle[object]:"));
    assert!(rendered.contains("    nexus_client = create_nexus_client("));
    assert!(rendered.contains("async def signal_with_start_workflow("));
    assert!(rendered.contains("@typing.overload"));
    assert!(rendered.contains("workflow: str,"));
    assert!(rendered.contains("*args: object,"));
    assert!(rendered.contains("id: str,"));
    assert!(rendered.contains("args: list[typing.Any] | None = ...,"));
    assert!(rendered.contains(
        "workflow: collections.abc.Callable[[SelfType, typing_extensions.Unpack[WorkflowArgs]], collections.abc.Awaitable[WorkflowResult]],"
    ));
    assert!(rendered.contains("WorkflowArgs = typing_extensions.TypeVarTuple(\"WorkflowArgs\")"));
    assert!(rendered.contains("WorkflowResult = typing.TypeVar(\"WorkflowResult\")"));
    assert!(rendered.contains("SelfType = typing.TypeVar(\"SelfType\")"));
    assert!(rendered.contains(") -> ExternalWorkflowHandle[SelfType]:"));
    assert!(rendered.contains("*args: typing_extensions.Unpack[WorkflowArgs],"));
    assert!(rendered.contains("args: list[typing.Any],"));
    assert!(!rendered.contains("tuple[FirstWorkflowArg"));
    assert!(rendered.contains(
        "signal: collections.abc.Callable[[SelfType, SignalArg], None | collections.abc.Awaitable[None]],"
    ));
    assert!(rendered.contains("SignalArg = typing.TypeVar(\"SignalArg\")"));
    assert!(rendered.contains("signal_args: SignalArg,"));
    assert!(rendered.contains("signal_args: list[typing.Any],"));
    assert!(rendered.contains(
        "async def signal_with_start_workflow(\n    workflow: str | collections.abc.Callable[..., collections.abc.Awaitable[object]],\n    *positional_args: object,\n    args: list[typing.Any] | None = None,\n    id: str,\n    task_queue: str,\n    signal: str | collections.abc.Callable[..., None | collections.abc.Awaitable[None]],\n    signal_args: object | list[typing.Any] | None = None,"
    ));
    assert!(rendered.contains(
        "signal: str | collections.abc.Callable[..., None | collections.abc.Awaitable[None]],"
    ));
    assert!(rendered.contains("args: list[typing.Any] | None = None,"));
    assert_eq!(
        rendered
            .matches("Signal a workflow, starting it first if needed.")
            .count(),
        1
    );
    assert!(rendered.contains(
        "\"\"\"Signal a workflow, starting it first if needed.\n\n    .. warning::\n        This API is experimental and subject to change.\n\n    Args:\n        workflow: Workflow type name or callable identifying the workflow to start.\n        positional_args: Positional arguments for workflow. Cannot be set if args is\n            set.\n        args: List-form arguments for workflow. Cannot be set if positional_args are\n            set. For typed workflow callables, list contents are not statically\n            typechecked; pass workflow arguments positionally for precise typechecking.\n        id: Unique identifier for the workflow execution. Must be nonempty.\n        task_queue: Task queue to run the workflow on.\n        signal: Signal name or callable to send with the start request.\n        signal_args: Argument value, or list of argument values, for signal. For typed\n            single-argument signals, scalar signal_args values are statically\n            typechecked. List-form signal_args values are not precisely typechecked. To\n            pass a single signal argument that is itself a list, wrap it in another\n            list; otherwise the list is interpreted as multiple signal arguments."
    ));
    assert!(rendered.contains(
        "cron_schedule: Cron schedule for recurring workflow executions. See\n            https://docs.temporal.io/cron-job."
    ));
    assert!(rendered.contains(
        "static_summary: Single-line fixed summary for the workflow execution that may\n            appear in UI and CLI. This can be in single-line Temporal Markdown format."
    ));
    assert!(rendered.contains(
        "\n\n    Returns:\n        A workflow handle to the started workflow.\n    \"\"\""
    ));
    assert!(rendered.contains(
        "id_reuse_policy: temporalio.common.WorkflowIDReusePolicy = (\n        temporalio.common.WorkflowIDReusePolicy.ALLOW_DUPLICATE\n    ),"
    ));
    assert!(
        rendered.contains(
            "id_conflict_policy: temporalio.common.WorkflowIDConflictPolicy | None = None,"
        )
    );
    assert!(rendered.contains("static_summary: str | None = None,"));
    assert!(rendered.contains("static_details: str | None = None,"));
    assert!(!rendered.contains("identity: str | None"));
    assert!(!rendered.contains("user_metadata_static_summary:"));
    assert!(!rendered.contains("user_metadata_static_details:"));
    assert!(rendered.contains("request = SignalWithStartWorkflowRequest("));
    assert!(rendered.contains("workflow=workflow,"));
    assert!(!rendered.contains("def _nexus_is_function_args_list("));
    assert!(!rendered.contains("def _nexus_normalize_function_args("));
    assert!(!rendered.contains("_nexus_arg_unset = object()"));
    assert!(rendered.contains("if positional_args and args is not None:"));
    assert!(
        rendered.contains("raise TypeError(\"cannot specify both positional arguments and args\")")
    );
    assert!(rendered.contains("normalized_args: list[typing.Any] | None = ("));
    assert!(rendered.contains("list(positional_args)"));
    assert!(rendered.contains("else args"));
    assert!(rendered.contains(
        "normalized_signal_args: list[typing.Any] | None\n    if signal_args is None:\n        normalized_signal_args = None\n    elif isinstance(signal_args, list):\n        normalized_signal_args = typing.cast(list[typing.Any], signal_args)\n    else:\n        normalized_signal_args = [signal_args]"
    ));
    assert!(rendered.contains("user_metadata = ("));
    assert!(rendered.contains("if static_summary is None and static_details is None"));
    assert!(rendered.contains("static_summary=static_summary,"));
    assert!(rendered.contains("static_details=static_details,"));
    assert!(rendered.contains("args=normalized_args,"));
    assert!(rendered.contains("id=id,"));
    assert!(rendered.contains("signal_args=normalized_signal_args,"));
    assert!(rendered.contains("user_metadata=user_metadata,"));
    assert!(rendered.contains("return await _signal_with_start_workflow(request)"));
    assert!(models.contains("payloads_to_proto(value.args)"));
    assert!(models.contains("def from_transfer_type("));
    assert!(models.contains("  # pyright: ignore[reportUnusedCallResult]\n"));
    assert!(!models.contains("_ = temporalio.converter.transfer_type_convertible("));
    assert!(!models.contains("@temporalio.converter.transfer_type_convertible("));
    assert!(!models.contains("def from_proto("));
    assert!(models.contains("import temporal_support as _support"));
    assert!(models.contains("_support.retry_policy_to_proto("));
    assert!(rendered.contains("import temporal_support as _support"));
    assert!(rendered.contains(
        "): _NexusOperationInfo(\n        operation=_services.WorkflowService.signal_with_start_workflow,\n        serialization_context=_support.signal_with_start_workflow_serialization_context,\n    ),"
    ));
    assert!(rendered.contains(
        "handle = await nexus_client.start_operation(\n        operation=\"SignalWithStartWorkflowExecution\",\n        input=request,\n        output_type=SignalWithStartWorkflowResponse,\n    )"
    ));
    assert!(!rendered.contains("class SignalWithStartWorkflowModelRequest(typing.Protocol):"));
    assert!(!rendered.contains("def signal_with_start_workflow_serialization_context("));

    let type_roundtrip_rendered = generate_python_to_string(
        &example_input_paths(&root, TYPE_ROUNDTRIP_EXAMPLE_ID),
        &[descriptor_path(&root)],
    );
    assert!(type_roundtrip_rendered.contains("async def activity_options_operation("));
    assert!(type_roundtrip_rendered.contains("task_queue: str | None = None,"));
    assert!(type_roundtrip_rendered.contains("retry_policy: temporalio.common.RetryPolicy,"));
    assert!(type_roundtrip_rendered.contains("request = ActivityOptions("));
}

#[test]
fn python_standalone_proto_oneof_models_are_exported_and_converted() {
    let root = project_root();
    let package = generate_python_package_files(
        &example_input_paths(&root, "proto-oneof"),
        &[descriptor_path(&root)],
    );
    let models = package
        .get(&PathBuf::from("models.py"))
        .expect("standalone Python package should include models.py");
    let package_init = package
        .get(&PathBuf::from("__init__.py"))
        .expect("standalone Python package should include __init__.py");
    assert!(!package.contains_key(&PathBuf::from("services.py")));
    assert!(!package_init.contains("OutcomeClient"));

    let outcome_model = models
        .find("class Outcome(typing.Generic[OutputT]):")
        .expect("generic model should be emitted");
    let outcome_converter = models
        .find("class _OutcomeTransferTypeConverter(")
        .expect("generic converter should be emitted");
    let outcome_registration = models
        .find(
            "temporalio.converter.transfer_type_convertible(_OutcomeTransferTypeConverter[typing.Any])(Outcome)",
        )
        .expect("generic converter should be registered");
    assert!(outcome_model < outcome_converter);
    assert!(outcome_converter < outcome_registration);
    assert!(models.contains("Outcome[OutputT]"));
    assert!(!models.contains("\"Outcome[typing.Any]\""));
    assert!(!models.contains("@typing.cast("));
    assert!(models.contains("output_type, = typing.get_args(type_hint) or (typing.Any,)"));
    assert!(models.contains(
        "@dataclasses.dataclass(slots=True)\nclass OutcomeValueSuccess(typing.Generic[OutputT]):\n    value: OutputT"
    ));
    assert!(!models.contains("@dataclasses.dataclass(slots=True, init=False)"));
    assert!(!models.contains("tag: typing.Literal[\"success\"]"));
    assert!(models.contains("class OutcomeValueFailure:"));
    assert!(models.contains(
        "OutcomeValue = (\n    OutcomeValueSuccess[OutputT]\n    | OutcomeValueFailure\n)"
    ));
    assert!(models.contains("    value: OutcomeValue[OutputT]\n"));
    assert!(!models.contains("value: OutcomeValue[OutputT] | None"));
    assert!(!models.contains("class Failure:"));
    assert!(!models.contains("class Payloads:"));
    assert!(models.contains(
        "        _oneof_value: OutcomeValueSuccess[OutputT] | OutcomeValueFailure\n        match value.WhichOneof(\"value\"):\n            case \"success\":\n"
    ));
    assert!(models.contains(
        "            case None:\n                raise ValueError(\"missing required field Outcome.value\")\n"
    ));
    assert!(!models.contains("unknown protobuf oneof case"));
    assert!(models.contains(
        "typing.cast(OutputT, _support.payloads_from_proto(value.success, [output_type])[0])"
    ));
    assert!(models.contains(
        "_oneof_value = OutcomeValueFailure(_support.failure_from_proto(value.failure))"
    ));
    assert!(models.contains(
        "        match value.value:\n            case OutcomeValueSuccess():\n                message.success.CopyFrom(_support.payloads_to_proto([value.value.value]))\n            case OutcomeValueFailure():\n                message.failure.CopyFrom(_support.failure_to_proto(value.value.value))\n"
    ));
    assert!(models.contains(
        "        if value.activity is not None:\n            match value.activity:\n                case ActivitySelectionId():\n"
    ));
    assert!(!models.contains("runtime_value"));
    assert!(!models.contains("unsupported variant case"));
    assert!(models.contains("class PauseActivityRequest:"));
    assert!(models.contains(
        "temporalio.converter.transfer_type_convertible(_PauseActivityRequestTransferTypeConverter)(PauseActivityRequest)"
    ));
    assert!(models.contains("namespace: str"));
    assert!(models.contains("execution: WorkflowExecution | None = None"));
    assert!(models.contains("identity: str"));
    assert!(models.contains("activity: ActivitySelection | None = None"));
    assert!(models.contains("reason: str"));
    assert!(models.contains("request_id: str"));
    assert!(models.contains("class WorkflowExecution:"));
    assert!(models.contains(
        "        _oneof_activity: ActivitySelectionId | ActivitySelectionType | None\n        match value.WhichOneof(\"activity\"):\n"
    ));
    assert!(models.contains("            case None:\n                _oneof_activity = None\n"));
    assert!(package_init.contains("ActivitySelection,"));
    assert!(package_init.contains("ActivitySelectionId,"));
    assert!(package_init.contains("ActivitySelectionType,"));
    assert!(package_init.contains("OutcomeValue,"));
    assert!(package_init.contains("OutcomeValueSuccess,"));
    assert!(package_init.contains("OutcomeValueFailure,"));
    assert!(package_init.contains("PauseActivityRequest,"));
}

#[test]
fn python_notification_models_do_not_emit_an_empty_service() {
    let root = project_root();
    let package = generate_python_package_files(
        &example_input_paths(&root, "notification-service"),
        &[descriptor_path(&root)],
    );
    let package_init = package
        .get(&PathBuf::from("__init__.py"))
        .expect("notification package should include __init__.py");

    assert!(package.contains_key(&PathBuf::from("models.py")));
    assert!(!package.contains_key(&PathBuf::from("services.py")));
    assert!(package_init.contains("OnCompleteRequest,"));
    assert!(package_init.contains("OnCompleteResponse,"));
    assert!(!package_init.contains("NotificationServiceClient"));
}

#[test]
fn python_non_proto_variants_remain_tagged_tuples_without_transfer_converters() {
    let root = project_root();
    let package = generate_python_package_files(
        &example_input_paths(&root, "type-showcase"),
        &[descriptor_path(&root)],
    );
    let models = package
        .get(&PathBuf::from("models.py"))
        .expect("type showcase should include models.py");
    let resource = package
        .get(&PathBuf::from("_resources/user.py"))
        .expect("type showcase should include the User resource module");

    assert!(models.contains(
        "NotificationTarget = (\n    tuple[typing.Literal[\"email\"], str]\n    | tuple[typing.Literal[\"sms\"], str]\n    | tuple[typing.Literal[\"none\"]]\n)"
    ));
    assert!(!models.contains("class NotificationTargetEmail"));
    assert!(!models.contains("TransferTypeConverter"));
    assert!(!resource.contains("TransferTypeConverter"));
}

#[test]
fn python_proto_generics_propagate_payload_type_hints() {
    let root = project_root();
    let package = generate_python_package_files(
        &example_input_paths(&root, "proto-generic"),
        &[descriptor_path(&root)],
    );
    let models = package
        .get(&PathBuf::from("models.py"))
        .expect("proto generic models should include models.py");
    assert!(
        !package.keys().any(|path| path.starts_with("_support")),
        "user support files must not be copied into generated packages"
    );
    assert!(models.contains("import temporal_support as _support"));

    assert!(models.contains("class PayloadBackedEnvelope(typing.Generic[OutputT, ContextT]):"));
    assert!(models.contains("PayloadBackedEnvelope[OutputT, ContextT]"));
    assert!(models.contains("PayloadBackedOutput[OutputT]"));
    assert!(models.contains("PayloadBackedContext[ContextT]"));
    assert!(!models.contains("@typing.cast("));
    assert!(models.contains(
        "temporalio.converter.transfer_type_convertible(_PayloadBackedEnvelopeTransferTypeConverter[typing.Any, typing.Any])(PayloadBackedEnvelope)"
    ));
    assert!(models.contains(
        "output_type, context_type = typing.get_args(type_hint) or (typing.Any, typing.Any)"
    ));
    assert!(models.contains(
        "_PayloadBackedOutputTransferTypeConverter().from_transfer_type(value.provider, PayloadBackedOutput[output_type])"
    ));
    assert!(models.contains(
        "_PayloadBackedContextTransferTypeConverter().from_transfer_type(value.scaler, PayloadBackedContext[context_type])"
    ));
    assert!(models.contains("output_type, = typing.get_args(type_hint) or (typing.Any,)"));
    assert!(models.contains("payload_from_proto(value.details, output_type)"));
    assert!(models.contains("context_type, = typing.get_args(type_hint) or (typing.Any,)"));
    assert!(models.contains("payload_from_proto(value.details, context_type)"));
    assert!(
        models.contains(
            "typing.cast(OutputT, _support.payload_from_proto(value.details, output_type))"
        )
    );
    assert!(models.contains(
        "typing.cast(ContextT, _support.payload_from_proto(value.details, context_type))"
    ));
    assert!(models.contains("typing.cast(PayloadBackedOutput[OutputT],"));
    assert!(models.contains("typing.cast(PayloadBackedContext[ContextT],"));
    assert!(models.contains("_support.payload_from_proto(value.details, output_type)"));
    assert!(models.contains("_support.payload_to_proto("));
}

#[test]
fn python_sourced_helper_import_is_automatic() {
    let root = project_root();
    let temp_dir = unique_output_path("python-sourced-support-only");
    fs::create_dir_all(&temp_dir).unwrap();
    let wit_path = temp_dir.join("source-only.wit");
    let wit = r#"
package temporal:source-only@1.0.0;

world system { export models; }

interface models {
  type placeholder = string;

  /// @nexus.proto "temporal.api.workflowservice.v1.RequestCancelWorkflowExecutionRequest"
  record cancel-request {
    /// @nexus.source python="workflow_namespace()"
    namespace: string,
    /// @nexus.omit
    workflow-execution: placeholder,
    /// @nexus.omit
    reason: placeholder,
    /// @nexus.omit
    identity: placeholder,
    /// @nexus.omit
    request-id: placeholder,
    /// @nexus.omit
    first-execution-run-id: placeholder,
    /// @nexus.omit
    links: placeholder,
  }
}
"#;
    fs::write(&wit_path, wit).unwrap();
    let files = generate_python_package_files(&[wit_path], &[descriptor_path(&root)]);
    let models = &files[&PathBuf::from("models.py")];
    assert!(models.contains("import temporal_support as _support"));
    assert!(models.contains("default_factory=_support.workflow_namespace"));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn python_proto_helpers_use_the_supplied_support_module() {
    let root = project_root();
    let spec = nexgen::parser::load_api_spec_from_wit_for_language_with_inputs(
        nexgen::language::Language::Python,
        &example_input_paths(&root, PRIMARY_EXAMPLE_ID),
    )
    .unwrap();
    let descriptors = nexgen::descriptors::DescriptorIndex::load(&descriptor_path(&root)).unwrap();
    let output = generate_source(
        nexgen::language::Language::Python,
        spec,
        &descriptors,
        "company.shared.temporal_support",
    )
    .unwrap();

    assert!(output.contains("import company.shared.temporal_support as _support"));
    assert!(output.contains("_support.retry_policy_to_proto("));
    assert!(output.contains("_support.signal_with_start_workflow_serialization_context"));
    assert!(!output.contains("### _support/"));
}

/// An inline **structured** object `oneOf` branch on a property: the branch is
/// named `<Union>Object` and emitted as a module-level dataclass, which the
/// union's dispatcher selects for the object member of the union.
/// See `specs/json-schema/features/oneOf.md` ("Object branches").
#[test]
fn python_json_names_inline_object_union_branch() {
    let temp_dir = unique_output_path("py-json-inline-branch");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("detail.yaml");
    fs::write(&input_path, INLINE_OBJECT_BRANCH_SCHEMA).unwrap();
    let output_path = temp_dir.join("detail");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_package: Some("temporal_support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();

    assert!(rendered.contains("payload: DetailPayloadObject | str | None"));
    assert!(rendered.contains("class DetailPayloadObject:"));
    assert!(rendered.contains("class _DetailPayloadObjectTransferTypeConverter("));
    assert!(rendered.contains("text: str"));
    // The branch model is part of the module surface, like any named definition.
    let exports = fs::read_to_string(output_path.join("__init__.py")).unwrap();
    assert!(exports.contains("DetailPayloadObject"));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Every constraint a **non-object** branch declares is enforced by the union's
/// dispatcher rather than by the annotation, so the field annotation is the plain
/// union of the branch types while the bound, the `pattern`, and the
/// `uniqueItems` check all live in the converter body.
/// See `specs/json-schema/features/oneOf.md` ("Validator mapping").
#[test]
fn python_json_validates_non_object_union_branch_constraints() {
    let temp_dir = unique_output_path("py-json-branch-constraints");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("bc.yaml");
    fs::write(&input_path, BRANCH_CONSTRAINT_SCHEMA).unwrap();
    let output_path = temp_dir.join("bc");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_package: Some("temporal_support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();

    // The string branch's `minLength`/`pattern` and the integer branch's
    // `minimum` leave no residue on the annotation: it is the plain branch union.
    assert!(rendered.contains("value: str | int | None"));
    // Same for the array branch's `minItems`/`uniqueItems`; a closed value set
    // still narrows to a `typing.Literal`.
    assert!(rendered.contains("list[float] | typing.Literal[\"auto\", \"manual\"] | None"));
    // The branch checks themselves live in the converter body: a `pattern` lowers
    // to a `.search` against a module-level compiled regex const, `uniqueItems` to
    // a runtime helper imported from the definitions module.
    assert!(
        rendered.contains("_PATTERN_F242E3A159C2422C = re.compile(\"^[a-z]+\\\\Z\", re.ASCII)")
    );
    assert!(rendered.contains("if _PATTERN_F242E3A159C2422C.search(value) is None:"));
    assert!(rendered.contains("_check_unique_items("));
    // The array branch's element schema is part of the serialize contract too:
    // its unconstrained `number` still contributes the uniform finiteness guard
    // at the indexed path.
    assert!(
        rendered
            .contains("checked_array_8 = typing.cast(\"list[typing.Any]\", list_or_name_value)")
    );
    assert!(rendered.contains("for item_index_8, item_element_8 in enumerate(checked_array_8):"));
    assert!(rendered.contains(
        "Violation(path=f'listOrName[{item_index_8}]', reason=f\"must be a finite number, got {item_element_8}\")"
    ));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A union in an element position: the loader names it, so Python emits an
/// ordinary union alias and the converter dispatches the branch per element. An
/// optional field whose *elements* are nullable still needs its own `| None` —
/// the element's `None` is not the field's.
/// See `specs/json-schema/features/oneOf.md` ("Unions in element positions").
#[test]
fn python_json_annotates_element_position_unions() {
    let temp_dir = unique_output_path("py-json-element-union");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("bag.yaml");
    fs::write(&input_path, ELEMENT_UNION_SCHEMA).unwrap();
    let output_path = temp_dir.join("bag");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_package: Some("temporal_support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();

    assert!(rendered.contains("BagSegmentsItem: typing.TypeAlias = str | int"));
    assert!(rendered.contains("segments: list[BagSegmentsItem] | None"));
    assert!(rendered.contains("choices: list[Choice] | None"));
    assert!(rendered.contains("slots: list[str | None] | None"));
    let exports = fs::read_to_string(output_path.join("__init__.py")).unwrap();
    assert!(exports.contains("BagSegmentsItem"));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// The entry file of a two-file closure. `get`'s output is the model the *other*
/// file declares, and `FindOutput.page` `$ref`s it from a property, so both
/// cross-module reference shapes are covered.
const CROSS_MODULE_ENTRY_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Pages:
    fqn: example.pages.v1.Pages
    operations:
      get:
        input: { $ref: "#/$defs/GetInput" }
        output: { $ref: "content/page.json" }
      find:
        input: { $ref: "#/$defs/GetInput" }
        output: { $ref: "#/$defs/FindOutput" }
$defs:
  GetInput:
    type: object
    additionalProperties: false
    properties:
      id: { type: string }
  FindOutput:
    type: object
    additionalProperties: false
    properties:
      page: { $ref: "content/page.json" }
"##;

/// The referenced file. Its model carries the name override the *consuming*
/// module has to resolve through.
const CROSS_MODULE_PAGE_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
x-py-name: RenamedPage
properties:
  title: { type: string }
"##;

/// Writes the two-file cross-module closure into `dir` and returns the input
/// directory to generate from.
fn write_cross_module_closure(dir: &Path) -> PathBuf {
    let input_dir = dir.join("input");
    fs::create_dir_all(input_dir.join("content")).unwrap();
    fs::write(
        input_dir.join("kb.nexusrpc.yaml"),
        CROSS_MODULE_ENTRY_SCHEMA,
    )
    .unwrap();
    fs::write(
        input_dir.join("content/page.json"),
        CROSS_MODULE_PAGE_SCHEMA,
    )
    .unwrap();
    input_dir
}

fn generate_python_schema_tree(name: &str, files: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let temp_dir = unique_output_path(name);
    let input_dir = temp_dir.join("input");
    fs::create_dir_all(&input_dir).unwrap();
    for (relative, contents) in files {
        let path = input_dir.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_dir],
        support_package: Some("temporal_support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    (temp_dir, output_path)
}

#[test]
fn python_json_cycle_graph_preserves_already_decoded_ref_names() {
    let (temp_dir, output_path) = generate_python_schema_tree(
        "py-json-escaped-cycle",
        &[
            (
                "a.yaml",
                r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  "a~1b":
    type: object
    properties:
      link: { $ref: "b.yaml#/$defs/Bee" }
"##,
            ),
            (
                "b.yaml",
                r##"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Bee:
    type: object
    properties:
      back: { $ref: "a.yaml#/$defs/a~01b" }
"##,
            ),
        ],
    );

    let recursive = fs::read_to_string(output_path.join("_recursive.py")).unwrap();
    assert!(recursive.contains("class A1b:"), "{recursive}");
    assert!(recursive.contains("class Bee:"), "{recursive}");
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn python_json_hoist_closes_leaf_dependency_import_cycles() {
    let (temp_dir, output_path) = generate_python_schema_tree(
        "py-json-hoist-dependency-cycle",
        &[
            (
                "a.yaml",
                r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
properties:
  b: { $ref: "b.yaml" }
  leaf: { $ref: "c.yaml#/$defs/Leaf" }
"##,
            ),
            (
                "b.yaml",
                r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
properties:
  a: { $ref: "a.yaml" }
"##,
            ),
            (
                "c.yaml",
                r##"
$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
properties:
  a: { $ref: "a.yaml" }
$defs:
  Leaf: { type: object, additionalProperties: false, properties: { n: { type: string } } }
"##,
            ),
        ],
    );

    let recursive = fs::read_to_string(output_path.join("_recursive.py")).unwrap();
    assert!(recursive.contains("class A:"), "{recursive}");
    assert!(recursive.contains("class B:"), "{recursive}");
    assert!(recursive.contains("class Leaf:"), "{recursive}");
    assert!(
        !recursive.contains("from .c.models import Leaf"),
        "{recursive}"
    );

    let c_models = fs::read_to_string(output_path.join("c/models.py")).unwrap();
    assert!(
        c_models.contains("from .._recursive import A"),
        "{c_models}"
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

/// An `x-py-name` override on a model in *another* input file moves every
/// reference the consuming module emits: the operation's `Operation[...]`
/// parameter, the relative model imports, and the annotation of a cross-module
/// `$ref` property. The override is declared in the referenced file, so only the
/// tree-wide name manifest can resolve it (P14/P15).
#[test]
fn python_json_cross_module_py_name_override_moves_every_reference() {
    let temp_dir = unique_output_path("py-json-cross-module-override");
    let input_dir = write_cross_module_closure(&temp_dir);
    let output_path = temp_dir.join("output");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Python,
        input_paths: vec![input_dir],
        support_package: Some("temporal_support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let declaring = fs::read_to_string(output_path.join("content/page/models.py")).unwrap();
    assert!(declaring.contains("class RenamedPage:"));

    let services = fs::read_to_string(output_path.join("kb/services.py")).unwrap();
    for expected in [
        "from ..content.page.models import RenamedPage",
        "        RenamedPage,\n",
    ] {
        assert!(services.contains(expected), "{expected}\n{services}");
    }

    let models = fs::read_to_string(output_path.join("kb/models.py")).unwrap();
    for expected in [
        "from ..content.page.models import RenamedPage",
        "    page: RenamedPage | None",
    ] {
        assert!(models.contains(expected), "{expected}\n{models}");
    }
    // Nothing names the pre-override identifier.
    for stale in ["import Page", " Page,", ": Page"] {
        assert!(!services.contains(stale), "{stale}\n{services}");
        assert!(!models.contains(stale), "{stale}\n{models}");
    }
    fs::remove_dir_all(temp_dir).unwrap();
}

/// The package barrel (`__init__.py`) re-exports every module by name, so two
/// modules declaring the same type name produce `from .a import Page` followed by
/// `from .b import Page`. Python raises nothing: the second binding silently wins
/// and `__all__` lists the name once, so `from pkg import Page` quietly resolves to
/// the wrong model. That silent incorrectness is what P7 forbids, so the generator
/// rejects at load. See `specs/json-schema/PRINCIPLES.md` §15.
#[test]
fn python_json_rejects_same_type_name_in_two_modules() {
    let temp_dir = unique_output_path("py-json-barrel-collision");
    let input_dir = temp_dir.join("input");
    fs::create_dir_all(input_dir.join("a")).unwrap();
    fs::create_dir_all(input_dir.join("b")).unwrap();
    fs::write(
        input_dir.join("a/page.json"),
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"properties":{"title":{"type":"string"}}}"#,
    )
    .unwrap();
    fs::write(
        input_dir.join("b/page.json"),
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"properties":{"count":{"type":"integer"}}}"#,
    )
    .unwrap();

    let request = |output: &str| GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Python,
        input_paths: vec![input_dir.clone()],
        support_package: Some("temporal_support".into()),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join(output),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    };

    let error = generate_to_file(&request("out"))
        .expect_err("two modules declaring `Page` collide in the package barrel")
        .to_string();
    // The diagnostic names both modules — the bare type name appears twice and
    // would otherwise read as one declaration seen twice.
    assert!(error.contains("collision"), "{error}");
    assert!(error.contains("a/page#Page"), "{error}");
    assert!(error.contains("b/page#Page"), "{error}");
    assert!(error.contains("x-py-name"), "{error}");

    // The documented escape hatch resolves it, and the barrel then re-exports both.
    fs::write(
        input_dir.join("b/page.json"),
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","x-py-name":"BPage","additionalProperties":false,"properties":{"count":{"type":"integer"}}}"#,
    )
    .unwrap();
    let output_path = temp_dir.join("out-renamed");
    generate_to_file(&request("out-renamed")).expect("the override resolves the collision");
    let barrel = fs::read_to_string(output_path.join("__init__.py")).unwrap();
    for expected in [
        "from .a import Page",
        "from .b import BPage",
        "\"BPage\",",
        "\"Page\",",
    ] {
        assert!(barrel.contains(expected), "{expected}\n{barrel}");
    }
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Re-emitting a `$ref`d type into the referencing service's module produced two
/// `class Page` in different modules, which the package barrel then imported
/// twice — `from .a import Page` followed by `from .svc import Page`, silently
/// binding one copy and dropping the other (P7). It happened whenever the service
/// module declared no types of its own, because reachability pruning read "this
/// module owns nothing" as "this front end does not scope by module".
#[test]
fn python_json_service_module_without_own_types_does_not_reemit_refs() {
    let temp_dir = unique_output_path("py-json-service-only-module");
    let input_dir = temp_dir.join("input");
    fs::create_dir_all(input_dir.join("a")).unwrap();
    fs::write(
        input_dir.join("a/page.json"),
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"properties":{"title":{"type":"string"}},"required":["title"]}"#,
    )
    .unwrap();
    fs::write(
        input_dir.join("svc.nexusrpc.yaml"),
        r#"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Svc:
    fqn: example.v1.Svc
    operations:
      one:
        input: { $ref: "a/page.json" }
"#,
    )
    .unwrap();

    let output_path = temp_dir.join("out");
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Python,
        input_paths: vec![input_dir],
        support_package: Some("temporal_support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let rendered = read_python_package_files(&output_path)
        .into_values()
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        rendered.matches("class Page:").count(),
        1,
        "`Page` must be declared once\n{rendered}"
    );
    // The root barrel binds each name exactly once.
    let barrel = fs::read_to_string(output_path.join("__init__.py")).unwrap();
    assert_eq!(barrel.matches("import Page").count(), 1, "{barrel}");
    // A module that declares nothing of its own emits no models file at all —
    // see `specs/json-schema/features/generated-file-layout.md`.
    assert!(
        !output_path.join("svc/models.py").exists(),
        "a service-only module must not emit an empty `models.py`"
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Generates a Python package from `schema_source` into a fresh directory under
/// `temp_dir`, without running `ruff`. Formatting is what has historically hidden
/// syntax that the declared floor rejects, so every probe below reads the
/// generator's own bytes.
fn generate_unformatted_python_package(
    temp_dir: &Path,
    schema_source: &str,
    config: nexgen::nexgen_config::NexgenConfig,
) -> PathBuf {
    fs::create_dir_all(temp_dir).unwrap();
    let input_path = temp_dir.join("probe.nexusrpc.yaml");
    fs::write(&input_path, schema_source).unwrap();
    let output_path = temp_dir.join("probe_package");
    generate_to_file(&GenerateRequest {
        config,
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_package: Some("temporal_support".into()),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    output_path
}

#[test]
fn python_json_omits_empty_dependent_required_bodies() {
    let temp_dir = unique_output_path("py-empty-dependent-required");
    let output_path = generate_unformatted_python_package(
        &temp_dir,
        r##"nexusrpc: "1.0.0"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Model:
    type: object
    properties:
      trigger: { type: string }
    dependentRequired:
      trigger: []
"##,
        nexgen::nexgen_config::NexgenConfig::default(),
    );

    let models = fs::read_to_string(output_path.join("models.py")).unwrap();
    assert_eq!(
        models.matches("if \"trigger\" in raw:").count(),
        1,
        "{models}"
    );
    assert!(!models.contains("if \"trigger\" in out:"), "{models}");
    fs::remove_dir_all(temp_dir).unwrap();
}

/// The Python key loop is opened only when the `propertyNames` subschema actually
/// contributes a key predicate — an empty `for` body is an `IndentationError` at
/// import. The two shapes that would produce one are refused at load: a subschema
/// that asserts nothing, and a materializing temporal `format`, which cannot
/// assert a key at all. Asserted here because the emitter's guard is the second
/// line of defence and is otherwise unreachable.
/// See `specs/json-schema/features/propertyNames.md`.
#[test]
fn python_json_property_names_without_a_key_predicate_are_refused() {
    for (label, subschema) in [
        ("vacuous", "{ type: string }"),
        ("temporal", "{ type: string, format: date-time }"),
    ] {
        let temp_dir = unique_output_path(&format!("py-json-property-names-{label}"));
        fs::create_dir_all(&temp_dir).unwrap();
        let input_path = temp_dir.join("probe.nexusrpc.yaml");
        fs::write(
            &input_path,
            format!(
                r##"nexusrpc: "1.0.0"
$schema: https://json-schema.org/draft/2020-12/schema
$defs:
  Keys:
    type: object
    propertyNames: {subschema}
    additionalProperties: {{ type: string }}
"##
            ),
        )
        .unwrap();
        let error = generate_to_file(&GenerateRequest {
            config: Default::default(),
            language: nexgen::language::Language::Python,
            input_paths: vec![input_path],
            support_package: Some("temporal_support".into()),
            descriptor_paths: Vec::new(),
            output_path: temp_dir.join("out"),
            format: false,
            java_package_name: None,
            ts_date_time_types: Default::default(),
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("propertyNames"), "{label}: {error}");
        fs::remove_dir_all(temp_dir).unwrap();
    }
}

/// `add-rpc` scaffolds a repeated field as a bare `list<T>` and a map as a bare
/// `map<K, V>`. Both are optional collections, as in WIT-direct records.
#[test]
fn python_accepts_bare_lists_and_maps_on_proto_collection_fields() {
    let root = project_root();
    let temp_dir = unique_output_path("python-proto-bare-collections");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("input.wit");
    fs::write(
        &input_path,
        r#"package temporal:nexus@1.0.0;

world system {
  export collections;
}

interface collections {
  /// @nexus.proto "temporal.api.taskqueue.v1.TaskQueuePartitionMetadata"
  record task-queue-partition-metadata {
    key: string,
    owner-host-name: string,
  }

  /// @nexus.proto "temporal.api.workflowservice.v1.ListTaskQueuePartitionsResponse"
  record list-task-queue-partitions-response {
    activity-task-queue-partitions: list<task-queue-partition-metadata>,
    workflow-task-queue-partitions: list<task-queue-partition-metadata>,
  }

  /// @nexus.proto "temporal.api.common.v1.Callback.Nexus"
  record callback-nexus {
    url: string,
    header: map<string, string>,
  }
}
"#,
    )
    .unwrap();
    let package = generate_python_package_files(&[input_path], &[descriptor_path(&root)]);
    fs::remove_dir_all(temp_dir).unwrap();
    let models = package
        .get(&PathBuf::from("models.py"))
        .expect("collections should include models.py");

    assert!(models.contains(
        "activity_task_queue_partitions: list[TaskQueuePartitionMetadata] | None = dataclasses.field(default_factory=list)"
    ), "{models}");
    assert!(
        models.contains("header: dict[str, str] | None = dataclasses.field(default_factory=dict)"),
        "{models}"
    );
}

#[test]
fn python_imports_no_model_that_only_a_docstring_names() {
    let root = project_root();
    let temp_dir = unique_output_path("python-docstring-names");
    fs::create_dir_all(&temp_dir).unwrap();
    let wit_path = temp_dir.join("docstring-names.wit");
    let wit = r#"
package temporal:docstring-names@1.0.0;

world system { export api; }

/// @nexus.endpoint "api"
interface api {
  record ping-request {
    text: string,
  }

  record pong-request {
    text: string,
  }

  /// @nexus.doc "Send a ping. A PongRequest answers it."
  ping: func(request: ping-request);

  pong: func(request: pong-request);
}
"#;
    fs::write(&wit_path, wit).unwrap();
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            mode: nexgen::generator::GenerationMode::NativeApi,
            ..Default::default()
        },
        language: nexgen::language::Language::Python,
        input_paths: vec![wit_path, linked_inputs_path(&root)],
        support_package: Some("temporal_support".into()),
        descriptor_paths: vec![descriptor_path(&root)],
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let ping = fs::read_to_string(output_path.join("operations/ping.py")).unwrap();
    assert!(ping.contains("A PongRequest answers it."), "{ping}");
    assert!(!ping.contains("    PongRequest,\n"), "{ping}");
    assert!(!ping.contains("import PongRequest\n"), "{ping}");
    fs::remove_dir_all(temp_dir).unwrap();
}
