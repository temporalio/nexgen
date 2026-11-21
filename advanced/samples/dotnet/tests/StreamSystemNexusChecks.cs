extern alias SystemNexusStreams;

using System;
using Xunit;
using StreamModels = SystemNexusStreams::Temporalio.Workflows;
using WorkflowInstance = SystemNexusStreams::Temporalio.Worker.WorkflowInstance;

namespace Nexgen.DotNetExamples.Tests
{
    public class StreamSystemNexusChecks
    {
        private const string StreamService = "temporal.api.streamservice.v1.StreamService";

        public sealed record Turn(string Role);

        [Fact]
        public void GenericModelsFailInsteadOfBypassingInterception()
        {
            var instance = new WorkflowInstance();
            var streamRef = new StreamModels.StreamRef<Turn>("stream-1");

            var appendError = Assert.Throws<NotSupportedException>(() =>
            {
                _ = instance.StartAsync<StreamModels.AppendRecordsResponse>(
                    StreamService,
                    "AppendRecords",
                    new StreamModels.AppendRecordsRequest<Turn>(
                        streamRef, new StreamModels.AppendRecord<Turn>[0], null, null));
            });
            var readError = Assert.Throws<NotSupportedException>(() =>
            {
                _ = instance.StartAsync<StreamModels.ReadRecordsResponse<Turn>>(
                    StreamService, "ReadRecords", new StreamModels.ReadRecordsRequest<Turn>(streamRef));
            });

            Assert.Contains("generic models", appendError.Message);
            Assert.Contains("generic models", readError.Message);
            Assert.Empty(instance.Root.Scheduled);
        }

        [Theory]
        [InlineData(StreamService, "UnknownOperation")]
        [InlineData("unknown.Service", "ReadRecords")]
        public void UnknownOperationsFail(string service, string operation)
        {
            var instance = new WorkflowInstance();

            var error = Assert.Throws<ArgumentException>(() =>
            {
                _ = instance.StartAsync<object>(service, operation, null);
            });

            Assert.Contains($"{service}/{operation}", error.Message);
            Assert.Empty(instance.Root.Scheduled);
        }
    }
}
