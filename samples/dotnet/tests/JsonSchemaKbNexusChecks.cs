using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using NexusRpc;
using NexusRpc.Handlers;
using Temporalio.Activities;
using Temporalio.Api.Common.V1;
using Temporalio.Api.Enums.V1;
using Temporalio.Client;
using Temporalio.Converters;
using Temporalio.Exceptions;
using Temporalio.Nexus;
using Temporalio.Testing;
using Temporalio.Worker;
using Temporalio.Workflows;
using Xunit;
using KbBlock = Nexgen.JsonSchema.Kb.Content.Block;
using KbCategory = Nexgen.JsonSchema.Kb.Tree.Category;
using KbOperations = Nexgen.JsonSchema.Kb.Kb;
using KbPage = Nexgen.JsonSchema.Kb.Content.Page;

namespace Nexgen.DotNetExamples.Tests
{
    /// <summary>
    /// Drives the generated <see cref="KbOperations.IKnowledgeBaseService"/>
    /// Nexus service definition end to end over a real Temporal + Nexus
    /// endpoint. Unlike <see cref="JsonSchemaRuntimeChecks"/>, which round-trips
    /// fixtures through the data converter, the caller workflow here calls every
    /// operation through the SDK's typed Nexus workflow client — there is no
    /// generated API client — and the handler implements the generated
    /// interface, exercising both the service/operation definitions and the
    /// models over the wire in both directions. <c>GetPage</c> and
    /// <c>PutBlock</c> are synchronous operations; <c>GetCategoryTree</c> is an
    /// asynchronous, workflow-backed one whose workflow loads the tree through
    /// an activity, so a generated model also crosses the workflow ⇄ activity
    /// and Nexus completion-callback boundaries.
    /// </summary>
    public class JsonSchemaKbNexusChecks
    {
        private const string Endpoint = "knowledge-base";
        private const string ServiceName = "example.kb.v1.KnowledgeBaseService";

        private static readonly NexusWorkflowOperationOptions OperationOptions =
            new() { ScheduleToCloseTimeout = TimeSpan.FromSeconds(30) };

        /// <summary>
        /// How long the category-tree activity works before answering, so the
        /// asynchronous operation is genuinely outstanding for a while.
        /// </summary>
        private static readonly TimeSpan ActivityDelay = TimeSpan.FromSeconds(1);

        /// <summary>
        /// Activities behind the asynchronous <c>GetCategoryTree</c> operation.
        /// Generated models are both the activity's input and its result.
        /// </summary>
        public sealed class KbActivities
        {
            [Activity]
            public async Task<KbCategory.Category> LoadCategoryTreeAsync(KbOperations.GetCategoryTreeInput input)
            {
                await Task.Delay(ActivityDelay, ActivityExecutionContext.Current.CancellationToken);
                Assert.Equal("root", input.RootId);
                return JsonSchemaRuntimeChecks.Decode<KbCategory.Category>("kb", "category-tree.json");
            }
        }

        /// <summary>
        /// The workflow an asynchronous <c>GetCategoryTree</c> operation starts:
        /// it hands the generated input to an activity and returns the
        /// generated result as the operation's result.
        /// </summary>
        [Workflow]
        public class CategoryTreeWorkflow
        {
            [WorkflowRun]
            public Task<KbCategory.Category> RunAsync(KbOperations.GetCategoryTreeInput input) =>
                Workflow.ExecuteActivityAsync(
                    (KbActivities activities) => activities.LoadCategoryTreeAsync(input),
                    new() { StartToCloseTimeout = TimeSpan.FromSeconds(10) });
        }

        /// <summary>
        /// Handler for the generated service interface, backed by wire fixtures.
        /// </summary>
        [NexusServiceHandler(typeof(KbOperations.IKnowledgeBaseService))]
        public sealed class KnowledgeBaseServiceHandler
        {
            private readonly List<string> calls = new();
            private readonly List<string> workflowIds = new();

            public IReadOnlyList<string> Calls
            {
                get
                {
                    lock (calls)
                    {
                        return calls.ToList();
                    }
                }
            }

