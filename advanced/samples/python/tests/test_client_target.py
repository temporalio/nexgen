from __future__ import annotations

import collections.abc
import dataclasses
import types
import typing

import pytest
import temporalio.converter
from typing_extensions import override

import temporal_support
import wit.stream_service._resources as workflow_resources
from wit.stream_service.client import Stream, StreamServiceClientExtension
from streamprotos.temporal.api.stream.v1 import message_pb2
from streamprotos.temporal.api.streamservice.v1 import request_response_pb2
import wit.stream_service.models as stream_models
from wit.stream_service.models import (
    AppendRecord,
    AppendRecordsRequest,
    AppendRecordsResponse,
    CreateStreamRequest,
    CreateStreamResult,
    DescribeStreamRequest,
    DescribeStreamResponse,
    StreamRef,
)

STREAM_SERVICE = "temporal.api.streamservice.v1.StreamService"


@dataclasses.dataclass
class Call:
    service: str
    endpoint: str | None
    operation: str
    input: object


class FakeNexusClient:
    def __init__(self, client: FakeClient, service: str, endpoint: str | None) -> None:
        self._client: FakeClient = client
        self._service: str = service
        self._endpoint: str | None = endpoint

    async def start_operation(
        self,
        operation: str,
        input: typing.Any,
        *,
        output_type: type[typing.Any] | None = None,
    ) -> collections.abc.Awaitable[typing.Any]:
        del output_type
        self._client.calls.append(Call(self._service, self._endpoint, operation, input))
        answer = self._client.answers[operation].pop(0)

        async def result() -> object:
            return answer

        return result()


class FakeClient(StreamServiceClientExtension):
    """A fake SDK client that inherits the extension of the Stream service."""

    def __init__(self, answers: dict[str, list[object]]) -> None:
        self.answers: dict[str, list[object]] = answers
        self.calls: list[Call] = []

    @override
    def _create_nexus_client(
        self,
        *,
        service: str,
        endpoint: str | None,
    ) -> FakeNexusClient:
        return FakeNexusClient(self, service, endpoint)


def created_stream(stream_id: str) -> CreateStreamResult:
    return stream_models._CreateStreamResultTransferTypeConverter().from_transfer_type(
        request_response_pb2.CreateStreamResponse(
            stream=message_pb2.StreamRef(id=stream_id), created=False
        ),
        CreateStreamResult,
    )


async def test_a_created_stream_calls_its_operations_through_the_client() -> None:
    ref = StreamRef(id="s1")
    client = FakeClient(
        {
            "CreateStream": [created_stream("s1")],
            "AppendRecords": [AppendRecordsResponse(first_offset=1, last_offset=2)],
        }
    )

    stream = await client.create_stream(id="s1")
    appended = await stream.publish(records=[AppendRecord(body="hi")])

    # `created` is a proto3 bool with no presence, so a false answer decodes as None.
    assert stream.created is None
    assert appended.last_offset == 2
    assert client.calls == [
        Call(
            STREAM_SERVICE,
            "__temporal_system",
            "CreateStream",
            CreateStreamRequest(id="s1"),
        ),
        Call(
            STREAM_SERVICE,
            "__temporal_system",
            "AppendRecords",
            AppendRecordsRequest(stream=ref, records=[AppendRecord(body="hi")]),
        ),
    ]


async def test_a_handle_built_by_hand_takes_the_current_client() -> None:
    ref = StreamRef(id="s1")
    client = FakeClient({"DescribeStream": [DescribeStreamResponse(last_offset=5)]})
    stream = Stream(stream=ref, created=None)

    with temporal_support.current_client(client):
        described = await stream.describe()

    assert described.last_offset == 5
    assert [call.input for call in client.calls] == [DescribeStreamRequest(stream=ref)]


def test_a_stream_crosses_the_targets_as_data() -> None:
    converter = temporalio.converter.PayloadConverter.default
    workflow_stream = workflow_resources.Stream(stream=StreamRef(id="s1"), created=True)

    # Each target has its own class, and a Stream converts as its fields.
    client_stream = converter.from_payload(
        converter.to_payload(workflow_stream), Stream
    )

    assert client_stream == Stream(stream=StreamRef(id="s1"), created=True)
    assert type(client_stream) is not type(workflow_stream)


def test_a_client_class_cant_get_an_operation_from_two_bases() -> None:
    class Base:
        def create_stream(self) -> None: ...

    with pytest.raises(TypeError, match="gets create_stream from .*Base and from"):
        _ = types.new_class("Client", (Base, FakeClient))
