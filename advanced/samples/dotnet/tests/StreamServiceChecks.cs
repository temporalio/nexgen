extern alias Streams;

using System;
using System.Linq;
using System.Reflection;
using Temporalio.Converters;
using Xunit;
using static Nexgen.DotNetExamples.Tests.TransferTypeTestSupport;
using StreamServiceExample = Streams::Temporalio.Workflows;
using WireDescribeStreamResponse = Streams::Temporalio.Api.StreamService.V1.DescribeStreamResponse;
using WireReadRecordsRequest = Streams::Temporalio.Api.StreamService.V1.ReadRecordsRequest;
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
                new StreamServiceExample.StreamRef<Turn>("stream-1"), null);

            var decoded = converter.ToValue<StreamServiceExample.Stream<Turn>>(
                converter.ToPayload(stream));

            Assert.Equal("stream-1", decoded.Ref.Id);
        }

        [Fact]
        public void DeclaredRefParametersPropagateThroughRequests()
        {
            var reference = new StreamServiceExample.StreamRef<Turn>("stream-1");
            var request = new StreamServiceExample.CreateStreamRequest<Turn>(reference, null);
            var decoded = FromPayload<StreamServiceExample.CreateStreamRequest<Turn>>(ToPayload(request));
            Assert.Equal(reference, decoded.StreamRef);
            Assert.IsType<StreamServiceExample.StreamRef<Turn>>(decoded.StreamRef);
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
                        new StreamServiceExample.StreamClose(new byte[] { 0, 0, 0, 0, 0, 0, 0, 2 })
                        {
                            Producer = new StreamServiceExample.Producer("wf:r1")
                            {
                                Kind = new StreamServiceExample.ProducerKind.Workflow(
                                    new StreamServiceExample.WorkflowProducer("r1")),
                            },
                            Status = StreamServiceExample.StreamCloseStatus.StreamCloseStatusClosed,
                        }),
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
            Assert.Equal(new byte[] { 0, 0, 0, 0, 0, 0, 0, 2 }, close.Value.Offset);
        }

        // The from-offset case of the read position is a proto `bytes` member of a oneof.
        [Fact]
        public void AReadPositionOffsetRoundTripsAsBytes()
        {
            var offset = new byte[] { 0, 0, 0, 0, 0, 0, 0, 7 };
            var model = new StreamServiceExample.ReadRecordsRequest<Turn>(
                new StreamServiceExample.StreamRef<Turn>("stream-1"))
            {
                Position = new StreamServiceExample.ReadPosition.FromOffset(offset),
            };

            var wire = Assert.IsType<WireReadRecordsRequest>(
                CreateSdkConverter(typeof(StreamServiceExample.ReadRecordsRequest<Turn>))
                    .ToTransferType(model));
            Assert.Equal(WireReadRecordsRequest.PositionOneofCase.FromOffset, wire.PositionCase);
            Assert.Equal(offset, wire.FromOffset.ToByteArray());

            var decoded = FromPayload<StreamServiceExample.ReadRecordsRequest<Turn>>(
                ToPayload(model));
            var position = Assert.IsType<StreamServiceExample.ReadPosition.FromOffset>(
                decoded.Position);
            Assert.Equal(offset, position.Value);
        }

        // The start case of the read position is a proto enum member of the same oneof.
        [Fact]
        public void AReadPositionStartRoundTripsAsItsEnum()
        {
            var model = new StreamServiceExample.ReadRecordsRequest<Turn>(
                new StreamServiceExample.StreamRef<Turn>("stream-1"))
            {
                Position = new StreamServiceExample.ReadPosition.Start(
                    StreamServiceExample.StreamReadStart.StreamReadStartEnd),
            };

            var wire = Assert.IsType<WireReadRecordsRequest>(
                CreateSdkConverter(typeof(StreamServiceExample.ReadRecordsRequest<Turn>))
                    .ToTransferType(model));
            Assert.Equal(WireReadRecordsRequest.PositionOneofCase.Start, wire.PositionCase);
            Assert.Equal(Streams::Temporalio.Api.Enums.V1.StreamReadStart.End, wire.Start);

            var decoded = FromPayload<StreamServiceExample.ReadRecordsRequest<Turn>>(
                ToPayload(model));
            Assert.Equal(model.Position, decoded.Position);
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

        // Four operations are api-omitted, so only the create keeps a public convenience method.
        [Fact]
        public void ApiOmitHidesTheOperationConvenienceMethods()
        {
            var assembly = typeof(StreamServiceExample.Stream<Turn>).Assembly;
            foreach (var name in new[]
            {
                "AppendRecordsOptions`1",
                "CloseStreamOptions`1",
                "ReadRecordsOptions`1",
                "DescribeStreamOptions`1",
            })
            {
                var type = assembly.GetType("Temporalio.Workflows." + name);
                Assert.NotNull(type);
                Assert.False(type!.IsPublic);
            }
            Assert.True(typeof(StreamServiceExample.CreateStreamOptions<Turn>).IsPublic);

            var publicOperationMethods = typeof(StreamServiceExample.Operations)
                .GetMethods(BindingFlags.Public | BindingFlags.Static)
                .Where(method => method.Name.EndsWith("Async", StringComparison.Ordinal))
                .Select(method => method.Name)
                .Distinct();
            Assert.Equal(new[] { "CreateStreamAsync" }, publicOperationMethods);

            // The Stream resource still binds every operation.
            foreach (var method in new[] { "AppendAsync", "CloseAsync", "ReadAsync", "DescribeAsync" })
            {
                Assert.NotNull(typeof(StreamServiceExample.Stream<Turn>).GetMethod(method));
            }
        }
    }
}
