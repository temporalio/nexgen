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
    info = temporalio.workflow.info()
    return models.Producer(
        kind=models.ProducerKindWorkflow(
            models.WorkflowProducer(
                namespace=info.namespace,
                workflow_id=info.workflow_id,
                run_id=info.run_id,
            )
        ),
    )


def default_producer(nexus_client: object) -> typing.Any:
    """An Activity's attempt, or a client."""
    # The sample has one client Producer, so it doesn't read the client.
    del nexus_client
    models = _models()
    if temporalio.activity.in_activity():
        info = temporalio.activity.info()
        scheduled = info.current_attempt_scheduled_time.timestamp()
        attempt = models.ActivityAttempt(
            attempt_scheduled_time=models.Timestamp(
                seconds=int(scheduled), nanos=round(scheduled % 1 * 1e9)
            ),
            attempt=info.attempt,
        )
        return models.Producer(
            kind=models.ProducerKindWorkflowActivity(
                models.WorkflowActivityProducer(
                    namespace=info.workflow_namespace,
                    workflow_id=info.workflow_id,
                    run_id=info.workflow_run_id,
                    activity_id=info.activity_id,
                    attempt=attempt,
                )
            ),
        )
    return models.Producer(
        kind=models.ProducerKindClient(
            models.ClientProducer(namespace="default", id="client:sample")
        ),
    )


def request_id() -> str:
    """A new id for each create call. A Workflow's replay gives the same id."""
    if temporalio.workflow.in_workflow():
        return str(temporalio.workflow.uuid4())
    return str(uuid.uuid4())


def append_request_id() -> str | None:
    """A new id for each append call, or none in a Workflow, where the SDK sets it."""
    if temporalio.workflow.in_workflow():
        return None
    return str(uuid.uuid4())


def stream_ref_to_id(ref: typing.Any) -> str:
    """The Stream id of a ref, which is all that a request message holds."""
    return typing.cast(str, ref.id)


def stream_ref_from_id(id: str, model_type: typing.Any) -> typing.Any:
    """A ref with the Stream id, of the model type of the caller. A message has no Store name."""
    return model_type(id=id)
