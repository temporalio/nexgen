"""A real ``temporalio.client.Client`` that inherits the generated client extensions.

basedpyright checks that ``Client.create_nexus_client`` fits the generated hook. An
interceptor answers each Nexus call in the process, so the test needs no server.
"""

from __future__ import annotations

import dataclasses
import collections.abc
import typing
import uuid

import pytest
import temporalio.client
import temporalio.converter
import temporalio.service
import temporalio.workflow
from typing_extensions import override

from wit.stream_service.both.client import Stream, StreamServiceClientExtension
from wit.stream_service.both.workflow import _resources as workflow_resources
from wit.stream_service.client.client import (
    StreamServiceClientExtension as ClientOnlyExtension,
)
from wit.stream_service.client import models as client_models
from wit.stream_service.both.models import (
    AppendRecordsRequest,
    AppendRecordsResponse,
    CloseStreamRequest,
    CloseStreamResponse,
    CreateStreamRequest,
    CreateStreamResult,
    DescribeStreamRequest,
    DescribeStreamResponse,
    StreamRef,
)


STREAM_SERVICE = "temporal.api.streamservice.v1.StreamService"
SYSTEM_ENDPOINT = "__temporal_system"
ServiceT = typing.TypeVar("ServiceT")


class ClientBase(temporalio.client.Client):
    def __init__(
        self,
        service_client: temporalio.service.ServiceClient,
        *,
        interceptors: collections.abc.Sequence[temporalio.client.Interceptor],
    ) -> None:
        super().__init__(service_client, interceptors=interceptors)
        self.created_clients: list[temporalio.client.NexusClient[typing.Any]] = []

    @override
    def create_nexus_client(
        self, service: type[ServiceT] | str, endpoint: str
    ) -> temporalio.client.NexusClient[ServiceT]:
        client = super().create_nexus_client(service, endpoint)
        self.created_clients.append(client)
        return client


class Client(ClientBase, StreamServiceClientExtension):
    pass


class ClientOnly(ClientBase, ClientOnlyExtension):
    pass


class SdkClient(temporalio.client.Client, StreamServiceClientExtension):
    pass


class Answers(temporalio.client.Interceptor):
    def __init__(self, answers: dict[str, object]) -> None:
        super().__init__()
        self.answers: dict[str, object] = answers
        self.started: list[temporalio.client.StartNexusOperationInput] = []
        self.client: temporalio.client.Client | None = None

    @override
    def intercept_client(
        self, next: temporalio.client.OutboundInterceptor
    ) -> temporalio.client.OutboundInterceptor:
        return _AnswersOutbound(next, self)


class _AnswersOutbound(temporalio.client.OutboundInterceptor):
    def __init__(
        self, next: temporalio.client.OutboundInterceptor, answers: Answers
    ) -> None:
        super().__init__(next)
        self._answers: Answers = answers

    @override
    async def start_nexus_operation(
        self, input: temporalio.client.StartNexusOperationInput
    ) -> temporalio.client.NexusOperationHandle[typing.Any]:
        self._answers.started.append(input)
        assert self._answers.client is not None
        return temporalio.client.NexusOperationHandle(
            self._answers.client,
            input.id,
            result_type=input.result_type,
            endpoint=input.endpoint,
            service=input.service,
            operation=input.operation,
        )

    @override
    async def get_nexus_operation_result(
        self, input: temporalio.client.GetNexusOperationResultInput
    ) -> typing.Any:
        answer = self._answers.answers[input.operation]
        if input.result_type is None:
            return answer
        assert self._answers.client is not None
        converter = self._answers.client.data_converter
        payloads = await converter.encode([answer])
        return (await converter.decode(payloads, [input.result_type]))[0]


ClientT = typing.TypeVar("ClientT", bound=temporalio.client.Client)


def connect(
    client_type: type[ClientT], answers: dict[str, object]
) -> tuple[ClientT, Answers]:
    interceptor = Answers(answers)
    # The interceptor answers each call before the client uses its service client.
    client = client_type(
        typing.cast(temporalio.service.ServiceClient, object()),
        interceptors=[interceptor],
    )
    interceptor.client = client
    return client, interceptor


async def test_sdk_factory_needs_no_adapter() -> None:
    ref = StreamRef(id="sdk-stream")
    client, answers = connect(
        SdkClient,
        {
            "CreateStream": CreateStreamResult(stream=ref, created=True),
            "DescribeStream": DescribeStreamResponse(last_offset=5),
        },
    )
    stream = await client.create_stream(id="sdk-stream")
    assert stream.stream == ref
    stored = client.get_stream(stream=ref, created=None)
    assert (await stored.describe()).last_offset == 5
    assert [call.operation for call in answers.started] == [
        "CreateStream",
        "DescribeStream",
    ]


