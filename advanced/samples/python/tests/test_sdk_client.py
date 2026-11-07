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
import temporalio.client
import temporalio.converter
import temporalio.nexus.system
import temporalio.service
import temporalio.workflow
from temporalio.api.common.v1 import Payload
from typing_extensions import assert_type, override

import wit.stream_service.both.workflow as workflow_package
import wit.stream_service.both.workflow._resources.stream as workflow_stream_module
from wit.stream_service.both._resource_call import ResourceCall
from wit.stream_service.both.client import Stream, StreamServiceClientExtension
from wit.stream_service.both.client._poll import PollIterator
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
    Execution,
    Producer,
    ReadRecordsRequest,
    ReadRecordsResponse,
    StreamData,
    StreamItemData,
    StreamReadStart,
    StreamRecord,
    StreamRef,
    _CreateStreamRequestTransferTypeConverter,
)


STREAM_SERVICE = "temporal.api.streamservice.v1.StreamService"
SYSTEM_ENDPOINT = "__temporal_system"
ServiceT = typing.TypeVar("ServiceT")

SYSTEM_CONVERTER = temporalio.nexus.system._SystemNexusPayloadConverter(
    temporalio.converter.PayloadConverter.default,
    temporalio.converter.FailureConverter.default,
)


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
            "DescribeStream": DescribeStreamResponse(last_offset=5),
        },
    )
    stream = await client.create_stream(id=ref.id, body_type=str)
    assert stream.ref == ref
    stored = client.get_stream(ref=ref, body_type=str)
    assert (await stored.describe()).last_offset == 5
    assert [call.operation for call in answers.started] == [
        "CreateStream",
        "DescribeStream",
    ]


def test_factory_types_require_an_explicit_runtime_type() -> None:
    async def check(client: Client, ref: StreamRef) -> None:
        _ = assert_type(client.get_stream(ref=ref), Stream[object])
        _ = assert_type(client.get_stream(ref=ref, body_type=None), Stream[object])
        _ = assert_type(client.get_stream(ref=ref, body_type=Turn), Stream[Turn])
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
                    records=[
                        StreamRecord(
                            item=StreamItemData(StreamData(body=Turn(role="model"))),
                            offset=1,
                        )
                    ]
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

    item = page.records[0].item
    assert isinstance(item, StreamItemData)
    assert item.value.body == {"role": "model"}


async def test_an_operation_starts_through_the_sdk_nexus_client() -> None:
    client, answers = connect(
        Client,
        {
            "CreateStream": CreateStreamResult(ref=StreamRef(id="s1"), created=True),
            "AppendRecords": AppendRecordsResponse(last_offset=7),
            "CloseStream": CloseStreamResponse(),
        },
    )

    for name in ("append_records", "close_stream", "read_records", "describe_stream"):
        assert not hasattr(client, name)
        assert not hasattr(client, f"_stream_service_{name}")

    stream = await client.create_stream(id="s1", body_type=str)
    response = await stream.publish(records=[])
    await stream.close()

    assert stream.ref.id == "s1"
    assert response.last_offset == 7
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
    assert create.result_type is CreateStreamResult
    assert (append.service, append.endpoint, append.operation) == (
        "temporal.api.streamservice.v1.StreamService",
        "__temporal_system",
        "AppendRecords",
    )
    # A client call fills the producer from its source.
    assert append.arg == AppendRecordsRequest(
        ref=stream.ref, records=[], producer=Producer(id="client:sample")
    )
    assert uuid.UUID(append.id)
    assert append.result_type is AppendRecordsResponse
    assert (close.service, close.endpoint, close.operation) == (
        STREAM_SERVICE,
        SYSTEM_ENDPOINT,
        "CloseStream",
    )
    assert close.arg == CloseStreamRequest(
        ref=stream.ref, producer=Producer(id="client:sample")
    )
    assert close.result_type is CloseStreamResponse


