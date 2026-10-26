from __future__ import annotations

import datetime
from pathlib import Path
import uuid

from nexusrpc import Operation
from nexusrpc.handler import StartOperationContext, service_handler, sync_operation
import pytest
import temporalio.common
import temporalio.converter
import temporalio.exceptions
import temporalio.api.command.v1
import temporalio.api.failure.v1
import temporalio.nexus.system
from temporalio import workflow
from temporalio.testing import WorkflowEnvironment
from temporalio.worker import UnsandboxedWorkflowRunner, Worker

import wit.proto_backed_showcase as proto_backed_showcase
import wit.proto_backed_showcase.models as proto_backed_showcase_models
import wit.proto_backed_showcase.services as proto_backed_showcase_services

OUTPUT_PATH = Path(__file__).resolve().parent.parent / "wit" / "proto_backed_showcase"
TASK_QUEUE = "demo-task-queue"

ACTIVITY_OPTIONS_OPERATION_INFO = proto_backed_showcase.__nexus_operation_registry__[
    ("ProtoBackedShowcaseService", "ActivityOptionsOperation")
]
ACTIVITY_OPTIONS_OPERATION = ACTIVITY_OPTIONS_OPERATION_INFO.operation
FAILURE_OPERATION_INFO = proto_backed_showcase.__nexus_operation_registry__[
    ("ProtoBackedShowcaseService", "FailureOperation")
]
FAILURE_OPERATION = FAILURE_OPERATION_INFO.operation


@service_handler(service=proto_backed_showcase_services.ProtoBackedShowcaseService)
class ProtoBackedShowcaseServiceHandler:
    def __init__(self) -> None:
        self.calls: list[tuple[str, object]] = []

    @sync_operation
    async def activity_options_operation(
        self,
        _ctx: StartOperationContext,
        input: proto_backed_showcase_models.ActivityOptions,
    ) -> proto_backed_showcase_models.ActivityOptions:
        self.calls.append(("ActivityOptionsOperation", input))
        converter = getattr(input, "__temporal_transfer_type_converter")
        proto = converter.to_transfer_type(input)
        assert proto.HasField("retry_policy")
        assert proto.task_queue.name == TASK_QUEUE
        assert proto.schedule_to_close_timeout.seconds == 7
        assert proto.priority.priority_key == 4
        return input

    @sync_operation
    async def failure_operation(
        self,
        _ctx: StartOperationContext,
        input: proto_backed_showcase_models.FailureContainer,
    ) -> proto_backed_showcase_models.FailureContainer:
        self.calls.append(("FailureOperation", input))
        failure = input.failure
        assert isinstance(failure, temporalio.exceptions.ApplicationError)
        assert failure.message == "outer failure"
        assert failure.type == "OuterFailure"
        assert failure.non_retryable
        assert list(failure.details) == ["detail"]
        assert isinstance(failure.__cause__, temporalio.exceptions.ApplicationError)
        assert failure.__cause__.message == "inner failure"
        return input

    @sync_operation
    async def workflow_execution_info_operation(
        self,
        _ctx: StartOperationContext,
        input: proto_backed_showcase_models.WorkflowExecutionInfo,
    ) -> proto_backed_showcase_models.WorkflowExecutionInfo:
        return input

    @sync_operation
    async def task_queue_partitions_operation(
        self,
        _ctx: StartOperationContext,
        input: proto_backed_showcase_models.TaskQueuePartitions,
    ) -> proto_backed_showcase_models.TaskQueuePartitions:
        return input

    @sync_operation
    async def bad_binaries_operation(
        self,
        _ctx: StartOperationContext,
        input: proto_backed_showcase_models.BadBinaries,
    ) -> proto_backed_showcase_models.BadBinaries:
        return input


