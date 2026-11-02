"""A real ``temporalio.client.Client`` that inherits the generated client extensions.

basedpyright checks that ``Client.create_nexus_client`` fits the generated hook. An
interceptor answers each Nexus call in the process, so the test needs no server.
"""

from __future__ import annotations

import typing
import uuid

import temporalio.client
import temporalio.service
from typing_extensions import override

import wit.target_sources.both.client._resources._resource_data as resource_data
from wit.target_sources.both.client import SourceServiceClientExtension
from wit.target_sources.both.models import CreateJobRequest, RestartRequest


class Client(temporalio.client.Client, SourceServiceClientExtension):
    @override
    def _create_nexus_client(
        self, *, service: str, endpoint: str
    ) -> temporalio.client.NexusClient[typing.Any]:
        return self.create_nexus_client(service, endpoint)


class Answers(temporalio.client.Interceptor):
    def __init__(self, answers: dict[str, object]) -> None:
        super().__init__()
        self.answers: dict[str, object] = answers
        self.started: list[temporalio.client.StartNexusOperationInput] = []
        self.client: temporalio.client.Client | None = None

    @override
    def intercept_client(
        self, next: temporalio.client.OutboundInterceptor
    ) -> temporalio.client.OutboundInterceptor:
        return _AnswersOutbound(next, self)


class _AnswersOutbound(temporalio.client.OutboundInterceptor):
    def __init__(
        self, next: temporalio.client.OutboundInterceptor, answers: Answers
    ) -> None:
        super().__init__(next)
        self._answers: Answers = answers

    @override
    async def start_nexus_operation(
        self, input: temporalio.client.StartNexusOperationInput
    ) -> temporalio.client.NexusOperationHandle[typing.Any]:
        self._answers.started.append(input)
        assert self._answers.client is not None
        return temporalio.client.NexusOperationHandle(
            self._answers.client,
            input.id,
            result_type=input.result_type,
            endpoint=input.endpoint,
            service=input.service,
            operation=input.operation,
        )

    @override
    async def get_nexus_operation_result(
        self, input: temporalio.client.GetNexusOperationResultInput
    ) -> typing.Any:
        return self._answers.answers[input.operation]


def connect(answers: dict[str, object]) -> tuple[Client, Answers]:
    interceptor = Answers(answers)
    # The interceptor answers each call before the client uses its service client.
    client = Client(
        typing.cast(temporalio.service.ServiceClient, object()),
        interceptors=[interceptor],
    )
    interceptor.client = client
    return client, interceptor


async def test_an_operation_starts_through_the_sdk_nexus_client() -> None:
    client, answers = connect(
        {
            "CreateJob": resource_data.JobData(job_id="job"),
            "Restart": None,
        }
    )

    job = await client.create_job(job_id="job")
    await job.restart()

    assert job.job_id == "job"
    create, restart = answers.started
    assert (create.service, create.endpoint, create.operation) == (
        "SourceService",
        "sources",
        "CreateJob",
    )
    assert create.arg == CreateJobRequest(job_id="job")
    assert uuid.UUID(create.id)
    assert create.result_type is resource_data.JobData
    assert (restart.service, restart.endpoint, restart.operation) == (
        "SourceService",
        "sources",
        "Restart",
    )
    assert restart.arg == RestartRequest(job_id="job", shared="shared")
    assert uuid.UUID(restart.id)
    assert restart.result_type is None
