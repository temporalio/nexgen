from __future__ import annotations

import dataclasses

import pytest
from temporalio.converter import PayloadConverter
import temporalio.nexus.system

import wit.notification_service as notification_service
from wit.notification_service import models


@dataclasses.dataclass
class Completion:
    message: str


@dataclasses.dataclass
class SourceContext:
    workflow_id: str


def test_on_complete_request_round_trips_generic_values(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        temporalio.nexus.system,
        "_current_user_payload_converter",
        lambda: PayloadConverter.default,
    )
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
