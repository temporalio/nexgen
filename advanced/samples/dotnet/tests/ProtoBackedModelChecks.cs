using System;
using Google.Protobuf;
using Nexgen.ProtoBackedShowcaseService;
using Nexgen.ProtoGeneric;
using Temporalio.Api.Common.V1;
using Temporalio.Api.Failure.V1;
using Temporalio.Converters;
using Xunit;
using static Nexgen.DotNetExamples.Tests.TransferTypeTestSupport;
using WireAlert = Temporalio.Api.Version.V1.Alert;

namespace Nexgen.DotNetExamples.Tests
{
    public class ProtoBackedModelChecks
    {
        public sealed record Value(string Message);

        [Fact]
        public void AuthoredEnumsRoundTripWithoutApplyingOperationDefaults()
        {
            var converter = new Alert.TransferTypeConverter();
            foreach (var level in Enum.GetValues<AlertLevel>())
            {
                var model = new Alert("alert") { Severity = level };
                var wire = Assert.IsType<WireAlert>(converter.ToTransferType(model));
                Assert.Equal((int)level, (int)wire.Severity);
                var decoded = Assert.IsType<Alert>(converter.FromTransferType(
                    WireAlert.Parser.ParseFrom(wire.ToByteArray())));
                Assert.Equal(model.Message, decoded.Message);
                // Proto3 has no presence for this enum. .NET optional scalars
                // decode the zero member as absence.
                Assert.Equal(level == AlertLevel.SeverityUnspecified ? (AlertLevel?)null : level, decoded.Severity);
            }
            var optional = new Alert("optional");
            Assert.Null(optional.Severity);
            var optionalWire = Assert.IsType<WireAlert>(converter.ToTransferType(optional));
            Assert.Equal(Temporalio.Api.Enums.V1.Severity.Unspecified, optionalWire.Severity);
            var defaultWire = Assert.IsType<Temporalio.Api.Namespace.V1.NamespaceInfo>(
                new NamespaceInfo.TransferTypeConverter().ToTransferType(
                    new NamespaceInfo(NamespaceState.NamespaceStateUnspecified)));
            Assert.Equal(Temporalio.Api.Enums.V1.NamespaceState.Unspecified, defaultWire.State);
        }

        [Fact]
        public void ByteListsRoundTripThroughProtobuf()
        {
            var converter = new CallbackInternal.TransferTypeConverter();
            foreach (var data in new[] { Array.Empty<byte>(), new byte[] { 0, 1, 127, 128, 255 } })
            {
                var wire = Assert.IsType<Callback.Types.Internal>(
                    converter.ToTransferType(new CallbackInternal(data)));
                Assert.Equal(data, wire.Data.ToByteArray());
                var decoded = Assert.IsType<CallbackInternal>(converter.FromTransferType(
                    Callback.Types.Internal.Parser.ParseFrom(wire.ToByteArray())));
                Assert.Equal(data, decoded.Data);
            }
        }

        [Fact]
        public void OptionalPayloadsPreserveTypeAndRejectMultipleValues()
        {
            using var context = PushConverterContext();
            var converter = CreateSdkConverter(typeof(OptionalPayloads<Value>));
            foreach (var details in new Value?[] { null, new Value("optional") })
            {
                var model = new OptionalPayloads<Value> { Details = details };
                var wire = Assert.IsType<CanceledFailureInfo>(converter.ToTransferType(model));
                Assert.Equal(details is null ? 0 : 1, wire.Details?.Payloads_.Count ?? 0);
                Assert.Equal(model, FromPayload<OptionalPayloads<Value>>(ToPayload(model)));
            }
            var empty = Assert.IsType<OptionalPayloads<Value>>(converter.FromTransferType(
                new CanceledFailureInfo { Details = new Payloads() }));
            Assert.Null(empty.Details);
            var values = new Payloads();
            var payload = DataConverter.Default.PayloadConverter.ToPayload(new Value("one"));
            values.Payloads_.Add(payload);
            values.Payloads_.Add(payload);
            var error = Assert.Throws<InvalidOperationException>(() => converter.FromTransferType(
                new CanceledFailureInfo { Details = values }));
            Assert.Equal("expected at most one payload in OptionalPayloads.Details, found 2", error.Message);
        }
    }
}