async def test_an_operation_starts_through_the_sdk_nexus_client() -> None:
    client, answers = connect(
        Client,
        {
            "CreateStream": CreateStreamResult(stream=StreamRef(id="s1"), created=True),
            "AppendRecords": AppendRecordsResponse(last_offset=7),
            "CloseStream": CloseStreamResponse(),
        },
    )

    for name in ("append_records", "close_stream", "read_records", "describe_stream"):
        assert not hasattr(client, name)
        assert not hasattr(client, f"_stream_service_{name}")

    stream = await client.create_stream(id="s1")
    response = await stream.publish(records=[])
    await stream.close()

    assert stream.stream.id == "s1"
    assert response.last_offset == 7
    assert len(client.created_clients) == 1
    create, append, close = answers.started
    assert (create.service, create.endpoint, create.operation) == (
        "temporal.api.streamservice.v1.StreamService",
        "__temporal_system",
        "CreateStream",
    )
    assert create.arg == CreateStreamRequest(id="s1")
    assert uuid.UUID(create.id)
    assert create.result_type is CreateStreamResult
    assert (append.service, append.endpoint, append.operation) == (
        "temporal.api.streamservice.v1.StreamService",
        "__temporal_system",
        "AppendRecords",
    )
    assert append.arg == AppendRecordsRequest(stream=stream.stream, records=[])
    assert uuid.UUID(append.id)
    assert append.result_type is AppendRecordsResponse
    assert (close.service, close.endpoint, close.operation) == (
        STREAM_SERVICE,
        SYSTEM_ENDPOINT,
        "CloseStream",
    )
    assert close.arg == CloseStreamRequest(stream=stream.stream)
    assert close.result_type is CloseStreamResponse


async def test_a_stored_stream_ref_rebuilds_the_handle() -> None:
    client, answers = connect(
        Client, {"DescribeStream": DescribeStreamResponse(last_offset=7)}
    )
    ref = StreamRef(id="stored-stream")
    response = await client.get_stream(stream=ref, created=None).describe()
    assert response.last_offset == 7
    assert len(client.created_clients) == 1
    assert answers.started[0].arg == DescribeStreamRequest(stream=ref)


async def test_a_resource_preserves_its_constructor_client_binding() -> None:
    client, answers = connect(
        Client, {"DescribeStream": DescribeStreamResponse(last_offset=9)}
    )
    nexus_client: temporalio.client.NexusClient[typing.Any] = (
        client.create_nexus_client(STREAM_SERVICE, "stored-endpoint")
    )
    ref = StreamRef(id="stored-stream")
    response = await Stream(nexus_client, stream=ref, created=None).describe()
    assert response.last_offset == 9
    assert client.created_clients == [nexus_client]
    assert answers.started[0].endpoint == "stored-endpoint"
    assert answers.started[0].service == STREAM_SERVICE
    assert answers.started[0].arg == DescribeStreamRequest(stream=ref)


async def test_client_only_calls_need_no_workflow_context(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def forbidden(**kwargs: typing.Any) -> typing.NoReturn:
        del kwargs
        raise AssertionError("client calls must not use Workflow APIs")

    monkeypatch.setattr(temporalio.workflow, "create_nexus_client", forbidden)
    ref = client_models.StreamRef(id="client-stream")
    client, answers = connect(
        ClientOnly,
        {
            "CreateStream": client_models.CreateStreamResult(stream=ref, created=True),
            "DescribeStream": client_models.DescribeStreamResponse(last_offset=8),
        },
    )
    stream = await client.create_stream(id="client-stream")
    response = await stream.describe()
    assert response.last_offset == 8
    assert len(client.created_clients) == 1
    create, describe = answers.started
    assert create.arg == client_models.CreateStreamRequest(id="client-stream")
    assert describe.arg == client_models.DescribeStreamRequest(stream=ref)
    assert describe.result_type is client_models.DescribeStreamResponse


async def test_each_call_gets_a_new_operation_id() -> None:
    client, answers = connect(
        Client,
        {
            "CreateStream": CreateStreamResult(stream=StreamRef(id="s1"), created=True),
            "DescribeStream": DescribeStreamResponse(last_offset=7),
        },
    )
    stream = await client.create_stream(id="s1")
    _ = await stream.describe()
    _ = await stream.describe()
    ids = [started.id for started in answers.started]
    assert len(ids) == 3
    assert len(set(ids)) == 3
    assert all(str(uuid.UUID(id)) == id for id in ids)


def test_a_resource_requires_a_nexus_client() -> None:
    with pytest.raises(TypeError):
        _ = Stream(stream=StreamRef(id="s1"), created=None)  # pyright: ignore[reportCallIssue]


def test_resource_serialization_carries_only_identity_fields() -> None:
    client, _ = connect(Client, {})
    ref = StreamRef(id="stored-stream")
    stream = client.get_stream(stream=ref, created=None)
    converter = temporalio.converter.PayloadConverter.default
    decoded = converter.from_payload(
        converter.to_payload(stream), workflow_resources.Stream
    )
    assert decoded == workflow_resources.Stream(stream=ref, created=None)
    assert dataclasses.asdict(stream) == {
        "stream": dataclasses.asdict(ref),
        "created": None,
    }