            /// <summary>IDs of the workflows asynchronous operations started.</summary>
            public IReadOnlyList<string> WorkflowIds
            {
                get
                {
                    lock (calls)
                    {
                        return workflowIds.ToList();
                    }
                }
            }

            [NexusOperationHandler]
            public IOperationHandler<KbOperations.GetPageInput, KbPage.Page> GetPage() =>
                OperationHandler.Sync<KbOperations.GetPageInput, KbPage.Page>((_, input) =>
                {
                    Record("GetPage");
                    Assert.Equal("page-1", input.PageId);
                    return JsonSchemaRuntimeChecks.Decode<KbPage.Page>("kb", "page.json");
                });

            [NexusOperationHandler]
            public IOperationHandler<KbBlock.Block, KbOperations.PutBlockOutput> PutBlock() =>
                OperationHandler.Sync<KbBlock.Block, KbOperations.PutBlockOutput>((_, input) =>
                {
                    Record("PutBlock");
                    Assert.Equal("block-1", input.BlockId);
                    Assert.True(input.Style?.Bold);
                    return JsonSchemaRuntimeChecks.Decode<KbOperations.PutBlockOutput>("kb", "put-block-output.json");
                });

            /// <summary>
            /// Asynchronous: starts <see cref="CategoryTreeWorkflow"/> and
            /// completes when it does. The generated interface is unchanged —
            /// sync or async is the handler's choice.
            /// </summary>
            [NexusOperationHandler]
            public IOperationHandler<KbOperations.GetCategoryTreeInput, KbCategory.Category> GetCategoryTree() =>
                WorkflowRunOperationHandler.FromHandleFactory(
                    (WorkflowRunOperationContext context, KbOperations.GetCategoryTreeInput input) =>
                    {
                        Record("GetCategoryTree");
                        Assert.Equal("root", input.RootId);
                        var workflowId = $"category-tree-{context.HandlerContext.RequestId}";
                        lock (calls)
                        {
                            workflowIds.Add(workflowId);
                        }
                        return context.StartWorkflowAsync(
                            (CategoryTreeWorkflow workflow) => workflow.RunAsync(input),
                            new() { Id = workflowId });
                    });

            private void Record(string operation)
            {
                lock (calls)
                {
                    calls.Add(operation);
                }
            }
        }

        /// <summary>
        /// What the caller workflow returns for assertion on the client side.
        /// </summary>
        public sealed record KbResult(
            string BlockId,
            string? CategoryChildId,
            bool PutBlockWasAsync,
            bool CategoryTreeWasAsync,
            string PageId,
            long Revision,
            string InvalidInputCause,
            string? InvalidInputErrorType,
            bool? InvalidInputRetryable,
            string InvalidInputMessage,
            string InvalidInputRootCause);

        [Workflow]
        public class KnowledgeBaseCallerWorkflow
        {
            [WorkflowRun]
            public async Task<KbResult> RunAsync()
            {
                var kb = Workflow.CreateNexusWorkflowClient<KbOperations.IKnowledgeBaseService>(
                    new NexusWorkflowClientOptions(Endpoint));

                var page = await kb.ExecuteNexusOperationAsync(
                    service => service.GetPage(new KbOperations.GetPageInput { PageId = "page-1" }),
                    OperationOptions);
                var block = page.Blocks?.FirstOrDefault()
                    ?? throw new InvalidOperationException("expected page block");
                // Started rather than executed, like GetCategoryTree below: a
                // synchronous operation answers with its result and no operation
                // token, the contrast for the asynchronous one.
                var putBlockOperation = await kb.StartNexusOperationAsync(
                    service => service.PutBlock(block), OperationOptions);
                var putBlockOutput = await putBlockOperation.GetResultAsync();
                // An asynchronous operation answers with an operation token; its
                // result arrives later through the completion callback.
                var categoryOperation = await kb.StartNexusOperationAsync(
                    service => service.GetCategoryTree(new KbOperations.GetCategoryTreeInput { RootId = "root" }),
                    OperationOptions);
                var category = await categoryOperation.GetResultAsync();

                // Deliberately bypass the generated input type on the caller, so
                // the payload reaches the handler worker unvalidated. The worker
                // must reject it as a non-retryable BAD_REQUEST before the user
                // handler runs.
                var raw = Workflow.CreateNexusWorkflowClient(
                    ServiceName, new NexusWorkflowClientOptions(Endpoint));
                NexusOperationFailureException failure;
                try
                {
                    await raw.ExecuteNexusOperationAsync(
                        "GetPage", new Dictionary<string, object?> { ["pageId"] = null }, OperationOptions);
                    throw new InvalidOperationException("invalid Nexus input was accepted");
                }
                catch (NexusOperationFailureException error)
                {
                    failure = error;
                }
                var handlerError = failure.InnerException as HandlerException;

                return new KbResult(
                    putBlockOutput.BlockId,
                    category.Children?.FirstOrDefault()?.Id,
                    putBlockOperation.OperationToken is not null,
                    categoryOperation.OperationToken is not null,
                    page.PageId,
                    putBlockOutput.Revision,
                    failure.InnerException?.GetType().Name ?? "none",
                    handlerError?.ErrorType.ToString(),
                    handlerError?.IsRetryable,
                    failure.InnerException?.Message ?? "",
                    Describe(failure.InnerException?.InnerException));
            }

