from __future__ import annotations

import asyncio
import collections.abc
import dataclasses
import importlib
import subprocess
import sys
import types
import typing
import uuid

import nexusrpc
import pytest
from temporalio.api.enums.v1 import stream_pb2
from temporalio.api.stream.v1 import message_pb2
from temporalio.api.streamservice.v1 import request_response_pb2
import temporalio.converter
from temporalio.converter import FailureConverter, PayloadConverter
import temporalio.nexus.system
from temporalio.worker import StartNexusOperationInput
import temporalio.workflow
from typing_extensions import assert_type, override

from wit.stream_service.models import models as models_only
from wit.stream_service.workflow import models as workflow_models
from wit.stream_service.client import models as client_models
from wit.stream_service.both import models as both_models
from wit.stream_service import workflow as workflow_only
from wit.stream_service.both import workflow as both_workflow
from wit.stream_service.workflow._resources.stream import Stream as WorkflowStream
from wit.stream_service.workflow._system_nexus_interceptor import (
    _start_system_nexus_operation,
    _SystemNexusWorkflowOutboundInterceptorBase,
    _SystemNexusWorkflowOutboundInterceptorTerminal,
)


def _offset(n: int) -> bytes:
    # Big-endian, so a bytes compare keeps the int order these tests assert.
    return n.to_bytes(8, "big")


@dataclasses.dataclass
class Turn:
    role: str


SYSTEM_CONVERTER = temporalio.nexus.system._SystemNexusPayloadConverter(
    PayloadConverter.default,
    FailureConverter.default,
)


@pytest.fixture(
    params=[models_only, workflow_models, client_models, both_models],
    ids=["models", "workflow", "client", "both"],
)
def models(request: pytest.FixtureRequest) -> types.ModuleType:
    return typing.cast(types.ModuleType, request.param)


@pytest.fixture(autouse=True)
def user_payload_converter(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        temporalio.nexus.system,
        "_current_user_payload_converter",
        lambda: PayloadConverter.default,
    )


def test_a_read_response_round_trips_with_its_records(models: types.ModuleType) -> None:
    converter = models._ReadRecordsResponseTransferTypeConverter[dict[str, str]]()
    model = models.ReadRecordsResponse(
        records=[
            models.StreamRecord(
                body={"text": "hi"},
                producer=models.Producer(
                    id="la:r1:1",
                    kind=models.ProducerKindLocalActivity(
                        models.LocalActivityProducer(
                            run_id="r1",
                            epoch=models.LocalActivityEpoch(
                                task=models.WorkflowTaskEpoch(
                                    started_event_id=4, attempt=1
                                ),
                                attempt=2,
                            ),
                        )
                    ),
                ),
                offset=_offset(1),
            ),
        ],
        last_offset=_offset(1),
        done=True,
    )

    wire = converter.to_transfer_type(model)

    # The wire messages come from `temporalio-stream-protos`, since no temporalio release has them.
    assert isinstance(wire, request_response_pb2.ReadRecordsResponse)
    assert all(isinstance(record, message_pb2.StreamRecord) for record in wire.records)
    assert wire.records[0].producer.WhichOneof("kind") == "local_activity"
    assert converter.from_transfer_type(wire, models.ReadRecordsResponse) == model


def test_a_request_without_its_handle_is_refused(models: types.ModuleType) -> None:
    converter = models._AppendRecordsRequestTransferTypeConverter[object]()
    wire = converter.to_transfer_type(
        models.AppendRecordsRequest(
            ref=models.StreamRef(id="s-1"),
            producer=models.Producer(id="client:sample"),
            records=[],
        )
    )
    assert wire.ref.id == "s-1"

    wire.ClearField("ref")
    with pytest.raises(
        ValueError, match="missing required field AppendRecordsRequest.ref"
    ):
        _ = converter.from_transfer_type(wire, models.AppendRecordsRequest)


def test_an_unknown_enum_number_decodes_as_the_fallback_case(
    models: types.ModuleType,
) -> None:
    converter = models._DescribeStreamResponseTransferTypeConverter()
    # A newer server can send a Stream status that this code doesn't know.
    wire = request_response_pb2.DescribeStreamResponse(
        status=stream_pb2.StreamStatus.ValueType(9)
    )

    decoded = converter.from_transfer_type(wire, models.DescribeStreamResponse)

    assert decoded.status is models.StreamStatus.StreamStatusUnspecified


