using System;
using Nexgen.NotificationService;
using Temporalio.Converters;
using Xunit;
using static Nexgen.DotNetExamples.Tests.TransferTypeTestSupport;
using WireOnCompleteRequest = Temporalio.Api.NotificationService.V1.OnCompleteRequest;

namespace Nexgen.DotNetExamples.Tests
{
    public class NotificationServiceChecks
    {
        public sealed record Completion(string Message);

        public sealed record SourceContext(string WorkflowId);

        [Fact]
        public void OnCompleteRequestRoundTripsGenericSuccessValues()
        {
            using var converterContext = PushConverterContext();
            var model = new OnCompleteRequest<Completion, SourceContext>(
                new OnCompleteRequestResult<Completion>.Success(new Completion("completed")),
                new SourceContext("workflow-id"));
            var converter = CreateSdkConverter(typeof(OnCompleteRequest<Completion, SourceContext>));

            Assert.Equal(typeof(WireOnCompleteRequest), converter.TransferType);
            var wire = Assert.IsType<WireOnCompleteRequest>(converter.ToTransferType(model));
            Assert.Equal(WireOnCompleteRequest.ResultOneofCase.Success, wire.ResultCase);

            var decoded = Assert.IsType<OnCompleteRequest<Completion, SourceContext>>(
                converter.FromTransferType(wire));
            Assert.Equal(model, decoded);
            var success = Assert.IsType<OnCompleteRequestResult<Completion>.Success>(decoded.Result);
            Assert.IsType<Completion>(success.Value);
            Assert.IsType<SourceContext>(decoded.SourceContext);
        }

        [Fact]
        public void OnCompleteRequestRoundTripsFailures()
        {
            using var converterContext = PushConverterContext();
            var model = new OnCompleteRequest<Completion, SourceContext>(
                new OnCompleteRequestResult<Completion>.Failure(
                    new Temporalio.Exceptions.ApplicationFailureException(
                        "operation failed",
                        errorType: "CompletionFailure")),
                new SourceContext("workflow-id"));
            var converter = new OnCompleteRequest<Completion, SourceContext>.TransferTypeConverter();

            var wire = Assert.IsType<WireOnCompleteRequest>(converter.ToTransferType(model));
            Assert.Equal(WireOnCompleteRequest.ResultOneofCase.Failure, wire.ResultCase);
            Assert.Equal("operation failed", wire.Failure.Message);

            var decoded = Assert.IsType<OnCompleteRequest<Completion, SourceContext>>(
                converter.FromTransferType(wire));
            var failure = Assert.IsType<OnCompleteRequestResult<Completion>.Failure>(decoded.Result);
            var exception = Assert.IsType<Temporalio.Exceptions.ApplicationFailureException>(
                failure.Value);
            Assert.Equal("operation failed", exception.Message);
            Assert.Equal("CompletionFailure", exception.ErrorType);
            Assert.Equal(new SourceContext("workflow-id"), decoded.SourceContext);
        }

        [Fact]
        public void OnCompleteRequestRejectsMissingRequiredFields()
        {
            using var converterContext = PushConverterContext();
            var converter = new OnCompleteRequest<Completion, SourceContext>.TransferTypeConverter();

            var missingResult = Assert.Throws<InvalidOperationException>(() =>
                converter.FromTransferType(new WireOnCompleteRequest
                {
                    SourceContext = DataConverter.Default.PayloadConverter.ToPayload(
                        new SourceContext("workflow-id")),
                }));
            Assert.Contains("OnCompleteRequest.Result", missingResult.Message);

            var missingSourceContext = Assert.Throws<InvalidOperationException>(() =>
                converter.FromTransferType(new WireOnCompleteRequest
                {
                    Success = DataConverter.Default.PayloadConverter.ToPayload(
                        new Completion("completed")),
                }));
            Assert.Contains("OnCompleteRequest.SourceContext", missingSourceContext.Message);
        }

        [Fact]
        public void OnCompleteRequestRejectsNullRequiredOneofWhenEncoding()
        {
            using var converterContext = PushConverterContext();
            var model = new OnCompleteRequest<Completion, SourceContext>(
                null!,
                new SourceContext("workflow-id"));
            var converter = CreateSdkConverter(typeof(OnCompleteRequest<Completion, SourceContext>));

            var error = Assert.Throws<InvalidOperationException>(
                () => converter.ToTransferType(model));
            Assert.Equal("missing required field OnCompleteRequest.Result", error.Message);
        }

        [Fact]
        public void OnCompleteResponseRoundTrips()
        {
            var converter = CreateSdkConverter(typeof(OnCompleteResponse));

            var wire = Assert.IsType<Temporalio.Api.NotificationService.V1.OnCompleteResponse>(
                converter.ToTransferType(new OnCompleteResponse()));
            Assert.Equal(new OnCompleteResponse(), converter.FromTransferType(wire));
        }
    }
}
