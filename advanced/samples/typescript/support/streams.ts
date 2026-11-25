// Sources of the stream sample, in place of those of the Streams SDK.

import * as workflow from "@temporalio/workflow";

import type { Producer } from "../wit/stream-service/workflow/models";

/** The run's Workflow Producer. */
export function workflowProducer(): Producer {
  const { namespace, workflowId, runId } = workflow.workflowInfo();
  return { kind: { tag: "workflow", value: { namespace, workflowId, runId } } };
}
