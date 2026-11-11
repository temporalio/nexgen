// Sources of the stream sample, in place of those of the Streams SDK.

import * as workflow from "@temporalio/workflow";

import type { Producer } from "../wit/stream-service/workflow/models";

export function workflowProducer(): Producer {
  const runId = workflow.workflowInfo().runId;
  return { id: `wf:${runId}`, runId };
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