@pytest.mark.parametrize(
    ("fields", "ref"),
    [
        ({}, StreamRef(id="s1")),
        (
            {"backend": "redis", "owner": Execution(business_id="wf-1", run_id="r1")},
            StreamRef(
                id="s1",
                backend="redis",
                owner=Execution(business_id="wf-1", run_id="r1"),
            ),
        ),
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
    assert wire.stream_ref.backend == fields.get("backend", "")
    assert wire.stream_ref.HasField("owner") == ("owner" in fields)
    assert wire.stream_ref.owner.business_id == ("wf-1" if fields else "")


def closing_client() -> tuple[Client, Answers]:
    return connect(
        Client,
        {
            "CreateStream": CreateStreamResult(ref=StreamRef(id="s1")),
            "CloseStream": CloseStreamResponse(),
        },
    )


def closes(answers: Answers) -> list[CloseStreamRequest]:
    return [
        typing.cast(CloseStreamRequest, started.arg)
        for started in answers.started
        if started.operation == "CloseStream"
    ]


async def test_a_clean_block_closes_its_stream() -> None:
    client, answers = closing_client()
    call = client.create_stream(id="s1", body_type=Turn)

    _ = assert_type(call, ResourceCall[Stream[Turn]])
    async with call as stream:
        _ = assert_type(stream, Stream[Turn])

    assert closes(answers) == [
        CloseStreamRequest(ref=stream.ref, producer=Producer(id="client:sample"))
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


async def test_a_stored_stream_ref_rebuilds_the_resource() -> None:
    client, answers = connect(
        Client, {"DescribeStream": DescribeStreamResponse(last_offset=7)}
    )
    ref = StreamRef(id="stored-stream")
    response = await client.get_stream(ref=ref, body_type=str).describe()
    assert response.last_offset == 7
    assert len(client.created_clients) == 1
    assert answers.started[0].arg == DescribeStreamRequest(ref=ref)


async def test_a_resource_preserves_its_constructor_client_binding() -> None:
    client, answers = connect(
        Client, {"DescribeStream": DescribeStreamResponse(last_offset=9)}
    )
    nexus_client: temporalio.client.NexusClient[typing.Any] = (
        client.create_nexus_client(STREAM_SERVICE, "stored-endpoint")
    )
    ref = StreamRef(id="stored-stream")
    response = await Stream(nexus_client, ref=ref, created=None).describe()
    assert response.last_offset == 9
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
            "DescribeStream": client_models.DescribeStreamResponse(last_offset=8),
        },
    )
    stream = await client.create_stream(id=ref.id, body_type=str)
    response = await stream.describe()
    assert response.last_offset == 8
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
            "DescribeStream": DescribeStreamResponse(last_offset=7),
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
            records=[StreamRecord[str](offset=1), StreamRecord[str](offset=2)],
            next_offset=3,
        ),
        ReadRecordsResponse[str](records=[], next_offset=3),
    ]
    client, answers = connect(Client, {"ReadRecords": list(pages)})
    stream = client.get_stream(ref=ref, body_type=str)

    read = [page async for page in stream.iter_read(from_offset=1)]

    assert read == pages
    assert [started.arg for started in answers.started] == [
        ReadRecordsRequest(ref=ref, from_offset=1),
        ReadRecordsRequest(ref=ref, from_offset=3),
    ]
    # The iteration uses one Nexus client, and each page is a new operation.
    assert len(client.created_clients) == 1
    assert len({started.id for started in answers.started}) == len(pages)


async def test_a_waiting_read_goes_on_after_an_empty_page_at_the_same_offset() -> None:
    ref = StreamRef(id="s1")
    wait = datetime.timedelta(seconds=30)
    pages = [
        ReadRecordsResponse[str](records=[StreamRecord[str](offset=1)], next_offset=2),
        ReadRecordsResponse[str](records=[], next_offset=2),
        ReadRecordsResponse[str](records=[], done=True),
    ]
    client, answers = connect(Client, {"ReadRecords": list(pages)})
    stream = client.get_stream(ref=ref, body_type=str)

    read = [page async for page in stream.iter_read(from_offset=1, wait=wait)]

    assert read == pages
    assert [started.arg for started in answers.started] == [
        ReadRecordsRequest(ref=ref, from_offset=1, wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=2, wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=2, wait=wait),
    ]


async def test_a_stream_read_from_the_end_stays_at_the_first_next_offset() -> None:
    ref = StreamRef(id="s1")
    wait = datetime.timedelta(seconds=30)
    pages = [
        ReadRecordsResponse[str](records=[], next_offset=5),
        ReadRecordsResponse[str](records=[StreamRecord[str](offset=5)], next_offset=6),
        ReadRecordsResponse[str](records=[], done=True),
    ]
    client, answers = connect(
        Client,
        {"CreateStream": CreateStreamResult(ref=ref), "ReadRecords": list(pages)},
    )
    stream = await client.create_stream(id="s1", body_type=str)

    read = [
        page
        async for page in stream.iter_read(
            start=StreamReadStart.StreamReadStartEnd, wait=wait
        )
    ]

    assert read == pages
    assert [started.arg for started in answers.started[1:]] == [
        ReadRecordsRequest(
            ref=ref, start=StreamReadStart.StreamReadStartEnd, wait=wait
        ),
        ReadRecordsRequest(ref=ref, from_offset=5, wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=6, wait=wait),
    ]
    # The handle reads on the Nexus client of its create.
    assert len(client.created_clients) == 1


