extern alias Streams;

using System.Linq;
using Temporalio.Converters;
using Xunit;
using static Nexgen.DotNetExamples.Tests.TransferTypeTestSupport;
using StreamServiceExample = Streams::Temporalio.Workflows;
using WireDescribeStreamResponse = Streams::Temporalio.Api.StreamService.V1.DescribeStreamResponse;
using WireReadRecordsResponse = Streams::Temporalio.Api.StreamService.V1.ReadRecordsResponse;
using WireStreamRecord = Streams::Temporalio.Api.Stream.V1.StreamRecord;

namespace Nexgen.DotNetExamples.Tests
{
    public class StreamServiceChecks
    {
        public sealed record Turn(string Role);

        [Fact]
        public void StreamWithItsHandleRoundTrips()
        {
            var converter = DataConverter.Default.PayloadConverter;
            var stream = new StreamServiceExample.Stream<Turn>(
                new StreamServiceExample.StreamRef("stream-1"), null);

            var decoded = converter.ToValue<StreamServiceExample.Stream<Turn>>(
                converter.ToPayload(stream));

            Assert.Equal("stream-1", decoded.Ref.Id);
        }

        // A data item's body goes through the payload converter of the sample assembly, which a
        // test can't set. The TypeScript test covers data items.
        [Fact]
        public void ReadItemsRoundTripAsTheirRecords()
        {
            var model = new StreamServiceExample.ReadRecordsResponse<Turn>(new[]
            {
                new StreamServiceExample.StreamRecord<Turn>
                {
                    Item = new StreamServiceExample.StreamItem<Turn>.Close(
                        new StreamServiceExample.StreamClose
                        {
                            Producer = new StreamServiceExample.Producer("wf:r1")
                            {
                                Kind = new StreamServiceExample.ProducerKind.Workflow(
                                    new StreamServiceExample.WorkflowProducer { RunId = "r1" }),
                            },
                            Status = StreamServiceExample.StreamCloseStatus.StreamCloseStatusClosed,
                        }),
                    Offset = 2,
                },
            });

            var wire = Assert.IsType<WireReadRecordsResponse>(
                CreateSdkConverter(typeof(StreamServiceExample.ReadRecordsResponse<Turn>))
                    .ToTransferType(model));
            Assert.Equal(
                new[] { WireStreamRecord.ItemOneofCase.Close },
                wire.Records.Select(record => record.ItemCase));

            var decoded = FromPayload<StreamServiceExample.ReadRecordsResponse<Turn>>(
                ToPayload(model));
            var close = Assert.IsType<StreamServiceExample.StreamItem<Turn>.Close>(
                decoded.Records[0].Item);
            Assert.Equal("wf:r1", close.Value.Producer?.Id);
            Assert.Equal(
                StreamServiceExample.StreamCloseStatus.StreamCloseStatusClosed,
                close.Value.Status);
            Assert.Equal(2, decoded.Records[0].Offset);
        }

        // A newer server can send a Stream status that this code doesn't know.
        [Fact]
        public void UnknownEnumNumberDecodesAsTheFallbackCase()
        {
            var converter = CreateSdkConverter(typeof(StreamServiceExample.DescribeStreamResponse));
            var wire = new WireDescribeStreamResponse
            {
                Status = (Streams::Temporalio.Api.Enums.V1.StreamStatus)9,
            };

            var decoded = Assert.IsType<StreamServiceExample.DescribeStreamResponse>(
                converter.FromTransferType(wire));
            Assert.Equal(StreamServiceExample.StreamStatus.StreamStatusUnspecified, decoded.Status);
        }

        // Replay needs a fixed range, so the WIT omits `last` from the Workflow API only.
        [Fact]
        public void AWorkflowReadTakesNoLast()
        {
            Assert.Null(typeof(StreamServiceExample.ReadRecordsOptions).GetProperty("Last"));
            Assert.NotNull(typeof(StreamServiceExample.ReadRecordsRequest).GetProperty("Last"));
        }
    }
}
