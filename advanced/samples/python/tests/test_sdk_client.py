"""A real ``temporalio.client.Client`` that inherits the generated client extensions.

basedpyright checks that ``Client.create_nexus_client`` fits the generated hook. An
interceptor answers each Nexus call in the process, so the test needs no server.
"""

from __future__ import annotations

import asyncio
import collections.abc
import dataclasses
import datetime
import typing
import uuid

import pytest
import temporal_support
import temporalio.activity
import temporalio.client
import temporalio.converter
import temporalio.nexus.system
import temporalio.service
import temporalio.workflow
from temporalio.api.common.v1 import Payload
from temporalio.api.stream.v1 import message_pb2
from typing_extensions import assert_type, override

import wit.stream_service.both.models as both_models
import wit.stream_service.both.workflow as workflow_package
import wit.stream_service.both.workflow._resources.stream as workflow_stream_module
from wit.stream_service.both._resource_context_manager import ResourceContextManager
from wit.stream_service.both.client import Stream, StreamServiceClientExtension
from wit.stream_service.both.client._poll import PollIterator
from wit.stream_service.both.workflow import _resources as workflow_resources
from wit.stream_service.client.client import (
    StreamServiceClientExtension as ClientOnlyExtension,
)
from wit.stream_service.client import models as client_models
from wit.stream_service.both.models import (
    ActivityEpoch,
    ActivityProducer,
    AppendRecord,
    AppendRecordsRequest,
    AppendRecordsResponse,
    ClientProducer,
    CloseStreamRequest,
    CloseStreamResponse,
    CreateStreamRequest,
    CreateStreamResult,
    DescribeStreamRequest,
    DescribeStreamResponse,
    Producer,
    ProducerKindClient,
    ProducerKindWorkflowActivity,
    ReadRecordsRequest,
    ReadRecordsResponse,
    StreamRecord,
    StreamRef,
    Timestamp,
    _CreateStreamRequestTransferTypeConverter,
)


STREAM_SERVICE = "temporal.api.streamservice.v1.StreamService"
SYSTEM_ENDPOINT = "__temporal_system"
ServiceT = typing.TypeVar("ServiceT")

SYSTEM_CONVERTER = temporalio.nexus.system._SystemNexusPayloadConverter(
    temporalio.converter.PayloadConverter.default,
    temporalio.converter.FailureConverter.default,
)


# The Producer that the sample's `default_producer()` source gives outside an Activity.
CLIENT_PRODUCER = Producer(
    id="client:sample",
    kind=ProducerKindClient(ClientProducer()),
)


def _offset(n: int) -> bytes:
    # Big-endian, so a bytes compare keeps the int order these tests assert.
    return n.to_bytes(8, "big")


def _record(n: int) -> StreamRecord[str]:
    return StreamRecord[str](body=f"r{n}", offset=_offset(n))


@dataclasses.dataclass
class Turn:
    role: str


@pytest.fixture(autouse=True)
def user_payload_converter(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        temporalio.nexus.system,
        "_current_user_payload_converter",
        lambda: temporalio.converter.PayloadConverter.default,
    )


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
        if isinstance(answer, list):
            # A list answers each call in turn.
            answer = typing.cast(list[object], answer).pop(0)
        # A payload answer decodes with the result type, as the System Nexus converter does.
        if isinstance(answer, Payload) and input.result_type is not None:
            return SYSTEM_CONVERTER.from_payload(answer, input.result_type)
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
            "CreateStream": CreateStreamResult(ref=ref, created=True),
            "DescribeStream": DescribeStreamResponse(last_offset=_offset(5)),
        },
    )
    stream = await client.create_stream(id=ref.id, body_type=str)
    assert stream.ref == ref
    stored = client.get_stream(ref=ref, body_type=str)
    assert (await stored.describe()).last_offset == _offset(5)
    assert [call.operation for call in answers.started] == [
        "CreateStream",
        "DescribeStream",
    ]


