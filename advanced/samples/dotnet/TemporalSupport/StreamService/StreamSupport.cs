using Temporalio.Workflows;

namespace Nexgen.Support
{
    // Sources of the stream sample, in place of those of the Streams SDK.
    internal static class StreamWorkflowContext
    {
        internal static Temporalio.Workflows.Producer Producer() =>
            new($"wf:{Workflow.Info.RunId}") { RunId = Workflow.Info.RunId };

        // The run that a retry run's Workflow retries. Null elsewhere.
        internal static string? WithdrawRun() =>
            Workflow.InWorkflow && Workflow.Info.Attempt > 1 ? Workflow.Info.ContinuedRunId : null;

        // A new id for each create call. A Workflow's replay gives the same id.
        internal static string RequestId() =>
            (Workflow.InWorkflow ? Workflow.NewGuid() : System.Guid.NewGuid()).ToString();
    }
}
