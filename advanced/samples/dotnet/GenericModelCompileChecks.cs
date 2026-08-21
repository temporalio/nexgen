using System.Collections.Generic;
using System.Threading.Tasks;
using GenericModels = Nexgen.GenericModelService;

namespace Nexgen.DotNetExamples
{
    internal static class GenericModelCompileChecks
    {
        private static readonly GenericModels.OperationCompletionResult<object> Success =
            new GenericModels.OperationCompletionResult<object>.Success(
                new GenericModels.OperationCompletionSuccess<object>(new object()));

        private static readonly GenericModels.OperationCompletionResult<object> Failure =
            new GenericModels.OperationCompletionResult<object>.Failure(
                new GenericModels.OperationCompletionFailure("failed"));

        internal static Task<GenericModels.GenericResponse<string, object, object>> CompleteAsync() =>
            GenericModels.Operations.CompleteAsync(
                new GenericModels.CompleteOptions<string>(
                    "context",
                    new[] { "first" },
                    new Dictionary<string, string> { ["primary"] = "value" },
                    new GenericModels.Inner<string>("nested")));
    }
}
