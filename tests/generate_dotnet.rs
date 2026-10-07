// The `dotnet` generate target lives behind the `advanced` feature.
#![cfg(feature = "advanced")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use nexgen::generator::generate_source;
use nexgen::spec::SupportFragmentSpec;
use nexgen::{GenerateRequest, SupportFiles, generate_to_file};

mod common;
use common::wit_input_path;

const WORKFLOW_SERVICE_EXAMPLE_ID: &str = "workflow-service";
const TYPE_SHOWCASE_EXAMPLE_ID: &str = "type-showcase";
static DOTNET_COMMAND_LOCK: Mutex<()> = Mutex::new(());
static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn descriptor_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/descriptors/temporal_api.bin")
}

fn linked_inputs_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/inputs/deps")
}

fn dotnet_root(root: &Path) -> PathBuf {
    root.join("advanced/samples/dotnet")
}

fn dotnet_command() -> (MutexGuard<'static, ()>, Command) {
    let guard = DOTNET_COMMAND_LOCK.lock().unwrap();
    let mut command = Command::new("dotnet");
    command
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
        .env("DOTNET_NOLOGO", "1");
    (guard, command)
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

fn read_dotnet_output_files(dir: &Path) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, files);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("cs") {
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

fn generate_dotnet_to_string(input_paths: &[PathBuf], descriptor_paths: &[PathBuf]) -> String {
    render_output_files(generate_dotnet_files(input_paths, descriptor_paths))
}

fn generate_dotnet_files(
    input_paths: &[PathBuf],
    descriptor_paths: &[PathBuf],
) -> BTreeMap<PathBuf, String> {
    let temp_dir = unique_output_path("dotnet-rendered");
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        config: nexgen::nexgen_config::NexgenConfig {
            mode: nexgen::generator::GenerationMode::NativeApi,
            ..Default::default()
        },
        language: nexgen::language::Language::Dotnet,
        input_paths: input_paths.to_vec(),
        support_paths: Vec::new(),
        descriptor_paths: descriptor_paths.to_vec(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let files = if output_path.is_file() {
        BTreeMap::from([(
            PathBuf::from("output.cs"),
            fs::read_to_string(&output_path).unwrap(),
        )])
    } else {
        read_dotnet_output_files(&output_path)
    };
    fs::remove_dir_all(temp_dir).unwrap();
    files
}

fn generate_dotnet_output(root: &Path, example_id: &str, output_path: &Path) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexgen"));
    command
        .arg("dotnet")
        .args(example_input_paths(root, example_id))
        .args([
            "--descriptors",
            descriptor_path(root).to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
            "--native-api",
        ]);
    if example_id == WORKFLOW_SERVICE_EXAMPLE_ID {
        command.arg("--system-nexus");
    }
    let status = command.status().unwrap();
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

fn dotnet_msbuild_dir(path: &Path) -> String {
    let mut value = path.to_string_lossy().into_owned();
    if !value.ends_with(std::path::MAIN_SEPARATOR) {
        value.push(std::path::MAIN_SEPARATOR);
    }
    value
}

#[test]
fn dotnet_system_nexus_generation_emits_typed_outbound_interceptor() {
    let root = project_root();
    let output_path = unique_output_path("dotnet-system-nexus-interceptor");
    generate_dotnet_output(&root, WORKFLOW_SERVICE_EXAMPLE_ID, &output_path);
    let interceptor =
        fs::read_to_string(output_path.join("SystemNexusWorkflowOutboundInterceptor.cs")).unwrap();
    let models = fs::read_to_string(output_path.join("Models.cs")).unwrap();

    assert!(interceptor.contains("namespace Temporalio.Worker.Interceptors"));
    assert!(interceptor.contains("public partial class WorkflowOutboundInterceptor"));
    assert!(!interceptor.contains("#pragma warning disable CS1591"));
    assert!(!interceptor.contains(
        "[GeneratedCode(\"nexgen\", null)]\n    public partial class WorkflowOutboundInterceptor"
    ));
    assert!(!interceptor.contains(
        "[GeneratedCode(\"nexgen\", null)]\n    internal partial class WorkflowInstance"
    ));
    assert!(interceptor.contains(
        "[GeneratedCode(\"nexgen\", null)]\n        private Task<NexusWorkflowOperationHandle<TResult>> StartSystemNexusOperationAsync<TResult>"
    ));
    assert!(interceptor.contains("outbound.Value.SignalWithStartWorkflowAsync(request)"));
    assert!(interceptor.contains("if (arg is not SignalWithStartWorkflowRequest request)"));
    assert!(
        interceptor.contains("if (typeof(TResult) != typeof(SignalWithStartWorkflowResponse))")
    );
    assert!(interceptor.contains("expects a SignalWithStartWorkflowRequest request."));
    assert!(interceptor.contains("expects a SignalWithStartWorkflowResponse result."));
    assert!(interceptor.contains(
        "Task<NexusWorkflowOperationHandle<SignalWithStartWorkflowResponse>> SignalWithStartWorkflowAsync(SignalWithStartWorkflowRequest request)"
    ));
    assert!(interceptor.contains("Next.SignalWithStartWorkflowAsync(request)"));
    assert!(interceptor.contains("internal partial class WorkflowInstance"));
    assert!(interceptor.contains("internal partial class OutboundImpl"));
    assert!(interceptor.contains(
        "[GeneratedCode(\"nexgen\", null)]\n            public override Task<NexusWorkflowOperationHandle<SignalWithStartWorkflowResponse>> SignalWithStartWorkflowAsync(SignalWithStartWorkflowRequest request) => instance.outbound.Value.ScheduleSystemNexusOperationAsync<SignalWithStartWorkflowResponse>"
    ));
    assert!(models.contains("/// Static metadata for a workflow execution."));
    assert!(models.contains("/// Result of signaling a workflow and starting it if needed."));
    assert!(models.contains("namespace Temporalio.Workflows"));

    let project_path = unique_output_path("dotnet-system-nexus-interceptor-build");
    fs::create_dir_all(&project_path).unwrap();
    fs::write(project_path.join("Generated.cs"), interceptor).unwrap();
    fs::write(
        project_path.join("SdkStubs.cs"),
        r#"
using System;
using System.Threading.Tasks;

namespace NexusRpc
{
    public sealed record OperationDefinition(string Name, System.Type InputType, System.Type OutputType);
}

namespace Temporalio.Workflows
{
    public sealed class SignalWithStartWorkflowRequest { }
    public sealed class SignalWithStartWorkflowResponse { }
    public class NexusWorkflowOperationHandle<TResult> { }
    public sealed record NexusWorkflowClientOptions(string Endpoint);
    public sealed class NexusWorkflowOperationOptions { }
}

namespace Temporalio.Worker.Interceptors
{
    public sealed record ScheduleNexusOperationInput(
        string Service,
        Temporalio.Workflows.NexusWorkflowClientOptions ClientOptions,
        string OperationName,
        object? Arg,
        Temporalio.Workflows.NexusWorkflowOperationOptions Options,
        object? Headers);

    public sealed record ScheduleSystemNexusOperationInput<TResult>(
        string Service,
        NexusRpc.OperationDefinition Operation,
        object? Arg,
        object? Headers);

    public partial class WorkflowOutboundInterceptor
    {
        protected WorkflowOutboundInterceptor Next { get; }

        protected WorkflowOutboundInterceptor(WorkflowOutboundInterceptor next) => Next = next;

        public virtual Task<Temporalio.Workflows.NexusWorkflowOperationHandle<TResult>> ScheduleNexusOperationAsync<TResult>(
            ScheduleNexusOperationInput input) => Next.ScheduleNexusOperationAsync<TResult>(input);

        public virtual Task<Temporalio.Workflows.NexusWorkflowOperationHandle<TResult>> ScheduleSystemNexusOperationAsync<TResult>(
            ScheduleSystemNexusOperationInput<TResult> input) => Next.ScheduleSystemNexusOperationAsync<TResult>(input);
    }

    public sealed class TestInterceptor : WorkflowOutboundInterceptor
    {
        public TestInterceptor(WorkflowOutboundInterceptor next) : base(next) { }

        public override Task<Temporalio.Workflows.NexusWorkflowOperationHandle<Temporalio.Workflows.SignalWithStartWorkflowResponse>> SignalWithStartWorkflowAsync(
            Temporalio.Workflows.SignalWithStartWorkflowRequest request) =>
            base.SignalWithStartWorkflowAsync(request);
    }
}

namespace Temporalio.Worker
{
    internal partial class WorkflowInstance
    {
        private readonly Lazy<Temporalio.Worker.Interceptors.WorkflowOutboundInterceptor> outbound = null!;

        internal partial class OutboundImpl : Temporalio.Worker.Interceptors.WorkflowOutboundInterceptor
        {
            private readonly WorkflowInstance instance = null!;

            internal OutboundImpl(Temporalio.Worker.Interceptors.WorkflowOutboundInterceptor next) : base(next) { }
        }
    }
}
"#,
    )
    .unwrap();
    fs::write(
        project_path.join("GeneratedInterceptor.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net8.0</TargetFramework><LangVersion>9.0</LangVersion></PropertyGroup></Project>",
    )
    .unwrap();
    let (_dotnet_guard, mut command) = dotnet_command();
    let output = command
        .current_dir(&project_path)
        .args(["build", "--nologo"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(output_path).unwrap();
    fs::remove_dir_all(project_path).unwrap();
}

#[test]
fn cli_generates_dotnet_support_file_from_parameter() {
    let root = project_root();
    let temp_dir = unique_output_path("dotnet-support-file-input");
    fs::create_dir_all(&temp_dir).unwrap();
    let support_path = temp_dir.join("CustomSupport.cs");
    let output_path = temp_dir.join("output");
    fs::write(
        &support_path,
        "namespace Custom\n{\npublic static class CustomSupport { }\n}\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "dotnet",
            wit_input_path(&root, "user-service").to_str().unwrap(),
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
    assert!(
        fs::read_to_string(output_path.join("Support/CustomSupport.cs"))
            .unwrap()
            .contains("public static class CustomSupport")
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn dotnet_support_fragments_with_same_file_name_collide() {
    let root = project_root();
    let spec = nexgen::parser::load_api_spec_from_wit_for_language_with_inputs(
        nexgen::language::Language::Dotnet,
        &example_input_paths(&root, WORKFLOW_SERVICE_EXAMPLE_ID),
    )
    .unwrap();
    let descriptors = nexgen::descriptors::DescriptorIndex::load(&descriptor_path(&root)).unwrap();
    let error = generate_source(
        nexgen::language::Language::Dotnet,
        spec,
        &descriptors,
        &SupportFiles {
            fragments: vec![
                SupportFragmentSpec {
                    path: "first/Helpers.cs".to_string(),
                    contents: String::new(),
                    namespace: None,
                },
                SupportFragmentSpec {
                    path: "second/Helpers.cs".to_string(),
                    contents: String::new(),
                    namespace: None,
                },
            ],
        },
    )
    .unwrap_err();
    let nexgen::error::Error::GeneratedFileSourceConflict {
        path,
        first_source,
        second_source,
        remedy,
    } = error
    else {
        panic!("expected generated-file source conflict, got {error}");
    };
    assert_eq!(path, PathBuf::from("Support/Helpers.cs"));
    assert_eq!(first_source, ".NET support file `first/Helpers.cs`");
    assert_eq!(second_source, ".NET support file `second/Helpers.cs`");
    assert!(remedy.contains("support file `first/Helpers.cs`"));
    assert!(remedy.contains("support file `second/Helpers.cs`"));
}

#[test]
fn dotnet_example_project_builds() {
    let root = project_root();
    let project =
        fs::read_to_string(dotnet_root(&root).join("Nexgen.DotNetExamples.csproj")).unwrap();
    assert!(project.contains("<LangVersion>9.0</LangVersion>"));
    let build_path = unique_output_path("dotnet-build");
    fs::create_dir_all(&build_path).unwrap();
    let base_intermediate_output_path = dotnet_msbuild_dir(&build_path.join("obj"));
    let base_output_path = dotnet_msbuild_dir(&build_path.join("bin"));
    let base_intermediate_output_arg =
        format!("-p:BaseIntermediateOutputPath={base_intermediate_output_path}");
    let base_output_arg = format!("-p:BaseOutputPath={base_output_path}");

    let (_dotnet_guard, mut command) = dotnet_command();
    let output = command
        .current_dir(dotnet_root(&root))
        .args([
            "build",
            "Nexgen.DotNetExamples.csproj",
            "--nologo",
            "-p:LangVersion=9.0",
            "-p:RestoreUseStaticGraphEvaluation=true",
            &base_intermediate_output_arg,
            &base_output_arg,
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(build_path).unwrap();
}

#[test]
fn dotnet_renders_nexus_service_interface_and_resources() {
    let root = project_root();
    let files = generate_dotnet_files(
        &example_input_paths(&root, TYPE_SHOWCASE_EXAMPLE_ID),
        &[descriptor_path(&root)],
    );
    let services = files.get(&PathBuf::from("Services.cs")).unwrap().clone();
    let operations = files.get(&PathBuf::from("Operations.cs")).unwrap().clone();
    let rendered = render_output_files(files);

    assert!(rendered.contains("[NexusService(\"TypeShowcase\")]"));
    assert!(rendered.contains("internal interface ITypeShowcase"));
    assert!(rendered.contains("[NexusOperation(\"SetProfile\")]"));
    assert!(rendered.contains("User SetProfile(SetProfileRequest request);"));
    assert!(!rendered.contains("public sealed class NexgenNexusOperation"));
    assert!(rendered.contains("NexgenOperationRegistry"));
    assert!(rendered.contains("NexgenOperationInfo<GetUserRequest, User>"));
    assert!(!rendered.contains("serializationContext:"));
    assert!(!rendered.contains("WorkflowServiceSerializationContexts"));
    assert!(services.contains("NexgenOperationRegistry"));
    assert!(
        services.contains(
            "IReadOnlyDictionary<(string Service, string Operation), INexgenOperationInfo>"
        )
    );
    assert!(services.contains(
        "private static readonly ServiceDefinition TypeShowcaseServiceDefinition = ServiceDefinition.FromType<ITypeShowcase>();"
    ));
    assert!(services.contains("TypeShowcaseServiceDefinition.Operations[\"GetUser\"]"));
    assert!(!services.contains("OperationDefinition.FromMethod"));
    assert!(!operations.contains("NexgenOperationRegistry"));
    assert!(!operations.contains("ServiceDefinition.FromType"));
    assert!(!rendered.contains("endpoint: \"type-showcase\""));
    assert!(!rendered.contains("requestType: typeof(SetProfileRequest)"));
    assert!(!rendered.contains("responseType: typeof(User)"));
    assert!(!rendered.contains("responseType: typeof(void)"));
    assert!(rendered.contains("[Flags]\n    public enum UserCapability"));
    assert!(rendered.contains("public abstract record NotificationTarget"));
    assert!(rendered.contains("public sealed record Email(string Value) : NotificationTarget;"));
    assert!(rendered.contains("public sealed record None : NotificationTarget;"));
    assert!(rendered.contains("public record User"));
    assert!(rendered.contains("public class GetUserOptions"));
    assert!(rendered.contains("public record GetUserRequest"));
    assert!(rendered.contains("using System.Threading.Tasks;"));
    assert!(
        rendered.contains("private static async Task<User> GetUserAsync(GetUserRequest request)")
    );
    assert!(rendered.contains("public static Task<User> GetUserAsync(GetUserOptions options)"));
    assert!(
        rendered.contains("[GeneratedCode(\"nexgen\", null)]\n    public static class Operations")
    );
    assert!(!rendered.contains("public static partial class Operations"));
    assert!(!rendered.contains("TypeShowcaseOperations"));
    assert!(
        !rendered.contains("public static async Task<User> GetUserAsync(GetUserRequest request)")
    );
    assert!(rendered.contains("public Task<User> UpdateEmailAsync(string email)\n        {\n            var request = new UpdateEmailOptions(UserId, email);\n            return Operations.UpdateEmailAsync(request);\n        }"));
    assert!(rendered.contains("public Task<User> RenameAsync(string displayName)\n        {\n            var request = new RenameOptions(UserId, displayName);\n            return Operations.RenameAsync(request);\n        }"));
    assert!(rendered.contains("public Task DeactivateAsync(string? reason)\n        {\n            var request = new DeactivateOptions(UserId) { Reason = reason };\n            return Operations.DeactivateAsync(request);\n        }"));
    assert!(!rendered.contains("Resource methods require a bound Nexus client."));
}

#[test]
fn dotnet_renders_proto_backed_temporal_types() {
    let root = project_root();
    let files = generate_dotnet_files(
        &example_input_paths(&root, WORKFLOW_SERVICE_EXAMPLE_ID),
        &[descriptor_path(&root)],
    );
    assert!(!files.contains_key(&PathBuf::from("SystemNexusWorkflowOutboundInterceptor.cs")));
    let services = files.get(&PathBuf::from("Services.cs")).unwrap().clone();
    let operations = files.get(&PathBuf::from("Operations.cs")).unwrap().clone();
    let rendered = render_output_files(files);

    assert!(rendered.contains("internal interface IWorkflowService"));
    assert!(rendered.contains("namespace Nexgen.WorkflowService\n{"));
    assert!(rendered.contains("namespace Nexgen.Support\n{"));
    assert!(!rendered.contains("namespace Temporalio.Workflows;"));
    assert!(!rendered.contains("namespace Nexgen.Support;"));
    assert!(rendered.contains(
        "SignalWithStartWorkflowResponse SignalWithStartWorkflow(SignalWithStartWorkflowRequest request);"
    ));
    assert!(rendered.contains("NexgenOperationRegistry"));
    assert!(rendered.contains(
        "NexgenOperationInfo<SignalWithStartWorkflowRequest, SignalWithStartWorkflowResponse>"
    ));
    assert!(rendered.contains(
        "serializationContext: Nexgen.Support.WorkflowServiceSerializationContexts.SignalWithStartWorkflow"
    ));
    assert!(services.contains("NexgenOperationRegistry"));
    assert!(services.contains(
        "serializationContext: Nexgen.Support.WorkflowServiceSerializationContexts.SignalWithStartWorkflow"
    ));
    assert!(
        services.contains(
            "IReadOnlyDictionary<(string Service, string Operation), INexgenOperationInfo>"
        )
    );
    assert!(services.contains(
        "private static readonly ServiceDefinition WorkflowServiceServiceDefinition = ServiceDefinition.FromType<IWorkflowService>();"
    ));
    assert!(services.contains(
        "WorkflowServiceServiceDefinition.Operations[\"SignalWithStartWorkflowExecution\"]"
    ));
    assert!(!services.contains("OperationDefinition.FromMethod"));
    assert!(!operations.contains("NexgenOperationRegistry"));
    assert!(rendered.contains(
        "internal static ISerializationContext SignalWithStartWorkflow(SignalWithStartWorkflowRequest request)"
    ));
    assert!(rendered.contains("new ISerializationContext.Workflow(request.Namespace, request.Id)"));
    assert!(!operations.contains("ServiceDefinition.FromType"));
    assert!(!rendered.contains("endpoint: \"temporal-system\""));
    assert!(!rendered.contains(
        "requestType: typeof(Temporalio.Api.WorkflowService.V1.SignalWithStartWorkflowExecutionRequest)"
    ));
    assert!(!rendered.contains(
        "responseType: typeof(Temporalio.Api.WorkflowService.V1.SignalWithStartWorkflowExecutionResponse)"
    ));
    assert!(rendered.contains("/// Signal a workflow, starting it first if needed."));
    assert!(rendered.contains("/// <returns>A workflow handle to the started workflow.</returns>"));
    assert!(
        rendered
            .contains("/// Request fields for signaling a workflow, starting it first if needed.")
    );
    assert!(rendered.contains("/// <param name=\"workflow\">Workflow type name or workflow expression identifying the workflow to start.</param>"));
    assert!(rendered.contains("/// <param name=\"args\">Arguments for the workflow.</param>"));
    assert!(rendered.contains("/// <param name=\"signal\">Signal name or signal expression to send with the start request.</param>"));
    assert!(rendered.contains("/// <param name=\"signalArgs\">Arguments for the signal.</param>"));
    assert!(rendered.contains("/// <param name=\"options\">Request fields for signaling a workflow, starting it first if needed.</param>"));
    assert!(
        rendered.contains("/// Unique identifier for the workflow execution. Must be nonempty.")
    );
    assert!(rendered.contains("/// Cron schedule for recurring workflow executions. See https://docs.temporal.io/cron-job."));
    assert!(rendered.contains("/// Single-line fixed summary for the workflow execution that may appear in UI and CLI. This can be in single-line Temporal Markdown format."));
    assert!(rendered.contains("/// <summary>\n        /// Arguments for the workflow.\n        /// </summary>\n        public IReadOnlyCollection<object?>? Args"));
    assert!(rendered.contains("/// <summary>\n        /// Arguments for the signal.\n        /// </summary>\n        public IReadOnlyCollection<object?>? SignalArgs"));
    assert!(rendered.contains("Task<Temporalio.Workflows.ExternalWorkflowHandle> SignalWithStartWorkflowAsync<TWorkflow, TResult>"));
    assert!(rendered.contains("public static class Operations"));
    assert!(!rendered.contains("public static partial class Workflow"));
    assert!(!rendered.contains("this NexusWorkflowClient<"));
    assert!(
        rendered.contains("private const string WorkflowServiceEndpoint = \"temporal-system\";")
    );
    assert!(
        rendered.contains(
            "Workflow.CreateNexusWorkflowClient<IWorkflowService>(WorkflowServiceEndpoint)"
        )
    );
    assert!(
        !rendered
            .contains("Workflow.CreateNexusWorkflowClient<IWorkflowService>(\"temporal-system\")")
    );
    assert!(rendered.contains("Expression<Func<TWorkflow, Task<TResult>>> workflow"));
    assert!(rendered.contains("using System.CodeDom.Compiler;"));
    assert!(rendered.contains("[GeneratedCode(\"nexgen\", null)]\n    [NexusService(\"temporal.api.workflowservice.v1.WorkflowService\")]\n    internal interface IWorkflowService"));
    assert!(rendered.contains(
        "[GeneratedCode(\"nexgen\", null)]\n    public record SignalWithStartWorkflowRequest"
    ));
    assert!(rendered.contains(
        "[GeneratedCode(\"nexgen\", null)]\n    public class SignalWithStartWorkflowOptions"
    ));
    assert!(
        !rendered.contains(
            "[GeneratedCode(\"nexgen\", null)]\n    public static partial class Workflow"
        )
    );
    assert!(!rendered.contains(
        "public static partial class Workflow\n    {\n        [GeneratedCode(\"nexgen\", null)]\n        private const string WorkflowServiceEndpoint"
    ));
    assert!(rendered.contains(
        "/// <remarks>WARNING: This API is experimental and may change in the future.</remarks>"
    ));
    assert!(rendered.contains("public class SignalWithStartWorkflowOptions"));
    assert!(
        rendered.contains("public SignalWithStartWorkflowOptions(string id, string taskQueue)")
    );
    assert!(rendered.contains("public SignalWithStartWorkflowRequest(string workflow, string id, string taskQueue, string signal, string @namespace)"));
    assert!(rendered.contains("public string Id { get; set; }\n"));
    assert!(rendered.contains("public string TaskQueue { get; set; }\n"));
    assert!(rendered.contains("public string Workflow { get; init; }\n"));
    assert!(rendered.contains("public string Id { get; init; }\n"));
    assert!(rendered.contains("public string TaskQueue { get; init; }\n"));
    assert!(rendered.contains("public string Signal { get; init; }\n"));
    assert!(rendered.contains(
        "new SignalWithStartWorkflowRequest(workflow, options.Id, options.TaskQueue, signal, Nexgen.Support.TemporalWorkflowContext.WorkflowNamespace())"
    ));
    assert!(rendered.contains("wire.SignalName, wire.Namespace)"));
    assert!(!rendered.contains("get => _namespace"));
    assert!(!rendered.contains("default!"));
    assert!(!rendered.contains("required "));
    assert!(rendered.contains("public record SignalWithStartWorkflowRequest"));
    assert!(rendered.contains("public record UserMetadata"));
    assert!(!rendered.contains("IReadOnlyCollection<object?>? Args { get; set; }"));
    assert!(!rendered.contains("IReadOnlyCollection<object?>? SignalArgs { get; set; }"));
    assert!(rendered.contains("SignalWithStartWorkflowAsync<TWorkflow, TResult>(Expression<Func<TWorkflow, Task<TResult>>> workflow, Expression<Func<TWorkflow, Task>> signal, SignalWithStartWorkflowOptions options)"));
    assert!(rendered.contains("SignalWithStartWorkflowAsync(string workflow, IReadOnlyCollection<object?>? args, string signal, IReadOnlyCollection<object?>? signalArgs, SignalWithStartWorkflowOptions options)"));
    assert!(rendered.contains("SignalWithStartWorkflowAsync<TWorkflow, TResult>(Expression<Func<TWorkflow, Task<TResult>>> workflow, string signal, IReadOnlyCollection<object?>? signalArgs, SignalWithStartWorkflowOptions options)"));
    assert!(rendered.contains("SignalWithStartWorkflowAsync<TWorkflow>(string workflow, IReadOnlyCollection<object?>? args, Expression<Func<TWorkflow, Task>> signal, SignalWithStartWorkflowOptions options)"));
    assert!(rendered.contains("Nexgen.Support.TemporalFunctionNames.ExtractCall(workflow)"));
    assert!(rendered.contains("Nexgen.Support.TemporalFunctionNames.ExtractCall(signal)"));
    assert!(!rendered.contains(
        "private static (MethodInfo Method, IReadOnlyCollection<object?> Args) ExtractCall"
    ));
    assert!(rendered.contains("private static async Task<Temporalio.Workflows.ExternalWorkflowHandle> SignalWithStartWorkflowAsync(SignalWithStartWorkflowRequest request)"));
    assert!(!rendered.contains("public static async Task<Temporalio.Workflows.ExternalWorkflowHandle> SignalWithStartWorkflowAsync(SignalWithStartWorkflowRequest request)"));
    assert!(!rendered.contains("NexusWorkflowOperationOptions"));
    assert!(!rendered.contains("System.TimeSpan? executionTimeout = null"));
    assert!(
        rendered.contains(
            "new SignalWithStartWorkflowRequest(Nexgen.Support.TemporalFunctionNames.WorkflowName(workflowMethod), options.Id, options.TaskQueue"
        )
    );
    assert!(rendered.contains(
        "options.TaskQueue, Nexgen.Support.TemporalFunctionNames.SignalName(signalMethod), Nexgen.Support.TemporalWorkflowContext.WorkflowNamespace())"
    ));
    assert!(rendered.contains("Args = workflowArgs"));
    assert!(rendered.contains("Args = args"));
    assert!(rendered.contains("SignalArgs = signalArgs"));
    assert!(!rendered.contains("Args = workflowArgs.ToProto()"));
    assert!(!rendered.contains("Args = args == null ? null : args.ToProto()"));
    assert!(!rendered.contains("SignalArgs = signalArgs == null ? null : signalArgs.ToProto()"));
    assert!(!rendered.contains("Args = options.Args"));
    assert!(!rendered.contains("SignalArgs = options.SignalArgs"));
    assert!(!rendered.contains("WorkflowNameFromRunMethod"));
    assert!(!rendered.contains("SignalNameFromMethod"));
    assert!(!rendered.contains("var wireRequest = request.ToProto();"));
    assert!(rendered.contains("svc.SignalWithStartWorkflow(request)"));
    assert!(rendered.contains(
        "[Temporalio.Converters.TemporalTransferTypeConverter(typeof(SignalWithStartWorkflowRequest.TransferTypeConverter))]"
    ));
    assert!(rendered.contains(
        "public sealed class TransferTypeConverter : Temporalio.Converters.ITemporalTransferTypeConverter"
    ));
    assert!(!rendered.contains("ITemporalIntermediate"));
    assert!(rendered.contains("internal static SignalWithStartWorkflowRequest FromTransferType("));
    assert!(!rendered.contains("TemporalToIntermediate("));
    assert!(rendered.contains(
        "internal Temporalio.Api.WorkflowService.V1.SignalWithStartWorkflowExecutionRequest ToTransferType()"
    ));
    assert!(!rendered.contains(
        "public Temporalio.Api.WorkflowService.V1.SignalWithStartWorkflowExecutionRequest ToProto(Temporalio.Converters.IPayloadConverter? payloadConverter = null)"
    ));
    assert!(!rendered.contains(
        "public Temporalio.Api.Sdk.V1.UserMetadata ToProto(Temporalio.Converters.IPayloadConverter? payloadConverter = null)"
    ));
    assert!(rendered.contains("using Nexgen.Support;"));
    assert!(rendered.contains("Nexgen.Support.ProtoExtensions.ToWorkflowTypeProto(Workflow"));
    assert!(rendered.contains("Nexgen.Support.ProtoExtensions.ToTaskQueueProto(TaskQueue"));
    assert!(rendered.contains(
        "proto.UserMetadata = (Temporalio.Api.Sdk.V1.UserMetadata)userMetadata.ToTransferType();"
    ));
    assert!(!rendered.contains("var wireRequest = ToProto(request);"));
    assert!(!rendered.contains("private static Temporalio.Api.WorkflowService.V1.SignalWithStartWorkflowExecutionRequest ToProto(SignalWithStartWorkflowRequest request)"));
    assert!(!rendered.contains("Temporalio.Api.Taskqueue.V1.TaskQueue"));
    assert!(rendered.contains("RetryPolicy = options.RetryPolicy"));
    assert!(rendered.contains("ExecutionTimeout = options.ExecutionTimeout"));
    assert!(rendered.contains("proto.RetryPolicy = retryPolicy.ToProto();"));
    assert!(rendered.contains("proto.WorkflowExecutionTimeout = executionTimeout.ToProto();"));
    assert!(rendered.contains("public IReadOnlyCollection<object?>? Args { get; init; }"));
    assert!(rendered.contains("proto.Input = Nexgen.Support.ProtoExtensions.ToPayloads(args);"));
    assert!(rendered.contains("Args = args,"));
    assert!(
        rendered
            .contains("proto.Summary = Nexgen.Support.ProtoExtensions.ToPayload(staticSummary);")
    );
    assert!(rendered.contains("internal static ApiCommon.Payload ToPayload(object? value)"));
    assert!(
        rendered
            .contains("internal static ApiCommon.Payloads ToPayloads(IEnumerable<object?> values)")
    );
    assert!(!rendered.contains("ToProto(this object? value)"));
    assert!(!rendered.contains("ToProto(this IEnumerable<object?> value)"));
    assert!(rendered.contains("internal static Duration ToProto(this TimeSpan value)"));
    assert!(!rendered.contains(" FromProto("));
    assert!(rendered.contains("internal static class TemporalWorkflowContext"));
    assert!(rendered.contains("internal static class TemporalFunctionNames"));
    assert!(rendered.contains("internal static class SystemNexusConverterContext"));
    assert!(rendered.contains("internal static class ProtoExtensions"));
    assert!(rendered.contains("SystemNexusConverterContext.PayloadConverter.ToPayload(value)"));
    assert!(rendered.contains("SystemNexusConverterContext.FailureConverter.ToFailure("));
    assert!(!rendered.contains("CurrentUserPayloadConverter"));
    assert!(
        rendered.contains(
            "internal static ApiCommon.WorkflowType ToWorkflowTypeProto(this string value)"
        )
    );
    assert!(
        rendered
            .contains("internal static ApiTaskQueue.TaskQueue ToTaskQueueProto(this string value)")
    );
    assert!(!rendered.contains("ToProto(default("));
    assert!(!rendered.contains("internal static TProto ToProto<TProto>(this string value)"));
    assert!(!rendered.contains("targetType == typeof"));
    assert!(!rendered.contains("public static class TemporalWorkflowContext"));
    assert!(!rendered.contains("public static class TemporalFunctionNames"));
    assert!(!rendered.contains("public static class ProtoExtensions"));
    assert!(!rendered.contains("ProtoConverters"));
    assert!(!rendered.contains("private static TValue Cast"));
    assert!(!rendered.contains("Cast<TProto>"));
    assert!(!rendered.contains("retryPolicy.ToProto<Temporalio.Api.Common.V1.RetryPolicy>()"));
    assert!(
        !rendered.contains("executionTimeout.ToProto<Google.Protobuf.WellKnownTypes.Duration>()")
    );
    assert!(rendered.contains("Temporalio.Common.RetryPolicy? RetryPolicy"));
    assert!(rendered.contains("Temporalio.Api.Enums.V1.WorkflowIdReusePolicy? IdReusePolicy"));
    assert!(rendered.contains("System.TimeSpan? ExecutionTimeout"));
}

#[test]
fn dotnet_uses_annotated_namespace_and_operations_class() {
    let temp_dir = unique_output_path("dotnet-annotated-namespace");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("main.wit");
    fs::write(
        &input_path,
        r#"
package example:nexus@1.0.0;

world system {
  export workflow-service;
}

/// @nexus.endpoint "temporal-system"
/// @nexus.namespace dotnet="Temporalio.Workflows"
/// @nexus.operations-class dotnet="Workflow"
interface workflow-service {
  record signal-request {
    id: string,
  }

  record signal-response {
    run-id: option<string>,
  }

  /// @nexus.operation name="SignalWithStartWorkflowExecution"
  signal-with-start-workflow: func(request: signal-request) -> signal-response;
}
"#,
    )
    .unwrap();

    let input_paths = vec![input_path];
    let rendered = generate_dotnet_to_string(&input_paths, &[]);

    assert!(rendered.contains("namespace Temporalio.Workflows\n{"));
    assert!(rendered.contains("public static partial class Workflow"));
    assert!(!rendered.contains("WorkflowServiceOperations"));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn dotnet_notification_models_convert_generic_oneofs_without_service() {
    let root = project_root();
    let files = generate_dotnet_files(
        &example_input_paths(&root, "notification-service"),
        &[descriptor_path(&root)],
    );
    let models = files
        .get(&PathBuf::from("Models.cs"))
        .expect("notification output should include Models.cs");

    assert!(!files.contains_key(&PathBuf::from("Services.cs")));
    assert!(!files.contains_key(&PathBuf::from("Operations.cs")));
    assert!(models.contains("namespace Nexgen.NotificationService\n{"));
    assert!(models.contains(
        "[Temporalio.Converters.TemporalTransferTypeConverter(typeof(OnCompleteRequest<,>.TransferTypeConverter))]"
    ));
    assert!(models.contains("public record OnCompleteRequest<TOutput, TSourceContext>"));
    assert!(models.contains(
        "internal static OnCompleteRequest<TOutput, TSourceContext> FromTransferType(Temporalio.Api.NotificationService.V1.OnCompleteRequest wire)"
    ));
    assert!(models.contains("OnCompleteRequestResult<TOutput> resultOneof;"));
    assert!(models.contains("switch (wire.ResultCase)"));
    assert!(models.contains(
        "case Temporalio.Api.NotificationService.V1.OnCompleteRequest.ResultOneofCase.Success:"
    ));
    assert!(models.contains(
        "resultOneof = new OnCompleteRequestResult<TOutput>.Success(Nexgen.Support.ProtoExtensions.FromPayload<TOutput>(wire.Success));"
    ));
    assert!(models.contains(
        "throw new System.InvalidOperationException(\"missing required field OnCompleteRequest.Result\");"
    ));
    assert!(models.contains("if (wire.SourceContext == null)"));
    assert!(models.contains(
        "throw new System.InvalidOperationException(\"missing required field OnCompleteRequest.SourceContext\");"
    ));
    assert!(models.contains(
        "return new OnCompleteRequest<TOutput, TSourceContext>(resultOneof, Nexgen.Support.ProtoExtensions.FromPayload<TSourceContext>(wire.SourceContext));"
    ));
    assert!(!models.contains(" switch {"));
    assert!(models.contains("case OnCompleteRequestResult<TOutput>.Failure failureCase:"));
    assert!(models.contains(
        "                    break;\n                default:\n                    throw new System.InvalidOperationException(\"missing required field OnCompleteRequest.Result\");\n            }\n            proto.SourceContext ="
    ));
    assert!(models.contains(
        "proto.Failure = Nexgen.Support.ProtoExtensions.ToFailureProto(failureCase.Value);"
    ));
    assert!(models.contains(
        "proto.SourceContext = Nexgen.Support.ProtoExtensions.ToPayload(SourceContext);"
    ));
    assert!(models.contains(
        "public object? ToTransferType(object? value) => value is null ? null : ((OnCompleteRequest<TOutput, TSourceContext>)value).ToTransferType();"
    ));
}

#[test]
fn dotnet_proto_oneofs_convert_payloads_and_optional_groups() {
    let root = project_root();
    let files = generate_dotnet_files(
        &example_input_paths(&root, "proto-oneof"),
        &[descriptor_path(&root)],
    );
    let models = files
        .get(&PathBuf::from("Models.cs"))
        .expect("proto oneof output should include Models.cs");

    assert!(models.contains("typeof(Outcome<>.TransferTypeConverter)"));
    assert!(models.contains(
        "new OutcomeValue<TOutput>.Success(wire.Success.Payloads_.Count == 0 ? default! : Nexgen.Support.ProtoExtensions.FromPayloads<TOutput>(wire.Success)[0])"
    ));
    assert!(models.contains("if (wire.Success.Payloads_.Count > 1)"));
    assert!(models.contains(
        "throw new System.InvalidOperationException($\"expected at most one payload in Outcome.Success, found {wire.Success.Payloads_.Count}\");"
    ));
    assert!(models.contains(
        "proto.Success = Nexgen.Support.ProtoExtensions.ToPayloads(new object?[] { successCase.Value });"
    ));
    assert!(models.contains("ActivitySelection? activityOneof;"));
    assert!(models.contains("activityOneof = new ActivitySelection.Type(wire.Type);"));
    assert!(models.contains("activityOneof = null;"));
    assert!(models.contains("Activity = activityOneof,"));
    assert!(models.contains("case ActivitySelection.Type typeCase:"));
    assert!(models.contains(
        "proto.Type = typeCase.Value;\n                    break;\n            }\n            proto.Reason = Reason;"
    ));
}

#[test]
fn dotnet_generic_proto_models_reference_nested_generic_converters() {
    let root = project_root();
    let files = generate_dotnet_files(
        &example_input_paths(&root, "proto-generic"),
        &[descriptor_path(&root)],
    );
    let models = files
        .get(&PathBuf::from("Models.cs"))
        .expect("proto generic output should include Models.cs");

    assert!(models.contains("typeof(PayloadBackedEnvelope<,>.TransferTypeConverter)"));
    assert!(models.contains(
        "return new PayloadBackedEnvelope<TOutput, TContext>(PayloadBackedOutput<TOutput>.FromTransferType(wire.Provider), PayloadBackedContext<TContext>.FromTransferType(wire.Scaler));"
    ));
    assert!(models.contains(
        "proto.Provider = (Temporalio.Api.Compute.V1.ComputeProvider)Provider.ToTransferType();"
    ));
}

#[test]
fn dotnet_model_only_interfaces_use_their_namespace_directive() {
    let temp_dir = unique_output_path("dotnet-model-only-namespace");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("models.wit");
    fs::write(
        &input_path,
        r#"package test:models@1.0.0;

world system {
  export notification-models;
}

/// @nexus.namespace dotnet="Acme.Notifications"
interface notification-models {
  record notification {
    message: string,
  }
}
"#,
    )
    .unwrap();

    let files = generate_dotnet_files(&[input_path], &[]);
    fs::remove_dir_all(temp_dir).unwrap();
    let models = files
        .get(&PathBuf::from("Models.cs"))
        .expect("model-only output should include Models.cs");

    assert!(models.contains("namespace Acme.Notifications\n{"));
    assert!(models.contains("public record Notification"));
}

#[test]
fn dotnet_rejects_type_parameters_with_the_same_csharp_name() {
    let temp_dir = unique_output_path("dotnet-type-parameter-conflict");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("conflict.wit");
    fs::write(
        &input_path,
        r#"package test:conflict@1.0.0;

world system {
  export conflict-models;
}

interface conflict-models {
  type placeholder = string;

  /// @nexus.type-parameter
  type output-t = placeholder;

  /// @nexus.type-parameter
  type output = placeholder;

  record pair {
    first: output-t,
    second: output,
  }
}
"#,
    )
    .unwrap();

    let spec = nexgen::parser::load_api_spec_from_wit_for_language_with_inputs(
        nexgen::language::Language::Dotnet,
        &[input_path],
    )
    .unwrap();
    let descriptors =
        nexgen::descriptors::DescriptorIndex::load(&descriptor_path(&project_root())).unwrap();
    let error = generate_source(
        nexgen::language::Language::Dotnet,
        spec,
        &descriptors,
        &SupportFiles::default(),
    )
    .unwrap_err();
    fs::remove_dir_all(temp_dir).unwrap();

    assert!(
        error.to_string().contains(
            ".NET type parameter `Output` in `conflict-models.pair` maps to `TOutput`, which conflicts with type parameter `OutputT`"
        ),
        "{error}"
    );
}
