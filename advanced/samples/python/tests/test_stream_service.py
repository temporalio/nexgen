from __future__ import annotations

import pytest
from temporalio.api.enums.v1 import stream_pb2
from temporalio.api.stream.v1 import message_pb2
from temporalio.api.streamservice.v1 import request_response_pb2
from temporalio.converter import PayloadConverter
import temporalio.nexus.system

from wit.stream_service.both import models
from wit.stream_service.both.models import (
    AppendRecordsRequest,
    ReadRecordsResponse,
    StreamClose,
    StreamCloseStatus,
    StreamData,
    StreamItemClose,
    StreamItemData,
    StreamItemSuperseded,
    StreamRecord,
    StreamRef,
    Supersession,
)


@pytest.fixture(autouse=True)
def user_payload_converter(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        temporalio.nexus.system,
        "_current_user_payload_converter",
        lambda: PayloadConverter.default,
    )


def test_read_items_round_trip_as_their_records() -> None:
    converter = models._ReadRecordsResponseTransferTypeConverter()
    model = ReadRecordsResponse(
        records=[
            StreamRecord(
                item=StreamItemData(StreamData(body={"text": "hi"})),
                offset=1,
            ),
            StreamRecord(
                item=StreamItemSuperseded(
                    Supersession(producer_id="workflow-1", failed_tasks=[7])
                ),
                offset=2,
            ),
            StreamRecord(
                item=StreamItemClose(
                    StreamClose(status=StreamCloseStatus.StreamCloseStatusCompleted)
                ),
                offset=3,
            ),
        ],
        next_offset=4,
        done=True,
    )

    wire = converter.to_transfer_type(model)

    # The wire messages come from `temporalio-stream-protos`, since no temporalio release has them.
    assert isinstance(wire, request_response_pb2.ReadRecordsResponse)
    assert all(isinstance(record, message_pb2.StreamRecord) for record in wire.records)
    assert [record.WhichOneof("item") for record in wire.records] == [
        "data",
        "superseded",
        "close",
    ]
    assert wire.records[1].superseded.failed_tasks == [7]
    assert converter.from_transfer_type(wire, ReadRecordsResponse) == model


def test_a_record_of_an_unknown_item_kind_has_no_item() -> None:
    converter = models._StreamRecordTransferTypeConverter()
    # Offset 1, and an item in field 5, which only a newer server knows.
    wire = message_pb2.StreamRecord.FromString(b"\x08\x01\x2a\x00")

    decoded = converter.from_transfer_type(wire, StreamRecord)

    assert decoded == StreamRecord(item=None, offset=1)


def test_a_request_without_its_stream_is_refused() -> None:
    converter = models._AppendRecordsRequestTransferTypeConverter()
    wire = converter.to_transfer_type(AppendRecordsRequest(stream=StreamRef(id="s-1")))
    assert wire.stream.id == "s-1"

    wire.ClearField("stream")
    with pytest.raises(
        ValueError, match="missing required field AppendRecordsRequest.stream"
    ):
        _ = converter.from_transfer_type(wire, AppendRecordsRequest)


def test_an_unknown_enum_number_decodes_as_the_fallback_case() -> None:
    converter = models._DescribeStreamResponseTransferTypeConverter()
    # A newer server can send a Stream status that this code doesn't know.
    wire = request_response_pb2.DescribeStreamResponse(
        status=stream_pb2.StreamStatus.ValueType(9)
    )

    decoded = converter.from_transfer_type(wire, models.DescribeStreamResponse)

    assert decoded.status is models.StreamStatus.StreamStatusUnspecified


def test_an_unknown_number_of_an_enum_with_no_fallback_fails_the_message() -> None:
    converter = models._ReadRecordsRequestTransferTypeConverter()
    wire = request_response_pb2.ReadRecordsRequest(
        stream=message_pb2.StreamRef(id="s-1"),
        start=stream_pb2.StreamReadStart.ValueType(9),
    )

    with pytest.raises(ValueError, match="9 is not a valid StreamReadStart"):
        _ = converter.from_transfer_type(wire, models.ReadRecordsRequest)