def test_factory_types_require_an_explicit_runtime_type() -> None:
    async def check(client: Client, ref: StreamRef) -> None:
        _ = assert_type(client.get_stream(ref=ref), Stream[object])
        _ = assert_type(client.get_stream(ref=ref, body_type=None), Stream[object])
        _ = assert_type(client.get_stream(ref=ref, body_type=Turn), Stream[Turn])
        typed = StreamRef[Turn](id=ref.id)
        _ = assert_type(client.get_stream(typed), Stream[Turn])
        _ = assert_type(client.get_stream(ref=typed), Stream[Turn])
        _ = assert_type(client.get_stream(ref=typed, body_type=None), Stream[Turn])
        _ = assert_type(client.get_stream(ref=typed, body_type=Turn), Stream[Turn])
        _ = assert_type(
            client.get_stream(ref=StreamRef[object](id=ref.id), body_type=str),
            Stream[str],
        )
        mismatched: Stream[str] = client.get_stream(ref=typed)  # pyright: ignore[reportAssignmentType]
        _ = mismatched
        _ = assert_type(await client.create_stream(id=ref.id), Stream[object])
        _ = assert_type(
            await client.create_stream(id=ref.id, body_type=None),
            Stream[object],
        )
        _ = assert_type(
            await client.create_stream(id=ref.id, body_type=Turn), Stream[Turn]
        )
        stored: Stream[Turn] = client.get_stream(ref=ref)  # pyright: ignore[reportAssignmentType]
        created: Stream[Turn] = await client.create_stream(id=ref.id)  # pyright: ignore[reportAssignmentType]
        _ = stored, created

        _ = assert_type(
            await workflow_package.create_stream(id=ref.id),
            workflow_stream_module.Stream[object],
        )
        _ = assert_type(
            await workflow_package.create_stream(id=ref.id, body_type=Turn),
            workflow_stream_module.Stream[Turn],
        )
        workflow: workflow_stream_module.Stream[
            Turn
        ] = await workflow_package.create_stream(id=ref.id)  # pyright: ignore[reportAssignmentType]
        _ = workflow

    # basedpyright checks the assertions and requires each ignored diagnostic to exist.
    _ = check


@pytest.mark.parametrize("construction", ["create", "get"])
async def test_an_untyped_factory_retains_untyped_decoding(construction: str) -> None:
    ref = StreamRef(id="untyped-stream")
    client, _ = connect(
        Client,
        {
            "CreateStream": CreateStreamResult(ref=ref),
            "ReadRecords": SYSTEM_CONVERTER.to_payload(
                ReadRecordsResponse[Turn](
                    records=[StreamRecord(body=Turn(role="model"), offset=_offset(1))],
                    done=False,
                )
            ),
        },
    )
    if construction == "create":
        stream = await client.create_stream(id=ref.id)
    else:
        stream = client.get_stream(ref=ref)
    _ = assert_type(stream, Stream[object])

    page = await stream.read()

    assert page.records[0].body == {"role": "model"}


async def test_an_operation_starts_through_the_sdk_nexus_client() -> None:
    client, answers = connect(
        Client,
        {
            "CreateStream": CreateStreamResult(ref=StreamRef(id="s1"), created=True),
            "AppendRecords": AppendRecordsResponse(
                first_offset=_offset(7), last_offset=_offset(7)
            ),
            "CloseStream": CloseStreamResponse(),
        },
    )

    for name in ("append_records", "close_stream", "read_records", "describe_stream"):
        assert not hasattr(client, name)
        assert not hasattr(client, f"_stream_service_{name}")

    stream = await client.create_stream(id="s1", body_type=str)
    response = await stream.append("a")
    await stream.close()

    assert stream.ref.id == "s1"
    assert response == AppendRecordsResponse(
        first_offset=_offset(7), last_offset=_offset(7)
    )
    assert len(client.created_clients) == 1
    create, append, close = answers.started
    assert (create.service, create.endpoint, create.operation) == (
        "temporal.api.streamservice.v1.StreamService",
        "__temporal_system",
        "CreateStream",
    )
    # A client create fills a new request id from its source.
    assert create.arg == CreateStreamRequest(
        stream_ref=StreamRef(id="s1"), request_id=create.arg.request_id
    )
    assert uuid.UUID(create.arg.request_id)
    assert uuid.UUID(create.id)
    assert create.result_type == CreateStreamResult[str]
    assert (append.service, append.endpoint, append.operation) == (
        "temporal.api.streamservice.v1.StreamService",
        "__temporal_system",
        "AppendRecords",
    )
    # A client call fills the producer from its source.
    assert append.arg == AppendRecordsRequest(
        ref=stream.ref, records=[AppendRecord(body="a")], producer=CLIENT_PRODUCER
    )
    assert uuid.UUID(append.id)
    assert append.result_type is AppendRecordsResponse
    assert (close.service, close.endpoint, close.operation) == (
        STREAM_SERVICE,
        SYSTEM_ENDPOINT,
        "CloseStream",
    )
    assert close.arg == CloseStreamRequest(ref=stream.ref, producer=CLIENT_PRODUCER)
    assert close.result_type is CloseStreamResponse


