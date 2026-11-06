extern alias SystemNexusStreams;

using System.Collections.Generic;
using System.Threading.Tasks;
using Temporalio.Workflows;
using Xunit;
using Interceptors = SystemNexusStreams::Temporalio.Worker.Interceptors;
using StreamModels = SystemNexusStreams::Temporalio.Workflows;
using WorkflowInstance = SystemNexusStreams::Temporalio.Worker.WorkflowInstance;

namespace Nexgen.DotNetExamples.Tests
{
    public class StreamSystemNexusChecks
    {
        private const string StreamService = "temporal.api.streamservice.v1.StreamService";

        public sealed record Turn(string Role);

        private sealed class Recorder : Interceptors.WorkflowOutboundInterceptor
        {
            public Recorder(Interceptors.WorkflowOutboundInterceptor next)
                : base(next)
            {
            }

            public List<string> Operations { get; } = new();

            public override Task<NexusWorkflowOperationHandle<TResult>> ScheduleSystemNexusOperationAsync<TResult>(
                Interceptors.ScheduleSystemNexusOperationInput input)
            {
                Operations.Add(input.Operation.Name);
                return base.ScheduleSystemNexusOperationAsync<TResult>(input);
            }
        }

        // C# generics are invariant, so a typed hook can't take the requests and responses of
        // every Stream. A Stream<Turn> publish and read go to the generic hook with their types.
        [Fact]
        public async Task StreamOfTurnCallsKeepTheirTypesThroughTheDispatch()
        {
            Recorder? recorder = null;
            var instance = new WorkflowInstance(root => recorder = new Recorder(root));
            var streamRef = new StreamModels.StreamRef("stream-1");
            var publish = new StreamModels.AppendRecordsRequest<Turn>(
                streamRef, new[] { new StreamModels.AppendRecord<Turn>(new Turn("user")) }, null, null);
            var read = new StreamModels.ReadRecordsRequest(streamRef);
            var describe = new StreamModels.DescribeStreamRequest(streamRef);

            await instance.StartAsync<StreamModels.AppendRecordsResponse>(
                StreamService, "AppendRecords", publish);
            await instance.StartAsync<StreamModels.ReadRecordsResponse<Turn>>(
                StreamService, "ReadRecords", read);
            await instance.StartAsync<StreamModels.DescribeStreamResponse>(
                StreamService, "DescribeStream", describe);

            Assert.Equal(new[] { "AppendRecords", "ReadRecords", "DescribeStream" }, recorder!.Operations);
            Assert.Collection(
                instance.Root.Scheduled,
                input =>
                {
                    Assert.Same(publish, input.Arg);
                    Assert.Equal(typeof(StreamModels.AppendRecordsRequest<Turn>), input.Operation.InputType);
                    Assert.Equal(typeof(StreamModels.AppendRecordsResponse), input.Operation.OutputType);
                },
                input =>
                {
                    Assert.Same(read, input.Arg);
                    Assert.Equal(typeof(StreamModels.ReadRecordsResponse<Turn>), input.Operation.OutputType);
                },
                // A non-generic operation goes through its typed hook first.
                input => Assert.Same(describe, input.Arg));
        }
    }
}
