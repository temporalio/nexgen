import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";
import * as nexus from "nexus-rpc";

import type {
  CancelWorkflowRequest,
  StartWorkflowRequest,
} from "../wit/start-workflow/models.ts";
import { StartedWorkflow } from "../wit/start-workflow/resources.ts";
import { startWorkflowService } from "../wit/start-workflow/services.ts";
import { executeWorkflowWithNexus, withWorkflowEnvironment } from "./helpers.ts";

const workflowsPath = fileURLToPath(
  new URL("./workflows/start-workflow.ts", import.meta.url),
);

describe("start-workflow generated output", () => {
  test("exposes workflow service metadata", () => {
    expect(startWorkflowService.name).toBe("StartWorkflowService");
    expect(startWorkflowService.operations.startWorkflow.name).toBe("StartWorkflow");
    expect(startWorkflowService.operations.restartWorkflow.name).toBe(
      "RestartWorkflow",
    );
    expect(startWorkflowService.operations.cancelWorkflow.name).toBe("CancelWorkflow");
  });

  test("returns a started workflow wrapper handle through a real Nexus client", async () => {
    await withWorkflowEnvironment(async (env) => {
      const calls: Array<[string, unknown]> = [];
      const handler = nexus.serviceHandler(startWorkflowService, {
        async startWorkflow(_ctx, input) {
          calls.push(["StartWorkflow", input]);
          return {
            runId: "run-123",
            started: true,
          };
        },
        async restartWorkflow(_ctx, input) {
          calls.push(["RestartWorkflow", input]);
          return {
            runId: "run-456",
            started: true,
          };
        },
        async cancelWorkflow(_ctx, input) {
          calls.push(["CancelWorkflow", input]);
          return {};
        },
      });

      const result = await executeWorkflowWithNexus<{
        namespace: string;
        restartedRunId: string | undefined;
        runId: string | undefined;
        workflowId: string;
      }>(env, {
        endpoint: "temporal-system",
        nexusServices: [handler],
        workflowType: "startWorkflowCaller",
        workflowsPath,
      });

      expect(result).toEqual({
        namespace: "default",
        restartedRunId: "run-456",
        runId: "run-123",
        workflowId: "workflow-id",
      });

      expect(calls).toHaveLength(3);
      const startRequest = calls[0]?.[1] as StartWorkflowRequest | undefined;
      expect(startRequest?.namespace).toBe("default");
      expect(startRequest?.workflowId).toBe("workflow-id");
      expect(startRequest?.workflow).toBe("exampleWorkflow");
      expect(startRequest?.taskQueue).toBe("demo-task-queue");

      const restartRequest = calls[1]?.[1] as StartWorkflowRequest | undefined;
      expect(restartRequest?.namespace).toBe("default");
      expect(restartRequest?.workflowId).toBe("workflow-id");
      expect(restartRequest?.workflow).toBe("exampleWorkflow");
      expect(restartRequest?.taskQueue).toBe("demo-task-queue");

      const cancelRequest = calls[2]?.[1] as CancelWorkflowRequest | undefined;
      expect(cancelRequest?.namespace).toBe("default");
      expect(cancelRequest?.workflowExecution.workflowId).toBe("workflow-id");
      expect(cancelRequest?.workflowExecution.runId).toBe("run-123");

      await expect(
        new StartedWorkflow("default", "workflow-id", "run-456").getResult(),
      ).rejects.toThrow("started-workflow.getResult is not yet implemented");
    });
  });
});
