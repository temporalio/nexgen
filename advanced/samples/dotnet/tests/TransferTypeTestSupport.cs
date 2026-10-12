using System;
using System.Reflection;
using Nexgen.Support;
using Temporalio.Api.Common.V1;
using Temporalio.Converters;
using Xunit;

namespace Nexgen.DotNetExamples.Tests
{
    internal static class TransferTypeTestSupport
    {
        // The samples compile the generated code outside the SDK. Thus the SDK's System Nexus
        // payload converter does not push this context. The test pushes the context here.
        internal static IDisposable PushConverterContext() =>
            SystemNexusConverterContext.Push(
                DataConverter.Default.PayloadConverter,
                DataConverter.Default.FailureConverter);

        // Mirrors how the SDK resolves a model's converter, closing open generic converters with
        // the model's type arguments.
        internal static ITemporalTransferTypeConverter CreateSdkConverter(Type modelType)
        {
            var attribute = modelType.GetCustomAttribute<TemporalTransferTypeConverterAttribute>(
                inherit: false);
            Assert.NotNull(attribute);
            var converterType = attribute!.ConverterType;
            if (converterType.ContainsGenericParameters)
            {
                converterType = converterType.MakeGenericType(modelType.GetGenericArguments());
            }
            return Assert.IsAssignableFrom<ITemporalTransferTypeConverter>(
                Activator.CreateInstance(converterType));
        }

        // Mirrors the SDK's transfer-type payload conversion: the model is converted to its
        // protobuf transfer type, which the default payload converter encodes.
        internal static Payload ToPayload<T>(T model)
        {
            var converter = CreateSdkConverter(typeof(T));
            return DataConverter.Default.PayloadConverter.ToPayload(
                converter.ToTransferType(model));
        }

        internal static T FromPayload<T>(Payload payload)
        {
            var converter = CreateSdkConverter(typeof(T));
            return Assert.IsType<T>(converter.FromTransferType(
                DataConverter.Default.PayloadConverter.ToValue(payload, converter.TransferType)));
        }
    }
}
