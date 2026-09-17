from __future__ import annotations

import dataclasses
import importlib
from pathlib import Path
import sys
import typing
import types

import pytest
from google.protobuf import descriptor_pb2, descriptor_pool, message_factory
from temporalio.converter import PayloadConverter
import temporalio.nexus.system


@dataclasses.dataclass
class Completion:
    message: str


@dataclasses.dataclass
class SourceContext:
    workflow_id: str


def install_notification_service_protos(monkeypatch: pytest.MonkeyPatch) -> None:
    # The SDK does not distribute bindings for every server API. The checked-in
    # descriptor set is the source used to generate this sample, so it also
    # supplies the concrete messages needed to exercise its converters.
    import temporalio.api

    _ = importlib.import_module("temporalio.api.common.v1.message_pb2")
    _ = importlib.import_module("temporalio.api.failure.v1.message_pb2")

    descriptor_set = descriptor_pb2.FileDescriptorSet.FromString(
        (Path(__file__).parents[2] / "descriptors" / "temporal_api.bin").read_bytes()
    )
    notification_file = next(
        file
        for file in descriptor_set.file
        if file.package == "temporal.api.notificationservice.v1"
    )
    pool = typing.cast(typing.Any, descriptor_pool.Default())
    _ = pool.Add(notification_file)

    service_module = types.ModuleType("temporalio.api.notificationservice")
    service_module.__path__ = []
    v1_module = types.ModuleType("temporalio.api.notificationservice.v1")
    v1_module.__path__ = []
    proto_module = types.ModuleType(
        "temporalio.api.notificationservice.v1.request_response_pb2"
    )
    for name in ["OnCompleteRequest", "OnCompleteResponse"]:
        message_descriptor = pool.FindMessageTypeByName(
            f"temporal.api.notificationservice.v1.{name}"
        )
        setattr(
            proto_module,
            name,
            message_factory.GetMessageClass(message_descriptor),
        )

    monkeypatch.setitem(sys.modules, service_module.__name__, service_module)
    monkeypatch.setitem(sys.modules, v1_module.__name__, v1_module)
    monkeypatch.setitem(sys.modules, proto_module.__name__, proto_module)
    monkeypatch.setattr(
        temporalio.api, "notificationservice", service_module, raising=False
    )
    monkeypatch.setattr(service_module, "v1", v1_module, raising=False)
    monkeypatch.setattr(v1_module, "request_response_pb2", proto_module, raising=False)


def test_on_complete_request_round_trips_generic_values(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    install_notification_service_protos(monkeypatch)
    monkeypatch.setattr(
        temporalio.nexus.system,
        "_current_user_payload_converter",
        lambda: PayloadConverter.default,
    )
    notification_service = importlib.import_module("wit.notification_service")
    models = importlib.import_module("wit.notification_service.models")
    model = notification_service.OnCompleteRequest[Completion, SourceContext](
        result=notification_service.OnCompleteRequestResultSuccess(
            Completion(message="completed")
        ),
        source_context=SourceContext(workflow_id="workflow-id"),
    )
    converter = models._OnCompleteRequestTransferTypeConverter[
        Completion, SourceContext
    ]()

    wire = converter.to_transfer_type(model)

    assert wire.WhichOneof("result") == "success"
    decoded = converter.from_transfer_type(
        wire,
        notification_service.OnCompleteRequest[Completion, SourceContext],
    )
    assert decoded == model
    assert isinstance(
        decoded.result, notification_service.OnCompleteRequestResultSuccess
    )
    assert isinstance(decoded.result.value, Completion)
    assert isinstance(decoded.source_context, SourceContext)
