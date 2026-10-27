//! Generated .NET compilation integration tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

mod support;

#[path = "support/dotnet_json_schema.rs"]
mod dotnet_json_schema;

#[path = "support/json_schema/selected_tests.rs"]
mod json_schema_tests;

const JSON_SCHEMA_TARGET: support::json_schema::toolchain::Target =
    support::json_schema::toolchain::Target::CSharp;

static DOTNET_COMMAND_LOCK: Mutex<()> = Mutex::new(());

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
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
fn checked_in_system_nexus_interceptor_compiles() {
    let root = project_root();
    let interceptor = fs::read_to_string(root.join(
        "advanced/samples/dotnet/wit/workflow-service/SystemNexusWorkflowOutboundInterceptor.cs",
    ))
    .unwrap();

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
