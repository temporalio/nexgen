using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using Google.Protobuf;
using Nexgen.Support;
using Xunit;
using ActivityOptions = Nexgen.TypeRoundtripService.ActivityOptions;
using FailureContainer = Nexgen.TypeRoundtripService.FailureContainer;
using Payload = Temporalio.Api.Common.V1.Payload;

namespace Nexgen.DotNetExamples.Tests
{

    public class ProtoWireCompatibilityChecks
    {
        [Fact]
        public void ActivityOptionsIntermediateFixturesDecodeToUserType()
        {
            var payloadConverter = new Temporalio.Converters.DefaultPayloadConverter();
            var transferConverter = new ActivityOptions.TransferTypeConverter();

            foreach (var fixtureName in new[]
            {
                "activity-options.python.payload.json",
                "activity-options.dotnet.payload.json",
            })
            {
                var payload = ReadPayload("type_roundtrip", fixtureName);
                Assert.Equal("json/protobuf", payload.Metadata["encoding"].ToStringUtf8());
                Assert.Equal(
                    "temporal.api.activity.v1.ActivityOptions",
                    payload.Metadata["messageType"].ToStringUtf8());
                Assert.DoesNotContain(
                    payload.Metadata,
                    item => item.Key.Contains("temporal-wire", StringComparison.Ordinal));
                AssertActivityOptionsModel(
                    transferConverter.FromTransferType(
                        payloadConverter.ToValue(payload, transferConverter.TransferType)));
            }
        }

        [Fact]
        public void FailureIntermediateUsesSdkFailureConverter()
        {
            var payloadConverter = new Temporalio.Converters.DefaultPayloadConverter();
            // The samples compile the generated code outside the SDK. Thus the SDK's System Nexus
            // payload converter does not push this context. The test pushes the context here.
            using var converterContext = SystemNexusConverterContext.Push(
                payloadConverter,
                Temporalio.Converters.DataConverter.Default.FailureConverter);
            var wire = new Temporalio.Api.Command.V1.FailWorkflowExecutionCommandAttributes
            {
                Failure = new Temporalio.Api.Failure.V1.Failure
                {
                    Message = "Encoded failure",
                    EncodedAttributes = payloadConverter.ToPayload(
                        new Dictionary<string, object?>
                        {
                            ["message"] = "decoded outer failure",
                            ["stack_trace"] = "decoded stack",
                        }),
                    ApplicationFailureInfo = new()
                    {
                        Type = "OuterFailure",
                        NonRetryable = true,
                    },
                    Cause = new Temporalio.Api.Failure.V1.Failure
                    {
                        Message = "inner failure",
                        ApplicationFailureInfo = new() { Type = "InnerFailure" },
                    },
                },
            };

            var transferConverter = new FailureContainer.TransferTypeConverter();
            var model = Assert.IsType<FailureContainer>(transferConverter.FromTransferType(wire));
            var failure = Assert.IsType<Temporalio.Exceptions.ApplicationFailureException>(
                model.Failure);
            Assert.Equal("decoded outer failure", failure.Message);
            Assert.Equal("OuterFailure", failure.ErrorType);
            Assert.True(failure.NonRetryable);
            var cause = Assert.IsType<Temporalio.Exceptions.ApplicationFailureException>(
                failure.InnerException);
            Assert.Equal("inner failure", cause.Message);

            var roundTripped = Assert.IsType<
                Temporalio.Api.Command.V1.FailWorkflowExecutionCommandAttributes>(
                transferConverter.ToTransferType(model));
            Assert.Equal("decoded outer failure", roundTripped.Failure.Message);
            Assert.Equal(
                "OuterFailure",
                roundTripped.Failure.ApplicationFailureInfo.Type);
            Assert.Equal("inner failure", roundTripped.Failure.Cause.Message);

            var absent = Assert.IsType<FailureContainer>(transferConverter.FromTransferType(
                new Temporalio.Api.Command.V1.FailWorkflowExecutionCommandAttributes()));
            Assert.Null(absent.Failure);
        }

        private static void AssertActivityOptionsModel(object? value)
        {
            var decoded = Assert.IsType<ActivityOptions>(value);
            Assert.Equal("demo-task-queue", decoded.TaskQueue);
            Assert.Equal(3, decoded.RetryPolicy.MaximumAttempts);
            Assert.Equal(TimeSpan.FromSeconds(7), decoded.ScheduleToCloseTimeout);
            Assert.NotNull(decoded.Priority);
            Assert.Equal(4, decoded.Priority.PriorityKey);
            Assert.Equal("tenant-a", decoded.Priority.FairnessKey);
            Assert.Equal(2.5f, decoded.Priority.FairnessWeight);
        }

        private static Payload ReadPayload(string exampleId, string fixtureName)
        {
            var json = File.ReadAllText(Path.Combine(FixtureDirectory(exampleId), fixtureName));
            using var doc = JsonDocument.Parse(json);
            var root = doc.RootElement;
            var payload = new Payload
            {
                Data = ByteString.FromBase64(root.GetProperty("data").GetString()),
            };
            foreach (var item in root.GetProperty("metadata").EnumerateObject())
            {
                payload.Metadata[item.Name] = ByteString.FromBase64(item.Value.GetString());
            }
            return payload;
        }

        private static string FixtureDirectory(string exampleId)
        {
            var directory = new DirectoryInfo(AppContext.BaseDirectory);
            while (directory is not null)
            {
                var candidate = Path.Combine(
                    directory.FullName,
                    "advanced",
                    "samples",
                    "wire",
                    "proto",
                    exampleId);
                if (Directory.Exists(candidate))
                {
                    return candidate;
                }
                directory = directory.Parent;
            }
            throw new DirectoryNotFoundException($"Could not find advanced/samples/wire/proto/{exampleId}");
        }
    }
}