def closing_client() -> tuple[Client, Answers]:
    return connect(
        Client,
        {
            "CreateStream": CreateStreamResult(ref=StreamRef(id="s1")),
            "CloseStream": CloseStreamResponse(),
        },
    )


def closes(answers: Answers) -> list[CloseStreamRequest[typing.Any]]:
    return [
        typing.cast(CloseStreamRequest[typing.Any], started.arg)
        for started in answers.started
        if started.operation == "CloseStream"
    ]


async def test_a_clean_block_closes_its_stream() -> None:
    client, answers = closing_client()
    call = client.create_stream(id="s1", body_type=Turn)

    _ = assert_type(call, ResourceContextManager[Stream[Turn]])
    async with call as stream:
        _ = assert_type(stream, Stream[Turn])

    assert closes(answers) == [
        CloseStreamRequest(ref=stream.ref, producer=CLIENT_PRODUCER)
    ]


async def test_a_failed_block_closes_its_stream() -> None:
    client, answers = closing_client()

    with pytest.raises(ValueError, match="boom"):
        async with client.create_stream(id="s1", body_type=Turn):
            raise ValueError("boom")

    assert len(closes(answers)) == 1


async def test_a_canceled_block_closes_its_stream() -> None:
    client, answers = closing_client()

    with pytest.raises(asyncio.CancelledError):
        async with client.create_stream(id="s1", body_type=Turn):
            raise asyncio.CancelledError("stop")

    assert len(closes(answers)) == 1


async def test_an_append_wraps_each_value_and_returns_the_offsets() -> None:
    ref = StreamRef(id="s1")
    client, answers = connect(
        Client,
        {
            "AppendRecords": [
                AppendRecordsResponse(first_offset=_offset(4), last_offset=_offset(5)),
                AppendRecordsResponse(),
                AppendRecordsResponse(first_offset=_offset(6), last_offset=_offset(6)),
            ]
        },
    )
    stream = client.get_stream(ref=ref, body_type=str)

    response = await stream.append("a", "b")
    empty = await stream.append()
    _ = await stream.append("c", sequence=3)

    _ = assert_type(response, AppendRecordsResponse)
    assert response == AppendRecordsResponse(
        first_offset=_offset(4), last_offset=_offset(5)
    )
    assert answers.started[0].arg == AppendRecordsRequest(
        ref=ref,
        records=[AppendRecord(body="a"), AppendRecord(body="b")],
        producer=CLIENT_PRODUCER,
    )
    # An append that wrote no Record has no offsets.
    assert empty == AppendRecordsResponse()
    # A sequence goes on the request, so the Store can dedupe a resent batch.
    assert answers.started[2].arg == AppendRecordsRequest(
        ref=ref,
        records=[AppendRecord(body="c")],
        producer=CLIENT_PRODUCER,
        sequence=3,
    )
    if typing.TYPE_CHECKING:
        # The items take the type argument, so a `Stream[str]` refuses an `int`.
        _ = stream.append(1)  # pyright: ignore[reportArgumentType]


