extern alias SystemNexusStreams;

using System.Linq;
using System.Threading.Tasks;
using Xunit;
using StreamModels = SystemNexusStreams::Temporalio.Workflows;
using WorkflowInstance = SystemNexusStreams::Temporalio.Worker.WorkflowInstance;

namespace Nexgen.DotNetExamples.Tests
{
    public class StreamSystemNexusChecks
    {
        private const string StreamService = "temporal.api.streamservice.v1.StreamService";

        public sealed record Turn(string Role);

        // C# generics are invariant, so a check against `<object>` refuses a `Stream<Turn>` call.
        [Fact]
        public async Task StreamOfTurnCallsKeepTheirTypes()
        {
            var instance = new WorkflowInstance();
            var streamRef = new StreamModels.StreamRef<Turn>("stream-1");

            await instance.StartAsync<StreamModels.AppendRecordsResponse>(
                StreamService,
                "AppendRecords",
                new StreamModels.AppendRecordsRequest<Turn>(
                    streamRef, new StreamModels.AppendRecord<Turn>[0], null));
            await instance.StartAsync<StreamModels.ReadRecordsResponse<Turn>>(
                StreamService, "ReadRecords", new StreamModels.ReadRecordsRequest<Turn>(streamRef));

            Assert.Equal(
                new[]
                {
                    (typeof(StreamModels.AppendRecordsRequest<Turn>),
                        typeof(StreamModels.AppendRecordsResponse)),
                    (typeof(StreamModels.ReadRecordsRequest<Turn>),
                        typeof(StreamModels.ReadRecordsResponse<Turn>)),
                },
                instance.Root.Scheduled.Select(
                    input => (input.Operation.InputType, input.Operation.OutputType)));
        }
    }
}
