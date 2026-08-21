from __future__ import annotations

import dataclasses

import temporalio.converter
import temporalio.nexus.system

from wit.proto_generic_python import (
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
    converter = temporalio.nexus.system._SystemNexusPayloadConverter(
        temporalio.converter.PayloadConverter.default
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
        temporalio.converter.PayloadConverter.default
    )

    payload = converter.to_payload(model)
    decoded = converter.from_payload(payload, PayloadBackedEnvelope)

    assert decoded == model