            /// <summary>
            /// The handler error's cause as the caller recovers it: the
            /// exception type, the application failure's type and
            /// retryability, and the violations decoded from its first detail.
            /// </summary>
            private static string Describe(Exception? error)
            {
                if (error is not ApplicationFailureException application)
                {
                    return error is null ? "none" : $"{error.GetType().Name}: {error.Message}";
                }
                var violations = application.Details.Count > 0
                    ? string.Join("; ", application.Details
                        .ElementAt<IReadOnlyList<Nexgen.JsonSchema.Kb.Violation>>(0)
                        .Select(violation => $"{violation.Path}: {violation.Reason}"))
                    : "no details";
                return $"{application.ErrorType}, nonRetryable={application.NonRetryable}, [{violations}]";
            }
        }

        [Fact]
        public async Task KbOperationsUseRealNexusClient()
        {
            // No custom data converter: the default one already carries the
            // generated models.
            await using var env = await WorkflowEnvironment.StartLocalAsync(new());
            var taskQueue = Guid.NewGuid().ToString();
            var handler = new KnowledgeBaseServiceHandler();
            using var worker = new TemporalWorker(
                env.Client,
                new TemporalWorkerOptions(taskQueue)
                    .AddWorkflow<KnowledgeBaseCallerWorkflow>()
                    .AddWorkflow<CategoryTreeWorkflow>()
                    .AddAllActivities(new KbActivities())
                    .AddNexusService(handler));

            var result = await worker.ExecuteAsync(async () =>
            {
                var endpoint = await env.CreateNexusEndpointAsync(Endpoint, taskQueue);
                try
                {
                    return await env.Client.ExecuteWorkflowAsync(
                        (KnowledgeBaseCallerWorkflow workflow) => workflow.RunAsync(),
                        new WorkflowOptions(Guid.NewGuid().ToString(), taskQueue));
                }
                finally
                {
                    await env.DeleteNexusEndpointAsync(endpoint);
                }
            }, CancellationToken.None);

            Assert.Equal("block-1", result.BlockId);
            Assert.Equal("child", result.CategoryChildId);
            Assert.False(result.PutBlockWasAsync, "PutBlock answered with an operation token");
            Assert.True(result.CategoryTreeWasAsync, "GetCategoryTree completed synchronously");
            Assert.Equal("page-1", result.PageId);
            Assert.Equal(7L, result.Revision);
            Assert.Equal(nameof(HandlerException), result.InvalidInputCause);
            Assert.Equal(nameof(HandlerErrorType.BadRequest), result.InvalidInputErrorType);
            Assert.False(result.InvalidInputRetryable);
            Assert.Equal("Invalid operation input", result.InvalidInputMessage);
            // The generated converter's failure survives the Nexus boundary
            // whole (P11.3): recoverable as a non-retryable
            // PayloadValidationError whose details decode to the violations.
            Assert.Equal(
                "PayloadValidationError, nonRetryable=True, [pageId: explicit null not allowed]",
                result.InvalidInputRootCause);
            Assert.Equal(new[] { "GetPage", "PutBlock", "GetCategoryTree" }, handler.Calls);

            // The workflow behind the asynchronous operation ran the activity,
            // which held the operation open for the full delay, and returned the
            // generated model as its own result.
            var backing = env.Client.GetWorkflowHandle(Assert.Single(handler.WorkflowIds));
            var tree = await backing.GetResultAsync<KbCategory.Category>();
            Assert.Equal("root", tree.Id);
            var events = (await backing.FetchHistoryAsync()).Events;
            var activityScheduled = Assert.Single(events, e => e.EventType == EventType.ActivityTaskScheduled);
            Assert.Equal(
                nameof(KbActivities.LoadCategoryTreeAsync).Replace("Async", ""),
                activityScheduled.ActivityTaskScheduledEventAttributes.ActivityType.Name);
            var activityStarted = Assert.Single(events, e => e.EventType == EventType.ActivityTaskStarted);
            var activityCompleted = Assert.Single(events, e => e.EventType == EventType.ActivityTaskCompleted);
            Assert.True(
                activityCompleted.EventTime.ToDateTime() - activityStarted.EventTime.ToDateTime() >= ActivityDelay,
                "the activity answered before its delay elapsed");
        }