async def test_an_optional_producer_parameter_replaces_its_source(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    evaluated: list[Producer] = []

    def default_producer() -> Producer:
        evaluated.append(CLIENT_PRODUCER)
        return CLIENT_PRODUCER

    monkeypatch.setattr(temporal_support, "default_producer", default_producer)
    given = Producer(id="given", kind=ProducerKindClient(ClientProducer()))
    client, answers = connect(
        Client,
        {
            "AppendRecords": AppendRecordsResponse(),
            "CloseStream": CloseStreamResponse(),
        },
    )
    stream = client.get_stream(ref=StreamRef(id="s1"), body_type=str)

    _ = await stream.append("a", producer=given)
    await stream.close(given)
    # A call that passes a producer never evaluates the source.
    assert evaluated == []

    _ = await stream.append("b")
    await stream.close()

    assert len(evaluated) == 2
    assert [call.arg.producer for call in answers.started] == [
        given,
        given,
        CLIENT_PRODUCER,
        CLIENT_PRODUCER,
    ]


@pytest.mark.parametrize(
    ("fields", "ref"),
    [
        ({}, StreamRef(id="s1")),
        ({"store": "redis"}, StreamRef(id="s1", store="redis")),
    ],
)
async def test_a_create_takes_the_fields_of_the_ref(
    fields: dict[str, typing.Any], ref: StreamRef
) -> None:
    client, answers = connect(Client, {"CreateStream": CreateStreamResult(ref=ref)})

    _ = await client.create_stream(id="s1", **fields)

    request = answers.started[0].arg
    assert request == CreateStreamRequest(stream_ref=ref, request_id=request.request_id)
    wire = _CreateStreamRequestTransferTypeConverter().to_transfer_type(request)
    assert wire.stream_ref.id == "s1"
    assert wire.stream_ref.store == fields.get("store", "")


@dataclasses.dataclass
class FakeActivityInfo:
    workflow_run_id: str
    activity_id: str
    attempt: int
    current_attempt_scheduled_time: datetime.datetime


async def test_a_client_append_sends_the_producer_of_its_activity(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    scheduled = datetime.datetime(
        2026, 1, 2, 3, 4, 5, 500000, tzinfo=datetime.timezone.utc
    )
    monkeypatch.setattr(temporalio.activity, "in_activity", lambda: True)
    monkeypatch.setattr(
        temporalio.activity, "info", lambda: FakeActivityInfo("r1", "7", 2, scheduled)
    )
    client, answers = connect(
        Client, {"AppendRecords": AppendRecordsResponse(last_offset=_offset(1))}
    )
    stream = client.get_stream(ref=StreamRef(id="s1"), body_type=str)

    _ = await stream.append()

    (append,) = answers.started
    epoch = ActivityEpoch(
        attempt_scheduled_time=Timestamp(
            seconds=int(scheduled.timestamp()), nanos=500_000_000
        ),
        attempt=2,
    )
    producer = Producer(
        id="act:r1:7",
        kind=ProducerKindWorkflowActivity(ActivityProducer(run_id="r1", epoch=epoch)),
    )
    assert append.arg == AppendRecordsRequest(
        ref=StreamRef(id="s1"), records=[], producer=producer
    )
    # The epoch is the Activity's attempt, so a retry fences the attempt before it.
    converter = both_models._AppendRecordsRequestTransferTypeConverter[str]()
    wire = converter.to_transfer_type(append.arg)
    assert wire.producer.id == "act:r1:7"
    assert wire.producer.workflow_activity.run_id == "r1"
    assert wire.producer.workflow_activity.epoch.attempt == 2


async def test_a_stored_stream_ref_rebuilds_the_resource() -> None:
    client, answers = connect(
        Client, {"DescribeStream": DescribeStreamResponse(last_offset=_offset(7))}
    )
    ref = StreamRef(id="stored-stream")
    response = await client.get_stream(ref=ref, body_type=str).describe()
    assert response.last_offset == _offset(7)
    assert len(client.created_clients) == 1
    assert answers.started[0].arg == DescribeStreamRequest(ref=ref)


async def test_a_resource_preserves_its_constructor_client_binding() -> None:
    client, answers = connect(
        Client, {"DescribeStream": DescribeStreamResponse(last_offset=_offset(9))}
    )
    nexus_client: temporalio.client.NexusClient[typing.Any] = (
        client.create_nexus_client(STREAM_SERVICE, "stored-endpoint")
    )
    ref = StreamRef(id="stored-stream")
    response = await Stream(nexus_client, ref=ref, created=None).describe()
    assert response.last_offset == _offset(9)
    assert client.created_clients == [nexus_client]
    assert answers.started[0].endpoint == "stored-endpoint"
    assert answers.started[0].service == STREAM_SERVICE
    assert answers.started[0].arg == DescribeStreamRequest(ref=ref)


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
            "CreateStream": client_models.CreateStreamResult(ref=ref, created=True),
            "DescribeStream": client_models.DescribeStreamResponse(
                last_offset=_offset(8)
            ),
        },
    )
    stream = await client.create_stream(id=ref.id, body_type=str)
    response = await stream.describe()
    assert response.last_offset == _offset(8)
    assert len(client.created_clients) == 1
    create, describe = answers.started
    assert create.arg == client_models.CreateStreamRequest(
        stream_ref=ref, request_id=create.arg.request_id
    )
    assert describe.arg == client_models.DescribeStreamRequest(ref=ref)
    assert describe.result_type is client_models.DescribeStreamResponse


async def test_each_call_gets_a_new_operation_id() -> None:
    client, answers = connect(
        Client,
        {
            "CreateStream": CreateStreamResult(ref=StreamRef(id="s1"), created=True),
            "DescribeStream": DescribeStreamResponse(last_offset=_offset(7)),
        },
    )
    stream = await client.create_stream(id="s1", body_type=str)
    _ = await stream.describe()
    _ = await stream.describe()
    ids = [started.id for started in answers.started]
    assert len(ids) == 3
    assert len(set(ids)) == 3
    assert all(str(uuid.UUID(id)) == id for id in ids)


