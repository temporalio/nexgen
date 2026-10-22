using System;
using Nexgen.ProtoEnums;
using Xunit;
using WireAlert = Temporalio.Api.Version.V1.Alert;
using WireCommand = Temporalio.Api.Command.V1.Command;
using WireNamespaceInfo = Temporalio.Api.Namespace.V1.NamespaceInfo;

namespace Nexgen.DotNetExamples.Tests
{
    public class ProtoEnumChecks
    {
        [Fact]
        public void OrderedEnumRoundTrips()
        {
            var converter = new NamespaceInfo.TransferTypeConverter();
            foreach (NamespaceState member in Enum.GetValues<NamespaceState>())
            {
                var model = new NamespaceInfo { State = member };
                var wire = Assert.IsType<WireNamespaceInfo>(converter.ToTransferType(model));
                Assert.Equal((int)member, (int)wire.State);
                var decoded = Assert.IsType<NamespaceInfo>(converter.FromTransferType(wire));
                // .NET represents the protobuf zero default as null.
                Assert.Equal((int)member == 0 ? (NamespaceState?)null : member, decoded.State);
            }
            Assert.Equal(1, (int)NamespaceState.NamespaceStateRegistered);
            Assert.Equal(1, (int)Assert.IsType<WireNamespaceInfo>(
                converter.ToTransferType(new NamespaceInfo())).State);
        }

        [Fact]
        public void ReorderedEnumRoundTrips()
        {
            Assert.Equal(3, (int)AlertLevel.SeverityLow);
            Assert.Equal(1, (int)AlertLevel.SeverityHigh);
            Assert.Equal(2, (int)AlertLevel.SeverityMedium);
            Assert.Equal(0, (int)AlertLevel.SeverityUnspecified);
            var converter = new Alert.TransferTypeConverter();
            foreach (AlertLevel member in Enum.GetValues<AlertLevel>())
            {
                var model = new Alert("test") { Severity = member };
                var wire = Assert.IsType<WireAlert>(converter.ToTransferType(model));
                Assert.Equal((int)member, (int)wire.Severity);
                var decoded = Assert.IsType<Alert>(converter.FromTransferType(wire));
                Assert.Equal((int)member == 0 ? (AlertLevel?)null : member, decoded.Severity);
            }
            Assert.Equal(3, (int)Assert.IsType<WireAlert>(
                converter.ToTransferType(new Alert("test"))).Severity);
        }

        [Fact]
        public void SparseEnumRoundTrips()
        {
            Assert.Equal(14, (int)CommandType.CommandTypeProtocolMessage);
            Assert.Equal(16, (int)CommandType.CommandTypeModifyWorkflowProperties);
            Assert.Equal(17, (int)CommandType.CommandTypeScheduleNexusOperation);
            Assert.Equal(18, (int)CommandType.CommandTypeRequestCancelNexusOperation);
            Assert.False(Enum.IsDefined(typeof(CommandType), 15));
            var converter = new CommandModel.TransferTypeConverter();
            foreach (CommandType member in Enum.GetValues<CommandType>())
            {
                var model = new CommandModel { CommandType = member };
                var wire = Assert.IsType<WireCommand>(converter.ToTransferType(model));
                Assert.Equal((int)member, (int)wire.CommandType);
                var decoded = Assert.IsType<CommandModel>(converter.FromTransferType(wire));
                Assert.Equal((int)member == 0 ? (CommandType?)null : member, decoded.CommandType);
            }
            Assert.Equal(16, (int)Assert.IsType<WireCommand>(
                converter.ToTransferType(new CommandModel())).CommandType);
        }
    }
}
