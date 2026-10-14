//! Generated .NET compilation integration tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

mod support;
use support::common;

use common::wit_input_path;

const WORKFLOW_SERVICE_EXAMPLE_ID: &str = "workflow-service";

static DOTNET_COMMAND_LOCK: Mutex<()> = Mutex::new(());

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
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

fn generate_dotnet_output(root: &Path, example_id: &str, output_path: &Path) {
    let mut command = Command::new(support::nexgen_binary());
    command
        .arg("dotnet")
        .args(example_input_paths(root, example_id))
        .args([
            "--descriptors",
            descriptor_path(root).to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
            "--native-api",
            "--support-package",
            "Nexgen.Support",
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
