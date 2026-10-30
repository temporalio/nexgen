"""A resource that another interface returns keeps the Nexus client of its own service."""

from __future__ import annotations

import typing

from typing_extensions import override

import wit.cross_service_resource._resources as workflow_resources
from wit.cross_service_resource.client import JobsClientExtension, OrdersClientExtension
from wit.cross_service_resource.models import PlaceRequest, RestartRequest


class Handle:
    def __init__(self, answer: object) -> None:
        self._answer: object = answer

    async def result(self) -> typing.Any:
        return self._answer


class NexusClient:
    def __init__(self, client: Client, service: str, endpoint: str) -> None:
        self._client: Client = client
        self._service: str = service
        self._endpoint: str = endpoint

    async def start_operation(
        self,
        operation: str,
        arg: typing.Any,
        *,
        id: str,
        result_type: type[typing.Any] | None = None,
    ) -> Handle:
        del id, result_type
        self._client.calls.append((self._service, self._endpoint, operation, arg))
        return Handle(self._client.answers[operation])


class Client(JobsClientExtension, OrdersClientExtension):
    def __init__(self, answers: dict[str, object]) -> None:
        self.answers: dict[str, object] = answers
        self.calls: list[tuple[str, str, str, object]] = []

    @override
    def _create_nexus_client(self, *, service: str, endpoint: str) -> NexusClient:
        return NexusClient(self, service, endpoint)


async def test_a_job_from_orders_restarts_on_the_jobs_endpoint() -> None:
    client = Client(
        {
            "Place": workflow_resources.Job(job_id="job"),
            "Restart": None,
        }
    )

    job = await client.place(job_id="job")
    await job.restart()

    assert client.calls == [
        ("Orders", "orders", "Place", PlaceRequest(job_id="job")),
        ("Jobs", "jobs", "Restart", RestartRequest(job_id="job")),
    ]
