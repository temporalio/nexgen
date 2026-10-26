from __future__ import annotations

import dataclasses
import typing

import pytest
from temporalio.api.update.v1 import Outcome as ProtoOutcome
from temporalio.api.common.v1 import Payloads
from temporalio.api.workflowservice.v1 import (
    PauseActivityRequest as ProtoPauseActivityRequest,
)
from temporalio.converter import DataConverter, PayloadConverter
import temporalio.nexus.system

from wit.proto_oneof import (
    Outcome,
    OutcomeValueFailure,
    OutcomeValueSuccess,
    PauseActivityRequest,
)
from wit.proto_oneof.models import (
    _OutcomeTransferTypeConverter,
    _PauseActivityRequestTransferTypeConverter,
)


@dataclasses.dataclass
class SuccessfulOutput:
    message: str


def test_proto_oneof_success_round_trip(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        temporalio.nexus.system,
        "_current_user_payload_converter",
        lambda: PayloadConverter.default,
    )
    converter = _OutcomeTransferTypeConverter[SuccessfulOutput]()
    model: Outcome[SuccessfulOutput] = Outcome(
        value=OutcomeValueSuccess(SuccessfulOutput(message="hello"))
    )

    wire = converter.to_transfer_type(model)
    assert wire.WhichOneof("value") == "success"

    decoded = converter.from_transfer_type(wire, Outcome[SuccessfulOutput])
    assert decoded == model
    assert isinstance(decoded.value, OutcomeValueSuccess)
    assert isinstance(decoded.value.value, SuccessfulOutput)


async def test_proto_oneof_success_payload_converter_round_trip(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        temporalio.nexus.system,
        "_current_user_payload_converter",
        lambda: PayloadConverter.default,
    )
    converter = DataConverter.default
    model: Outcome[SuccessfulOutput] = Outcome(
        value=OutcomeValueSuccess(SuccessfulOutput(message="hello"))
    )

    payload = (await converter.encode([model]))[0]

    assert payload.metadata["encoding"] == b"json/protobuf"
    assert payload.metadata["messageType"] == b"temporal.api.update.v1.Outcome"
    decoded = (await converter.decode([payload], [Outcome[SuccessfulOutput]]))[0]
    assert decoded == model
    assert isinstance(decoded.value, OutcomeValueSuccess)
    assert isinstance(decoded.value.value, SuccessfulOutput)


def test_required_proto_oneof_failure_round_trip() -> None:
    converter = _OutcomeTransferTypeConverter[SuccessfulOutput]()

    wire = converter.to_transfer_type(
        Outcome(value=OutcomeValueFailure(RuntimeError("boom")))
    )
    assert wire.WhichOneof("value") == "failure"
    decoded = converter.from_transfer_type(wire, Outcome)
    assert isinstance(decoded.value, OutcomeValueFailure)
    assert str(decoded.value.value).endswith("boom")


async def test_proto_oneof_failure_payload_converter_round_trip(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        temporalio.nexus.system,
        "_current_user_payload_converter",
        lambda: PayloadConverter.default,
    )
    converter = DataConverter.default
    model: Outcome[object] = Outcome(value=OutcomeValueFailure(RuntimeError("boom")))

    payload = (await converter.encode([model]))[0]

    assert payload.metadata["encoding"] == b"json/protobuf"
    assert payload.metadata["messageType"] == b"temporal.api.update.v1.Outcome"
    decoded = (await converter.decode([payload], [Outcome]))[0]
    assert isinstance(decoded.value, OutcomeValueFailure)
    assert str(decoded.value.value).endswith("boom")


def test_required_proto_oneof_rejects_unset_wire() -> None:
    converter = _OutcomeTransferTypeConverter[typing.Any]()

    with pytest.raises(ValueError, match="missing required field Outcome.value"):
        _ = converter.from_transfer_type(ProtoOutcome(), Outcome)


def test_proto_oneof_payloads_handle_empty_and_reject_multiple_values() -> None:
    converter = _OutcomeTransferTypeConverter[SuccessfulOutput | None]()
    decoded = converter.from_transfer_type(
        ProtoOutcome(success=Payloads()), Outcome[SuccessfulOutput | None]
    )
    assert isinstance(decoded.value, OutcomeValueSuccess)
    assert decoded.value.value is None
    payload = PayloadConverter.default.to_payload(SuccessfulOutput("one"))
    with pytest.raises(
        ValueError, match="expected at most one payload in Outcome.value, found 2"
    ):
        _ = converter.from_transfer_type(
            ProtoOutcome(success=Payloads(payloads=[payload, payload])),
            Outcome[SuccessfulOutput | None],
        )


def test_optional_proto_oneof_round_trips_unset_as_none() -> None:
    converter = _PauseActivityRequestTransferTypeConverter()
    model = PauseActivityRequest(
        namespace="namespace",
        identity="worker",
        reason="maintenance",
        request_id="request-id",
    )

    wire = converter.to_transfer_type(model)
    assert wire.WhichOneof("activity") is None

    proto = ProtoPauseActivityRequest(
        namespace="namespace",
        identity="worker",
        reason="maintenance",
        request_id="request-id",
    )
    assert converter.from_transfer_type(proto, PauseActivityRequest) == model
