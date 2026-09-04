import * as common from "@temporalio/common";
import {
  activityOptionsOperation,
  failureOperation,
} from "../../wit/type-roundtrip/index.ts";

const TASK_QUEUE = "demo-task-queue";

function durationSecondsToMillis(
  seconds:
    { low?: number; toNumber?: () => number } | number | string | null | undefined,
): number | undefined {
  if (seconds == null) {
    return undefined;
  }
  if (typeof seconds === "object") {
    if (seconds.toNumber != null) {
      return seconds.toNumber() * 1000;
    }
    return seconds.low == null ? undefined : seconds.low * 1000;
  }
  return Number(seconds) * 1000;
}

export async function typeRoundtripCaller(): Promise<{
  failureCauseMessage: string | undefined;
  failureDetails: unknown[] | undefined;
  failureMessage: string | undefined;
  failureNonRetryable: boolean | undefined;
  failureType: string | undefined;
  priorityKey: number | undefined;
  retryMaximumAttempts: number | undefined;
  scheduleToCloseTimeout: number | undefined;
  taskQueue: string | undefined;
}> {
  const retryPolicy: common.RetryPolicy = { maximumAttempts: 3 };
  const activityHandle = await activityOptionsOperation({
    taskQueue: TASK_QUEUE,
    scheduleToCloseTimeout: "7 seconds",
    retryPolicy,
    priority: {
      priorityKey: 4,
      fairnessKey: "tenant-a",
      fairnessWeight: 2.5,
    },
  });
  const activityRoundTrip = await activityHandle.result();
  const scheduleToCloseSeconds = activityRoundTrip.scheduleToCloseTimeout?.seconds;
  const failureHandle = await failureOperation({
    failure: common.ApplicationFailure.create({
      message: "outer failure",
      type: "OuterFailure",
      nonRetryable: true,
      cause: common.ApplicationFailure.create({
        message: "inner failure",
        type: "InnerFailure",
      }),
    }),
  });
  const failureRoundTrip = await failureHandle.result();
  const applicationFailure = failureRoundTrip.failure as
    common.ApplicationFailure | undefined;

  return {
    failureCauseMessage: applicationFailure?.cause?.message,
    failureDetails: applicationFailure?.details ?? undefined,
    failureMessage: applicationFailure?.message,
    failureNonRetryable: applicationFailure?.nonRetryable ?? undefined,
    failureType: applicationFailure?.type ?? undefined,
    priorityKey: activityRoundTrip.priority?.priorityKey ?? undefined,
    retryMaximumAttempts: activityRoundTrip.retryPolicy?.maximumAttempts ?? undefined,
    scheduleToCloseTimeout: durationSecondsToMillis(scheduleToCloseSeconds),
    taskQueue: activityRoundTrip.taskQueue ?? undefined,
  };
}
