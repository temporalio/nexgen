from __future__ import annotations

from pathlib import Path
import uuid

from nexusrpc.handler import StartOperationContext, service_handler, sync_operation
from nexusrpc import Operation
from temporalio import workflow
from temporalio.testing import WorkflowEnvironment
from temporalio.worker import UnsandboxedWorkflowRunner, Worker

APP_ROOT = Path(__file__).resolve().parent
OUTPUT_PATH = APP_ROOT.parent / "wit" / "type_showcase"

import wit.type_showcase as type_showcase
import wit.type_showcase.models as type_showcase_models
import wit.type_showcase.services as type_showcase_services
from wit.type_showcase._resources import User

GET_USER_OPERATION_INFO = type_showcase.__nexus_operation_registry__[
    ("TypeShowcase", "GetUser")
]
GET_USER_OPERATION = GET_USER_OPERATION_INFO.operation
UPDATE_EMAIL_OPERATION_INFO = type_showcase.__nexus_operation_registry__[
    ("TypeShowcase", "UpdateEmail")
]
UPDATE_EMAIL_OPERATION = UPDATE_EMAIL_OPERATION_INFO.operation
RENAME_OPERATION_INFO = type_showcase.__nexus_operation_registry__[
    ("TypeShowcase", "Rename")
]
RENAME_OPERATION = RENAME_OPERATION_INFO.operation
DEACTIVATE_OPERATION_INFO = type_showcase.__nexus_operation_registry__[
    ("TypeShowcase", "Deactivate")
]
DEACTIVATE_OPERATION = DEACTIVATE_OPERATION_INFO.operation


def user_profile() -> type_showcase_models.UserProfile:
    return type_showcase_models.UserProfile(
        capabilities=type_showcase_models.UserCapabilityReadProfile
        | type_showcase_models.UserCapabilityUpdateEmail,
        notification_target=("email", "old@example.com"),
        sync_state=("ok", "synced"),
        address=type_showcase_models.PostalAddress(
            street="1 Main St",
            city="Portland",
            country="US",
            coordinates=(45.5152, -122.6784),
        ),
        metadata={"tier": "enterprise"},
        tags=["admin", "beta"],
    )


def sync_report() -> type_showcase_models.SyncReport:
    return type_showcase_models.SyncReport(
        route=[(45.5152, -122.6784), (47.6062, -122.3321)],
        attempts=[("ok", "synced"), ("err", "timeout")],
        region_status={
            "us-west": ("ok", "healthy"),
            "eu-central": ("err", "degraded"),
        },
    )


def user_resource(
    *,
    email: str,
    display_name: str,
) -> User:
    return User(
        user_id="user-123",
        email=email,
        display_name=display_name,
        status=type_showcase_models.UserStatus.Active,
        profile=user_profile(),
    )


@service_handler(service=type_showcase_services.TypeShowcase)
class TypeShowcaseHandler:
    def __init__(self) -> None:
        self.calls: list[tuple[str, object]] = []

    @sync_operation
    async def get_user(
        self,
        _ctx: StartOperationContext,
        input: type_showcase_models.GetUserRequest,
    ) -> User:
        self.calls.append(("GetUser", input))
        assert input.user_id == "user-123"
        assert input.consistency_token == "read-123"
        return user_resource(email="old@example.com", display_name="Old Name")

    @sync_operation
    async def update_email(
        self,
        _ctx: StartOperationContext,
        input: type_showcase_models.UpdateEmailRequest,
    ) -> User:
        self.calls.append(("UpdateEmail", input))
        assert input.user_id == "user-123"
        assert input.email == "new@example.com"
        return user_resource(email=input.email, display_name="Old Name")

    @sync_operation
    async def rename(
        self,
        _ctx: StartOperationContext,
        input: type_showcase_models.RenameRequest,
    ) -> User:
        self.calls.append(("Rename", input))
        assert input.user_id == "user-123"
        assert input.display_name == "New Name"
        return user_resource(email="new@example.com", display_name=input.display_name)

    @sync_operation
    async def set_profile(
        self,
        _ctx: StartOperationContext,
        input: type_showcase_models.SetProfileRequest,
    ) -> User:
        self.calls.append(("SetProfile", input))
        return user_resource(email="old@example.com", display_name="Old Name")

    @sync_operation
    async def record_sync(
        self,
        _ctx: StartOperationContext,
        input: type_showcase_models.RecordSyncRequest,
    ) -> None:
        self.calls.append(("RecordSync", input))
        assert input.user_id == "user-123"
        assert input.report == sync_report()

    @sync_operation
    async def deactivate(
        self,
        _ctx: StartOperationContext,
        input: type_showcase_models.DeactivateRequest,
    ) -> None:
        self.calls.append(("Deactivate", input))
        assert input.user_id == "user-123"
        assert input.reason == "requested"


@workflow.defn
class TypeShowcaseCallerWorkflow:
    @workflow.run
    async def run(self) -> User:
        user = await type_showcase.get_user(
            user_id="user-123",
            consistency_token="read-123",
        )
        updated_user = await user.update_email("new@example.com")
        renamed_user = await updated_user.rename("New Name")
        await renamed_user.deactivate(reason="requested")
        record_sync_handle = await type_showcase.record_sync(
            user_id="user-123",
            report=sync_report(),
        )
        await record_sync_handle
        return renamed_user


