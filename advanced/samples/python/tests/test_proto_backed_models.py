from __future__ import annotations

import inspect
from unittest.mock import AsyncMock, Mock, patch

import pytest
from typing_extensions import assert_type

import wit.proto_backed_showcase as proto_backed_showcase
from wit.proto_backed_showcase.models import (
    Alert,
    AlertLevel,
    BadBinaries,
    BadBinary,
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


def test_bare_collections_are_required_and_round_trip() -> None:
    with pytest.raises(TypeError, match="activity_task_queue_partitions"):
        _ = inspect.signature(TaskQueuePartitions).bind()
    first = TaskQueuePartitions(activity_task_queue_partitions=[])
    second = TaskQueuePartitions(activity_task_queue_partitions=[])
    _ = assert_type(first.activity_task_queue_partitions, list[TaskQueuePartition])
    assert first.workflow_task_queue_partitions is None
    assert first.activity_task_queue_partitions == []
    assert second.activity_task_queue_partitions == []
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

    with pytest.raises(TypeError, match="header"):
        _ = inspect.signature(CallbackNexus).bind(url="https://example.test")
    callback = CallbackNexus(url="https://example.test", header={})
    other = CallbackNexus(url="https://other.test", header={})
    _ = assert_type(callback.header, dict[str, str])
    assert callback.header == {}
    callback.header["x-test"] = "value"
    assert other.header == {}
    converter = getattr(CallbackNexus, "__temporal_transfer_type_converter")
    wire = converter.to_transfer_type(callback)
    assert dict(wire.header) == {"x-test": "value"}
    assert converter.from_transfer_type(wire, CallbackNexus) == callback


def test_required_collection_arguments_have_no_defaults() -> None:
    with pytest.raises(TypeError, match="activity_task_queue_partitions"):
        _ = inspect.signature(
            proto_backed_showcase.task_queue_partitions_operation
        ).bind()
    with pytest.raises(TypeError, match="header"):
        _ = inspect.signature(proto_backed_showcase.callback_nexus_operation).bind(
            url="https://example.test"
        )


@pytest.mark.parametrize("mode", ["omitted", "none", "empty", "populated"])
async def test_collection_arguments_preserve_absence_before_encoding(mode: str) -> None:
    handle = object()
    start_operation = AsyncMock(return_value=handle)
    client = Mock(start_operation=start_operation)
    required_list: list[TaskQueuePartition] = []
    required_map: dict[str, str] = {}
    optional_list: list[TaskQueuePartition] | None = None
    optional_map: dict[str, BadBinary] | None = None
    if mode == "empty":
        optional_list, optional_map = [], {}
    elif mode == "populated":
        optional_list = [TaskQueuePartition(key="one", owner_host_name="worker")]
        optional_map = {"one": BadBinary(reason="bad", operator="operator")}

    # Capture public client calls before protobuf erases collection presence.
    with patch("temporalio.workflow.create_nexus_client", return_value=client):
        if mode == "omitted":
            result = await proto_backed_showcase.task_queue_partitions_operation(
                activity_task_queue_partitions=required_list
            )
        else:
            result = await proto_backed_showcase.task_queue_partitions_operation(
                activity_task_queue_partitions=required_list,
                workflow_task_queue_partitions=optional_list,
            )
        assert result is handle
        assert start_operation.await_args is not None
        request = start_operation.await_args.kwargs["input"]
        assert isinstance(request, TaskQueuePartitions)
        assert request.activity_task_queue_partitions is required_list
        assert request.workflow_task_queue_partitions is optional_list
        start_operation.reset_mock()

        if mode == "omitted":
            result = await proto_backed_showcase.bad_binaries_operation()
        else:
            result = await proto_backed_showcase.bad_binaries_operation(
                binaries=optional_map
            )
        assert result is handle
        assert start_operation.await_args is not None
        request = start_operation.await_args.kwargs["input"]
        assert isinstance(request, BadBinaries)
        assert request.binaries is optional_map
        start_operation.reset_mock()

        result = await proto_backed_showcase.callback_nexus_operation(
            url="https://example.test", header=required_map
        )
        assert result is handle
        assert start_operation.await_args is not None
        request = start_operation.await_args.kwargs["input"]
        assert isinstance(request, CallbackNexus)
        assert request.header is required_map
