from __future__ import annotations

import dataclasses
from collections.abc import Generator

import pytest
import temporalio.converter
from temporalio import workflow

import temporal_support
from wit.defaults.models import (
    MyEnum,
    RecordWithDefaults,
    Request,
)
from wit.defaults.operations.submit import submit
from wit.defaults._resources import Sender


@dataclasses.dataclass
class Calls:
    requests: list[Request] = dataclasses.field(default_factory=list)
    namespace: str = "first-namespace"
    namespace_calls: int = 0


@pytest.fixture
def calls(monkeypatch: pytest.MonkeyPatch) -> Calls:
    captured = Calls()

    class Handle:
        def __init__(self, value: Request) -> None:
            self.value: Request = value

        async def result(self) -> Request:
            return self.value

        def __await__(self) -> Generator[object, None, Request]:
            return self.result().__await__()

    class Client:
        async def start_operation(
            self,
            *,
            operation: str,
            input: Request,
            output_type: type[Request],
        ) -> Handle:
            assert operation in {"Submit", "SubmitFromSender"}
            assert output_type is Request
            captured.requests.append(input)
            return Handle(input)

    def create_client(*, service: str, endpoint: str) -> Client:
        assert service == "DefaultsService"
        assert endpoint == "defaults"
        return Client()

    def namespace() -> str:
        captured.namespace_calls += 1
        return captured.namespace

    monkeypatch.setattr(workflow, "create_nexus_client", create_client)
    monkeypatch.setattr(temporal_support, "workflow_namespace", namespace)
    return captured


async def test_defaults_are_lazy_and_per_invocation(calls: Calls) -> None:
    assert calls.namespace_calls == 0
    _ = await submit()
    first = calls.requests[-1]
    assert first.namespace == "first-namespace"
    assert first.attempts == 3
    assert first.enabled is True
    assert first.ratio == 1.5
    assert first.mode == MyEnum.Standard
    assert first.permissions == 3
    assert first.alias_label == "alias"
    assert first.retry is None
    assert first.metadata is None
    assert calls.namespace_calls == 1

    calls.namespace = "second-namespace"
    _ = await submit(namespace=None)
    second = calls.requests[-1]
    assert second.namespace == "second-namespace"
    assert calls.namespace_calls == 2


async def test_supplied_falsy_values_do_not_invoke_providers(calls: Calls) -> None:
    _ = await submit(
        namespace="",
        attempts=0,
        enabled=False,
        ratio=0,
        mode=MyEnum.Unspecified,
        permissions=0,
        alias_label="",
        summary="",
    )
    request = calls.requests[-1]
    assert request.namespace == request.alias_label == ""
    assert request.attempts == request.ratio == request.permissions == 0
    assert request.enabled is False
    assert request.mode == MyEnum.Unspecified
    assert request.metadata is not None
    assert request.metadata.summary == ""
    assert calls.namespace_calls == 0


async def test_nested_records_pass_through_and_flattened_arguments_default(
    calls: Calls,
) -> None:
    retry = RecordWithDefaults(attempts=0, label="")
    _ = await submit(retry=retry, details="details")
    request = calls.requests[-1]
    assert request.retry is retry
    assert retry.attempts == 0
    assert retry.label == ""
    assert request.metadata is not None
    assert request.metadata.summary == "untitled"
    assert request.metadata.details == "details"


async def test_encoding_and_inbound_decoding_never_apply_defaults(calls: Calls) -> None:
    _ = await submit()
    inbound = dataclasses.replace(
        calls.requests[-1],
        namespace="",
        attempts=0,
        retry=RecordWithDefaults(attempts=0, label=""),
    )
    converter = temporalio.converter.DataConverter.default
    for _ in range(2):
        payloads = await converter.encode([inbound])
        decoded = await converter.decode(payloads, [Request])
        assert decoded == [inbound]
    assert calls.namespace_calls == 1
    malformed = dataclasses.asdict(inbound)
    malformed["retry"] = {}
    with pytest.raises((TypeError, ValueError)):
        _ = await converter.decode(await converter.encode([malformed]), [Request])
    assert calls.namespace_calls == 1


async def test_resource_forwarding_initializes_the_same_defaults(calls: Calls) -> None:
    sender = Sender(token="unused")
    _ = await sender.submit()
    request = calls.requests[-1]
    assert request.namespace == "first-namespace"
    assert request.attempts == 3
    assert request.retry is None
    assert request.metadata is None
    assert calls.namespace_calls == 1
