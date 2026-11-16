using System.Collections.Generic;
using Nexgen.ProtoBackedShowcaseService;
using Xunit;
using Wire = Temporalio.Api;

namespace Nexgen.DotNetExamples.Tests
{
    public class NestedProtoModelChecks
    {
        [Theory]
        [InlineData(false)]
        [InlineData(true)]
        public void NestedModelCollectionsRoundTrip(bool populated)
        {
            var partitions = new TaskQueuePartitions(System.Array.Empty<TaskQueuePartition>())
            {
                WorkflowTaskQueuePartitions = populated
                    ? new[]
                    {
                        new TaskQueuePartition("first", "first-host"),
                        new TaskQueuePartition("second", "second-host"),
                    }
                    : null,
            };
            var partitionConverter = new TaskQueuePartitions.TransferTypeConverter();
            var wirePartitions = Assert.IsType<Wire.WorkflowService.V1.ListTaskQueuePartitionsResponse>(
                partitionConverter.ToTransferType(partitions));
            var expectedPartitions = new Wire.WorkflowService.V1.ListTaskQueuePartitionsResponse();
            if (populated)
            {
                expectedPartitions.WorkflowTaskQueuePartitions.Add(new[]
                {
                    new Wire.TaskQueue.V1.TaskQueuePartitionMetadata { Key = "first", OwnerHostName = "first-host" },
                    new Wire.TaskQueue.V1.TaskQueuePartitionMetadata { Key = "second", OwnerHostName = "second-host" },
                });
            }
            Assert.Equal(expectedPartitions, wirePartitions);
            var decodedPartitions = Assert.IsType<TaskQueuePartitions>(
                partitionConverter.FromTransferType(wirePartitions));
            Assert.NotNull(decodedPartitions.WorkflowTaskQueuePartitions);
            if (populated)
            {
                Assert.Collection(decodedPartitions.WorkflowTaskQueuePartitions,
                    first => Assert.Equal(new TaskQueuePartition("first", "first-host"), first),
                    second => Assert.Equal(new TaskQueuePartition("second", "second-host"), second));
            }
            else
            {
                Assert.Empty(decodedPartitions.WorkflowTaskQueuePartitions);
            }

            var binaries = new BadBinaries
            {
                Binaries = populated
                    ? new Dictionary<string, BadBinary>
                    {
                        ["first"] = new BadBinary("first-reason", "first-operator"),
                        ["second"] = new BadBinary("second-reason", "second-operator"),
                    }
                    : null,
            };
            var binaryConverter = new BadBinaries.TransferTypeConverter();
            var wireBinaries = Assert.IsType<Wire.Namespace.V1.BadBinaries>(
                binaryConverter.ToTransferType(binaries));
            var expectedBinaries = new Wire.Namespace.V1.BadBinaries();
            if (populated)
            {
                expectedBinaries.Binaries.Add("first", new Wire.Namespace.V1.BadBinaryInfo
                {
                    Reason = "first-reason",
                    Operator = "first-operator",
                });
                expectedBinaries.Binaries.Add("second", new Wire.Namespace.V1.BadBinaryInfo
                {
                    Reason = "second-reason",
                    Operator = "second-operator",
                });
            }
            Assert.Equal(expectedBinaries, wireBinaries);
            var decodedBinaries = Assert.IsType<BadBinaries>(
                binaryConverter.FromTransferType(wireBinaries));
            Assert.NotNull(decodedBinaries.Binaries);
            if (populated)
            {
                Assert.Equal(2, decodedBinaries.Binaries.Count);
                Assert.Equal(new BadBinary("first-reason", "first-operator"), decodedBinaries.Binaries["first"]);
                Assert.Equal(new BadBinary("second-reason", "second-operator"), decodedBinaries.Binaries["second"]);
            }
            else
            {
                Assert.Empty(decodedBinaries.Binaries);
            }
        }
    }
}
