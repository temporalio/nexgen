extern alias SystemNexusOperations;

using System.Threading.Tasks;
using Xunit;
using Request = SystemNexusOperations::Temporalio.Workflows.Request;
using Response = SystemNexusOperations::Temporalio.Workflows.Response;
using WorkflowInstance = SystemNexusOperations::Temporalio.Worker.WorkflowInstance;

namespace Nexgen.DotNetExamples.Tests
{
    public class SystemNexusSchedulingChecks
    {
        [Theory]
        [InlineData("FirstOperation")]
        [InlineData("SecondOperation")]
        public async Task TypedOperationsScheduleDirectly(string operation)
        {
            var instance = new WorkflowInstance();
            var request = new Request("request-id");

            await instance.StartAsync<Response>("InterceptorService", operation, request);

            var input = Assert.Single(instance.Root.Scheduled);
            Assert.Equal("InterceptorService", input.Service);
            Assert.Equal(operation, input.Operation.Name);
            Assert.Equal(typeof(Request), input.Operation.InputType);
            Assert.Equal(typeof(Response), input.Operation.OutputType);
            Assert.Same(request, input.Arg);
            Assert.Null(input.Headers);
        }
    }
}
