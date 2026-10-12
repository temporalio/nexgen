from __future__ import annotations

import base64
import datetime
import json
from pathlib import Path

from temporalio.api.common.v1 import Payload
import temporalio.common
from temporalio.converter import DataConverter

from wit.type_roundtrip.models import ActivityOptions


WIRE_FIXTURE_DIR = (
    Path(__file__).resolve().parents[2] / "wire" / "proto" / "type_roundtrip"
)


def read_payload(name: str) -> Payload:
    fixture = json.loads((WIRE_FIXTURE_DIR / name).read_text(encoding="utf-8"))
    return Payload(
        metadata={
            key: base64.b64decode(value) for key, value in fixture["metadata"].items()
        },
        data=base64.b64decode(fixture["data"]),
    )


def assert_activity_options_model(decoded: object) -> None:
    assert isinstance(decoded, ActivityOptions)
    assert decoded.task_queue == "demo-task-queue"
    assert decoded.retry_policy.maximum_attempts == 3
    assert decoded.schedule_to_close_timeout == datetime.timedelta(seconds=7)
    assert decoded.priority == temporalio.common.Priority(
        priority_key=4,
        fairness_key="tenant-a",
        fairness_weight=2.5,
    )


async def test_activity_options_intermediate_conversion_delegates_payload_encoding() -> (
    None
):
    converter = DataConverter.default
    activity_options = ActivityOptions(
        retry_policy=temporalio.common.RetryPolicy(maximum_attempts=3),
        task_queue="demo-task-queue",
        schedule_to_close_timeout=datetime.timedelta(seconds=7),
        priority=temporalio.common.Priority(
            priority_key=4,
            fairness_key="tenant-a",
            fairness_weight=2.5,
        ),
    )

    payload = (await converter.encode([activity_options]))[0]
    assert payload.metadata["encoding"] == b"json/protobuf"
    assert (
        payload.metadata["messageType"] == b"temporal.api.activity.v1.ActivityOptions"
    )
    assert all("temporal-wire" not in key for key in payload.metadata)
    assert all(b"temporal-wire" not in value for value in payload.metadata.values())
    encoded = json.loads(payload.data)
    assert encoded["taskQueue"]["name"] == "demo-task-queue"
    assert encoded["retryPolicy"]["maximumAttempts"] == 3

    decoded = (await converter.decode([payload], [ActivityOptions]))[0]
    assert_activity_options_model(decoded)


async def test_activity_options_fixtures_decode_to_user_type() -> None:
    converter = DataConverter.default
    for fixture_name in (
        "activity-options.python.payload.json",
        "activity-options.dotnet.payload.json",
    ):
        payload = read_payload(fixture_name)
        assert payload.metadata["encoding"] == b"json/protobuf"
        assert (
            payload.metadata["messageType"]
            == b"temporal.api.activity.v1.ActivityOptions"
        )
        decoded = (await converter.decode([payload], [ActivityOptions]))[0]
        assert_activity_options_model(decoded)
