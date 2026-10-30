from __future__ import annotations

from pathlib import Path
import uuid

from nexusrpc import Operation
from nexusrpc.handler import StartOperationContext, service_handler, sync_operation
import pytest
from temporalio import workflow
from temporalio.api.common.v1 import WorkflowExecution
from temporalio.api.workflowservice.v1 import RequestCancelWorkflowExecutionRequest
from temporalio.testing import WorkflowEnvironment
from temporalio.worker import UnsandboxedWorkflowRunner, Worker
import wit.start_workflow as start_workflow
import wit.start_workflow.models as start_workflow_models
import wit.start_workflow.services as start_workflow_services
from wit.start_workflow._resources import StartedWorkflow

APP_ROOT = Path(__file__).resolve().parent
OUTPUT_PATH = APP_ROOT.parent / "wit" / "start_workflow"
TASK_QUEUE = "demo-task-queue"

START_WORKFLOW_OPERATION_INFO = start_workflow.__nexus_operation_registry__[
    ("StartWorkflowService", "StartWorkflow")
]
START_WORKFLOW_OPERATION = START_WORKFLOW_OPERATION_INFO.operation
RESTART_WORKFLOW_OPERATION_INFO = start_workflow.__nexus_operation_registry__[
    ("StartWorkflowService", "RestartWorkflow")
]
RESTART_WORKFLOW_OPERATION = RESTART_WORKFLOW_OPERATION_INFO.operation
CANCEL_WORKFLOW_OPERATION_INFO = start_workflow.__nexus_operation_registry__[
    ("StartWorkflowService", "CancelWorkflow")
]
CANCEL_WORKFLOW_OPERATION = CANCEL_WORKFLOW_OPERATION_INFO.operation


@service_handler(service=start_workflow_services.StartWorkflowService)
class StartWorkflowServiceHandler:
    def __init__(self) -> None:
        self.calls: list[tuple[str, object]] = []

    @sync_operation
    async def start_workflow(
        self,
        _ctx: StartOperationContext,
        input: start_workflow_models.StartWorkflowRequest,
    ) -> start_workflow_models.StartWorkflowResult:
        self.calls.append(("StartWorkflow", input))
        assert input.namespace == "default"
        assert input.workflow_id == "workflow-id"
        assert input.workflow == "ExampleWorkflow"
        assert input.task_queue == TASK_QUEUE

        return start_workflow_models.StartWorkflowResult(run_id="run-123")

    @sync_operation
    async def restart_workflow(
        self,
        _ctx: StartOperationContext,
        input: start_workflow_models.StartWorkflowRequest,
    ) -> start_workflow_models.StartWorkflowResult:
        self.calls.append(("RestartWorkflow", input))
        assert input.namespace == "default"
        assert input.workflow_id == "workflow-id"
        assert input.workflow == "ExampleWorkflow"
        assert input.task_queue == TASK_QUEUE

        return start_workflow_models.StartWorkflowResult(run_id="run-456")

    @sync_operation
    async def cancel_workflow(
        self,
        _ctx: StartOperationContext,
        input: start_workflow_models.CancelWorkflowRequest,
    ) -> start_workflow_models.CancelWorkflowResponse:
        self.calls.append(("CancelWorkflow", input))
        assert input.namespace == "default"
        assert input.workflow_execution.workflow_id == "workflow-id"
        assert input.workflow_execution.run_id == "run-123"
        return start_workflow_models.CancelWorkflowResponse()

    @sync_operation
    async def signal_with_start(
        self,
        _ctx: StartOperationContext,
        _input: start_workflow_models.CancelWorkflowRequest,
    ) -> start_workflow_models.SignalResultResponse:
        return start_workflow_models.SignalResultResponse(started=True)


@workflow.defn
class StartWorkflowCallerWorkflow:
    @workflow.run
    async def run(self) -> tuple[str, str, str | None, str | None]:
        handle = await start_workflow.start_workflow(
            workflow="ExampleWorkflow",
            workflow_id="workflow-id",
            task_queue=TASK_QUEUE,
        )
        await handle.cancel()
        restarted_handle = await handle.restart_workflow(
            workflow="ExampleWorkflow",
            task_queue=TASK_QUEUE,
        )
        try:
            _ = await restarted_handle.get_result()
        except NotImplementedError:
            pass
        else:
            raise AssertionError(
                "started-workflow.get_result should not be implemented"
            )
        return (
            handle.namespace,
            handle.workflow_id,
            handle.run_id,
            restarted_handle.run_id,
        )