        /// <summary>
        /// Minimal public-API equivalent of Temporal's internal Nexus payload
        /// serializer: Nexus content is a payload's data, its headers the
        /// payload's metadata.
        /// </summary>
        private sealed class TestPayloadSerializer : ISerializer
        {
            private static readonly IPayloadConverter Converter = DataConverter.Default.PayloadConverter;

            public Task<ISerializer.Content> SerializeAsync(object? value)
            {
                var payload = Converter.ToPayload(value);
                return Task.FromResult(new ISerializer.Content(
                    payload.Data.ToByteArray(),
                    payload.Metadata.ToDictionary(entry => entry.Key, entry => entry.Value.ToStringUtf8())));
            }

            public Task<object?> DeserializeAsync(ISerializer.Content content, Type type)
            {
                var payload = new Payload { Data = Google.Protobuf.ByteString.CopyFrom(content.Data) };
                foreach (var header in content.Headers ?? new Dictionary<string, string>())
                {
                    payload.Metadata[header.Key] = Google.Protobuf.ByteString.CopyFromUtf8(header.Value);
                }
                return Task.FromResult(Converter.ToValue(payload, type));
            }
        }

        /// <summary>
        /// The bare NexusRpc dispatcher, below the Temporal worker: the
        /// generated converter rejects the wire while the input is decoded, so
        /// the operation never runs. NexusRpc itself propagates the converter's
        /// failure unclassified; the Temporal worker is what reports it as a
        /// non-retryable BAD_REQUEST, which
        /// <see cref="KbOperationsUseRealNexusClient"/> pins over a real endpoint.
        /// </summary>
        [Fact]
        public async Task InvalidWireIsRejectedBeforeTheNexusOperationRuns()
        {
            var service = new KnowledgeBaseServiceHandler();
            var handler = new Handler(
                new[] { ServiceHandlerInstance.FromInstance(service) }, new TestPayloadSerializer());

            var failure = await Assert.ThrowsAsync<ApplicationFailureException>(() => handler.StartOperationAsync(
                new OperationStartContext(ServiceName, "GetPage", CancellationToken.None, "invalid-wire"),
                new HandlerContent(
                    Encoding.UTF8.GetBytes("{\"pageId\":null}"),
                    new Dictionary<string, string> { ["encoding"] = "json/plain" })));

            Assert.Equal("PayloadValidationError", failure.ErrorType);
            Assert.True(failure.NonRetryable);
            var violations = failure.Details.ElementAt<IReadOnlyList<Nexgen.JsonSchema.Kb.Violation>>(0);
            var violation = Assert.Single(violations);
            Assert.Equal("pageId", violation.Path);
            Assert.Equal("explicit null not allowed", violation.Reason);
            Assert.Empty(service.Calls);
        }
    }
}
