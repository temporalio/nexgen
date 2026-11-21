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
using WireProducer = Streams::Temporalio.Api.Stream.V1.Producer;
using WireReadRecordsRequest = Streams::Temporalio.Api.StreamService.V1.ReadRecordsRequest;
using WireStreamRecord = Streams::Temporalio.Api.Stream.V1.StreamRecord;

namespace Nexgen.DotNetExamples.Tests
{
    public class StreamServiceChecks
    {
        public sealed record Turn(string Role);

        private static readonly StreamServiceExample.Producer ClientProducer =
            new StreamServiceExample.Producer.Client(
                new StreamServiceExample.ClientProducer("ns", "c1"));

        private static StreamServiceExample.AppendRecordsRequest<Turn> AppendRequest(
            StreamServiceExample.Producer producer, string? requestId = null) =>
            new(
                new StreamServiceExample.StreamRef<Turn>("stream-1"),
                new StreamServiceExample.AppendRecord<Turn>[0],
                producer,
                requestId);

        // A body decodes with the converter context of the stream assembly, which is internal.
        private static IDisposable PushStreamConverterContext() =>
            (IDisposable)typeof(StreamServiceExample.Stream<Turn>).Assembly
                .GetType("Nexgen.Support.SystemNexusConverterContext")!
                .GetMethod("Push", BindingFlags.NonPublic | BindingFlags.Static)!
                .Invoke(null, new object[]
                {
                    DataConverter.Default.PayloadConverter,
                    DataConverter.Default.FailureConverter,
                })!;

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
            var model = AppendRequest(ClientProducer, requestId);

            var wire = Assert.IsType<WireAppendRecordsRequest>(
                CreateSdkConverter(typeof(StreamServiceExample.AppendRecordsRequest<Turn>))
                    .ToTransferType(model));
            Assert.Equal(requestId ?? string.Empty, wire.RequestId);

            var decoded = FromPayload<StreamServiceExample.AppendRecordsRequest<Turn>>(
                ToPayload(model));
            Assert.Equal(requestId, decoded.RequestId);
        }

        [Theory]
        [InlineData(WireProducer.KindOneofCase.Workflow)]
        [InlineData(WireProducer.KindOneofCase.Client)]
        public void ARequestSendsItsProducerAsTheMemberOfItsCase(WireProducer.KindOneofCase kind)
        {
            var producer = kind == WireProducer.KindOneofCase.Client
                ? ClientProducer
                : new StreamServiceExample.Producer.Workflow(
                    new StreamServiceExample.WorkflowProducer("ns", "w1", "r1"));
            var model = AppendRequest(producer);

            var wire = Assert.IsType<WireAppendRecordsRequest>(
                CreateSdkConverter(typeof(StreamServiceExample.AppendRecordsRequest<Turn>))
                    .ToTransferType(model));
            Assert.Equal(kind, wire.Producer.KindCase);

            var decoded = FromPayload<StreamServiceExample.AppendRecordsRequest<Turn>>(
                ToPayload(model));
            Assert.Equal(producer, decoded.Producer);
        }

        // A newer server can send a kind in field 6, which this code doesn't know.
        [Theory]
        [InlineData(null)]
        [InlineData(new byte[0])]
        [InlineData(new byte[] { 0x32, 0x00 })]
        public void AProducerWithNoKnownMemberIsAbsent(byte[]? producer)
        {
            using var converterContext = PushStreamConverterContext();
            var wireProducer = producer == null ? null : WireProducer.Parser.ParseFrom(producer);
            var record = new WireStreamRecord
            {
                Producer = wireProducer,
                Body = DataConverter.Default.PayloadConverter.ToPayload(new Turn("user")),
                Offset = ByteString.CopyFrom(1),
            };

            var decoded = Assert.IsType<StreamServiceExample.StreamRecord<Turn>>(
                CreateSdkConverter(typeof(StreamServiceExample.StreamRecord<Turn>))
                    .FromTransferType(record));
            Assert.Null(decoded.Producer);

            // A request can't go without its Producer, so the same message fails there.
            var error = Assert.Throws<InvalidOperationException>(() =>
                CreateSdkConverter(typeof(StreamServiceExample.AppendRecordsRequest<Turn>))
                    .FromTransferType(new WireAppendRecordsRequest
                    {
                        StreamId = "stream-1",
                        Producer = wireProducer,
                    }));
            Assert.Equal("missing required field AppendRecordsRequest.Producer", error.Message);
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
