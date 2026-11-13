"""Sources of the stream sample, in place of those of the Streams SDK."""

from __future__ import annotations

import importlib
import typing
import uuid

import temporalio.activity
import temporalio.workflow


def _models() -> typing.Any:
    # The sample's models import the support package, so this finds them when it runs.
    return importlib.import_module("wit.stream_service.both.models")


def workflow_producer() -> typing.Any:
    """The run's Workflow Producer."""
    models = _models()
    run_id = temporalio.workflow.info().run_id
    return models.Producer(
        id=f"wf:{run_id}",
        kind=models.ProducerKindWorkflow(models.WorkflowProducer(run_id=run_id)),
    )


def default_producer() -> typing.Any:
    """An Activity's attempt, or a client."""
    models = _models()
    if temporalio.activity.in_activity():
        info = temporalio.activity.info()
        scheduled = info.current_attempt_scheduled_time.timestamp()
        epoch = models.ActivityEpoch(
            attempt_scheduled_time=models.Timestamp(
                seconds=int(scheduled), nanos=round(scheduled % 1 * 1e9)
            ),
            attempt=info.attempt,
        )
        return models.Producer(
            id=f"act:{info.workflow_run_id}:{info.activity_id}",
            kind=models.ProducerKindWorkflowActivity(
                models.ActivityProducer(run_id=info.workflow_run_id, epoch=epoch)
            ),
        )
    return models.Producer(
        id="client:sample",
        kind=models.ProducerKindClient(models.ClientProducer()),
    )


def last_offset(response: typing.Any) -> bytes:
    """The offset of the last Record that an append wrote, or empty for none."""
    return response.last_offset or b""


def request_id() -> str:
    """A new id for each create call. A Workflow's replay gives the same id."""
    if temporalio.workflow.in_workflow():
        return str(temporalio.workflow.uuid4())
    return str(uuid.uuid4())