def test_an_unknown_number_of_an_enum_with_no_fallback_fails_the_message(
    models: types.ModuleType,
) -> None:
    converter = models._CreateStreamRequestTransferTypeConverter()
    wire = request_response_pb2.CreateStreamRequest(
        stream_ref=message_pb2.StreamRef(id="s-1"),
        id_conflict_policy=stream_pb2.StreamIdConflictPolicy.ValueType(9),
    )

    with pytest.raises(ValueError, match="9 is not a valid StreamIdConflictPolicy"):
        _ = converter.from_transfer_type(wire, models.CreateStreamRequest)


@pytest.mark.parametrize(
    ("api", "model_types"),
    [(workflow_only, workflow_models), (both_workflow, both_models)],
    ids=["workflow", "both"],
)
async def test_workflow_apis_call_the_stream_service(
    monkeypatch: pytest.MonkeyPatch,
    api: types.ModuleType,
    model_types: types.ModuleType,
) -> None:
    ref = model_types.StreamRef(id="workflow-stream")
    replies = {
        "CreateStream": model_types.CreateStreamResult(ref=ref, created=True),
        "DescribeStream": model_types.DescribeStreamResponse(last_offset=_offset(3)),
    }
    calls: list[tuple[str, typing.Any]] = []

    class NexusClient:
        async def start_operation(
            self,
            operation: str,
            input: typing.Any,
            *,
            output_type: type[typing.Any] | None = None,
        ) -> collections.abc.Awaitable[typing.Any]:
            del output_type
            calls.append((operation, input))
            future: asyncio.Future[typing.Any] = (
                asyncio.get_running_loop().create_future()
            )
            future.set_result(replies[operation])
            return future

    def create_client(*, service: str, endpoint: str) -> NexusClient:
        assert service == "temporal.api.streamservice.v1.StreamService"
        assert endpoint == "__temporal_system"
        return NexusClient()

    monkeypatch.setattr(temporalio.workflow, "create_nexus_client", create_client)
    monkeypatch.setattr(temporalio.workflow, "in_workflow", lambda: True)
    request_id = uuid.UUID(int=7)
    monkeypatch.setattr(temporalio.workflow, "uuid4", lambda: request_id)
    stream = await api.create_stream(id=ref.id, body_type=str)
    assert await stream.describe() == replies["DescribeStream"]
    # The Workflow API fills the request id from its source.
    expected = [
        (
            "CreateStream",
            model_types.CreateStreamRequest(stream_ref=ref, request_id=str(request_id)),
        ),
        ("DescribeStream", model_types.DescribeStreamRequest(ref=ref)),
    ]
    # The sources build the models of one package, so the calls compare as data.
    assert [(name, dataclasses.asdict(arg)) for name, arg in calls] == [
        (name, dataclasses.asdict(arg)) for name, arg in expected
    ]


@pytest.mark.parametrize("construction", ["create", "direct"])
async def test_workflow_handle_decodes_with_its_type_parameter(
    monkeypatch: pytest.MonkeyPatch, construction: str
) -> None:
    ref = workflow_models.StreamRef[Turn](id="typed-stream")
    record = workflow_models.StreamRecord(body=Turn(role="model"), offset=_offset(1))
    payload = SYSTEM_CONVERTER.to_payload(
        workflow_models.ReadRecordsResponse[Turn](records=[record], done=False)
    )
    inputs: list[typing.Any] = []

    class NexusClient:
        async def start_operation(
            self,
            operation: str,
            input: typing.Any,
            *,
            output_type: type[typing.Any],
        ) -> collections.abc.Awaitable[typing.Any]:
            inputs.append(input)
            future: asyncio.Future[typing.Any] = (
                asyncio.get_running_loop().create_future()
            )
            if operation == "CreateStream":
                future.set_result(workflow_models.CreateStreamResult(ref=ref))
            else:
                future.set_result(SYSTEM_CONVERTER.from_payload(payload, output_type))
            return future

    def create_client(*, service: str, endpoint: str) -> NexusClient:
        del service, endpoint
        return NexusClient()

    monkeypatch.setattr(temporalio.workflow, "create_nexus_client", create_client)
    monkeypatch.setattr(temporalio.workflow, "uuid4", lambda: uuid.UUID(int=7))
    if construction == "create":
        stream = await workflow_only.create_stream(id=ref.id, body_type=Turn)
    else:
        stream = WorkflowStream[Turn](ref=ref, created=None)

    page = await stream.read(from_offset=_offset(1))

    assert page.records == [record]
    assert inputs[-1].from_offset == _offset(1)


