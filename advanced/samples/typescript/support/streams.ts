// Sources of the stream sample, in place of those of the Streams SDK.

import * as workflow from "@temporalio/workflow";

import type { StreamWriter } from "../wit/stream-service/workflow/models";

/** The run's Workflow Producer. */
export function workflowWriter(): StreamWriter {
  const runId = workflow.workflowInfo().runId;
  return {
    producer: { id: `wf:${runId}`, kind: { tag: "workflow", value: { runId } } },
  };
}

/** A new id for each create call. A Workflow's replay gives the same id. */
export function requestId(): string {
  return workflow.inWorkflowContext() ? workflow.uuid4() : crypto.randomUUID();
}
