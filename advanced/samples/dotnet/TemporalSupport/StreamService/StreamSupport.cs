using Temporalio.Workflows;

namespace Nexgen.Support
{
    // Sources of the stream sample, in place of those of the Streams SDK.
    internal static class StreamWorkflowContext
    {
        // The run's Workflow Producer.
        internal static Temporalio.Workflows.StreamWriter Writer() =>
            new()
            {
                Producer = new($"wf:{Workflow.Info.RunId}")
                {
                    Kind = new ProducerKind.Workflow(
                        new WorkflowProducer { RunId = Workflow.Info.RunId }),
                },
            };

        // A new id for each create call. A Workflow's replay gives the same id.
        internal static string RequestId() =>
            (Workflow.InWorkflow ? Workflow.NewGuid() : System.Guid.NewGuid()).ToString();
    }
}
