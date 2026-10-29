from __future__ import annotations

import pytest
from typing_extensions import assert_type

from wit.proto_backed_showcase.models import (
    Alert,
    AlertLevel,
    CallbackInternal,
    CallbackNexus,
    NamespaceInfo,
    NamespaceState,
    TaskQueuePartition,
    TaskQueuePartitions,
)


@pytest.mark.parametrize("level", list(AlertLevel))
def test_authored_enum_round_trip(level: AlertLevel) -> None:
    model = Alert(message="alert", severity=level)
    converter = getattr(Alert, "__temporal_transfer_type_converter")
    wire = converter.to_transfer_type(model)
    assert wire.severity == level.value
    decoded = converter.from_transfer_type(
        type(wire).FromString(wire.SerializeToString()), Alert
    )
    assert decoded.message == model.message
    # This proto3 enum has no presence: zero decodes as absence.
    assert decoded.severity == (
        None if level == AlertLevel.SeverityUnspecified else level
    )
    if decoded.severity is not None:
        assert isinstance(decoded.severity, AlertLevel)


def test_optional_authored_enum_without_default() -> None:
    model = Alert(message="optional")
    _ = assert_type(model.severity, AlertLevel | None)
    assert model.severity is None
    converter = getattr(Alert, "__temporal_transfer_type_converter")
    wire = converter.to_transfer_type(model)
    assert wire.severity == 0
    assert converter.from_transfer_type(wire, Alert) == model


def test_conversion_does_not_apply_operation_defaults() -> None:
    model = NamespaceInfo(state=NamespaceState.NamespaceStateUnspecified)
    converter = getattr(NamespaceInfo, "__temporal_transfer_type_converter")
    wire = converter.to_transfer_type(model)
    assert wire.state == 0
    assert converter.from_transfer_type(wire, NamespaceInfo).state == model.state


@pytest.mark.parametrize("data", [b"", b"\x00\x01\x7f\x80\xff"])
def test_wit_byte_list_is_bytes(data: bytes) -> None:
    model = CallbackInternal(data=data)
    _ = assert_type(model.data, bytes)
    converter = getattr(CallbackInternal, "__temporal_transfer_type_converter")
    wire = converter.to_transfer_type(model)
    assert wire.data == data
    if not data:
        # Required protobuf scalars treat a default wire value as missing.
        with pytest.raises(
            ValueError, match="missing required field CallbackInternal.data"
        ):
            converter.from_transfer_type(wire, CallbackInternal)
        return
    assert (
        converter.from_transfer_type(
            type(wire).FromString(wire.SerializeToString()), CallbackInternal
        )
        == model
    )


def test_bare_collections_have_independent_defaults_and_round_trip() -> None:
    first = TaskQueuePartitions()
    second = TaskQueuePartitions()
    assert first.activity_task_queue_partitions == []
    assert second.activity_task_queue_partitions == []
    assert first.activity_task_queue_partitions is not None
    first.activity_task_queue_partitions.append(
        TaskQueuePartition(key="one", owner_host_name="worker")
    )
    assert second.activity_task_queue_partitions == []
    converter = getattr(TaskQueuePartitions, "__temporal_transfer_type_converter")
    wire = converter.to_transfer_type(first)
    decoded = converter.from_transfer_type(wire, TaskQueuePartitions)
    assert (
        decoded.activity_task_queue_partitions == first.activity_task_queue_partitions
    )
    assert (
        converter.from_transfer_type(
            type(wire)(), TaskQueuePartitions
        ).activity_task_queue_partitions
        == []
    )

    callback = CallbackNexus(url="https://example.test")
    other = CallbackNexus(url="https://other.test")
    assert callback.header == {}
    assert callback.header is not None
    callback.header["x-test"] = "value"
    assert other.header == {}
    converter = getattr(CallbackNexus, "__temporal_transfer_type_converter")
    wire = converter.to_transfer_type(callback)
    assert dict(wire.header) == {"x-test": "value"}
    assert converter.from_transfer_type(wire, CallbackNexus) == callback
