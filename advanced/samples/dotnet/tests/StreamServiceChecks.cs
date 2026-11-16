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

            // The Stream resource still binds the reads.
            foreach (var method in new[] { "ReadAsync", "DescribeAsync" })
            {
                Assert.NotNull(typeof(StreamServiceExample.Stream<Turn>).GetMethod(method));
            }
        }

        // `append` and `close` are api-omitted for the Workflow target, so Workflow code can't
        // append to a Stream or close it.
        [Fact]
        public void WorkflowStreamHasNoAppendAndNoClose()
        {
            var stream = typeof(StreamServiceExample.Stream<Turn>);
            Assert.Null(stream.GetMethod("AppendAsync"));
            Assert.Null(stream.GetMethod("CloseAsync"));
        }
    }
}