def test_a_subclass_of_a_specialized_handle_is_rejected() -> None:
    class TurnStream(WorkflowStream[Turn]):
        pass

    ref = workflow_models.StreamRef[Turn](id="typed-stream")

    with pytest.raises(TypeError, match="Generic resource subclasses"):
        _ = TurnStream(ref=ref, created=None)
    assert (
        WorkflowStream(ref=workflow_models.StreamRef(id="s1"), created=None).body_type
        is object
    )


async def test_a_decoded_ref_builds_a_typed_workflow_handle(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    record = workflow_models.StreamRecord(body=Turn(role="model"), offset=_offset(1))
    payload = SYSTEM_CONVERTER.to_payload(
        workflow_models.ReadRecordsResponse[Turn](records=[record], done=False)
    )

    # A Workflow input crosses the data converter, which sends a ref as its wire message. The
    # parameter's annotation is the type that it decodes with.
    data_converter = temporalio.converter.DataConverter.default
    plain = workflow_models.StreamRef(id="typed-stream")
    payloads = await data_converter.encode([plain])
    assert payloads == await data_converter.encode(
        [message_pb2.StreamRef(id="typed-stream")]
    )
    [decoded] = await data_converter.decode(payloads, [workflow_models.StreamRef[Turn]])
    ref: workflow_models.StreamRef[Turn] = decoded

    class NexusClient:
        async def start_operation(
            self, operation: str, input: typing.Any, *, output_type: type[typing.Any]
        ) -> collections.abc.Awaitable[typing.Any]:
            del operation, input
            future: asyncio.Future[typing.Any] = (
                asyncio.get_running_loop().create_future()
            )
            future.set_result(SYSTEM_CONVERTER.from_payload(payload, output_type))
            return future

    def create_client(*, service: str, endpoint: str) -> NexusClient:
        del service, endpoint
        return NexusClient()

    monkeypatch.setattr(temporalio.workflow, "create_nexus_client", create_client)
    monkeypatch.setattr(temporalio.workflow, "uuid4", lambda: uuid.UUID(int=7))

    stream = WorkflowStream[Turn](ref=ref, created=None)
    page = await stream.read()

    _ = assert_type(stream.ref, workflow_models.StreamRef[Turn])
    assert page.records == [record]
    # The type argument rides in memory, so the ref still equals the plain one.
    assert stream.ref == plain
    assert repr(stream.ref) == repr(plain)


@pytest.mark.parametrize(
    "construction",
    ["typed", "matching", "none", "unknown", "override", "explicit-unknown"],
)
async def test_a_workflow_handle_reads_with_its_refs_payload_type(
    monkeypatch: pytest.MonkeyPatch, construction: str
) -> None:
    payload = SYSTEM_CONVERTER.to_payload(
        workflow_models.ReadRecordsResponse[Turn](
            records=[
                workflow_models.StreamRecord(body=Turn(role="model"), offset=_offset(1))
            ],
            done=False,
        )
    )

    class NexusClient:
        async def start_operation(
            self, operation: str, input: typing.Any, *, output_type: type[typing.Any]
        ) -> collections.abc.Awaitable[typing.Any]:
            assert operation == "ReadRecords"
            assert input.ref.id == "s1"
            future: asyncio.Future[typing.Any] = (
                asyncio.get_running_loop().create_future()
            )
            future.set_result(SYSTEM_CONVERTER.from_payload(payload, output_type))
            return future

    def create_client(*, service: str, endpoint: str) -> NexusClient:
        del service, endpoint
        return NexusClient()

    monkeypatch.setattr(temporalio.workflow, "create_nexus_client", create_client)
    typed = workflow_models.StreamRef[Turn](id="s1")
    unknown = workflow_models.StreamRef(id="s1")
    # The operations are api-omitted, so a read goes through the Stream resource.
    if construction == "typed":
        stream = workflow_only.get_stream(ref=typed)
    elif construction == "matching":
        stream = workflow_only.get_stream(ref=typed, body_type=Turn)
    elif construction == "none":
        stream = workflow_only.get_stream(ref=typed, body_type=None)
    elif construction == "override":
        stream = workflow_only.get_stream(ref=unknown, body_type=Turn)
    elif construction == "explicit-unknown":
        stream = workflow_only.get_stream(
            ref=workflow_models.StreamRef[object](id="s1"), body_type=Turn
        )
    else:
        unknown_stream = workflow_only.get_stream(ref=unknown)
        _ = assert_type(unknown_stream, WorkflowStream[object])
        [record] = (await unknown_stream.read()).records
        assert record == workflow_models.StreamRecord(
            body={"role": "model"}, offset=_offset(1)
        )
        return

    _ = assert_type(stream, WorkflowStream[Turn])
    [record] = (await stream.read()).records
    assert record == workflow_models.StreamRecord(
        body=Turn(role="model"), offset=_offset(1)
    )
    # The combined package reads with the same payload type.
    both_stream = both_workflow.get_stream(ref=both_models.StreamRef[Turn](id="s1"))
    assert (await both_stream.read()).records[0] == both_models.StreamRecord(
        body=Turn(role="model"), offset=_offset(1)
    )
    with pytest.raises(TypeError, match="decoding type must match"):
        _ = workflow_only.get_stream(ref=typing.cast(typing.Any, typed), body_type=str)

    if typing.TYPE_CHECKING:
        _ = workflow_only.get_stream(ref=typed, body_type=str)  # pyright: ignore[reportArgumentType, reportCallIssue, reportUnknownVariableType]


def test_the_ref_of_a_workflow_handle_keeps_its_type_argument() -> None:
    plain = workflow_models.StreamRef(id="typed-stream")
    typed = workflow_models.StreamRef[Turn](id="typed-stream")

    typed_ref = WorkflowStream[Turn](ref=typed, created=None).ref
    unknown_ref = WorkflowStream(ref=plain, created=None).ref
    inferred = WorkflowStream(ref=typed, created=None)

    _ = assert_type(typed_ref, workflow_models.StreamRef[Turn])
    _ = assert_type(inferred, WorkflowStream[Turn])
    assert inferred.body_type is Turn
    _ = assert_type(plain, workflow_models.StreamRef[object])
    _ = assert_type(unknown_ref, workflow_models.StreamRef[object])
    assert typed_ref is typed
    assert unknown_ref is plain
    assert typing.get_args(getattr(typed_ref, "__orig_class__", None)) == (Turn,)
    # A hand-built typed ref keeps its own type argument.
    assert typing.get_args(
        getattr(workflow_models.StreamRef[Turn](id="s1"), "__orig_class__", None)
    ) == (Turn,)

    if typing.TYPE_CHECKING:
        _ = WorkflowStream[Turn](
            ref=plain,  # pyright: ignore[reportArgumentType]
            created=None,
        )


def test_declared_ref_parameters_propagate_through_wire_models() -> None:
    ref = both_models.StreamRef[Turn](id="typed-stream")
    create = both_models.CreateStreamRequest(stream_ref=ref)
    read = both_models.ReadRecordsRequest(ref=ref)
    result = both_models.CreateStreamResult(ref=ref, created=True)
    _ = assert_type(create, both_models.CreateStreamRequest[Turn])
    _ = assert_type(read, both_models.ReadRecordsRequest[Turn])
    _ = assert_type(result, both_models.CreateStreamResult[Turn])

    decoded_create = SYSTEM_CONVERTER.from_payload(
        SYSTEM_CONVERTER.to_payload(create), both_models.CreateStreamRequest[Turn]
    )
    decoded_read = SYSTEM_CONVERTER.from_payload(
        SYSTEM_CONVERTER.to_payload(read), both_models.ReadRecordsRequest[Turn]
    )
    decoded_result = SYSTEM_CONVERTER.from_payload(
        SYSTEM_CONVERTER.to_payload(result), both_models.CreateStreamResult[Turn]
    )
    for decoded_ref in (
        decoded_create.stream_ref,
        decoded_read.ref,
        decoded_result.ref,
    ):
        assert decoded_ref == ref
        assert typing.get_args(getattr(decoded_ref, "__orig_class__", None)) == (Turn,)


async def test_system_nexus_dispatch_decodes_a_read_with_its_type_argument() -> None:
    record = workflow_models.StreamRecord(body=Turn(role="model"), offset=_offset(1))
    payload = SYSTEM_CONVERTER.to_payload(
        workflow_models.ReadRecordsResponse[Turn](records=[record], done=False)
    )
    pages: list[workflow_models.ReadRecordsResponse[typing.Any]] = []

    # Like the SDK's outbound root, the terminal is also the last interceptor.
    class Root(
        _SystemNexusWorkflowOutboundInterceptorTerminal,
        _SystemNexusWorkflowOutboundInterceptorBase,
    ):
        @override
        def _next_system_nexus_interceptor(
            self,
        ) -> _SystemNexusWorkflowOutboundInterceptorBase:
            return self

        @override
        async def start_system_nexus_operation(
            self,
            input: StartNexusOperationInput[typing.Any, typing.Any],
        ) -> temporalio.workflow.NexusOperationHandle[typing.Any]:
            raise AssertionError("a read goes through its typed hook")

        @override
        async def _intercept_system_nexus_operation(
            self,
            input: StartNexusOperationInput[typing.Any, typing.Any],
        ) -> temporalio.workflow.NexusOperationHandle[typing.Any]:
            assert input.output_type is not None
            pages.append(SYSTEM_CONVERTER.from_payload(payload, input.output_type))
            return typing.cast(
                temporalio.workflow.NexusOperationHandle[typing.Any], object()
            )

    class PassThrough(_SystemNexusWorkflowOutboundInterceptorBase):
        def __init__(self, next: _SystemNexusWorkflowOutboundInterceptorBase) -> None:
            self.next: _SystemNexusWorkflowOutboundInterceptorBase = next

        @override
        def _next_system_nexus_interceptor(
            self,
        ) -> _SystemNexusWorkflowOutboundInterceptorBase:
            return self.next

        @override
        async def start_system_nexus_operation(
            self,
            input: StartNexusOperationInput[typing.Any, typing.Any],
        ) -> temporalio.workflow.NexusOperationHandle[typing.Any]:
            return await self.next.start_system_nexus_operation(input)

    _ = await _start_system_nexus_operation(
        PassThrough(Root()),
        StartNexusOperationInput(
            endpoint=temporalio.nexus.system.TEMPORAL_SYSTEM_ENDPOINT,
            service="temporal.api.streamservice.v1.StreamService",
            operation="ReadRecords",
            input=workflow_models.ReadRecordsRequest(
                ref=workflow_models.StreamRef(id="typed-stream")
            ),
            output_type=workflow_models.ReadRecordsResponse[Turn],
            schedule_to_close_timeout=None,
            schedule_to_start_timeout=None,
            start_to_close_timeout=None,
            cancellation_type=temporalio.workflow.NexusOperationCancellationType.WAIT_COMPLETED,
            headers=None,
            summary=None,
        ),
    )

    [page] = pages
    assert page.records == [record]


@pytest.mark.parametrize(
    ("api", "model_types"),
    [(workflow_only, workflow_models), (both_workflow, both_models)],
    ids=["workflow", "both"],
)
def test_a_workflow_factory_builds_a_handle_with_no_call(
    api: types.ModuleType, model_types: types.ModuleType
) -> None:
    ref = model_types.StreamRef(id="turns", store="redis")
    stream = api.get_stream(ref=ref, body_type=str)

    assert stream.ref == ref
    assert stream.created is None
    assert stream.body_type is str


@pytest.mark.parametrize(
    ("api", "model_types"),
    [(workflow_only, workflow_models), (both_workflow, both_models)],
    ids=["workflow", "both"],
)
def test_api_omit_hides_the_public_operation_functions(
    api: types.ModuleType, model_types: types.ModuleType
) -> None:
    hidden = (
        "append_records",
        "close_stream",
        "describe_stream",
        "read_records",
    )
    for name in hidden:
        assert not hasattr(api, name)
        assert name not in api.__all__
    # No resource method creates a Stream, so the create function stays public.
    assert "create_stream" in api.__all__
    # The resource factory and its methods still reach the operations.
    assert "get_stream" in api.__all__
    stream = api.get_stream(ref=model_types.StreamRef(id="s-1"), body_type=str)
    assert all(callable(getattr(stream, method)) for method in ("read", "describe"))
    # Every operation stays registered, so the service still serves all five.
    assert len(api.__nexus_operation_registry__) == 5
    # The private helpers that the resource methods call stay in the resource module.
    resources = importlib.import_module(api.__name__ + "._resources.stream")
    for helper in ("_read_records", "_describe_stream"):
        assert hasattr(resources, helper)
    assert not hasattr(resources, "append_records")


@pytest.mark.parametrize(
    ("api", "model_types"),
    [(workflow_only, workflow_models), (both_workflow, both_models)],
    ids=["workflow", "both"],
)
async def test_a_workflow_stream_can_not_append_or_close(
    monkeypatch: pytest.MonkeyPatch,
    api: types.ModuleType,
    model_types: types.ModuleType,
) -> None:
    ref = model_types.StreamRef(id="s-1")
    stream = api.get_stream(ref=ref, body_type=str)
    # `append` and `close` are api-omitted for the Workflow target, with their helpers.
    for name in ("append", "close", "__aenter__", "__aexit__"):
        assert not hasattr(stream, name)
    resources = importlib.import_module(api.__name__ + "._resources.stream")
    for helper in ("_append_records", "_close_stream"):
        assert not hasattr(resources, helper)

    class NexusClient:
        async def start_operation(
            self,
            operation: str,
            input: typing.Any,
            *,
            output_type: type[typing.Any] | None = None,
        ) -> collections.abc.Awaitable[typing.Any]:
            del operation, input, output_type
            future: asyncio.Future[typing.Any] = (
                asyncio.get_running_loop().create_future()
            )
            future.set_result(model_types.CreateStreamResult(ref=ref))
            return future

    def create_client(*, service: str, endpoint: str) -> NexusClient:
        del service, endpoint
        return NexusClient()

    monkeypatch.setattr(temporalio.workflow, "create_nexus_client", create_client)
    monkeypatch.setattr(temporalio.workflow, "uuid4", lambda: uuid.UUID(int=7))
    # With nothing to close, a create is a plain call and not a context manager.
    call = api.create_stream(id=ref.id, body_type=str)
    assert not hasattr(call, "__aenter__")
    assert (await call).ref == ref

    if typing.TYPE_CHECKING:
        typed = workflow_only.get_stream(ref=workflow_models.StreamRef[str](id="s-1"))
        _ = typed.append  # pyright: ignore[reportAttributeAccessIssue, reportUnknownVariableType]
        _ = typed.close  # pyright: ignore[reportAttributeAccessIssue, reportUnknownVariableType]


@pytest.mark.parametrize(
    ("package", "forbidden"),
    [
        ("wit.stream_service.both.client", "wit.stream_service.both.workflow"),
        ("wit.stream_service.both.workflow", "wit.stream_service.both.client"),
        ("wit.stream_service.client.client", "wit.stream_service.client.workflow"),
    ],
)
def test_api_packages_do_not_import_another_target(
    package: str, forbidden: str
) -> None:
    script = """
import importlib
import importlib.abc
import sys

package, forbidden = sys.argv[1:]

class RejectOtherTarget(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path=None, target=None):
        if fullname == forbidden or fullname.startswith(forbidden + "."):
            raise AssertionError("API imported another target: " + fullname)
        return None

sys.meta_path.insert(0, RejectOtherTarget())
api = importlib.import_module(package)
shared = importlib.import_module(package.rsplit(".", 1)[0])
services = importlib.import_module(shared.__name__ + ".services")
assert shared.StreamService is services.StreamService
resources = importlib.import_module(package + "._resources.stream")
assert resources.Stream
"""
    result = subprocess.run(
        [sys.executable, "-c", script, package, forbidden],
        check=False,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize(
    "package",
    ["wit.stream_service.both.client", "wit.stream_service.client.client"],
)
def test_a_client_package_loads_before_the_sdk_client_module(package: str) -> None:
    # The SDK's `Client` inherits the generated extension, so `temporalio.client` imports this
    # package while it loads. A package that imported it back would be a circular import.
    script = """
import importlib
import sys

importlib.import_module(sys.argv[1])
assert "temporalio.client" not in sys.modules
"""
    result = subprocess.run(
        [sys.executable, "-c", script, package],
        check=False,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize(
    "package",
    [
        "wit.stream_service.models",
        "wit.stream_service.client",
        "wit.stream_service.both",
    ],
)
def test_root_service_contract_uses_shared_wire_models(package: str) -> None:
    shared = importlib.import_module(package)
    definition = nexusrpc.get_service_definition(shared.StreamService)
    assert definition is not None
    operation = definition.operation_definitions["CreateStream"]
    assert operation.input_type == shared.CreateStreamRequest[typing.Any]
    assert operation.output_type == shared.CreateStreamResult[typing.Any]