@pytest.mark.parametrize("populated", [False, True])
def test_nested_model_collections_round_trip(populated: bool) -> None:
    partitions = proto_backed_showcase_models.TaskQueuePartitions(
        workflow_task_queue_partitions=[
            proto_backed_showcase_models.TaskQueuePartition(
                key="first", owner_host_name="first-host"
            ),
            proto_backed_showcase_models.TaskQueuePartition(
                key="second", owner_host_name="second-host"
            ),
        ]
        if populated
        else None
    )
    partition_converter = getattr(partitions, "__temporal_transfer_type_converter")
    wire_partitions = partition_converter.to_transfer_type(partitions)
    assert [
        (item.key, item.owner_host_name)
        for item in wire_partitions.workflow_task_queue_partitions
    ] == ([("first", "first-host"), ("second", "second-host")] if populated else [])
    decoded_partitions = partition_converter.from_transfer_type(
        wire_partitions, proto_backed_showcase_models.TaskQueuePartitions
    )
    assert decoded_partitions.workflow_task_queue_partitions == (
        partitions.workflow_task_queue_partitions or []
    )

    binaries = proto_backed_showcase_models.BadBinaries(
        binaries={
            "first": proto_backed_showcase_models.BadBinary(
                reason="first-reason", operator="first-operator"
            ),
            "second": proto_backed_showcase_models.BadBinary(
                reason="second-reason", operator="second-operator"
            ),
        }
        if populated
        else None
    )
    binary_converter = getattr(binaries, "__temporal_transfer_type_converter")
    wire_binaries = binary_converter.to_transfer_type(binaries)
    assert {
        key: (item.reason, item.operator)
        for key, item in wire_binaries.binaries.items()
    } == (
        {
            "first": ("first-reason", "first-operator"),
            "second": ("second-reason", "second-operator"),
        }
        if populated
        else {}
    )
    decoded_binaries = binary_converter.from_transfer_type(
        wire_binaries, proto_backed_showcase_models.BadBinaries
    )
    assert decoded_binaries.binaries == (binaries.binaries or {})


@workflow.defn
class ProtoBackedShowcaseCallerWorkflow:
    @workflow.run
    async def run(
        self,
    ) -> tuple[str | None, int | None, int | None, str, str, bool, str]:
        retry_policy = temporalio.common.RetryPolicy(maximum_attempts=3)
        activity_handle = await proto_backed_showcase.activity_options_operation(
            task_queue=TASK_QUEUE,
            schedule_to_close_timeout=datetime.timedelta(seconds=7),
            retry_policy=retry_policy,
            priority=temporalio.common.Priority(
                priority_key=4,
                fairness_key="tenant-a",
                fairness_weight=2.5,
            ),
        )
        activity_response = await activity_handle
        cause = temporalio.exceptions.ApplicationError(
            "inner failure",
            type="InnerFailure",
        )
        failure = temporalio.exceptions.ApplicationError(
            "outer failure",
            "detail",
            type="OuterFailure",
            non_retryable=True,
        )
        failure.__cause__ = cause
        failure_handle = await proto_backed_showcase.failure_operation(failure=failure)
        failure_response = await failure_handle
        converted_failure = failure_response.failure
        assert isinstance(converted_failure, temporalio.exceptions.ApplicationError)
        converted_cause = converted_failure.__cause__
        assert isinstance(converted_cause, temporalio.exceptions.ApplicationError)
        return (
            activity_response.task_queue,
            int(activity_response.schedule_to_close_timeout.total_seconds())
            if activity_response.schedule_to_close_timeout is not None
            else None,
            activity_response.priority.priority_key
            if activity_response.priority is not None
            else None,
            converted_failure.message,
            converted_failure.type or "",
            converted_failure.non_retryable,
            converted_cause.message,
        )


def retry_policy() -> temporalio.common.RetryPolicy:
    return temporalio.common.RetryPolicy(maximum_attempts=3)


def priority() -> temporalio.common.Priority:
    return temporalio.common.Priority(
        priority_key=4,
        fairness_key="tenant-a",
        fairness_weight=2.5,
    )


def test_generated_metadata() -> None:
    assert OUTPUT_PATH.exists(), f"expected generated package at {OUTPUT_PATH}"
    assert isinstance(ACTIVITY_OPTIONS_OPERATION, Operation)
    assert isinstance(FAILURE_OPERATION, Operation)
    assert (
        proto_backed_showcase.__nexus_operation_registry__[
            ("ProtoBackedShowcaseService", "ActivityOptionsOperation")
        ].operation
        is ACTIVITY_OPTIONS_OPERATION
    )
    assert (
        proto_backed_showcase.__nexus_operation_registry__[
            ("ProtoBackedShowcaseService", "ActivityOptionsOperation")
        ].serialization_context
        is None
    )
    assert not hasattr(proto_backed_showcase, "ProtoBackedShowcaseService")
    assert not hasattr(proto_backed_showcase, "ActivityOptions")
    assert not hasattr(proto_backed_showcase, "FailureContainer")


