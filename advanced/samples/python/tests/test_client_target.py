from __future__ import annotations

import collections.abc
import dataclasses
import types
import typing

import pytest
import temporalio.converter
from temporalio.api.workflowservice.v1 import (
    SignalWithStartWorkflowExecutionResponse,
    StartWorkflowExecutionResponse,
)
from typing_extensions import override

import wit.target_sources.both._resources as workflow_resources
from wit.target_sources.both.client import (
    Job,
    Marker,
    OrdersClientExtension,
    SourceServiceClientExtension,
)
from wit.target_sources.both.models import (
    CancelRunRequest,
    CreateJobRequest,
    PlaceRequest,
    RestartRequest,
)

SOURCE_SERVICE = "SourceService"


@dataclasses.dataclass
class Call:
    service: str
    endpoint: str | None
    operation: str
    input: object


class FakeNexusClient:
    def __init__(self, client: FakeClient, service: str, endpoint: str | None) -> None:
        self._client: FakeClient = client
        self._service: str = service
        self._endpoint: str | None = endpoint

    async def start_operation(
        self,
        operation: str,
        input: typing.Any,
        *,
        output_type: type[typing.Any] | None = None,
    ) -> collections.abc.Awaitable[typing.Any]:
        self._client.calls.append(Call(self._service, self._endpoint, operation, input))
        answer = self._client.answers[operation].pop(0)

        async def result() -> object:
            if output_type is not None:
                converter = temporalio.converter.DataConverter.default
                return (
                    await converter.decode(
                        await converter.encode([answer]), [output_type]
                    )
                )[0]
            return answer

        return result()


class FakeClient(SourceServiceClientExtension):
    """A fake SDK client that inherits the generated service extension."""

    def __init__(self, answers: dict[str, list[object]]) -> None:
        self.answers: dict[str, list[object]] = answers
        self.calls: list[Call] = []
        self.created_clients: list[FakeNexusClient] = []

    @override
    def _create_nexus_client(
        self,
        *,
        service: str,
        endpoint: str | None,
    ) -> FakeNexusClient:
        client = FakeNexusClient(self, service, endpoint)
        self.created_clients.append(client)
        return client


class FakeOrdersClient(FakeClient, OrdersClientExtension):
    """A fake SDK client that inherits the extensions of two services."""


async def test_a_resource_from_another_service_calls_its_own_service() -> None:
    client = FakeOrdersClient(
        {
            "Place": [workflow_resources.Job(job_id="job")],
            "Restart": [None],
        }
    )

    job = await client.place(job_id="job")
    await job.restart()

    assert client.calls == [
        Call("Orders", "orders", "Place", PlaceRequest(job_id="job")),
        Call(
            SOURCE_SERVICE,
            "sources",
            "Restart",
            RestartRequest(job_id="job", shared="shared"),
        ),
    ]


async def test_a_created_resource_calls_its_operations_through_the_client() -> None:
    client = FakeClient(
        {
            "CreateJob": [workflow_resources.Job(job_id="job")],
            "Restart": [None],
        }
    )

    job = await client.create_job(job_id="job")
    await job.restart()

    assert job.job_id == "job"
    assert len(client.created_clients) == 1
    assert client.calls == [
        Call(
            SOURCE_SERVICE,
            "sources",
            "CreateJob",
            CreateJobRequest(job_id="job"),
        ),
        Call(
            SOURCE_SERVICE,
            "sources",
            "Restart",
            RestartRequest(job_id="job", shared="shared"),
        ),
    ]


async def test_a_handle_uses_the_nexus_client_passed_to_its_constructor() -> None:
    client = FakeClient({"Restart": [None]})
    nexus_client = FakeNexusClient(client, SOURCE_SERVICE, "sources")
    job = Job(nexus_client, job_id="job")
    await job.restart()

    assert not client.created_clients
    assert [call.input for call in client.calls] == [
        RestartRequest(job_id="job", shared="shared")
    ]


async def test_a_resource_returned_by_a_method_keeps_the_same_nexus_client() -> None:
    client = FakeClient(
        {
            "Refresh": [workflow_resources.Job(job_id="refreshed")],
            "Restart": [None],
        }
    )
    nexus_client = FakeNexusClient(client, SOURCE_SERVICE, "sources")
    original = Job(nexus_client, job_id="original")
    refreshed = await original.refresh()
    await refreshed.restart()
    assert client.calls == [
        Call(SOURCE_SERVICE, "sources", "Refresh", CreateJobRequest(job_id="original")),
        Call(
            SOURCE_SERVICE,
            "sources",
            "Restart",
            RestartRequest(job_id="refreshed", shared="shared"),
        ),
    ]
    assert not client.created_clients


async def test_a_resource_without_methods_also_receives_the_nexus_client() -> None:
    client = FakeClient({"CreateMarker": [workflow_resources.Marker()]})
    marker = await client.create_marker()
    assert isinstance(marker, Marker)
    assert dataclasses.asdict(marker) == {}
    assert len(client.created_clients) == 1


async def test_a_proto_backed_resource_receives_the_same_nexus_client() -> None:
    client = FakeClient(
        {
            "CreateRun": [StartWorkflowExecutionResponse(run_id="run")],
            "CancelRun": [None],
        }
    )
    run = await client.create_run(job_id="job")
    await run.cancel()
    assert run.run_id == "run"
    assert len(client.created_clients) == 1
    assert client.calls[-1] == Call(
        SOURCE_SERVICE,
        "sources",
        "CancelRun",
        CancelRunRequest(job_id="job", run_id="run"),
    )


async def test_a_resource_in_a_proto_result_holder_receives_the_same_client() -> None:
    client = FakeClient(
        {
            "CreateRunWithStatus": [
                SignalWithStartWorkflowExecutionResponse(run_id="run", started=True)
            ],
            "CancelRun": [None],
        }
    )
    result = await client.create_run_with_status(job_id="job")
    await result.run.cancel()
    assert result.started is True
    assert len(client.created_clients) == 1
    assert client.calls[-1] == Call(
        SOURCE_SERVICE,
        "sources",
        "CancelRun",
        CancelRunRequest(job_id="job", run_id="run"),
    )


def test_a_resource_requires_a_nexus_client() -> None:
    with pytest.raises(TypeError):
        _ = Job(job_id="job")  # pyright: ignore[reportCallIssue]


def test_resource_serialization_contains_only_identity_data() -> None:
    client = FakeClient({})
    nexus_client = FakeNexusClient(client, SOURCE_SERVICE, "sources")
    job = Job(nexus_client, job_id="job")
    converter = temporalio.converter.PayloadConverter.default
    workflow_job = converter.from_payload(
        converter.to_payload(job), workflow_resources.Job
    )
    assert workflow_job == workflow_resources.Job(job_id="job")
    assert dataclasses.asdict(job) == {"job_id": "job"}


def test_a_client_class_cant_get_an_operation_from_two_bases() -> None:
    class Base:
        def create_job(self) -> None: ...

    with pytest.raises(TypeError, match="gets create_job from .*Base and from"):
        _ = types.new_class("Client", (Base, FakeClient))