def test_generated_metadata() -> None:
    assert OUTPUT_PATH.exists(), f"expected generated package at {OUTPUT_PATH}"
    start_operation = START_WORKFLOW_OPERATION
    restart_operation = RESTART_WORKFLOW_OPERATION
    cancel_operation = CANCEL_WORKFLOW_OPERATION
    registry = start_workflow.__nexus_operation_registry__

    assert isinstance(start_operation, Operation)
    assert start_operation.name == "StartWorkflow"
    assert (
        registry[("StartWorkflowService", "StartWorkflow")].operation is start_operation
    )
    assert (
        registry[("StartWorkflowService", "StartWorkflow")].serialization_context
        is None
    )
    assert isinstance(cancel_operation, Operation)
    assert cancel_operation.name == "CancelWorkflow"
    assert (
        registry[("StartWorkflowService", "CancelWorkflow")].operation
        is cancel_operation
    )
    assert (
        registry[("StartWorkflowService", "CancelWorkflow")].serialization_context
        is None
    )
    assert isinstance(restart_operation, Operation)
    assert restart_operation.name == "RestartWorkflow"
    assert (
        registry[("StartWorkflowService", "RestartWorkflow")].operation
        is restart_operation
    )
    assert (
        registry[("StartWorkflowService", "RestartWorkflow")].serialization_context
        is None
    )
    assert not hasattr(start_workflow, "WorkflowService")
    assert not hasattr(start_workflow, "StartWorkflowExecutionRequest")
    assert not hasattr(start_workflow, "StartedWorkflow")


def test_workflow_execution_serializes() -> None:
    request = start_workflow_models.WorkflowExecution(workflow_id="workflow-id")
    converter = getattr(
        start_workflow_models.WorkflowExecution, "__temporal_transfer_type_converter"
    )
    proto = converter.to_transfer_type(request)

    assert proto.workflow_id == "workflow-id"
    assert proto.run_id == ""


def test_decode_accepts_an_empty_sourced_namespace() -> None:
    converter = getattr(
        start_workflow_models.CancelWorkflowRequest,
        "__temporal_transfer_type_converter",
    )
    request = converter.from_transfer_type(
        RequestCancelWorkflowExecutionRequest(
            workflow_execution=WorkflowExecution(workflow_id="workflow-id"),
        ),
        start_workflow_models.CancelWorkflowRequest,
    )

    assert request.namespace == ""
    assert request.workflow_execution.workflow_id == "workflow-id"


async def test_start_workflow_returns_wrapper_handle(
    env: WorkflowEnvironment,
) -> None:
    task_queue = str(uuid.uuid4())
    service_handler = StartWorkflowServiceHandler()

    async with Worker(
        env.client,
        task_queue=task_queue,
        workflows=[StartWorkflowCallerWorkflow],
        nexus_service_handlers=[service_handler],
        workflow_runner=UnsandboxedWorkflowRunner(),
    ):
        endpoint = await env.create_nexus_endpoint("temporal-system", task_queue)
        try:
            result = await env.client.execute_workflow(
                StartWorkflowCallerWorkflow.run,
                id=str(uuid.uuid4()),
                task_queue=task_queue,
            )
        finally:
            await env.delete_nexus_endpoint(endpoint)

    assert result == ("default", "workflow-id", "run-123", "run-456")
    assert len(service_handler.calls) == 3
    start_operation, start_request = service_handler.calls[0]
    assert start_operation == "StartWorkflow"
    assert isinstance(start_request, start_workflow_models.StartWorkflowRequest)
    cancel_operation, cancel_request = service_handler.calls[1]
    assert cancel_operation == "CancelWorkflow"
    assert isinstance(cancel_request, start_workflow_models.CancelWorkflowRequest)
    restart_operation, restart_request = service_handler.calls[2]
    assert restart_operation == "RestartWorkflow"
    assert isinstance(restart_request, start_workflow_models.StartWorkflowRequest)

    result_error: pytest.ExceptionInfo[NotImplementedError]
    with pytest.raises(NotImplementedError) as result_error:
        _ = await StartedWorkflow(
            namespace="default",
            workflow_id="workflow-id",
            run_id="run-456",
        ).get_result()
    assert (
        result_error.value.args[0]
        == "started-workflow.get_result is not yet implemented"
    )
