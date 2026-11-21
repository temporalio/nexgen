extern alias Streams;

using System;
using System.Linq;
using System.Reflection;
using System.Text;
using Google.Protobuf;
using Temporalio.Converters;
using Xunit;
using static Nexgen.DotNetExamples.Tests.TransferTypeTestSupport;
using StreamServiceExample = Streams::Temporalio.Workflows;
using WireAppendRecordsRequest = Streams::Temporalio.Api.StreamService.V1.AppendRecordsRequest;
using WireCreateStreamRequest = Streams::Temporalio.Api.StreamService.V1.CreateStreamRequest;
using WireDescribeStreamRequest = Streams::Temporalio.Api.StreamService.V1.DescribeStreamRequest;
using WireDescribeStreamResponse = Streams::Temporalio.Api.StreamService.V1.DescribeStreamResponse;
using WireReadRecordsRequest = Streams::Temporalio.Api.StreamService.V1.ReadRecordsRequest;

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
        public void ARequestKeepsOnlyTheStreamIdOfItsRef()
        {
            var request = new StreamServiceExample.CreateStreamRequest<Turn>(
                new StreamServiceExample.StreamRef<Turn>("chat-42") { Store = "redis-a" }, null);

            var wire = Assert.IsType<WireCreateStreamRequest>(
                CreateSdkConverter(typeof(StreamServiceExample.CreateStreamRequest<Turn>))
                    .ToTransferType(request));
            Assert.Equal("chat-42", wire.StreamId);
            // The Store name is for the SDK, so no field of the message holds it.
            Assert.DoesNotContain("redis-a", Encoding.UTF8.GetString(wire.ToByteArray()));

            var decoded = FromPayload<StreamServiceExample.CreateStreamRequest<Turn>>(ToPayload(request));
            Assert.Equal(new StreamServiceExample.StreamRef<Turn>("chat-42"), decoded.Ref);
            Assert.IsType<StreamServiceExample.StreamRef<Turn>>(decoded.Ref);
        }

        [Fact]
        public void ARequestWithoutItsStreamIdIsRefused()
        {
            var converter = CreateSdkConverter(typeof(StreamServiceExample.DescribeStreamRequest<Turn>));

            var error = Assert.Throws<InvalidOperationException>(() =>
                converter.FromTransferType(new WireDescribeStreamRequest()));

            Assert.Equal("missing required field DescribeStreamRequest.Ref", error.Message);
        }

        [Fact]
        public void AReadFromOffsetRoundTripsAsBytes()
        {
            var offset = new byte[] { 0, 0, 0, 0, 0, 0, 0, 7 };
            var model = new StreamServiceExample.ReadRecordsRequest<Turn>(
                new StreamServiceExample.StreamRef<Turn>("stream-1"))
            {
                FromOffset = offset,
            };

            var wire = Assert.IsType<WireReadRecordsRequest>(
                CreateSdkConverter(typeof(StreamServiceExample.ReadRecordsRequest<Turn>))
                    .ToTransferType(model));
            Assert.Equal(offset, wire.FromOffset.ToByteArray());

            var decoded = FromPayload<StreamServiceExample.ReadRecordsRequest<Turn>>(
                ToPayload(model));
            Assert.Equal(offset, decoded.FromOffset);
        }

        // Workflow code sends no request id, since the SDK sets it when it sends the batch.
        [Theory]
        [InlineData("append-1")]
        [InlineData(null)]
        public void AnAppendSendsItsRequestId(string? requestId)
        {
            var model = new StreamServiceExample.AppendRecordsRequest<Turn>(
                new StreamServiceExample.StreamRef<Turn>("stream-1"),
                new StreamServiceExample.AppendRecord<Turn>[0],
                new StreamServiceExample.Producer
                {
                    Kind = new StreamServiceExample.ProducerKind.Client(
                        new StreamServiceExample.ClientProducer("ns", "c1")),
                },
                requestId);

            var wire = Assert.IsType<WireAppendRecordsRequest>(
                CreateSdkConverter(typeof(StreamServiceExample.AppendRecordsRequest<Turn>))
                    .ToTransferType(model));
            Assert.Equal(requestId ?? string.Empty, wire.RequestId);

            var decoded = FromPayload<StreamServiceExample.AppendRecordsRequest<Turn>>(
                ToPayload(model));
            Assert.Equal(requestId, decoded.RequestId);
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