def test_a_resource_requires_a_nexus_client() -> None:
    with pytest.raises(TypeError):
        _ = Stream[str](ref=StreamRef(id="s1"), created=None)  # pyright: ignore[reportCallIssue]


def test_a_resource_is_not_data_but_its_identity_field_is() -> None:
    client, _ = connect(Client, {})
    ref = StreamRef(id="stored-stream")
    stream = client.get_stream(ref=ref, body_type=str)
    converter = temporalio.converter.PayloadConverter.default

    # A handle has calls, so it never crosses the converter in either direction.
    with pytest.raises(TypeError, match="not JSON serializable"):
        _ = converter.to_payload(stream)
    payload = converter.to_payload(stream.ref)
    # A generic alias fails for another reason, so the test decodes to the bare class.
    handle_class: type[workflow_resources.Stream[str]] = workflow_resources.Stream
    with pytest.raises(TypeError, match="Unserializable type"):
        _ = converter.from_payload(payload, handle_class)
    # Code passes the identity field and builds the handle explicitly.
    decoded = converter.from_payload(payload, StreamRef)
    assert decoded == ref
    assert client.get_stream(ref=decoded, body_type=str).ref == stream.ref


async def test_a_read_that_does_not_wait_stops_at_an_empty_page_that_did_not_move() -> (
    None
):
    ref = StreamRef(id="s1")
    pages = [
        ReadRecordsResponse[str](
            records=[
                _record(1),
                _record(2),
            ],
            last_offset=_offset(3),
            done=False,
        ),
        ReadRecordsResponse[str](records=[], last_offset=_offset(3), done=False),
    ]
    client, answers = connect(Client, {"ReadRecords": list(pages)})
    stream = client.get_stream(ref=ref, body_type=str)

    read = [page async for page in stream.iter_read(from_offset=_offset(1))]

    assert read == pages
    assert [started.arg for started in answers.started] == [
        ReadRecordsRequest(ref=ref, from_offset=_offset(1)),
        ReadRecordsRequest(ref=ref, from_offset=_offset(3)),
    ]
    # The iteration uses one Nexus client, and each page is a new operation.
    assert len(client.created_clients) == 1
    assert len({started.id for started in answers.started}) == len(pages)


async def test_a_waiting_read_goes_on_after_an_empty_page_at_the_same_offset() -> None:
    ref = StreamRef(id="s1")
    wait = datetime.timedelta(seconds=30)
    pages = [
        ReadRecordsResponse[str](
            records=[_record(1)], last_offset=_offset(2), done=False
        ),
        ReadRecordsResponse[str](records=[], last_offset=_offset(2), done=False),
        ReadRecordsResponse[str](records=[], done=True),
    ]
    client, answers = connect(Client, {"ReadRecords": list(pages)})
    stream = client.get_stream(ref=ref, body_type=str)

    read = [page async for page in stream.iter_read(from_offset=_offset(1), wait=wait)]

    assert read == pages
    assert [started.arg for started in answers.started] == [
        ReadRecordsRequest(ref=ref, from_offset=_offset(1), wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=_offset(2), wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=_offset(2), wait=wait),
    ]


async def test_a_read_from_the_beginning_resumes_after_its_first_last_offset() -> None:
    ref = StreamRef(id="s1")
    wait = datetime.timedelta(seconds=30)
    pages = [
        ReadRecordsResponse[str](records=[], last_offset=_offset(5), done=False),
        ReadRecordsResponse[str](
            records=[_record(5)], last_offset=_offset(6), done=False
        ),
        ReadRecordsResponse[str](records=[], done=True),
    ]
    client, answers = connect(
        Client,
        {"CreateStream": CreateStreamResult(ref=ref), "ReadRecords": list(pages)},
    )
    stream = await client.create_stream(id="s1", body_type=str)

    read = [page async for page in stream.iter_read(wait=wait)]

    assert read == pages
    assert [started.arg for started in answers.started[1:]] == [
        ReadRecordsRequest(ref=ref, wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=_offset(5), wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=_offset(6), wait=wait),
    ]
    # The handle reads on the Nexus client of its create.
    assert len(client.created_clients) == 1


