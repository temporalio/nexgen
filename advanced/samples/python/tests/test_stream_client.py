"""The client target of the Stream service, on its System Nexus endpoint."""

from __future__ import annotations

import typing

from typing_extensions import override

from wit.stream_service.both.client import Stream, StreamServiceClientExtension
from wit.stream_service.both.models import (
    AppendRecordsRequest,
    AppendRecordsResponse,
    CreateStreamRequest,
    CreateStreamResult,
    DescribeStreamRequest,
    DescribeStreamResponse,
    StreamRef,
)

STREAM_SERVICE = "temporal.api.streamservice.v1.StreamService"
SYSTEM_ENDPOINT = "__temporal_system"


class Handle:
    def __init__(self, answer: object) -> None:
        self._answer: object = answer

    async def result(self) -> typing.Any:
        return self._answer


class NexusClient:
    def __init__(self, client: Client, service: str, endpoint: str) -> None:
        self._client: Client = client
        self._service: str = service
        self._endpoint: str = endpoint

    async def start_operation(
        self,
        operation: str,
        arg: typing.Any,
        *,
        id: str,
        result_type: type[typing.Any] | None = None,
    ) -> Handle:
        del id, result_type
        self._client.calls.append((self._service, self._endpoint, operation, arg))
        return Handle(self._client.answers[operation])


class Client(StreamServiceClientExtension):
    def __init__(self, answers: dict[str, object]) -> None:
        self.answers: dict[str, object] = answers
        self.calls: list[tuple[str, str, str, object]] = []

    @override
    def _create_nexus_client(self, *, service: str, endpoint: str) -> NexusClient:
        return NexusClient(self, service, endpoint)


async def test_a_created_stream_publishes_on_the_system_endpoint() -> None:
    ref = StreamRef(id="s1")
    client = Client(
        {
            "CreateStream": CreateStreamResult(stream=ref, created=True),
            "AppendRecords": AppendRecordsResponse(first_offset=0, last_offset=0),
        }
    )

    stream = await client.create_stream(id="s1")
    response = await stream.publish(records=[])

    assert isinstance(stream, Stream)
    assert (stream.stream, stream.created) == (ref, True)
    assert response.last_offset == 0
    assert client.calls == [
        (STREAM_SERVICE, SYSTEM_ENDPOINT, "CreateStream", CreateStreamRequest(id="s1")),
        (
            STREAM_SERVICE,
            SYSTEM_ENDPOINT,
            "AppendRecords",
            AppendRecordsRequest(stream=ref, records=[]),
        ),
    ]


async def test_a_stored_stream_ref_rebuilds_the_handle() -> None:
    ref = StreamRef(id="s2")
    client = Client({"DescribeStream": DescribeStreamResponse(last_offset=7)})

    response = await client.get_stream(stream=ref, created=None).describe()

    assert response.last_offset == 7
    assert client.calls == [
        (
            STREAM_SERVICE,
            SYSTEM_ENDPOINT,
            "DescribeStream",
            DescribeStreamRequest(stream=ref),
        )
    ]
