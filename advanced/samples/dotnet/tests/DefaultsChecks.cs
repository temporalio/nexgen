using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Reflection;
using System.Threading;
using System.Threading.Tasks;
using NexusRpc;
using NexusRpc.Handlers;
using Temporalio.Client;
using Temporalio.Testing;
using Temporalio.Worker;
using Temporalio.Workflows;
using Defaults = Nexgen.DefaultsService;
using Xunit;

namespace Nexgen.DotNetExamples.Tests
{
    public class DefaultsChecks
    {
        [Fact]
        public void OptionsAndModelSerializationDoNotEvaluateDefaults()
        {
            // Reading workflow context here would fail: this is not a workflow.
            var options = new Defaults.SubmitOptions();
            Assert.Null(options.Namespace);
            Assert.Null(options.Attempts);
            var model = new Defaults.Request(
                0, false, 0, Defaults.MyEnum.Unspecified,
                Defaults.MyFlags.None, "", "")
            {
                Retry = new Defaults.RecordWithDefaults(0, ""),
            };
            for (var i = 0; i < 2; i++)
            {
                var decoded = JsonSerializer.Deserialize<Defaults.Request>(
                    JsonSerializer.Serialize(model))!;
                Assert.Equal("", decoded.Namespace);
                Assert.Equal(0u, decoded.Attempts);
                Assert.False(decoded.Enabled);
                Assert.NotNull(decoded.Retry);
                uint attempts = decoded.Retry!.Attempts;
                Assert.Equal(0u, attempts);
                Assert.Equal("", decoded.Retry.Label);
            }
            var nullability = new NullabilityInfoContext();
            Assert.Equal(NullabilityState.NotNull, nullability.Create(typeof(Defaults.RecordWithDefaults).GetProperty("Label")!).ReadState);
        }

        [Workflow]
        public class DefaultsCaller
        {
            [WorkflowRun]
            public async Task<Defaults.Request[]> RunAsync()
            {
                var options = new Defaults.SubmitOptions();
                var first = await Defaults.Operations.SubmitAsync(options);
                var second = await Defaults.Operations.SubmitAsync(options);
                if (options.Namespace != null || options.Attempts != null)
                {
                    throw new InvalidOperationException("caller options were mutated");
                }
                var retry = new Defaults.RecordWithDefaults(0, "");
                var third = await Defaults.Operations.SubmitAsync(new Defaults.SubmitOptions
                {
                    Namespace = "", Attempts = 0, Enabled = false, Ratio = 0,
                    Mode = Defaults.MyEnum.Unspecified, Permissions = Defaults.MyFlags.None,
                    AliasLabel = "",
                    Retry = retry,
                    Details = "details",
                });
                if (retry.Attempts != 0 || retry.Label != "")
                {
                    throw new InvalidOperationException("caller nested values were mutated");
                }
                await new Defaults.Sender("unused").SubmitAsync();
                return new[] { first, second, third };
            }
        }

        [Fact]
        public async Task OperationsInitializeDefaultsAndPreserveInboundValues()
        {
            await using var env = await WorkflowEnvironment.StartLocalAsync(new());
            var taskQueue = Guid.NewGuid().ToString();
            var calls = new List<Defaults.Request>();
            var definition = new ServiceDefinition("DefaultsService", new[]
            {
                new OperationDefinition("Submit", typeof(Defaults.Request), typeof(Defaults.Request)),
                new OperationDefinition("SubmitFromSender", typeof(Defaults.Request), typeof(Defaults.Request)),
            });
            var handlers = new Dictionary<string, IOperationHandler<object?, object?>>
            {
                ["Submit"] = OperationHandler.WrapAsGenericHandler(
                    OperationHandler.Sync<Defaults.Request, Defaults.Request>((_ctx, input) =>
                    {
                        calls.Add(input);
                        return input with { Namespace = "", Attempts = 0, Retry = new Defaults.RecordWithDefaults(0, "") };
                    })),
            };
            handlers["SubmitFromSender"] = handlers["Submit"];
            using var worker = new TemporalWorker(env.Client, new TemporalWorkerOptions(taskQueue)
                .AddWorkflow<DefaultsCaller>()
                .AddNexusService(new ServiceHandlerInstance(definition, handlers)));
            var results = await worker.ExecuteAsync(async () =>
            {
                var endpoint = await env.CreateNexusEndpointAsync("defaults", taskQueue);
                try
                {
                    return await env.Client.ExecuteWorkflowAsync(
                        (DefaultsCaller workflow) => workflow.RunAsync(),
                        new WorkflowOptions(Guid.NewGuid().ToString(), taskQueue));
                }
                finally
                {
                    await env.DeleteNexusEndpointAsync(endpoint);
                }
            }, CancellationToken.None);

            Assert.Equal(4, calls.Count);
            var first = calls[0];
            Assert.Equal(env.Client.Options.Namespace, first.Namespace);
            Assert.Equal(3u, first.Attempts);
            Assert.True(first.Enabled);
            Assert.Equal(1.5, first.Ratio);
            Assert.Equal(Defaults.MyEnum.Standard, first.Mode);
            Assert.Equal(Defaults.MyFlags.Read | Defaults.MyFlags.Write, first.Permissions);
            Assert.Equal("alias", first.AliasLabel);
            Assert.Null(first.Retry);
            Assert.Null(first.Metadata);
            Assert.Equal(first.Namespace, calls[3].Namespace);
            Assert.Null(calls[3].Retry);
            Assert.Null(calls[3].Metadata);

            var explicitValue = calls[2];
            Assert.Equal("", explicitValue.Namespace);
            Assert.Equal(0u, explicitValue.Attempts);
            Assert.False(explicitValue.Enabled);
            Assert.Equal(Defaults.MyEnum.Unspecified, explicitValue.Mode);
            Assert.Equal(0u, explicitValue.Retry!.Attempts);
            Assert.Equal("", explicitValue.Retry.Label);
            Assert.Equal("untitled", explicitValue.Metadata!.Summary);
            Assert.Equal("details", explicitValue.Metadata.Details);
            Assert.All(results, result =>
            {
                Assert.Equal("", result.Namespace);
                Assert.Equal(0u, result.Attempts);
                Assert.NotNull(result.Retry);
                Assert.Equal(0u, result.Retry!.Attempts);
                Assert.Equal("", result.Retry.Label);
            });
        }
    }
}
