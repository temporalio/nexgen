using System;
using Nexgen.ProtoOneof;
using NexusRpc.Handlers;
using Xunit;
using static Nexgen.DotNetExamples.Tests.TransferTypeTestSupport;
using WireOutcome = Temporalio.Api.Update.V1.Outcome;
using WirePauseActivityRequest = Temporalio.Api.WorkflowService.V1.PauseActivityRequest;

namespace Nexgen.DotNetExamples.Tests
{
    public class ProtoOneofChecks
    {
        public sealed record SuccessfulOutput(string Message);

        [Fact]
        public void ProtoOneofSuccessRoundTripsThroughPayload()
        {
            using var converterContext = PushConverterContext();
            var model = new Outcome<SuccessfulOutput>(
                new OutcomeValue<SuccessfulOutput>.Success(new SuccessfulOutput("hello")));

            var wire = Assert.IsType<WireOutcome>(
                CreateSdkConverter(typeof(Outcome<SuccessfulOutput>)).ToTransferType(model));
            Assert.Equal(WireOutcome.ValueOneofCase.Success, wire.ValueCase);

            var payload = ToPayload(model);
            Assert.Equal("json/protobuf", payload.Metadata["encoding"].ToStringUtf8());
            Assert.Equal(
                "temporal.api.update.v1.Outcome",
                payload.Metadata["messageType"].ToStringUtf8());

            var decoded = FromPayload<Outcome<SuccessfulOutput>>(payload);
            Assert.Equal(model, decoded);
            var success = Assert.IsType<OutcomeValue<SuccessfulOutput>.Success>(decoded.Value);
            Assert.IsType<SuccessfulOutput>(success.Value);
        }

        [Fact]
        public void ProtoOneofFailureRoundTripsThroughPayload()
        {
            using var converterContext = PushConverterContext();
            var model = new Outcome<SuccessfulOutput>(
                new OutcomeValue<SuccessfulOutput>.Failure(
                    new Temporalio.Exceptions.ApplicationFailureException("boom")));

            var wire = Assert.IsType<WireOutcome>(
                CreateSdkConverter(typeof(Outcome<SuccessfulOutput>)).ToTransferType(model));
            Assert.Equal(WireOutcome.ValueOneofCase.Failure, wire.ValueCase);

            var decoded = FromPayload<Outcome<SuccessfulOutput>>(ToPayload(model));
            var failure = Assert.IsType<OutcomeValue<SuccessfulOutput>.Failure>(decoded.Value);
            Assert.Equal("boom", failure.Value.Message);
        }

        [Fact]
        public void RequiredProtoOneofRejectsUnsetWire()
        {
            using var converterContext = PushConverterContext();
            var converter = CreateSdkConverter(typeof(Outcome<SuccessfulOutput>));

            var error = Assert.Throws<InvalidOperationException>(
                () => converter.FromTransferType(new WireOutcome()));
            Assert.Contains("missing required field Outcome.Value", error.Message);
        }

        [Fact]
        public void ProtoOneofPayloadsCarrierRejectsMoreThanOnePayload()
        {
            using var converterContext = PushConverterContext();
            var payload = Temporalio.Converters.DataConverter.Default.PayloadConverter.ToPayload(
                new SuccessfulOutput("hello"));
            var success = new Temporalio.Api.Common.V1.Payloads();
            success.Payloads_.Add(payload);
            success.Payloads_.Add(payload);

            var error = Assert.Throws<InvalidOperationException>(
                () => CreateSdkConverter(typeof(Outcome<SuccessfulOutput>)).FromTransferType(
                    new WireOutcome { Success = success }));
            Assert.Equal("expected at most one payload in Outcome.Success, found 2", error.Message);
        }

        [Fact]
        public void ProtoOneofPayloadsCarrierDecodesNoPayloadAsDefault()
        {
            var wire = new WireOutcome { Success = new Temporalio.Api.Common.V1.Payloads() };

            var decoded = Assert.IsType<Outcome<SuccessfulOutput>>(
                CreateSdkConverter(typeof(Outcome<SuccessfulOutput>)).FromTransferType(wire));
            var success = Assert.IsType<OutcomeValue<SuccessfulOutput>.Success>(decoded.Value);
            Assert.Null(success.Value);
        }

        [Fact]
        public void ProtoOneofPayloadsCarrierDecodesNoPayloadAsNoValue()
        {
            var wire = new WireOutcome { Success = new Temporalio.Api.Common.V1.Payloads() };

            var decoded = Assert.IsType<Outcome<NoValue>>(
                CreateSdkConverter(typeof(Outcome<NoValue>)).FromTransferType(wire));
            Assert.IsType<OutcomeValue<NoValue>.Success>(decoded.Value);
        }

        [Fact]
        public void RequiredProtoOneofRejectsNullWhenEncoding()
        {
            var model = new Outcome<SuccessfulOutput>(
                new OutcomeValue<SuccessfulOutput>.Success(new SuccessfulOutput("hello")))
            {
                Value = null!,
            };

            var error = Assert.Throws<InvalidOperationException>(
                () => CreateSdkConverter(typeof(Outcome<SuccessfulOutput>)).ToTransferType(model));
            Assert.Equal("missing required field Outcome.Value", error.Message);
        }

        [Fact]
        public void OptionalProtoOneofRoundTripsUnsetAsNull()
        {
            var converter = CreateSdkConverter(typeof(PauseActivityRequest));
            var model = new PauseActivityRequest("namespace", "worker", "maintenance", "request-id");

            var wire = Assert.IsType<WirePauseActivityRequest>(converter.ToTransferType(model));
            Assert.Equal(WirePauseActivityRequest.ActivityOneofCase.None, wire.ActivityCase);

            var decoded = Assert.IsType<PauseActivityRequest>(converter.FromTransferType(
                new WirePauseActivityRequest
                {
                    Namespace = "namespace",
                    Identity = "worker",
                    Reason = "maintenance",
                    RequestId = "request-id",
                }));
            Assert.Equal(model, decoded);
            Assert.Null(decoded.Activity);
        }

        [Fact]
        public void OptionalProtoOneofRoundTripsSelectedCase()
        {
            var model = new PauseActivityRequest("namespace", "worker", "maintenance", "request-id")
            {
                Activity = new ActivitySelection.Type("activity-type"),
            };

            var wire = Assert.IsType<WirePauseActivityRequest>(
                CreateSdkConverter(typeof(PauseActivityRequest)).ToTransferType(model));
            Assert.Equal(WirePauseActivityRequest.ActivityOneofCase.Type, wire.ActivityCase);
            Assert.Equal("activity-type", wire.Type);

            Assert.Equal(model, FromPayload<PauseActivityRequest>(ToPayload(model)));
        }
    }
}
