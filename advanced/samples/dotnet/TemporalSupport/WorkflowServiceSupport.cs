using Temporalio.Converters;
using Temporalio.Workflows;

namespace Nexgen.Support
{
    internal static class WorkflowServiceSerializationContexts
    {
        internal static ISerializationContext SignalWithStartWorkflow(SignalWithStartWorkflowRequest request) =>
            new ISerializationContext.Workflow(request.Namespace, request.Id);
    }

    internal static class WorkflowServiceStorageTargets
    {
        internal static IStorageDriverTargetInfo SignalWithStartWorkflow(SignalWithStartWorkflowRequest request) =>
            new IStorageDriverTargetInfo.Workflow(
                request.Namespace, request.Id, RunId: null, request.Workflow);
    }
}
