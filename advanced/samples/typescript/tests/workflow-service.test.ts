import { fileURLToPath } from "node:url";
import { describe, expect, test, vi } from "vitest";
import * as workflow from "@temporalio/workflow";
import * as nexus from "nexus-rpc";

import { signalWithStartWorkflow } from "../wit/workflow-service/index.ts";
import type { SignalWithStartWorkflowRequest } from "../wit/workflow-service/index.ts";
import { workflowService } from "../wit/workflow-service/services.ts";
import { executeWorkflowWithNexus, withWorkflowEnvironment } from "./helpers.ts";

const workflowsPath = fileURLToPath(
  new URL("./workflows/workflow-service.ts", import.meta.url),
);

// The handler is only a test transport. Give its copy of the generated
// converters a fixed converter; the Workflow bundle imports the real module
// and continues to obtain its converter from the activation context.
vi.mock("../wit/workflow-service/support.ts", async (importOriginal) => {
  const support =
    await importOriginal<typeof import("../wit/workflow-service/support.ts")>();
  const common = await import("@temporalio/common");
  return {
    ...support,
    payloadsFromProto: () => [],
    payloadToValue: <T>() => undefined as T,
  };
});

describe("workflow-service generated output", () => {
  test("exposes workflow service metadata", () => {
    expect(workflowService.name).toBe(
      "temporal.api.workflowservice.v1.WorkflowService",
    );
    expect(workflowService.operations.signalWithStartWorkflow.name).toBe(
      "SignalWithStartWorkflowExecution",
    );
  });

  test("serializes signal-with-start requests through a Nexus client", async () => {
    await withWorkflowEnvironment(async (env) => {
      const calls: Array<[string, unknown]> = [];
      const handler = nexus.serviceHandler(workflowService, {
        async signalWithStartWorkflow(_ctx, input) {
          calls.push(["SignalWithStartWorkflowExecution", input]);
          return {
            runId: "run-123",
            started: true,
          };
        },
      });

      const result = await executeWorkflowWithNexus<{
        runId: string | undefined;
        workflowId: string;
      }>(env, {
        endpoint: "temporal-system",
        nexusServices: [handler],
        workflowType: "workflowServiceCaller",
        workflowsPath,
      });

      expect(result).toEqual({
        runId: "run-123",
        workflowId: "workflow-id",
      });
      expect(calls).toHaveLength(1);

      const request = calls[0]?.[1] as SignalWithStartWorkflowRequest | undefined;
      expect(request?.namespace).toBe("default");
      expect(request?.workflow).toBe("exampleWorkflow");
      expect(request?.id).toBe("workflow-id");
      expect(request?.taskQueue).toBe("demo-task-queue");
      expect(request?.signal).toBe("wake-up");
      expect(request?.cronSchedule).toBe("");
    });
  });
});

if (false) {
  async function exampleWorkflow(attempts: number, note: string): Promise<void> {
    void attempts;
    void note;
  }

  const taskQueue = "demo-task-queue";
  const cronSchedule = "";
  const wakeUpSignal = workflow.defineSignal<[number, string]>("wake-up");

  const request: SignalWithStartWorkflowRequest<
    typeof exampleWorkflow,
    typeof wakeUpSignal
  > = {
    workflow: exampleWorkflow,
    args: [3, "nexus"],
    id: "workflow-id",
    taskQueue,
    signal: wakeUpSignal,
    signalArgs: [7, "hello"],
    cronSchedule,
    staticSummary: "Workflow summary",
    staticDetails: "Workflow details",
    namespace: "default",
  };

  // @ts-expect-error flattened user metadata fields should be strings
  request.staticSummary = 7;

  request.namespace;

  // @ts-expect-error missing workflow args for a callable workflow
  signalWithStartWorkflow<typeof exampleWorkflow, typeof wakeUpSignal>({
    workflow: exampleWorkflow,
    id: "missing-workflow-input",
    taskQueue,
    signal: "wake-up",
    cronSchedule,
  });

  // @ts-expect-error workflow args must match the workflow callable
  signalWithStartWorkflow<typeof exampleWorkflow, typeof wakeUpSignal>({
    workflow: exampleWorkflow,
    args: [3, 4],
    id: "bad-workflow-input",
    taskQueue,
    signal: "wake-up",
    cronSchedule,
  });

  // @ts-expect-error missing signal args for a signal definition
  signalWithStartWorkflow<typeof exampleWorkflow, typeof wakeUpSignal>({
    workflow: "ExampleWorkflow",
    id: "missing-signal-input",
    taskQueue,
    signal: wakeUpSignal,
    cronSchedule,
  });

  // @ts-expect-error signal args must match the signal definition
  signalWithStartWorkflow<typeof exampleWorkflow, typeof wakeUpSignal>({
    workflow: "ExampleWorkflow",
    id: "bad-signal-input",
    taskQueue,
    signal: wakeUpSignal,
    signalArgs: ["wrong", 7],
    cronSchedule,
  });

  // @ts-expect-error request models are write-only
  SignalWithStartWorkflowRequest.fromProto({});
}
