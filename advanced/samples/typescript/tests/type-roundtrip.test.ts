import { fileURLToPath } from "node:url";
import { describe, expect, test, vi } from "vitest";
import * as common from "@temporalio/common";
import type { temporal } from "@temporalio/proto";
import * as nexus from "nexus-rpc";

import type { ActivityOptions, FailureContainer } from "../wit/type-roundtrip/index.ts";
import { typeRoundtripService } from "../wit/type-roundtrip/services.ts";
import { failureFromProto, failureToProto } from "../wit/type-roundtrip/support.ts";
import { executeWorkflowWithNexus, withWorkflowEnvironment } from "./helpers.ts";

const workflowsPath = fileURLToPath(
  new URL("./workflows/type-roundtrip.ts", import.meta.url),
);

// The handler is only a test transport. Give its copy of the generated
// converters a fixed converter; the Workflow bundle imports the real module
// and continues to obtain its converter from the activation context.
vi.mock("../wit/type-roundtrip/support.ts", async (importOriginal) => {
  const support =
    await importOriginal<typeof import("../wit/type-roundtrip/support.ts")>();
  const common = await import("@temporalio/common");
  return {
    ...support,
    failureFromProto: (failure: temporal.api.failure.v1.IFailure) =>
      common.defaultFailureConverter.failureToError(
        failure,
        common.defaultPayloadConverter,
      ),
    failureToProto: (failure: Error) =>
      common.defaultFailureConverter.errorToFailure(
        failure,
        common.defaultPayloadConverter,
      ),
  };
});

describe("type-roundtrip generated output", () => {
  test("exposes type roundtrip service metadata", () => {
    expect(typeRoundtripService.name).toBe("TypeRoundtripService");
    expect(typeRoundtripService.operations.activityOptionsOperation.name).toBe(
      "ActivityOptionsOperation",
    );
    expect(typeRoundtripService.operations.failureOperation.name).toBe(
      "FailureOperation",
    );
  });

  test("exposes activity options as a public type", () => {
    const retryPolicy: common.RetryPolicy = { maximumAttempts: 3 };
    const activityOptions: ActivityOptions = {
      taskQueue: "demo-task-queue",
      retryPolicy,
      scheduleToCloseTimeout: "1 minute",
      priority: {
        priorityKey: 1,
        fairnessKey: "customer-123",
      },
    };

    expect(activityOptions.taskQueue).toBe("demo-task-queue");
    expect(activityOptions.retryPolicy.maximumAttempts).toBe(3);
    expect(activityOptions.priority?.priorityKey).toBe(1);
  });

  test("uses the SDK failure converter for encoded attributes", () => {
    const runtime = globalThis as typeof globalThis & {
      __TEMPORAL_ACTIVATOR__?: { payloadConverter?: common.PayloadConverter };
    };
    const previousActivator = runtime.__TEMPORAL_ACTIVATOR__;
    runtime.__TEMPORAL_ACTIVATOR__ = {
      payloadConverter: common.defaultPayloadConverter,
    };

    try {
      const proto = failureToProto(
        common.ApplicationFailure.create({
          message: "unencoded failure",
          type: "EncodedFailure",
          details: ["detail"],
        }),
      );
      proto.message = "Encoded failure";
      proto.stackTrace = "";
      proto.encodedAttributes = common.defaultPayloadConverter.toPayload({
        message: "decoded encoded failure",
        stack_trace: "decoded stack",
      });

      const failure = failureFromProto(proto) as common.ApplicationFailure;
      expect(failure.message).toBe("decoded encoded failure");
      expect(failure.stack).toBe("decoded stack");
      expect(failure.type).toBe("EncodedFailure");
      expect(failure.details).toEqual(["detail"]);

      const roundTripped = failureToProto(failure);
      expect(roundTripped.message).toBe("decoded encoded failure");
      expect(roundTripped.applicationFailureInfo?.type).toBe("EncodedFailure");
      expect(roundTripped.encodedAttributes).toBeDefined();
    } finally {
      runtime.__TEMPORAL_ACTIVATOR__ = previousActivator;
    }
  });

  test("round-trips proto-backed types through a real Nexus client", async () => {
    await withWorkflowEnvironment(async (env) => {
      const calls: Array<[string, unknown]> = [];
      const handler = nexus.serviceHandler(typeRoundtripService, {
        async activityOptionsOperation(_ctx, input) {
          calls.push(["ActivityOptionsOperation", input]);
          return input;
        },
        async failureOperation(_ctx, input) {
          calls.push(["FailureOperation", input]);
          return input;
        },
      });

      const result = await executeWorkflowWithNexus<{
        failureCauseMessage: string | undefined;
        failureDetails: unknown[] | undefined;
        failureMessage: string | undefined;
        failureNonRetryable: boolean | undefined;
        failureType: string | undefined;
        priorityKey: number | undefined;
        retryMaximumAttempts: number | undefined;
        scheduleToCloseTimeout: common.Duration | undefined;
        taskQueue: string | undefined;
      }>(env, {
        endpoint: "temporal-system",
        nexusServices: [handler],
        workflowType: "typeRoundtripCaller",
        workflowsPath,
      });

      expect(result).toEqual({
        failureCauseMessage: "inner failure",
        failureDetails: [],
        failureMessage: "outer failure",
        failureNonRetryable: true,
        failureType: "OuterFailure",
        priorityKey: 4,
        retryMaximumAttempts: 3,
        scheduleToCloseTimeout: 7_000,
        taskQueue: "demo-task-queue",
      });
      expect(calls).toHaveLength(2);

      const activityRequest = calls[0]?.[1] as ActivityOptions | undefined;
      expect(activityRequest?.taskQueue).toBe("demo-task-queue");
      expect(activityRequest?.retryPolicy?.maximumAttempts).toBe(3);
      expect(activityRequest?.scheduleToCloseTimeout).toBe(7_000);
      expect(activityRequest?.priority?.priorityKey).toBe(4);

      const failureRequest = calls[1]?.[1] as FailureContainer | undefined;
      const failure = failureRequest?.failure as common.ApplicationFailure | undefined;
      expect(failure?.message).toBe("outer failure");
      expect(failure?.type).toBe("OuterFailure");
      expect(failure?.nonRetryable).toBe(true);
      expect(failure?.cause?.message).toBe("inner failure");
    });
  });
});
