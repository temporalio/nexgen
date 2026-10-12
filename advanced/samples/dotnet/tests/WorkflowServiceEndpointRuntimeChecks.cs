using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;
using NexusRpc;
using NexusRpc.Handlers;
using Nexgen.Support;
using Temporalio.Client;
using Temporalio.Common;
using Temporalio.Converters;
using Temporalio.Testing;
using Temporalio.Worker;
using Temporalio.Workflows;
using Xunit;

namespace Nexgen.DotNetExamples.Tests
{

    public class WorkflowServiceEndpointRuntimeChecks
    {
        [Workflow]
        public class WorkflowServiceCallerWorkflow
        {
            [WorkflowRun]
            public async Task<WorkflowHandleResult> RunAsync(string taskQueue)
            {
                var handle = await Operations.SignalWithStartWorkflowAsync(
                    "StartedWorkflow",
                    new object?[] { "workflow-input" },
                    "WakeUp",
                    new object?[] { "signal-input" },
                    new SignalWithStartWorkflowOptions("started-workflow-id", taskQueue)
                    {
                        ExecutionTimeout = TimeSpan.FromSeconds(30),
                        RetryPolicy = new RetryPolicy
                        {
                            MaximumAttempts = 3,
                        },
                        StaticSummary = "workflow summary",
                        StaticDetails = "workflow details",
                    });
                return new WorkflowHandleResult(handle.Id, handle.RunId);
            }
        }

        public class WorkflowHandleResult
        {
            public WorkflowHandleResult(string id, string? runId)
            {
                Id = id;
                RunId = runId;
            }

            public string Id { get; }
            public string? RunId { get; }
        }

        [Fact]
        public async Task GeneratedWorkflowServiceOperationRoundTripsThroughRuntime()
        {
            await using var env = await WorkflowEnvironment.StartLocalAsync(new());
            var client = new TemporalClient(
                env.Client.Connection,
                new TemporalClientOptions
                {
                    DataConverter = DataConverter.Default,
                    Namespace = "default",
                });
            var taskQueue = Guid.NewGuid().ToString();
            var calls = new List<SignalWithStartWorkflowRequest>();
            var serviceHandler = WorkflowServiceHandler(calls);
            var workerOptions = new TemporalWorkerOptions(taskQueue)
                .AddWorkflow<WorkflowServiceCallerWorkflow>()
                .AddNexusService(serviceHandler);
            using var worker = new TemporalWorker(client, workerOptions);

            // The samples compile the generated code outside the SDK. Thus the SDK's System Nexus
            // payload converter does not push this context. The test pushes the context here.
            using var converterContext = SystemNexusConverterContext.Push(
                DataConverter.Default.PayloadConverter,
                DataConverter.Default.FailureConverter);
            var result = await worker.ExecuteAsync(async () =>
            {
                var endpoint = await env.CreateNexusEndpointAsync("temporal-system", taskQueue);
                try
                {
                    return await client.ExecuteWorkflowAsync(
                        (WorkflowServiceCallerWorkflow workflow) => workflow.RunAsync(taskQueue),
                        new WorkflowOptions(Guid.NewGuid().ToString(), taskQueue));
                }
                finally
                {
                    await env.DeleteNexusEndpointAsync(endpoint);
                }
            }, CancellationToken.None);

            Assert.Equal("started-workflow-id", result.Id);
            Assert.Equal("run-123", result.RunId);
            var call = Assert.Single(calls);
            Assert.Equal("started-workflow-id", call.Id);
            Assert.Equal("StartedWorkflow", call.Workflow);
            Assert.Equal(taskQueue, call.TaskQueue);
            Assert.Equal("WakeUp", call.Signal);
            Assert.NotNull(call.Args);
            Assert.Single(call.Args);
            Assert.NotNull(call.SignalArgs);
            Assert.Single(call.SignalArgs);
            Assert.Equal(TimeSpan.FromSeconds(30), call.ExecutionTimeout);
            Assert.NotNull(call.RetryPolicy);
            Assert.Equal(3, call.RetryPolicy.MaximumAttempts);
            Assert.NotNull(call.UserMetadata);
            Assert.Equal("workflow summary", call.UserMetadata.StaticSummary?.ToString());
            Assert.Equal("workflow details", call.UserMetadata.StaticDetails?.ToString());
        }

        private static ServiceHandlerInstance WorkflowServiceHandler(
            List<SignalWithStartWorkflowRequest> calls)
        {
            var definition = new ServiceDefinition(
                "temporal.api.workflowservice.v1.WorkflowService",
                new[]
                {
                    new OperationDefinition(
                        "SignalWithStartWorkflowExecution",
                        typeof(SignalWithStartWorkflowRequest),
                        typeof(SignalWithStartWorkflowResponse)),
                });
            var handlers = new Dictionary<string, IOperationHandler<object?, object?>>
            {
                ["SignalWithStartWorkflowExecution"] = OperationHandler.WrapAsGenericHandler(
                    OperationHandler.Sync<SignalWithStartWorkflowRequest, SignalWithStartWorkflowResponse>(
                        (_ctx, input) =>
                        {
                            calls.Add(input);
                            return new SignalWithStartWorkflowResponse
                            {
                                RunId = "run-123",
                                Started = true,
                            };
                        })),
            };
            return new ServiceHandlerInstance(definition, handlers);
        }
    }
}
