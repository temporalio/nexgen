package support;

import io.temporal.api.common.v1.Priority;
import io.temporal.api.common.v1.RetryPolicy;
import io.temporal.api.common.v1.WorkflowType;
import io.temporal.api.taskqueue.v1.TaskQueue;
import io.temporal.common.RetryOptions;
import io.temporal.common.VersioningOverride;
import io.temporal.internal.common.ProtoConverters;
import io.temporal.internal.common.ProtobufTimeUtils;
import io.temporal.internal.common.RetryOptionsUtils;
import io.temporal.payload.context.WorkflowSerializationContext;
import io.temporal.workflow.Workflow;
import java.time.Duration;
import wit.workflowservice.SignalWithStartWorkflowRequest;

/** Structural conversion hooks consumed by the generated models. */
public final class TemporalModelConverters {
  private TemporalModelConverters() {}

  public static String workflowNamespace() {
    return Workflow.getInfo().getNamespace();
  }

  public static WorkflowSerializationContext signalWithStartWorkflowSerializationContext(
      SignalWithStartWorkflowRequest request) {
    return new WorkflowSerializationContext(request.getNamespace(), request.getId());
  }

  public static WorkflowType workflowTypeToProto(String value) {
    return WorkflowType.newBuilder().setName(value).build();
  }

  public static String workflowTypeFromProto(WorkflowType value) {
    return value.getName();
  }

  public static TaskQueue taskQueueToProto(String value) {
    return TaskQueue.newBuilder().setName(value).build();
  }

  public static String taskQueueFromProto(TaskQueue value) {
    return value.getName();
  }

  public static com.google.protobuf.Duration durationToProto(Duration value) {
    return ProtobufTimeUtils.toProtoDuration(value);
  }

  public static Duration durationFromProto(com.google.protobuf.Duration value) {
    return ProtobufTimeUtils.toJavaDuration(value);
  }

  public static RetryPolicy retryPolicyToProto(RetryOptions value) {
    return RetryOptionsUtils.toRetryPolicy(value).build();
  }

  public static RetryOptions retryPolicyFromProto(RetryPolicy value) {
    return RetryOptionsUtils.toRetryOptions(value);
  }

  public static Priority priorityToProto(io.temporal.common.Priority value) {
    return ProtoConverters.toProto(value);
  }

  public static io.temporal.common.Priority priorityFromProto(Priority value) {
    return ProtoConverters.fromProto(value);
  }

  public static io.temporal.api.workflow.v1.VersioningOverride versioningOverrideToProto(
      VersioningOverride value) {
    return ProtoConverters.toProto(value);
  }

  public static VersioningOverride versioningOverrideFromProto(
      io.temporal.api.workflow.v1.VersioningOverride value) {
    return io.temporal.internal.nexus.system.TemporalModelConverters.versioningOverrideFromProto(value);
  }
}
