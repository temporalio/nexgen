// Temporalio doesn't have the SDK members that the generated interceptor partial needs yet.
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
        string Service, SystemNexusOperation Operation, object? Arg, object? Headers);

    public partial class WorkflowOutboundInterceptor
    {
        protected WorkflowOutboundInterceptor Next => throw new NotSupportedException();

        public virtual Task<NexusWorkflowOperationHandle<TResult>>
            ScheduleSystemNexusOperationAsync<TResult>(ScheduleSystemNexusOperationInput input) =>
            throw new NotSupportedException();
    }
}

namespace Temporalio.Worker
{
    internal partial class WorkflowInstance
    {
        private readonly Lazy<Interceptors.WorkflowOutboundInterceptor> outbound;

        public WorkflowInstance()
        {
            Root = new(this);
            outbound = new(() => Root);
        }

        public OutboundImpl Root { get; }

        public Task<NexusWorkflowOperationHandle<TResult>> StartAsync<TResult>(
            string service, string operationName, object? arg) =>
            StartSystemNexusOperationAsync<TResult>(service, operationName, arg);

        internal partial class OutboundImpl : Interceptors.WorkflowOutboundInterceptor
        {
            private readonly WorkflowInstance instance;

            public OutboundImpl(WorkflowInstance instance) => this.instance = instance;

            public List<Interceptors.ScheduleSystemNexusOperationInput> Scheduled { get; } = new();

            public override Task<NexusWorkflowOperationHandle<TResult>>
                ScheduleSystemNexusOperationAsync<TResult>(
                    Interceptors.ScheduleSystemNexusOperationInput input)
            {
                Scheduled.Add(input);
                return Task.FromResult<NexusWorkflowOperationHandle<TResult>>(null!);
            }
        }
    }
}
