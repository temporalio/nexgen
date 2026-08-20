// Drives the `nexgen` binary over the WIT/proto CLI surface (`--descriptors`,
// `--native-api`, `--support-file`), all behind the `advanced` feature.
#![cfg(feature = "advanced")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use heck::ToSnakeCase;
use nexgen::SupportFiles;
use nexgen::descriptors::DescriptorIndex;
use nexgen::generator::generate_source;
use nexgen::spec::SupportFragmentSpec;
use nexgen::{GenerateRequest, generate_to_file};

mod common;
use common::json_input_path;

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
    vec![input_path(root, example_id), linked_inputs_path(root)]
}

fn python_root(root: &Path) -> PathBuf {
    root.join("advanced/samples/python")
}

fn samples_python_root(root: &Path) -> PathBuf {
    root.join("samples/python")
}

fn input_path(root: &Path, example_id: &str) -> PathBuf {
    let flat_path = root
        .join("advanced/samples/inputs")
        .join(format!("{example_id}.wit"));
    if flat_path.is_file() {
        flat_path
    } else {
        root.join("advanced/samples/inputs")
            .join(example_id)
            .join("main.wit")
    }
}

fn python_output_path(root: &Path, example_id: &str) -> PathBuf {
    python_root(root)
        .join("wit")
        .join(example_id.to_snake_case())
}

fn python_json_definitions_output_path(root: &Path, example_id: &str) -> PathBuf {
    // Definitions are the beginner-facing samples, flattened under samples/python/<snake>.
    samples_python_root(root).join(example_id.to_snake_case())
}

fn python_json_api_output_path(root: &Path, example_id: &str) -> PathBuf {
    python_root(root)
        .join("json_schema")
        .join("api")
        .join(example_id.to_snake_case())
}

