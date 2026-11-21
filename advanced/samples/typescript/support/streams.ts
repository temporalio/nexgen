// Sources of the stream sample, in place of those of the Streams SDK.

import * as workflow from "@temporalio/workflow";

import type { Producer, StreamRef } from "../wit/stream-service/workflow/models";

/** The run's Workflow Producer. */
export function workflowProducer(): Producer {
  const { namespace, workflowId, runId } = workflow.workflowInfo();
  return { tag: "workflow", value: { namespace, workflowId, runId } };
}

/** A new id for each create call. A Workflow's replay gives the same id. */
export function requestId(): string {
  return workflow.inWorkflowContext() ? workflow.uuid4() : crypto.randomUUID();
}

/** A new id for each append call, or none in a Workflow, where the SDK sets it. */
export function appendRequestId(): string | undefined {
  return workflow.inWorkflowContext() ? undefined : crypto.randomUUID();
}

/** The proto of a request has the id of its ref, and no Store name. */
export function streamRefToId(ref: StreamRef<unknown>): string {
  return ref.id;
}

/** The ref of a request has only the id, since the proto has no Store name. */
export function streamRefFromId<BodyT>(id: string): StreamRef<BodyT> {
  return { id };
}
