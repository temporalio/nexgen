using Temporalio.Workflows;

namespace Nexgen.Support
{
    // Sources of the stream sample, in place of those of the Streams SDK.
    internal static class StreamWorkflowContext
    {
        // The run's Workflow Producer.
        internal static Temporalio.Workflows.Producer Producer() =>
            new()
            {
                Kind = new ProducerKind.Workflow(new WorkflowProducer(
                    Workflow.Info.Namespace, Workflow.Info.WorkflowId, Workflow.Info.RunId)),
            };
    }
}
