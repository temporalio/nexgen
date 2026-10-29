from __future__ import annotations

from pathlib import Path
import uuid

from nexusrpc import Operation
from nexusrpc.handler import StartOperationContext, service_handler, sync_operation
from temporalio import workflow
from temporalio.testing import WorkflowEnvironment
from temporalio.worker import UnsandboxedWorkflowRunner, Worker

import wit.user_service as user_service
import wit.user_service.models as user_service_models
import wit.user_service.services as user_service_services
from wit.user_service._resources import User

APP_ROOT = Path(__file__).resolve().parent
OUTPUT_PATH = APP_ROOT.parent / "wit" / "user_service"

GET_USER_OPERATION_INFO = user_service.__nexus_operation_registry__[
    ("UserService", "GetUser")
]
GET_USER_OPERATION = GET_USER_OPERATION_INFO.operation
UPDATE_EMAIL_OPERATION_INFO = user_service.__nexus_operation_registry__[
    ("UserService", "UpdateEmail")
]
UPDATE_EMAIL_OPERATION = UPDATE_EMAIL_OPERATION_INFO.operation


def user_resource(
    *,
    email: str,
) -> User:
    return User(
        user_id="user-123",
        email=email,
    )


@service_handler(service=user_service_services.UserService)
class UserServiceHandler:
    def __init__(self) -> None:
        self.calls: list[tuple[str, object]] = []

    @sync_operation
    async def get_user(
        self,
        _ctx: StartOperationContext,
        input: user_service_models.GetUserRequest,
    ) -> User:
        self.calls.append(("GetUser", input))
        assert input.user_id == "user-123"
        return user_resource(email="old@example.com")

    @sync_operation
    async def update_email(
        self,
        _ctx: StartOperationContext,
        input: user_service_models.UpdateEmailRequest,
    ) -> User:
        self.calls.append(("UpdateEmail", input))
        assert input.user_id == "user-123"
        assert input.email == "new@example.com"
        return user_resource(email=input.email)

    @sync_operation
    async def notify(
        self,
        _ctx: StartOperationContext,
        input: user_service_models.NotifyRequest,
    ) -> None:
        self.calls.append(("Notify", input))

    @sync_operation
    async def update_profile(
        self,
        _ctx: StartOperationContext,
        input: user_service_models.UpdateProfileRequest,
    ) -> None:
        self.calls.append(("UpdateProfile", input))


@workflow.defn
class UserServiceCallerWorkflow:
    @workflow.run
    async def run(self) -> User:
        service = user_service.UserServiceClient("user-service")
        user = await service.get_user(user_id="user-123")
        await user.notify(sender="ops")
        return await user.update_email("new@example.com")


def test_generated_metadata() -> None:
    assert OUTPUT_PATH.exists(), f"expected generated package at {OUTPUT_PATH}"
    registry = user_service.__nexus_operation_registry__

    assert isinstance(GET_USER_OPERATION, Operation)
    assert GET_USER_OPERATION.name == "GetUser"
    assert registry[("UserService", "GetUser")].operation is GET_USER_OPERATION
    assert registry[("UserService", "GetUser")].serialization_context is None
    assert isinstance(UPDATE_EMAIL_OPERATION, Operation)
    assert UPDATE_EMAIL_OPERATION.name == "UpdateEmail"
    assert registry[("UserService", "UpdateEmail")].operation is UPDATE_EMAIL_OPERATION
    assert registry[("UserService", "UpdateEmail")].serialization_context is None
    assert hasattr(user_service, "UserServiceClient")
    assert not hasattr(user_service, "get_user")
    assert not hasattr(user_service, "update_email")
    assert not hasattr(user_service, "User")
    assert not hasattr(user_service_models.GetUserRequest, "to_proto")


async def test_get_user_returns_user_resource(env: WorkflowEnvironment) -> None:
    task_queue = str(uuid.uuid4())
    service_handler = UserServiceHandler()

    async with Worker(
        env.client,
        task_queue=task_queue,
        workflows=[UserServiceCallerWorkflow],
        nexus_service_handlers=[service_handler],
        workflow_runner=UnsandboxedWorkflowRunner(),
    ):
        endpoint = await env.create_nexus_endpoint("user-service", task_queue)
        try:
            user = await env.client.execute_workflow(
                UserServiceCallerWorkflow.run,
                id=str(uuid.uuid4()),
                task_queue=task_queue,
            )
        finally:
            await env.delete_nexus_endpoint(endpoint)

    assert isinstance(user, User)
    assert user.user_id == "user-123"
    assert user.email == "new@example.com"

    assert len(service_handler.calls) == 3
    get_user_operation, get_user_request = service_handler.calls[0]
    assert get_user_operation == "GetUser"
    assert isinstance(get_user_request, user_service_models.GetUserRequest)
    assert get_user_request.user_id == "user-123"

    notify_operation, notify_request = service_handler.calls[1]
    assert notify_operation == "Notify"
    assert notify_request == user_service_models.NotifyRequest(
        user_id="user-123", reason=None, channels=[], sender="ops"
    )

    update_operation, update_request = service_handler.calls[2]
    assert update_operation == "UpdateEmail"
    assert isinstance(update_request, user_service_models.UpdateEmailRequest)
    assert update_request.user_id == "user-123"
    assert update_request.email == "new@example.com"


@workflow.defn
class UpdateProfileWorkflow:
    @workflow.run
    async def run(self) -> None:
        service = user_service.UserServiceClient("user-service")
        await (
            await service.update_profile(user_id="user-123", city="Berlin", locale="de")
        )
        await (
            await service.update_profile(
                user_id="user-123", city="Oslo", postal_code="0150"
            )
        )


async def test_update_profile_builds_its_flattened_records(
    env: WorkflowEnvironment,
) -> None:
    task_queue = str(uuid.uuid4())
    service_handler = UserServiceHandler()

    async with Worker(
        env.client,
        task_queue=task_queue,
        workflows=[UpdateProfileWorkflow],
        nexus_service_handlers=[service_handler],
        workflow_runner=UnsandboxedWorkflowRunner(),
    ):
        endpoint = await env.create_nexus_endpoint("user-service", task_queue)
        try:
            await env.client.execute_workflow(
                UpdateProfileWorkflow.run,
                id=str(uuid.uuid4()),
                task_queue=task_queue,
            )
        finally:
            await env.delete_nexus_endpoint(endpoint)

    # The optional record stays unset when none of its parameters is given.
    assert service_handler.calls == [
        (
            "UpdateProfile",
            user_service_models.UpdateProfileRequest(
                user_id="user-123",
                address=user_service_models.Address(city="Berlin"),
                preferences=user_service_models.ContactPreferences(locale="de"),
            ),
        ),
        (
            "UpdateProfile",
            user_service_models.UpdateProfileRequest(
                user_id="user-123",
                address=user_service_models.Address(city="Oslo", postal_code="0150"),
                preferences=None,
            ),
        ),
    ]
