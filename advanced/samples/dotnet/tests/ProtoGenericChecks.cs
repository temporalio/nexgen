using Nexgen.ProtoGeneric;
using Xunit;
using static Nexgen.DotNetExamples.Tests.TransferTypeTestSupport;

namespace Nexgen.DotNetExamples.Tests
{
    public class ProtoGenericChecks
    {
        public sealed record EchoOutput(string Value);

        public sealed record MyCtx(string Neat);

        [Fact]
        public void ProtoBackedGenericTypeArgumentsArePreserved()
        {
            using var converterContext = PushConverterContext();
            var model = new PayloadBackedEnvelope<EchoOutput, MyCtx>(
                new PayloadBackedOutput<EchoOutput>(new EchoOutput("hello")),
                new PayloadBackedContext<MyCtx>(new MyCtx("very")));

            var payload = ToPayload(model);
            Assert.Equal(
                "temporal.api.compute.v1.ComputeConfigScalingGroup",
                payload.Metadata["messageType"].ToStringUtf8());

            var decoded = FromPayload<PayloadBackedEnvelope<EchoOutput, MyCtx>>(payload);
            Assert.Equal(model, decoded);
            Assert.IsType<EchoOutput>(decoded.Provider.Details);
            Assert.IsType<MyCtx>(decoded.Scaler.Details);
        }
    }
}
