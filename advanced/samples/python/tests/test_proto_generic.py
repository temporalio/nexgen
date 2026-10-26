from __future__ import annotations

import dataclasses
import typing

import pytest
from temporalio.api.common.v1 import Payloads
from temporalio.api.failure.v1 import CanceledFailureInfo
import temporalio.converter
import temporalio.nexus.system
from typing_extensions import assert_type

from wit.proto_generic import (
    OptionalPayloads,
    PayloadBackedContext,
    PayloadBackedEnvelope,
    PayloadBackedOutput,
)


@dataclasses.dataclass
class EchoOutput:
    value: str


@dataclasses.dataclass
class MyCtx:
    neat: str


def test_proto_backed_generic_type_hints_are_preserved() -> None:
    model = PayloadBackedEnvelope(
        provider=PayloadBackedOutput(details=EchoOutput(value="hello")),
        scaler=PayloadBackedContext(details=MyCtx(neat="very")),
    )
    _ = assert_type(model, PayloadBackedEnvelope[EchoOutput, MyCtx])
    converter = temporalio.nexus.system._SystemNexusPayloadConverter(
        temporalio.converter.PayloadConverter.default,
        temporalio.converter.FailureConverter.default,
    )

    payload = converter.to_payload(model)
    decoded = converter.from_payload(
        payload,
        PayloadBackedEnvelope[EchoOutput, MyCtx],
    )

    assert isinstance(decoded, PayloadBackedEnvelope)
    assert isinstance(decoded.provider.details, EchoOutput)
    assert decoded.provider.details == EchoOutput(value="hello")
    assert isinstance(decoded.scaler.details, MyCtx)
    assert decoded.scaler.details == MyCtx(neat="very")


def test_unparameterized_proto_backed_generic_decodes_payload_values() -> None:
    model = PayloadBackedEnvelope(
        provider=PayloadBackedOutput(details={"value": "hello"}),
        scaler=PayloadBackedContext(details={"neat": "very"}),
    )
    converter = temporalio.nexus.system._SystemNexusPayloadConverter(
        temporalio.converter.PayloadConverter.default,
        temporalio.converter.FailureConverter.default,
    )

    payload = converter.to_payload(model)
    decoded = typing.cast(
        PayloadBackedEnvelope[typing.Any, typing.Any],
        converter.from_payload(payload, PayloadBackedEnvelope),
    )

    assert decoded == model


def test_optional_payloads_preserve_typed_values_and_absence() -> None:
    converter = temporalio.nexus.system._SystemNexusPayloadConverter(
        temporalio.converter.PayloadConverter.default,
        temporalio.converter.FailureConverter.default,
    )
    for model in [
        OptionalPayloads[EchoOutput](),
        OptionalPayloads(details=EchoOutput("optional")),
    ]:
        payload = converter.to_payload(model)
        decoded = converter.from_payload(payload, OptionalPayloads[EchoOutput])
        assert decoded == model
        if decoded.details is not None:
            assert isinstance(decoded.details, EchoOutput)


def test_optional_payloads_handle_empty_and_reject_multiple_values() -> None:
    converter = getattr(OptionalPayloads, "__temporal_transfer_type_converter")
    decoded = converter.from_transfer_type(
        CanceledFailureInfo(details=Payloads()), OptionalPayloads[EchoOutput]
    )
    assert decoded == OptionalPayloads[EchoOutput]()
    payload = temporalio.converter.PayloadConverter.default.to_payload(
        EchoOutput("one")
    )
    with pytest.raises(
        ValueError,
        match="expected at most one payload in OptionalPayloads.details, found 2",
    ):
        converter.from_transfer_type(
            CanceledFailureInfo(details=Payloads(payloads=[payload, payload])),
            OptionalPayloads[EchoOutput],
        )