async def test_a_done_first_page_ends_the_read() -> None:
    ref = StreamRef(id="s1")
    pages = [
        ReadRecordsResponse[str](
            records=[StreamRecord[str](offset=1)], next_offset=2, done=True
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
                    records=[StreamRecord[str](offset=1)], next_offset=2
                ),
                ReadRecordsResponse[str](records=[], done=True),
            ]
        },
    )
    stream = client.get_stream(ref=ref, body_type=str)

    async for page in stream.iter_read(from_offset=1, wait=wait):
        page.next_offset = 9

    assert [started.arg for started in answers.started] == [
        ReadRecordsRequest(ref=ref, from_offset=1, wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=2, wait=wait),
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
                        StreamRecord[str](offset=10),
                        StreamRecord[str](offset=12),
                    ],
                    next_offset=15,
                ),
                ReadRecordsResponse[str](records=[], next_offset=15),
                ReadRecordsResponse[str](
                    records=[StreamRecord[str](offset=20)], next_offset=21, done=True
                ),
            ]
        },
    )
    stream = client.get_stream(ref=ref, body_type=str)

    read = stream.iter_read_items(from_offset=10, wait=wait)
    seen: list[tuple[int | None, int | None]] = []
    async for record in read:
        seen.append(
            (record.offset, read.next_request and read.next_request.from_offset)
        )

    # After a Record, the read resumes past it. After a page's last Record, it goes on
    # where the page ends.
    assert seen == [(10, 11), (12, 15), (20, None)]
    assert read.next_request is None
    assert [started.arg for started in answers.started] == [
        ReadRecordsRequest(ref=ref, from_offset=10, wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=15, wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=15, wait=wait),
    ]


async def test_a_new_read_resumes_from_the_next_request_of_a_stopped_read() -> None:
    ref = StreamRef(id="s1")
    client, answers = connect(
        Client,
        {
            "ReadRecords": [
                ReadRecordsResponse[str](
                    records=[StreamRecord[str](offset=1), StreamRecord[str](offset=2)],
                    next_offset=3,
                ),
                ReadRecordsResponse[str](
                    records=[StreamRecord[str](offset=2)], next_offset=3, done=True
                ),
            ]
        },
    )
    stream = client.get_stream(ref=ref, body_type=str)

    read = stream.iter_read_items(from_offset=1)
    first = await anext(read)
    saved = read.next_request
    assert first.offset == 1
    assert saved is not None

    resumed = stream.iter_read_items(from_offset=saved.from_offset)

    assert [record.offset async for record in resumed] == [2]
    assert [started.arg for started in answers.started] == [
        ReadRecordsRequest(ref=ref, from_offset=1),
        ReadRecordsRequest(ref=ref, from_offset=2),
    ]


async def test_a_changed_record_does_not_change_the_next_request() -> None:
    ref = StreamRef(id="s1")
    wait = datetime.timedelta(seconds=30)
    client, answers = connect(
        Client,
        {
            "ReadRecords": [
                ReadRecordsResponse[str](
                    records=[StreamRecord[str](offset=1), StreamRecord[str](offset=2)],
                    next_offset=3,
                ),
                ReadRecordsResponse[str](records=[], done=True),
            ]
        },
    )
    stream = client.get_stream(ref=ref, body_type=str)

    read = stream.iter_read_items(from_offset=1, wait=wait)
    next_offsets: list[int | None] = []
    async for record in read:
        record.offset = 9
        next_offsets.append(read.next_request and read.next_request.from_offset)

    assert next_offsets == [2, 3]
    assert [started.arg for started in answers.started] == [
        ReadRecordsRequest(ref=ref, from_offset=1, wait=wait),
        ReadRecordsRequest(ref=ref, from_offset=3, wait=wait),
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
    assert hasattr(workflow_stream_module, "read_records")
    assert not hasattr(workflow_stream_module, "iter_read_records")
    assert not hasattr(workflow_package, "iter_read_records")
    assert not hasattr(workflow_stream_module.Stream, "iter_read")
    assert not hasattr(workflow_stream_module.Stream, "iter_read_items")
    assert hasattr(Stream, "iter_read")
    assert hasattr(Stream, "iter_read_items")


async def test_a_call_and_a_resource_method_pass_their_rpc_options() -> None:
    client, answers = connect(
        Client,
        {
            "CreateStream": CreateStreamResult(ref=StreamRef(id="s1")),
            "DescribeStream": DescribeStreamResponse(last_offset=5),
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
    ref = StreamRef(id="s1")
    record = StreamRecord(
        item=StreamItemData(StreamData(body=Turn(role="model"))), offset=1
    )
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
