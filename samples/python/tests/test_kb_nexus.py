"""Runtime test driving the generated KB Nexus service definition end-to-end.

Unlike ``test_kb.py`` (which round-trips wire fixtures through the data
converter), this exercises the generated ``KnowledgeBaseService`` *service and
operation definitions* over a real Temporal + Nexus endpoint. The caller
workflow uses the Temporal SDK's built-in Nexus client directly — there is no
generated API client — and references the generated operation definitions for
end-to-end type safety.

It is also the end-to-end proof that the generated dataclasses need **no data
converter wiring at all**: the environment below runs on the SDK's default data
converter, which finds each model's ``TransferTypeConverter`` through the class
attribute ``temporalio.converter.transfer_type_convertible`` set.
"""

from __future__ import annotations

import shutil
import typing
import uuid

import nexusrpc
from nexusrpc.handler import StartOperationContext, service_handler, sync_operation
from temporalio import exceptions, workflow
from temporalio.testing import WorkflowEnvironment
from temporalio.worker import UnsandboxedWorkflowRunner, Worker

from kb import (
    Block,
    Category,
    GetCategoryTreeInput,
    GetPageInput,
    KnowledgeBaseService,
    Page,
    PutBlockOutput,
)

from tests.json_converter_helper import converter_for, load_fixture

SUITE = "kb"
ENDPOINT = "knowledge-base"
RAW_GET_PAGE = nexusrpc.Operation[dict[str, typing.Any], Page](
    name="GetPage", input_type=dict, output_type=Page
)


def parse_fixture(model_type: type[typing.Any], name: str) -> typing.Any:
    return converter_for(model_type).from_transfer_type(
        load_fixture(SUITE, name), model_type
    )


@service_handler(service=KnowledgeBaseService)
class KnowledgeBaseServiceHandler:
    def __init__(self) -> None:
        self.calls: list[tuple[str, object]] = []

    @sync_operation
    async def get_page(self, _ctx: StartOperationContext, input: GetPageInput) -> Page:
        self.calls.append(("GetPage", input))
        assert input.page_id == "page-1"
        return typing.cast(Page, parse_fixture(Page, "page.json"))

    @sync_operation
    async def put_block(
        self, _ctx: StartOperationContext, input: Block
    ) -> PutBlockOutput:
        self.calls.append(("PutBlock", input))
        assert input.block_id == "block-1"
        assert input.style is not None
        assert input.style.bold is True
        return typing.cast(
            PutBlockOutput, parse_fixture(PutBlockOutput, "put-block-output.json")
        )

    @sync_operation
    async def get_category_tree(
        self, _ctx: StartOperationContext, input: GetCategoryTreeInput
    ) -> Category:
        self.calls.append(("GetCategoryTree", input))
        assert input.root_id == "root"
        return typing.cast(Category, parse_fixture(Category, "category-tree.json"))


@workflow.defn
class KnowledgeBaseCallerWorkflow:
    @workflow.run
    async def run(self) -> dict[str, typing.Any]:
        client = workflow.create_nexus_client(
            service=KnowledgeBaseService, endpoint=ENDPOINT
        )

        page = await client.execute_operation(
            KnowledgeBaseService.get_page, GetPageInput(page_id="page-1")
        )
        block = page.blocks[0] if page.blocks is not None else None
        if block is None:
            raise RuntimeError("expected page block")

        put_block_output = await client.execute_operation(
            KnowledgeBaseService.put_block, block
        )

        category = await client.execute_operation(
            KnowledgeBaseService.get_category_tree,
            GetCategoryTreeInput(root_id="root"),
        )

        # Deliberately bypass the generated input type on the caller. The server
        # must classify the generated converter's ValidationError as BAD_REQUEST
        # and must do so before dispatching the user handler.
        try:
            _ = await client.execute_operation(RAW_GET_PAGE, {"unexpected": True})
        except exceptions.NexusOperationError as error:
            cause = error.__cause__
            cause_type = getattr(cause, "type", None)
            invalid_input = {
                "cause": type(cause).__name__,
                "message": str(cause),
                "type": getattr(cause_type, "value", cause_type),
            }
        else:
            raise RuntimeError("invalid Nexus input was accepted")

        return {
            "blockId": put_block_output.block_id,
            "categoryChildId": category.children[0].id
            if category.children is not None
            else None,
            "pageId": page.page_id,
            "revision": put_block_output.revision,
            "invalidInput": invalid_input,
        }


async def test_kb_operations_use_real_nexus_client() -> None:
    # No `data_converter=` argument: the default data converter already carries
    # the generated models.
    env = await WorkflowEnvironment.start_local(
        dev_server_existing_path=shutil.which("temporal"),
    )
    task_queue = str(uuid.uuid4())
    handler = KnowledgeBaseServiceHandler()

    try:
        async with Worker(
            env.client,
            task_queue=task_queue,
            workflows=[KnowledgeBaseCallerWorkflow],
            nexus_service_handlers=[handler],
            workflow_runner=UnsandboxedWorkflowRunner(),
        ):
            endpoint = await env.create_nexus_endpoint(ENDPOINT, task_queue)
            try:
                result = await env.client.execute_workflow(
                    KnowledgeBaseCallerWorkflow.run,
                    id=str(uuid.uuid4()),
                    task_queue=task_queue,
                )
            finally:
                await env.delete_nexus_endpoint(endpoint)
    finally:
        await env.shutdown()

    assert result == {
        "blockId": "block-1",
        "categoryChildId": "child",
        "pageId": "page-1",
        "revision": 7,
        "invalidInput": {
            "cause": "HandlerError",
            "message": "Payload converter failed to decode Nexus operation input",
            "type": "BAD_REQUEST",
        },
    }
    assert [operation for operation, _ in handler.calls] == [
        "GetPage",
        "PutBlock",
        "GetCategoryTree",
    ]