async def test_a_done_first_page_ends_the_read() -> None:
    ref = StreamRef(id="s1")
    pages = [
        ReadRecordsResponse[str](
            records=[_record(1)],
            last_offset=_offset(2),
            done=True,
        )
    ]
    client, answers = connect(Client, {"ReadRecords": list(pages)})
    stream = client.get_stream(ref=ref, body_type=str)

    read = [page async for page in stream.iter_read()]

    assert read == pages
    assert [started.arg for started in answers.started] == [ReadRecordsRequest(ref=ref)]


async def test_a_changed_page_does_not_change_the_next_request() -> None:
    ref = StreamRef(id="s1")
    wait = datetime.timedelta(seconds=30)
    client, answers = connect(
        Client,
        {
            "ReadRecords": [
                ReadRecordsResponse[str](
                    records=[_record(1)], last_offset=_offset(2), done=False
                ),
                ReadRecordsResponse[str](records=[], done=True),
            ]
        },
    )
    stream = client.get_stream(ref=ref, body_type=str)

    async for page in stream.iter_read(from_offset=_offset(1), wait=wait):
        page.last_offset = _offset(9)

    assert [started.arg for started in answers.started] == [
        ReadRecordsRequest(ref=ref, from_offset=_offset(1), wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=_offset(2), wait=wait),
    ]


async def test_iter_read_items_yields_each_record_and_resumes_after_it() -> None:
    ref = StreamRef(id="s1")
    wait = datetime.timedelta(seconds=30)
    client, answers = connect(
        Client,
        {
            "ReadRecords": [
                ReadRecordsResponse[str](
                    records=[
                        _record(10),
                        _record(12),
                    ],
                    last_offset=_offset(12),
                    done=False,
                ),
                ReadRecordsResponse[str](
                    records=[], last_offset=_offset(12), done=False
                ),
                ReadRecordsResponse[str](
                    records=[_record(20)],
                    last_offset=_offset(20),
                    done=True,
                ),
            ]
        },
    )
    stream = client.get_stream(ref=ref, body_type=str)

    read = stream.iter_read_items(from_offset=_offset(10), wait=wait)
    seen: list[tuple[bytes, bytes | None]] = []
    async for record in read:
        seen.append(
            (record.offset, read.next_request and read.next_request.from_offset)
        )

    # A from-offset is exclusive, so after a Record the read resumes after that Record's own
    # offset. After a page's last Record, it goes on after the page's newest offset.
    assert seen == [
        (_offset(10), _offset(10)),
        (_offset(12), _offset(12)),
        (_offset(20), None),
    ]
    assert read.next_request is None
    assert [started.arg for started in answers.started] == [
        ReadRecordsRequest(ref=ref, from_offset=_offset(10), wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=_offset(12), wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=_offset(12), wait=wait),
    ]


async def test_a_new_read_resumes_from_the_next_request_of_a_stopped_read() -> None:
    ref = StreamRef(id="s1")
    client, answers = connect(
        Client,
        {
            "ReadRecords": [
                ReadRecordsResponse[str](
                    records=[
                        _record(1),
                        _record(2),
                    ],
                    last_offset=_offset(2),
                    done=False,
                ),
                ReadRecordsResponse[str](
                    records=[_record(2)],
                    last_offset=_offset(2),
                    done=True,
                ),
            ]
        },
    )
    stream = client.get_stream(ref=ref, body_type=str)

    read = stream.iter_read_items(from_offset=_offset(1))
    first = await anext(read)
    saved = read.next_request
    assert first.offset == _offset(1)
    assert saved is not None

    resumed = stream.iter_read_items(from_offset=saved.from_offset)

    assert [record.offset async for record in resumed] == [_offset(2)]
    assert [started.arg for started in answers.started] == [
        ReadRecordsRequest(ref=ref, from_offset=_offset(1)),
        ReadRecordsRequest(ref=ref, from_offset=_offset(1)),
    ]


async def test_a_changed_record_does_not_change_the_next_request() -> None:
    ref = StreamRef(id="s1")
    wait = datetime.timedelta(seconds=30)
    client, answers = connect(
        Client,
        {
            "ReadRecords": [
                ReadRecordsResponse[str](
                    records=[
                        _record(1),
                        _record(2),
                    ],
                    last_offset=_offset(2),
                    done=False,
                ),
                ReadRecordsResponse[str](records=[], done=True),
            ]
        },
    )
    stream = client.get_stream(ref=ref, body_type=str)

    read = stream.iter_read_items(from_offset=_offset(1), wait=wait)
    next_offsets: list[bytes | None] = []
    async for record in read:
        record.offset = _offset(9)
        next_offsets.append(read.next_request and read.next_request.from_offset)

    assert next_offsets == [_offset(1), _offset(2)]
    assert [started.arg for started in answers.started] == [
        ReadRecordsRequest(ref=ref, from_offset=_offset(1), wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=_offset(2), wait=wait),
    ]