def test_activity_options_round_trip() -> None:
    activity_options = proto_backed_showcase_models.ActivityOptions(
        retry_policy=retry_policy(),
        task_queue=TASK_QUEUE,
        schedule_to_close_timeout=datetime.timedelta(seconds=7),
        priority=priority(),
    )
    converter = getattr(
        proto_backed_showcase_models.ActivityOptions,
        "__temporal_transfer_type_converter",
    )
    activity_proto = converter.to_transfer_type(activity_options)
    assert activity_proto.HasField("retry_policy")
    assert activity_proto.task_queue.name == TASK_QUEUE
    assert activity_proto.schedule_to_close_timeout.seconds == 7
    assert activity_proto.priority.priority_key == 4
    round_tripped_activity = converter.from_transfer_type(
        activity_proto,
        proto_backed_showcase_models.ActivityOptions,
    )
    assert isinstance(
        round_tripped_activity.retry_policy, temporalio.common.RetryPolicy
    )
    assert round_tripped_activity.retry_policy.maximum_attempts == 3
    assert round_tripped_activity.task_queue == TASK_QUEUE
    assert round_tripped_activity.schedule_to_close_timeout == datetime.timedelta(
        seconds=7
    )
    assert round_tripped_activity.priority == priority()


def test_failure_encoded_attributes_round_trip() -> None:
    payload_converter = temporalio.converter.PayloadConverter.default
    encoded_attributes = payload_converter.to_payloads(
        [{"message": "decoded failure", "stack_trace": "decoded stack"}]
    )[0]
    failure = temporalio.api.failure.v1.Failure(
        message="Encoded failure",
        encoded_attributes=encoded_attributes,
        application_failure_info=temporalio.api.failure.v1.ApplicationFailureInfo(
            type="EncodedFailure",
            non_retryable=True,
        ),
    )
    proto = temporalio.api.command.v1.FailWorkflowExecutionCommandAttributes(
        failure=failure
    )
    converter = getattr(
        proto_backed_showcase_models.FailureContainer,
        "__temporal_transfer_type_converter",
    )
    user_converters = temporalio.nexus.system._SystemNexusUserConverters(
        payload_converter,
        temporalio.converter.FailureConverter.default,
    )

    with temporalio.nexus.system._user_converter_context(user_converters):
        model = converter.from_transfer_type(
            proto,
            proto_backed_showcase_models.FailureContainer,
        )
        round_tripped = converter.to_transfer_type(model)

    converted_failure = model.failure
    assert isinstance(converted_failure, temporalio.exceptions.ApplicationError)
    assert converted_failure.message == "decoded failure"
    assert converted_failure.type == "EncodedFailure"
    assert converted_failure.non_retryable
    assert round_tripped.failure.message == "decoded failure"
    assert round_tripped.failure.application_failure_info.type == "EncodedFailure"

    with temporalio.nexus.system._user_converter_context(user_converters):
        absent = converter.from_transfer_type(
            temporalio.api.command.v1.FailWorkflowExecutionCommandAttributes(),
            proto_backed_showcase_models.FailureContainer,
        )
    assert absent.failure is None


async def test_operations_use_real_nexus_client(env: WorkflowEnvironment) -> None:
    task_queue = str(uuid.uuid4())
    service_handler = ProtoBackedShowcaseServiceHandler()

    async with Worker(
        env.client,
        task_queue=task_queue,
        workflows=[ProtoBackedShowcaseCallerWorkflow],
        nexus_service_handlers=[service_handler],
        workflow_runner=UnsandboxedWorkflowRunner(),
    ):
        endpoint = await env.create_nexus_endpoint("temporal-system", task_queue)
        try:
            result = await env.client.execute_workflow(
                ProtoBackedShowcaseCallerWorkflow.run,
                id=str(uuid.uuid4()),
                task_queue=task_queue,
            )
        finally:
            await env.delete_nexus_endpoint(endpoint)

    assert result == (
        TASK_QUEUE,
        7,
        4,
        "outer failure",
        "OuterFailure",
        True,
        "inner failure",
    )
    assert len(service_handler.calls) == 2
    activity_operation, activity_request = service_handler.calls[0]
    assert activity_operation == "ActivityOptionsOperation"
    assert isinstance(activity_request, proto_backed_showcase_models.ActivityOptions)
    failure_operation, failure_request = service_handler.calls[1]
    assert failure_operation == "FailureOperation"
    assert isinstance(failure_request, proto_backed_showcase_models.FailureContainer)
