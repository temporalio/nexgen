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


def workflow_writer() -> typing.Any:
    """The run's Workflow Producer, with the run that a retry run retries."""
    models = _models()
    run_id = temporalio.workflow.info().run_id
    return models.StreamWriter(
        producer=models.Producer(id=f"wf:{run_id}", run_id=run_id),
        failed_tasks=[],
        withdraw_run=withdraw_run(),
    )


def client_writer() -> typing.Any:
    """An Activity's attempt, or a client."""
    models = _models()
    if temporalio.activity.in_activity():
        info = temporalio.activity.info()
        producer = models.Producer(
            id=f"act:{info.workflow_run_id}:{info.activity_id}",
            run_id=info.workflow_run_id,
        )
    else:
        producer = models.Producer(id="client:sample")
    # The sample has no Local Activity context, so a client writer withdraws nothing.
    return models.StreamWriter(producer=producer, failed_tasks=[])


def withdraw_run() -> str | None:
    """The run that a retry run's Workflow retries. None elsewhere."""
    if not temporalio.workflow.in_workflow():
        return None
    info = temporalio.workflow.info()
    return info.continued_run_id if info.attempt > 1 else None


def request_id() -> str:
    """A new id for each create call. A Workflow's replay gives the same id."""
    if temporalio.workflow.in_workflow():
        return str(temporalio.workflow.uuid4())
    return str(uuid.uuid4())


def last_offset(response: typing.Any) -> int:
    """The offset of the last Record that an append wrote, or 0 for none."""
    return response.last_offset or 0