fn python_example_ids(root: &Path) -> Vec<String> {
    let mut ids = fs::read_dir(root.join("advanced/samples/inputs"))
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let example_id = if path.is_file() {
                path.file_stem()?.to_string_lossy().into_owned()
            } else if path.join("main.wit").is_file() {
                path.file_name()?.to_string_lossy().into_owned()
            } else {
                return None;
            };
            if python_output_path(root, &example_id).is_dir() {
                Some(example_id)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids
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
        language: nexgen::language::Language::Python,
        input_paths: input_paths.to_vec(),
        support_paths: Vec::new(),
        descriptor_paths: descriptor_paths.to_vec(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: true,
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
        language: nexgen::language::Language::Python,
        input_paths: input_paths.to_vec(),
        support_paths: Vec::new(),
        descriptor_paths: descriptor_paths.to_vec(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: true,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let files = read_python_package_files(&output_path);
    fs::remove_dir_all(temp_dir).unwrap();
    files
}

fn generate_formatted_python_output(root: &Path, example_id: &str, output_path: &Path) {
    let status = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "python",
            input_path(root, example_id).to_str().unwrap(),
            linked_inputs_path(root).to_str().unwrap(),
            "--descriptors",
            descriptor_path(root).to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
            "--native-api",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let format_status = Command::new("uv")
        .current_dir(python_root(root))
        .args([
            "run",
            "ruff",
            "format",
            "--line-length",
            "88",
            "--config",
            "pyproject.toml",
            output_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(format_status.success());
}

fn generate_formatted_json_python_output(
    root: &Path,
    example_id: &str,
    output_path: &Path,
    generate_native_api: bool,
) {
    let input_path = json_input_path(root, example_id);
    let mut args = vec![
        "python",
        input_path.to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
    ];
    if generate_native_api {
        args.push("--native-api");
    }

    let status = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args(args)
        .status()
        .unwrap();
    assert!(status.success());

    let format_status = Command::new("uv")
        .current_dir(python_root(root))
        .args([
            "run",
            "ruff",
            "format",
            "--line-length",
            "88",
            "--config",
            "pyproject.toml",
            output_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(format_status.success());
}

fn assert_python_310_syntax_compatible(package_dir: &Path) {
    let checker = r#"
import ast
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
for path in sorted(root.rglob("*.py")):
    source = path.read_text()
    try:
        ast.parse(source, filename=str(path), feature_version=(3, 10))
    except SyntaxError as exc:
        print(f"{path}: {exc}")
        raise
"#;
    let status = Command::new(
        project_root()
            .join("advanced/samples/python/.venv/bin/python")
            .to_str()
            .unwrap(),
    )
    .args(["-c", checker, package_dir.to_str().unwrap()])
    .status()
    .unwrap();
    assert!(status.success());
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
fn python_examples_generation_matches_checked_in_output() {
    let root = project_root();
    for example_id in python_example_ids(&root) {
        let output_path = unique_output_path(&format!("python-{example_id}"));
        generate_formatted_python_output(&root, &example_id, &output_path);
        assert_python_310_syntax_compatible(&output_path);
        let rendered = read_python_package_files(&output_path);
        let expected = read_python_package_files(&python_output_path(&root, &example_id));
        assert_eq!(rendered, expected, "snapshot mismatch for {example_id}");
        fs::remove_dir_all(output_path).unwrap();
    }
}

#[test]
fn python_json_example_generation_matches_checked_in_output() {
    let root = project_root();
    for example_id in ["chat", "kb", "showcase", "temporal"] {
        let output_path = unique_output_path(&format!("python-json-{example_id}"));
        generate_formatted_json_python_output(&root, example_id, &output_path, false);
        assert_python_310_syntax_compatible(&output_path);
        let rendered = read_python_package_files(&output_path);
        let expected =
            read_python_package_files(&python_json_definitions_output_path(&root, example_id));
        assert_eq!(rendered, expected, "snapshot mismatch for {example_id}");
        if example_id == "showcase" {
            let all = rendered.values().cloned().collect::<Vec<_>>().join("\n");
            // Scalar defaults surface natively via the Pydantic field default.
            assert!(all.contains("greeting: str = pydantic.Field(default=\"hello\")"));
            assert!(all.contains("debug: bool = pydantic.Field(default=False)"));
            // `deprecated` → PEP 702 marker (no runtime warning); `title` → docstring.
            assert!(all.contains(
                "typing_extensions.deprecated(\"This field is deprecated.\", category=None)"
            ));
            assert!(all.contains("Retry budget"));
            // `x-py-name` override (Stage 4): the attribute uses the override
            // while the wire name is pinned by `Field(alias="legacyId")`.
            assert!(all.contains("legacy_id_py:"));
            assert!(all.contains("alias=\"legacyId\""));
            // A free-form object inlines as a mapping — both as a union branch
            // and (extra="allow" + a member-count validator) as a named model.
            assert!(all.contains("payload: dict[str, typing.Any] | str | None"));
            assert!(all.contains("class Extras(pydantic.BaseModel):"));
            // A tagged union whose branches are written inline: each branch names
            // itself with `x-py-name` and becomes a model Pydantic selects on.
            assert!(all.contains("class TextNote(pydantic.BaseModel):"));
            assert!(all.contains("Note: typing.TypeAlias = TextNote | LinkNote"));
            // The lone inline object branch of a property union derives its name
            // from the union it belongs to.
            assert!(all.contains("class ShowcaseDetailObject(pydantic.BaseModel):"));
            assert!(all.contains("detail: ShowcaseDetailObject | str | None"));
            assert!(all.contains("must have at most 4 properties"));
        }
        fs::remove_dir_all(output_path).unwrap();
    }
}

#[test]
fn python_json_api_example_generation_matches_checked_in_output() {
    let root = project_root();
    for example_id in ["chat", "kb", "showcase", "temporal"] {
        let output_path = unique_output_path(&format!("python-json-api-{example_id}"));
        generate_formatted_json_python_output(&root, example_id, &output_path, true);
        assert_python_310_syntax_compatible(&output_path);
        let rendered = read_python_package_files(&output_path);
        let expected = read_python_package_files(&python_json_api_output_path(&root, example_id));
        assert_eq!(rendered, expected, "snapshot mismatch for {example_id}");
        fs::remove_dir_all(output_path).unwrap();
    }
}

#[test]
fn cli_generates_wit_direct_example_without_descriptors() {
    let root = project_root();
    let output_path = unique_output_path("python-user-service-no-descriptors");
    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "python",
            input_path(&root, "user-service").to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
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
fn cli_generates_python_support_file_from_parameter() {
    let root = project_root();
    let temp_dir = unique_output_path("python-support-file-input");
    fs::create_dir_all(&temp_dir).unwrap();
    let support_path = temp_dir.join("custom_support.py");
    let output_path = temp_dir.join("output");
    fs::write(
        &support_path,
        "def custom_support_hook() -> str:\n    return 'custom'\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "python",
            input_path(&root, "user-service").to_str().unwrap(),
            "--support-file",
            support_path.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(output_path.join("_support/custom_support.py")).unwrap(),
        "def custom_support_hook() -> str:\n    return 'custom'\n"
    );
    assert!(
        fs::read_to_string(output_path.join("_support/__init__.py"))
            .unwrap()
            .contains("from .custom_support import *")
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn python_example_suite_type_checks_and_runs() {
    let root = project_root();

    // The advanced project holds the WIT + proto-wire suites and the snapshot-only
    // native-api outputs; the samples project holds the JSON-Schema definitions and
    // their round-trip tests. Both must type-check and pass.
    for example_dir in [python_root(&root), samples_python_root(&root)] {
        let typecheck_status = Command::new("uv")
            .current_dir(&example_dir)
            .args(["run", "basedpyright"])
            .status()
            .unwrap();
        assert!(
            typecheck_status.success(),
            "basedpyright failed in {example_dir:?}"
        );

        let pytest_status = Command::new("uv")
            .current_dir(&example_dir)
            .args(["run", "pytest"])
            .status()
            .unwrap();
        assert!(pytest_status.success(), "pytest failed in {example_dir:?}");
    }
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
        "@dataclasses.dataclass(slots=True, kw_only=True)\nclass SignalWithStartWorkflowRequest:\n    \"\"\"\n    .. warning::\n        This API is experimental and subject to change.\n    \"\"\"\n    workflow: str | collections.abc.Callable[..., collections.abc.Awaitable[object]]\n    args: list[typing.Any] | None = None\n    id: str\n    task_queue: str\n    signal: str | collections.abc.Callable[..., None | collections.abc.Awaitable[None]]\n    signal_args: list[typing.Any] | None = None\n    execution_timeout: datetime.timedelta | None = None"
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
    assert!(rendered.contains("dataclasses.field(default_factory=workflow_namespace)"));
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
        "\"\"\"Signal a workflow, starting it first if needed.\n\n    .. warning::\n        This API is experimental and subject to change.\n\n    Args:\n        workflow: Workflow type name or callable identifying the workflow to start.\n        positional_args: Positional arguments for workflow. Cannot be set if args is\n            set.\n        args: List-form arguments for workflow. Cannot be set if positional_args are\n            set. For typed workflow callables, list contents are not statically\n            typechecked; pass workflow arguments positionally for precise typechecking.\n        id: Unique identifier for the workflow execution.\n        task_queue: Task queue to run the workflow on.\n        signal: Signal name or callable to send with the start request.\n        signal_args: Argument value, or list of argument values, for signal. For typed\n            single-argument signals, scalar signal_args values are statically\n            typechecked. List-form signal_args values are not precisely typechecked. To\n            pass a single signal argument that is itself a list, wrap it in another\n            list; otherwise the list is interpreted as multiple signal arguments."
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
    assert!(models.contains("@temporalio.converter.transfer_type_convertible("));
    assert!(!models.contains("def from_proto("));
    assert!(models.contains("from ._support import ("));
    assert!(models.contains("retry_policy_to_proto,"));
    assert!(
        rendered.contains("from ._support import signal_with_start_workflow_serialization_context")
    );
    assert!(rendered.contains(
        "): _NexusOperationInfo(\n        operation=_services.WorkflowService.signal_with_start_workflow,\n        serialization_context=signal_with_start_workflow_serialization_context,\n    ),"
    ));
    assert!(rendered.contains(
        "handle = await nexus_client.start_operation(\n        operation=\"SignalWithStartWorkflowExecution\",\n        input=request,\n        output_type=SignalWithStartWorkflowResponse,\n    )"
    ));
    assert!(rendered.contains(
        "class SignalWithStartWorkflowModelRequest(typing.Protocol):\n    namespace: str\n    id: str"
    ));
    assert!(rendered.contains(
        "def signal_with_start_workflow_serialization_context(\n    request: SignalWithStartWorkflowModelRequest,\n) -> temporalio_converter.WorkflowSerializationContext:"
    ));
    assert!(rendered.contains(
        "return temporalio_converter.WorkflowSerializationContext(\n        namespace=request.namespace,\n        workflow_id=request.id,\n    )"
    ));

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

    assert!(models.contains("class Outcome(typing.Generic[OutputT]):"));
    assert!(models.contains("\"Outcome[typing.Any]\""));
    assert!(models.contains("output_type, = typing.get_args(type_hint) or (typing.Any,)"));
    assert!(models.contains("@dataclasses.dataclass(slots=True, init=False)"));
    assert!(models.contains("class OutcomeValueSuccess(typing.Generic[OutputT]):"));
    assert!(models.contains("tag: typing.Literal[\"success\"] = dataclasses.field(init=False)"));
    assert!(models.contains(
        "def __init__(self, value: OutputT) -> None:\n        self.tag = \"success\"\n        self.value = value"
    ));
    assert!(models.contains("class OutcomeValueFailure:"));
    assert!(models.contains(
        "OutcomeValue = (\n    OutcomeValueSuccess[OutputT]\n    | OutcomeValueFailure\n)"
    ));
    assert!(models.contains("    value: OutcomeValue[OutputT]\n"));
    assert!(!models.contains("value: OutcomeValue[OutputT] | None"));
    assert!(!models.contains("class Failure:"));
    assert!(!models.contains("class Payloads:"));
    assert!(models.contains("_oneof_value_case = value.WhichOneof(\"value\")"));
    assert!(models.contains(
        "if _oneof_value_case is None:\n            raise ValueError(\"missing required field Outcome.value\")"
    ));
    assert!(models.contains(
        "_oneof_value = OutcomeValueSuccess(payloads_from_proto(value.success, [output_type])[0])"
    ));
    assert!(
        models.contains("_oneof_value = OutcomeValueFailure(failure_from_proto(value.failure))")
    );
    assert!(models.contains("if isinstance(_oneof_value_value, OutcomeValueSuccess):"));
    assert!(models.contains(
        "if runtime_value.value is None:\n            raise ValueError(\"missing required field Outcome.value\")"
    ));
    assert!(
        models.contains("message.success.CopyFrom(payloads_to_proto([_oneof_value_value.value]))")
    );
    assert!(models.contains("elif isinstance(_oneof_value_value, OutcomeValueFailure):"));
    assert!(
        models.contains("message.failure.CopyFrom(failure_to_proto(_oneof_value_value.value))")
    );
    assert!(models.contains("unsupported variant case Outcome.value:"));
    assert!(models.contains("class PauseActivityRequest:"));
    assert!(models.contains("namespace: str"));
    assert!(models.contains("execution: WorkflowExecution | None = None"));
    assert!(models.contains("identity: str"));
    assert!(models.contains("activity: ActivitySelection | None = None"));
    assert!(models.contains("reason: str"));
    assert!(models.contains("request_id: str"));
    assert!(models.contains("class WorkflowExecution:"));
    assert!(models.contains("_oneof_activity_case = value.WhichOneof(\"activity\")"));
    assert!(
        models.contains("if _oneof_activity_case is None:\n            _oneof_activity = None")
    );
    assert!(package_init.contains("ActivitySelection,"));
    assert!(package_init.contains("ActivitySelectionId,"));
    assert!(package_init.contains("ActivitySelectionType,"));
    assert!(package_init.contains("OutcomeValue,"));
    assert!(package_init.contains("OutcomeValueSuccess,"));
    assert!(package_init.contains("OutcomeValueFailure,"));
    assert!(package_init.contains("PauseActivityRequest,"));
}

#[test]
fn python_proto_generics_propagate_payload_type_hints() {
    let root = project_root();
    let package = generate_python_package_files(
        &example_input_paths(&root, "proto-generic-python"),
        &[descriptor_path(&root)],
    );
    let models = package
        .get(&PathBuf::from("models.py"))
        .expect("proto generic models should include models.py");
    let support = package
        .get(&PathBuf::from("_support/temporal_model_converters.py"))
        .expect("proto generic models should include the Temporal converter support module");

    assert!(models.contains("class PayloadBackedEnvelope(typing.Generic[OutputT, ContextT]):"));
    assert!(models.contains("\"PayloadBackedEnvelope[typing.Any, typing.Any]\""));
    assert!(models.contains("\"PayloadBackedOutput[typing.Any]\""));
    assert!(models.contains("\"PayloadBackedContext[typing.Any]\""));
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
    assert!(support.contains("type_hint: type[typing.Any] | None = None,"));
    assert!(support.contains("converter.from_payload(_clone_payload(proto), type_hint)"));
}

#[test]
fn python_rejects_variant_case_class_name_collisions() {
    let root = project_root();
    let temp_dir = unique_output_path("python-variant-case-collision");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("collision.wit");
    fs::write(
        &input_path,
        r#"package example:collision@1.0.0;

world system {
  export example;
}

interface example {
  variant notification-target {
    email(string),
  }

  record notification-target-email {
    value: string,
  }

  record request {
    target: notification-target,
    reserved: notification-target-email,
  }

  call: func(request: request);
}
"#,
    )
    .unwrap();
    let spec = nexgen::parser::load_api_spec_from_wit_for_language_with_inputs(
        nexgen::language::Language::Python,
        &[input_path],
    )
    .unwrap();
    let descriptors = DescriptorIndex::load(&descriptor_path(&root)).unwrap();

    let error = generate_source(
        nexgen::language::Language::Python,
        spec,
        &descriptors,
        &SupportFiles::default(),
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("Python generated name `NotificationTargetEmail`")
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn python_rejects_support_namespace() {
    let root = project_root();
    let spec = nexgen::parser::load_api_spec_from_wit_for_language_with_inputs(
        nexgen::language::Language::Python,
        &example_input_paths(&root, PRIMARY_EXAMPLE_ID),
    )
    .unwrap();
    let descriptors = nexgen::descriptors::DescriptorIndex::load(&descriptor_path(&root)).unwrap();
    let err = generate_source(
        nexgen::language::Language::Python,
        spec.clone(),
        &descriptors,
        &SupportFiles {
            fragments: vec![SupportFragmentSpec {
                path: "support.py".to_string(),
                contents: String::new(),
                namespace: Some("example.support".to_string()),
            }],
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("support namespace"));
}

/// An inline **structured** object `oneOf` branch on a property: the branch is
/// named `<Union>Object` and emitted as a module-level `BaseModel`, which is what
/// Pydantic selects on for the object member of the union.
/// See `specs/json-schema/features/oneOf.md` ("Object branches").
#[test]
fn python_json_names_inline_object_union_branch() {
    let temp_dir = unique_output_path("py-json-inline-branch");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("detail.yaml");
    fs::write(&input_path, INLINE_OBJECT_BRANCH_SCHEMA).unwrap();
    let output_path = temp_dir.join("detail");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();

    assert!(rendered.contains("payload: DetailPayloadObject | str | None"));
    assert!(rendered.contains("class DetailPayloadObject(pydantic.BaseModel):"));
    assert!(rendered.contains("text: str"));
    // The branch model is part of the module surface, like any named definition.
    let exports = fs::read_to_string(output_path.join("__init__.py")).unwrap();
    assert!(exports.contains("DetailPayloadObject"));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Every constraint a **non-object** branch declares rides inside the union
/// member's own annotation, so Pydantic holds the value to the branch it selected
/// — the native `Field` bounds innermost, the refinement validators wrapping
/// them, and the `uniqueItems`/`contains` validators Pydantic has no native form
/// for. See `specs/json-schema/features/oneOf.md` ("Validator mapping").
#[test]
fn python_json_validates_non_object_union_branch_constraints() {
    let temp_dir = unique_output_path("py-json-branch-constraints");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("bc.yaml");
    fs::write(&input_path, BRANCH_CONSTRAINT_SCHEMA).unwrap();
    let output_path = temp_dir.join("bc");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();

    // The string branch: native length bound innermost, `pattern` wrapping it;
    // the integer branch's bound is its own.
    assert!(rendered.contains(
        "value: typing.Annotated[typing.Annotated[str, pydantic.Field(min_length=3)], pydantic.AfterValidator(_check_pattern(\"^[a-z]+\\\\Z\"))] | typing.Annotated[SpecInt, pydantic.Field(ge=1)] | None"
    ));
    // The array branch: `minItems` natively, `uniqueItems` through the validator
    // a position with no declared field of its own needs.
    assert!(rendered.contains(
        "typing.Annotated[typing.Annotated[list[float], pydantic.Field(min_length=1)], pydantic.AfterValidator(_check_unique_items)] | typing.Literal[\"auto\", \"manual\"] | None"
    ));
    // The validators are imported from the runtime module.
    assert!(rendered.contains("_check_pattern,"));
    assert!(rendered.contains("_check_unique_items,"));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A union in an element position: the loader names it, so Python emits an
/// ordinary union alias and Pydantic selects the branch per element. An optional
/// field whose *elements* are nullable still needs its own `| None` — the
/// element's `None` is not the field's.
/// See `specs/json-schema/features/oneOf.md` ("Unions in element positions").
#[test]
fn python_json_annotates_element_position_unions() {
    let temp_dir = unique_output_path("py-json-element-union");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("bag.yaml");
    fs::write(&input_path, ELEMENT_UNION_SCHEMA).unwrap();
    let output_path = temp_dir.join("bag");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();

    assert!(rendered.contains("BagSegmentsItem: typing.TypeAlias = str | SpecInt"));
    assert!(rendered.contains("segments: list[BagSegmentsItem] | None"));
    assert!(rendered.contains("choices: list[Choice] | None"));
    assert!(rendered.contains("slots: list[str | None] | None"));
    let exports = fs::read_to_string(output_path.join("__init__.py")).unwrap();
    assert!(exports.contains("BagSegmentsItem"));
    fs::remove_dir_all(temp_dir).unwrap();
}
#[test]
fn python_direct_transfer_hooks_preserve_model_types() {
    let root = project_root();
    let package = generate_python_package_files(
        &example_input_paths(&root, "generic-models"),
        &[descriptor_path(&root)],
    );
    let models = package
        .get(&PathBuf::from("models.py"))
        .expect("Python package should include models.py");
    let operation = package
        .get(&PathBuf::from("operations/complete.py"))
        .expect("Python package should include the Complete operation module");

    assert!(models.contains(
        "class _GenericResponseTransferTypeConverter(temporalio.converter.TransferTypeConverter[GenericResponse[ContextT, OutputT, MetadataT], dict[str, typing.Any]]):"
    ));
    assert!(models.contains("value: dict[str, typing.Any],"));
    assert!(models.contains(") -> GenericResponse[ContextT, OutputT, MetadataT]:"));
    assert!(models.contains(
        "(_context_t_type, _output_t_type, _metadata_t_type) = typing.get_args(type_hint)"
    ));
    assert!(models.contains("def _operation_completion_result_from_transfer_type("));
    assert!(models.contains(") -> OperationCompletionResult[OutputT]:"));
    assert!(models.contains("def _operation_completion_result_to_transfer_type("));
    assert!(models.contains("class _ReuseCompletionResultTransferTypeConverter("));
    assert!(models.contains("completion=_operation_completion_result_from_transfer_type("));
    assert!(models.contains("result=_operation_completion_result_from_transfer_type("));
    assert!(models.contains(
        "\"completion\": _operation_completion_result_to_transfer_type(value.completion)"
    ));
    assert!(
        models.contains("\"result\": _operation_completion_result_to_transfer_type(value.result)")
    );
    assert_eq!(
        models
            .matches("def _operation_completion_result_from_transfer_type(")
            .count(),
        1
    );
    assert_eq!(
        models
            .matches("def _operation_completion_result_to_transfer_type(")
            .count(),
        1
    );
    assert!(!models.contains("class _OperationCompletionResultTransferTypeConverter"));
    assert!(!models.contains("_generic_response_transfer_type_converter"));
    assert!(!models.contains("_operation_completion_result_transfer_type_converter"));
    assert!(!models.contains("transfer_type: type[dict[str, typing.Any]] | None = dict"));
    assert!(operation.contains("output_type=GenericResponse[typing.Any, typing.Any, typing.Any],"));

    let showcase = generate_python_package_files(
        &example_input_paths(&root, "type-showcase"),
        &[descriptor_path(&root)],
    );
    let user = showcase
        .get(&PathBuf::from("_resources/user.py"))
        .expect("Python package should include the User resource module");
    assert!(user.contains(
        "class _UserTransferTypeConverter(temporalio.converter.TransferTypeConverter[User, dict[str, typing.Any]]):"
    ));
}

#[test]
fn python_direct_transfer_hooks_cover_resource_variant_fields() {
    let root = project_root();
    let temp_dir = unique_output_path("python-resource-variant");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("resource-variant.wit");
    fs::write(
        &input_path,
        r#"package example:resource-variant@1.0.0;

world system {
  export example;
}

interface example {
  record address {
    line: string,
  }

  variant destination {
    email(address),
    none,
  }

  resource subscription {
    constructor(destination: destination);
  }
}
"#,
    )
    .unwrap();

    let package = generate_python_package_files(&[input_path], &[descriptor_path(&root)]);
    let models = package
        .get(&PathBuf::from("models.py"))
        .expect("Python package should include models.py");
    let resource = package
        .get(&PathBuf::from("_resources/subscription.py"))
        .expect("Python package should include the Subscription resource module");

    assert!(models.contains("def _destination_from_transfer_type("));
    assert!(models.contains("def _destination_to_transfer_type("));
    assert!(models.contains("temporalio.converter.value_to_type(Address, value[\"value\"])"));
    assert!(models.contains("dataclasses.asdict(value.value)"));
    assert!(resource.contains("class _SubscriptionTransferTypeConverter("));
    assert!(resource.contains("_destination_from_transfer_type("));
    assert!(resource.contains("_destination_to_transfer_type("));
    fs::remove_dir_all(temp_dir).unwrap();
}
