// The SDK members that the generated System Nexus interceptor partial needs. Temporalio doesn't
// have them yet, so this stand-in lets a test run the generated dispatch.
#nullable enable

using System;
using System.Collections.Generic;
using System.Runtime.CompilerServices;
using System.Threading.Tasks;
using Temporalio.Workflows;

[assembly: InternalsVisibleTo("Nexgen.DotNetExamples.Tests")]

namespace Temporalio.Worker.Interceptors
{
    public record SystemNexusOperation(string Name, Type InputType, Type OutputType);

    public record ScheduleSystemNexusOperationInput(
        string Service,
        SystemNexusOperation Operation,
        object? Arg,
        IReadOnlyDictionary<string, string>? Headers);

    public partial class WorkflowOutboundInterceptor
    {
        private readonly WorkflowOutboundInterceptor? next;

        protected WorkflowOutboundInterceptor(WorkflowOutboundInterceptor next) => this.next = next;

        private protected WorkflowOutboundInterceptor()
        {
        }

        protected WorkflowOutboundInterceptor Next =>
            next ?? throw new InvalidOperationException("The root interceptor has no next one.");

        public virtual Task<NexusWorkflowOperationHandle<TResult>> ScheduleSystemNexusOperationAsync<TResult>(
            ScheduleSystemNexusOperationInput input) =>
            Next.ScheduleSystemNexusOperationAsync<TResult>(input);
    }
}

namespace Temporalio.Worker
{
    using Temporalio.Worker.Interceptors;

    internal partial class WorkflowInstance
    {
        private readonly Lazy<WorkflowOutboundInterceptor> outbound;

        public WorkflowInstance(Func<WorkflowOutboundInterceptor, WorkflowOutboundInterceptor> wrapRoot)
        {
            Root = new OutboundImpl(this);
            outbound = new(() => wrapRoot(Root));
        }

        public OutboundImpl Root { get; }

        public Task<NexusWorkflowOperationHandle<TResult>> StartAsync<TResult>(
            string service, string operationName, object? arg) =>
            StartSystemNexusOperationAsync<TResult>(service, operationName, arg);

        internal partial class OutboundImpl : WorkflowOutboundInterceptor
        {
            private readonly WorkflowInstance instance;

            public OutboundImpl(WorkflowInstance instance) => this.instance = instance;

            public List<ScheduleSystemNexusOperationInput> Scheduled { get; } = new();

            // The SDK schedules the operation here. The stand-in keeps the input.
            public override Task<NexusWorkflowOperationHandle<TResult>> ScheduleSystemNexusOperationAsync<TResult>(
                ScheduleSystemNexusOperationInput input)
            {
                Scheduled.Add(input);
                return Task.FromResult<NexusWorkflowOperationHandle<TResult>>(null!);
            }
        }
    }
}
