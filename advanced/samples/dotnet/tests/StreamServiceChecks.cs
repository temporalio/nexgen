using Temporalio.Converters;
using Xunit;
using StreamServiceExample = Nexgen.StreamService;

namespace Nexgen.DotNetExamples.Tests
{
    public class StreamServiceChecks
    {
        [Fact]
        public void StreamWithItsOwnFieldNameRoundTrips()
        {
            var converter = DataConverter.Default.PayloadConverter;
            var stream = new StreamServiceExample.Stream(new StreamServiceExample.StreamRef("stream-1"), null, null);

            var decoded = converter.ToValue<StreamServiceExample.Stream>(converter.ToPayload(stream));

            Assert.Equal("stream-1", decoded.StreamValue.Id);
        }
    }
}
