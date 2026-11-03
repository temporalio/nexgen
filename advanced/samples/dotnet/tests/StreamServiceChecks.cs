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
        [Fact]
        public void StreamWithItsOwnFieldNameRoundTrips()
        {
            var converter = DataConverter.Default.PayloadConverter;
            var stream = new StreamServiceExample.Stream(new StreamServiceExample.StreamRef("stream-1"), null);

            var decoded = converter.ToValue<StreamServiceExample.Stream>(converter.ToPayload(stream));

            Assert.Equal("stream-1", decoded.StreamValue.Id);
        }

        // A data item's body goes through the payload converter of the sample assembly, which a
        // test can't set. The TypeScript test covers data items.
        [Fact]
        public void ReadItemsRoundTripAsTheirRecords()
        {
            var model = new StreamServiceExample.ReadRecordsResponse(new[]
            {
                new StreamServiceExample.StreamRecord
                {
                    Item = new StreamServiceExample.StreamItem.Superseded(
                        new StreamServiceExample.Supersession("workflow-1", new long[] { 7 })),
                    Offset = 2,
                },
                new StreamServiceExample.StreamRecord
                {
                    Item = new StreamServiceExample.StreamItem.Close(
                        new StreamServiceExample.StreamClose
                        {
                            Status = StreamServiceExample.StreamCloseStatus
                                .StreamCloseStatusCompleted,
                        }),
                    Offset = 3,
                },
            });

            var wire = Assert.IsType<WireReadRecordsResponse>(
                CreateSdkConverter(typeof(StreamServiceExample.ReadRecordsResponse))
                    .ToTransferType(model));
            Assert.Equal(
                new[]
                {
                    WireStreamRecord.ItemOneofCase.Superseded,
                    WireStreamRecord.ItemOneofCase.Close,
                },
                wire.Records.Select(record => record.ItemCase));

            var decoded = FromPayload<StreamServiceExample.ReadRecordsResponse>(ToPayload(model));
            var superseded = Assert.IsType<StreamServiceExample.StreamItem.Superseded>(
                decoded.Records[0].Item);
            Assert.Equal("workflow-1", superseded.Value.ProducerId);
            Assert.Equal(new long[] { 7 }, superseded.Value.FailedTasks);
            var close = Assert.IsType<StreamServiceExample.StreamItem.Close>(
                decoded.Records[1].Item);
            Assert.Equal(
                StreamServiceExample.StreamCloseStatus.StreamCloseStatusCompleted,
                close.Value.Status);
            Assert.Equal(3, decoded.Records[1].Offset);
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
    }
}
