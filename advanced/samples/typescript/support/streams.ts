// Sources of the stream sample, in place of those of the Streams SDK.

import * as workflow from "@temporalio/workflow";

import type { StreamWriter } from "../wit/stream-service/workflow/models";

/** The run's Workflow Producer, with the run that a retry run retries. */
export function workflowWriter(): StreamWriter {
  const runId = workflow.workflowInfo().runId;
  return {
    producer: { id: `wf:${runId}`, runId },
    failedTasks: [],
    withdrawRun: withdrawRun(),
  };
}

/** The run that a retry run's Workflow retries. Undefined elsewhere. */
export function withdrawRun(): string | undefined {
  if (!workflow.inWorkflowContext()) {
    return undefined;
  }
  const info = workflow.workflowInfo();
  return info.attempt > 1 ? info.continuedFromExecutionRunId : undefined;
}

/** A new id for each create call. A Workflow's replay gives the same id. */
export function requestId(): string {
  return workflow.inWorkflowContext() ? workflow.uuid4() : crypto.randomUUID();
}
