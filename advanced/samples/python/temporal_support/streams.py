"""Sources of the stream sample, in place of those of the Streams SDK."""

from __future__ import annotations

import importlib
import typing
import uuid

import temporalio.activity
import temporalio.workflow


def _producer(producer_id: str, run_id: str | None = None) -> typing.Any:
    # The sample's models import the support package, so this finds them when it runs.
    models = importlib.import_module("wit.stream_service.both.models")
    return models.Producer(id=producer_id, run_id=run_id)


def workflow_producer() -> typing.Any:
    run_id = temporalio.workflow.info().run_id
    return _producer(f"wf:{run_id}", run_id)


def client_producer() -> typing.Any:
    """An Activity's attempt, or a client."""
    if temporalio.activity.in_activity():
        info = temporalio.activity.info()
        return _producer(
            f"act:{info.workflow_run_id}:{info.activity_id}", info.workflow_run_id
        )
    return _producer("client:sample")


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
