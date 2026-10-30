import { startWorkflow } from "../../wit/start-workflow/index.ts";
import { StartedWorkflow } from "../../wit/start-workflow/resources.ts";

export async function exampleWorkflow(customerId: string): Promise<string> {
  return customerId;
}

export async function startWorkflowCaller(): Promise<{
  namespace: string;
  restartedRunId: string | undefined;
  runId: string | undefined;
  workflowId: string;
}> {
  const handle = await startWorkflow({
    workflow: "exampleWorkflow",
    workflowId: "workflow-id",
    taskQueue: "demo-task-queue",
  });
  const restartedHandle = await handle.restartWorkflow(
    "exampleWorkflow",
    "demo-task-queue",
  );
  // The handle's namespace wins over the namespace source.
  await new StartedWorkflow(
    "stored-namespace",
    handle.workflowId,
    handle.runId,
  ).cancel();
  return {
    namespace: handle.namespace,
    restartedRunId: restartedHandle.runId,
    runId: handle.runId,
    workflowId: handle.workflowId,
  };
}