async def test_without_a_resume_helper_the_next_request_reads_the_page_again() -> None:
    async def pages() -> collections.abc.AsyncGenerator[list[int], None]:
        yield [1, 2]
        yield [3]

    read = PollIterator(
        "first",
        pages(),
        lambda page: page,
        lambda request, _page: "second" if request == "first" else None,
        None,
    )

    seen = [(item, read.next_request) async for item in read]

    assert seen == [(1, "first"), (2, "second"), (3, None)]


def test_only_the_client_target_has_the_read_poll() -> None:
    # `read-records` is api-omitted, so only its private helper stays in the workflow module.
    assert not hasattr(workflow_stream_module, "read_records")
    assert hasattr(workflow_stream_module, "_read_records")
    assert not hasattr(workflow_stream_module, "iter_read_records")
    assert not hasattr(workflow_package, "iter_read_records")
    assert not hasattr(workflow_stream_module.Stream, "iter_read")
    assert not hasattr(workflow_stream_module.Stream, "iter_read_items")
    assert hasattr(Stream, "iter_read")
    assert hasattr(Stream, "iter_read_items")


def test_only_the_client_stream_appends_and_closes() -> None:
    # `append` and `close` are api-omitted for the Workflow target only.
    for name in ("append", "close", "__aenter__", "__aexit__"):
        assert hasattr(Stream, name)
        assert not hasattr(workflow_stream_module.Stream, name)


async def test_a_call_and_a_resource_method_pass_their_rpc_options() -> None:
    client, answers = connect(
        Client,
        {
            "CreateStream": CreateStreamResult(ref=StreamRef(id="s1")),
            "DescribeStream": DescribeStreamResponse(last_offset=_offset(5)),
        },
    )
    timeout = datetime.timedelta(seconds=3)

    stream = await client.create_stream(id="s1", body_type=str, rpc_timeout=timeout)
    _ = await stream.describe(rpc_metadata={"authorization": "token"})

    assert [
        (started.rpc_metadata, started.rpc_timeout) for started in answers.started
    ] == [
        ({}, timeout),
        ({"authorization": "token"}, None),
    ]


@pytest.mark.parametrize("construction", ["create", "get", "direct"])
async def test_a_handle_decodes_its_bodies_with_its_type_parameter(
    construction: str,
) -> None:
    ref = StreamRef[Turn](id="s1")
    record = StreamRecord(body=Turn(role="model"), offset=_offset(1))
    client, _ = connect(
        Client,
        {
            "CreateStream": CreateStreamResult(ref=ref),
            "ReadRecords": SYSTEM_CONVERTER.to_payload(
                ReadRecordsResponse[Turn](records=[record], done=True)
            ),
        },
    )

    if construction == "create":
        stream = await client.create_stream(id=ref.id, body_type=Turn)
    elif construction == "get":
        stream = client.get_stream(ref=ref, body_type=Turn)
    else:
        stream = Stream[Turn](
            client.create_nexus_client(
                service=STREAM_SERVICE, endpoint=SYSTEM_ENDPOINT
            ),
            ref=ref,
            created=None,
        )
    page = await stream.read()

    _ = assert_type(stream, Stream[Turn])
    _ = assert_type(page, ReadRecordsResponse[Turn])
    # A poll decodes with the specialization too.
    assert page.records == [record]
    assert [page.records async for page in stream.iter_read()] == [[record]]
    assert [item async for item in stream.iter_read_items()] == [record]


def turn_pages() -> Payload:
    record = StreamRecord(body=Turn(role="model"), offset=_offset(1))
    return SYSTEM_CONVERTER.to_payload(
        ReadRecordsResponse[Turn](records=[record], done=True)
    )


