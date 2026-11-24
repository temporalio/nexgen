"""Sources of the stream sample, in place of those of the Streams SDK."""

from __future__ import annotations

import importlib
import typing

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
        # The Activity info has no scheduled event id, so the Producer sends zero for it.
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
