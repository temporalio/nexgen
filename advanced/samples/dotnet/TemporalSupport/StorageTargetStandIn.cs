// Remove this adapter when the samples can use a Temporalio release containing
// temporalio/sdk-dotnet#1056. The pinned SDK predates the storage target contract;
// the SDK PR itself validates routing and replay against the real implementation.
namespace Temporalio.Converters
{
    internal interface IStorageDriverTargetInfo
    {
        public sealed record Workflow(
            string Namespace,
            string? WorkflowId,
            string? RunId,
            string? WorkflowType) : IStorageDriverTargetInfo;
    }
}