async def test_a_typed_ref_round_trips_into_a_typed_handle() -> None:
    client, _ = connect(
        Client,
        {
            "CreateStream": CreateStreamResult(ref=StreamRef(id="s1")),
            "ReadRecords": turn_pages(),
        },
    )
    created = await client.create_stream(id="s1", body_type=Turn)

    # A Workflow input crosses the client's data converter, which sends a ref as its wire
    # message. The parameter's annotation is the type that it decodes with.
    payloads = await client.data_converter.encode([created.ref])
    assert payloads == await client.data_converter.encode(
        [message_pb2.StreamRef(id="s1")]
    )
    [decoded] = await client.data_converter.decode(payloads, [StreamRef[Turn]])
    ref: StreamRef[Turn] = decoded
    stream = client.get_stream(ref)

    _ = assert_type(created.ref, StreamRef[Turn])
    _ = assert_type(stream, Stream[Turn])
    assert stream.ref == created.ref
    # The ref of a created handle keeps its type argument too.
    assert client.get_stream(created.ref).body_type is Turn
    [record] = (await stream.read()).records
    assert record == StreamRecord(body=Turn(role="model"), offset=_offset(1))


async def test_a_ref_gives_its_type_argument_to_the_handle() -> None:
    client, _ = connect(Client, {"ReadRecords": turn_pages()})
    typed = StreamRef[Turn](id="s1")
    plain = StreamRef(id="s1")

    typed_stream = client.get_stream(typed)
    plain_stream = client.get_stream(plain)

    _ = assert_type(typed_stream, Stream[Turn])
    _ = assert_type(plain, StreamRef[object])
    _ = assert_type(plain_stream, Stream[object])
    _ = assert_type(client.get_stream(ref=typed), Stream[Turn])
    assert plain_stream.body_type is object
    assert typed_stream.ref is typed
    assert client.get_stream(typed, body_type=Turn).ref is typed
    [typed_record] = (await typed_stream.read()).records
    [plain_record] = (await plain_stream.read()).records
    assert typed_record == StreamRecord(body=Turn(role="model"), offset=_offset(1))
    # An unknown payload stays a decoded dict.
    assert plain_record == StreamRecord(body={"role": "model"}, offset=_offset(1))
    # An explicit body_type overrides a ref with no type argument, positionally or by keyword.
    override = client.get_stream(plain, body_type=Turn)
    _ = assert_type(override, Stream[Turn])
    assert override.body_type is Turn
    assert override.ref is not plain
    assert not typing.get_args(getattr(plain, "__orig_class__", None))
    assert client.get_stream(ref=plain, body_type=Turn).body_type is Turn
    unknown_ref = StreamRef[object](id="s1")
    explicit_unknown = client.get_stream(unknown_ref)
    assert explicit_unknown.body_type is object
    assert explicit_unknown.ref is unknown_ref
    assert (await explicit_unknown.read()).records == (
        await plain_stream.read()
    ).records
    specialized = client.get_stream(unknown_ref, body_type=Turn)
    assert specialized.body_type is Turn
    assert specialized.ref is not unknown_ref
    assert typing.get_args(getattr(unknown_ref, "__orig_class__", None)) == (object,)
    # A mismatch is rejected at runtime too, even when the caller bypasses static checking.
    with pytest.raises(TypeError, match="decoding type must match"):
        _ = client.get_stream(typing.cast(typing.Any, typed), body_type=str)
    # The ref of a typed handle keeps its type argument, which is not part of its data.
    ref = client.get_stream(ref=plain, body_type=Turn).ref
    assert client.get_stream(ref).body_type is Turn
    assert ref == plain
    assert repr(ref) == repr(plain)

    if typing.TYPE_CHECKING:
        _ = client.get_stream(typed, body_type=str)  # pyright: ignore[reportArgumentType, reportCallIssue, reportUnknownVariableType]
        _ = Stream[Turn](
            client.create_nexus_client(
                service=STREAM_SERVICE, endpoint=SYSTEM_ENDPOINT
            ),
            ref=plain,  # pyright: ignore[reportArgumentType]
            created=None,
        )
        # A ref of another type is an error where the handle fixes its own type.
        _ = Stream[str](
            client.create_nexus_client(
                service=STREAM_SERVICE, endpoint=SYSTEM_ENDPOINT
            ),
            ref=StreamRef[Turn](id="s1"),  # pyright: ignore[reportArgumentType]
            created=None,
        )


def test_a_client_stream_subclass_is_rejected() -> None:
    class TurnStream(Stream[Turn]):
        pass

    client, _ = connect(Client, {})
    with pytest.raises(TypeError, match="Generic resource subclasses"):
        _ = TurnStream(
            client.create_nexus_client(
                service=STREAM_SERVICE, endpoint=SYSTEM_ENDPOINT
            ),
            ref=StreamRef[Turn](id="s1"),
            created=None,
        )