def test_generated_metadata() -> None:
    assert OUTPUT_PATH.exists(), f"expected generated package at {OUTPUT_PATH}"
    registry = type_showcase.__nexus_operation_registry__

    assert isinstance(GET_USER_OPERATION, Operation)
    assert GET_USER_OPERATION.name == "GetUser"
    assert registry[("TypeShowcase", "GetUser")].operation is GET_USER_OPERATION
    assert registry[("TypeShowcase", "GetUser")].serialization_context is None
    assert isinstance(UPDATE_EMAIL_OPERATION, Operation)
    assert UPDATE_EMAIL_OPERATION.name == "UpdateEmail"
    assert registry[("TypeShowcase", "UpdateEmail")].operation is UPDATE_EMAIL_OPERATION
    assert registry[("TypeShowcase", "UpdateEmail")].serialization_context is None
    assert isinstance(RENAME_OPERATION, Operation)
    assert RENAME_OPERATION.name == "Rename"
    assert registry[("TypeShowcase", "Rename")].operation is RENAME_OPERATION
    assert registry[("TypeShowcase", "Rename")].serialization_context is None
    set_profile_operation_info = type_showcase.__nexus_operation_registry__[
        ("TypeShowcase", "SetProfile")
    ]
    set_profile_operation = set_profile_operation_info.operation
    assert isinstance(set_profile_operation, Operation)
    assert set_profile_operation.name == "SetProfile"
    assert registry[("TypeShowcase", "SetProfile")].operation is set_profile_operation
    assert registry[("TypeShowcase", "SetProfile")].serialization_context is None
    assert isinstance(DEACTIVATE_OPERATION, Operation)
    assert DEACTIVATE_OPERATION.name == "Deactivate"
    assert registry[("TypeShowcase", "Deactivate")].operation is DEACTIVATE_OPERATION
    assert registry[("TypeShowcase", "Deactivate")].serialization_context is None
    assert not hasattr(type_showcase, "TypeShowcase")
    assert not hasattr(type_showcase, "User")
    assert not hasattr(type_showcase_models, "DeactivateResponse")
    assert not hasattr(type_showcase_models.GetUserRequest, "to_proto")
    assert type_showcase_models.UserStatus.Active == 0
    assert type_showcase_models.UserCapabilityReadProfile == 1
    assert type_showcase_models.UserCapabilityUpdateEmail == 2


def test_generated_wit_native_models_cover_common_wit_shapes() -> None:
    profile = user_profile()

    assert profile.notification_target == ("email", "old@example.com")
    assert profile.capabilities == (
        type_showcase_models.UserCapabilityReadProfile
        | type_showcase_models.UserCapabilityUpdateEmail
    )
    assert profile.sync_state == ("ok", "synced")
    assert profile.address is not None
    assert profile.address.coordinates == (45.5152, -122.6784)
    assert profile.metadata == {"tier": "enterprise"}
    assert profile.tags == ["admin", "beta"]


async def test_get_user_returns_wit_user_resource_through_real_nexus_client(
    env: WorkflowEnvironment,
) -> None:
    task_queue = str(uuid.uuid4())
    service_handler = TypeShowcaseHandler()

    async with Worker(
        env.client,
        task_queue=task_queue,
        workflows=[TypeShowcaseCallerWorkflow],
        nexus_service_handlers=[service_handler],
        workflow_runner=UnsandboxedWorkflowRunner(),
    ):
        endpoint = await env.create_nexus_endpoint("type-showcase", task_queue)
        try:
            user = await env.client.execute_workflow(
                TypeShowcaseCallerWorkflow.run,
                id=str(uuid.uuid4()),
                task_queue=task_queue,
            )
        finally:
            await env.delete_nexus_endpoint(endpoint)

    assert isinstance(user, User)
    assert user.user_id == "user-123"
    assert user.email == "new@example.com"
    assert user.display_name == "New Name"
    assert user.status is type_showcase_models.UserStatus.Active
    assert user.profile.notification_target == ("email", "old@example.com")
    assert user.profile.sync_state == ("ok", "synced")
    assert (
        user.profile.capabilities & type_showcase_models.UserCapabilityReadProfile
    ) == type_showcase_models.UserCapabilityReadProfile

    assert len(service_handler.calls) == 5
    get_user_operation, get_user_request = service_handler.calls[0]
    assert get_user_operation == "GetUser"
    assert isinstance(get_user_request, type_showcase_models.GetUserRequest)
    assert get_user_request.user_id == "user-123"
    assert get_user_request.consistency_token == "read-123"

    update_operation, update_request = service_handler.calls[1]
    assert update_operation == "UpdateEmail"
    assert isinstance(update_request, type_showcase_models.UpdateEmailRequest)
    assert update_request.user_id == "user-123"
    assert update_request.email == "new@example.com"

    rename_operation, rename_request = service_handler.calls[2]
    assert rename_operation == "Rename"
    assert isinstance(rename_request, type_showcase_models.RenameRequest)
    assert rename_request.user_id == "user-123"
    assert rename_request.display_name == "New Name"

    deactivate_operation, deactivate_request = service_handler.calls[3]
    assert deactivate_operation == "Deactivate"
    assert isinstance(deactivate_request, type_showcase_models.DeactivateRequest)
    assert deactivate_request.user_id == "user-123"
    assert deactivate_request.reason == "requested"

    record_sync_operation, record_sync_request = service_handler.calls[4]
    assert record_sync_operation == "RecordSync"
    assert isinstance(record_sync_request, type_showcase_models.RecordSyncRequest)
    assert record_sync_request.user_id == "user-123"
    assert record_sync_request.report == sync_report()
