import asyncio
import collections.abc
import dataclasses
import importlib
import subprocess
import sys
import types
import typing

import pytest
import temporalio.converter
import temporalio.workflow
from temporalio.api.workflowservice.v1 import (
    SignalWithStartWorkflowExecutionResponse,
    StartWorkflowExecutionResponse,
)
from typing_extensions import override

import temporal_support as support
from wit.target_sources import client as client_only
from wit.target_sources import models as models_only
from wit.target_sources import workflow as workflow_only
from wit.target_sources.both import models as both_models
from wit.target_sources.both.workflow._resources import Job as WorkflowJob
from wit.target_sources.both import client as both_client
from wit.target_sources.both import workflow as both_workflow
from wit.target_sources.client import client as client_api
from wit.target_sources.workflow import models as workflow_models


class Handle:
    async def result(self) -> typing.Any:
        return None


class WorkflowRecorder:
    """Records the calls of the Workflow API, whose handles are awaitable."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, object]] = []

    async def start_operation(
        self,
        operation: str,
        input: typing.Any,
        *,
        output_type: type[typing.Any] | None = None,
    ) -> collections.abc.Awaitable[typing.Any]:
        del output_type
        self.calls.append((operation, input))
        future: asyncio.Future[typing.Any] = asyncio.get_running_loop().create_future()
        future.set_result(None)
        return future


class Recorder:
    def __init__(self) -> None:
        self.calls: list[tuple[str, object]] = []

    async def start_operation(
        self,
        operation: str,
        arg: typing.Any,
        *,
        id: str,
        result_type: type[typing.Any] | None = None,
    ) -> Handle:
        del id, result_type
        self.calls.append((operation, arg))
        return Handle()


class ClientOnly(client_api.SourceServiceClientExtension):
    def __init__(self, recorder: Recorder) -> None:
        self.recorder: Recorder = recorder

    @override
    def _create_nexus_client(self, *, service: str, endpoint: str) -> Recorder:
        assert (service, endpoint) == ("SourceService", "sources")
        return self.recorder


class BothClient(both_client.SourceServiceClientExtension):
    def __init__(self, recorder: Recorder) -> None:
        self.recorder: Recorder = recorder

    @override
    def _create_nexus_client(self, *, service: str, endpoint: str) -> Recorder:
        assert (service, endpoint) == ("SourceService", "sources")
        return self.recorder


@pytest.fixture
def source_calls(monkeypatch: pytest.MonkeyPatch) -> list[str]:
    calls: list[str] = []

    def source(name: str) -> str:
        calls.append(name)
        return name

    monkeypatch.setattr(support, "shared_source", lambda: source("shared"))
    monkeypatch.setattr(support, "workflow_source", lambda: source("workflow"))
    monkeypatch.setattr(support, "client_source", lambda: source("client"))
    return calls


@pytest.fixture
def workflow_recorder(monkeypatch: pytest.MonkeyPatch) -> WorkflowRecorder:
    recorder = WorkflowRecorder()

    def create(*, service: str, endpoint: str) -> WorkflowRecorder:
        assert (service, endpoint) == ("SourceService", "sources")
        return recorder

    monkeypatch.setattr(temporalio.workflow, "create_nexus_client", create)
    return recorder


async def test_workflow_only_uses_applicable_sources(
    workflow_recorder: WorkflowRecorder, source_calls: list[str]
) -> None:
    _ = await workflow_only.submit(id="id", client="supplied")
    assert source_calls == ["shared", "workflow"]
    assert workflow_recorder.calls == [
        (
            "Submit",
            workflow_models.SubmitRequest(
                id="id", shared="shared", workflow="workflow", client="supplied"
            ),
        )
    ]


async def test_client_only_uses_applicable_sources(
    monkeypatch: pytest.MonkeyPatch, source_calls: list[str]
) -> None:
    def forbidden(**kwargs: typing.Any) -> typing.NoReturn:
        del kwargs
        raise AssertionError("client-only calls must not use the workflow API")

    monkeypatch.setattr(temporalio.workflow, "create_nexus_client", forbidden)
    recorder = Recorder()
    client = ClientOnly(recorder)
    await client.submit(id="id", workflow="supplied")
    assert source_calls == ["shared", "client"]
    assert recorder.calls == [
        (
            "Submit",
            client_only.SubmitRequest(
                id="id", shared="shared", workflow="supplied", client="client"
            ),
        )
    ]

    await client_api.Job(recorder, job_id="job").restart()
    assert source_calls == ["shared", "client", "shared"]
    assert recorder.calls[-1] == (
        "Restart",
        client_only.RestartRequest(job_id="job", shared="shared"),
    )


async def test_combined_apis_share_models_but_not_source_policy(
    workflow_recorder: WorkflowRecorder, source_calls: list[str]
) -> None:
    recorder = Recorder()
    client = BothClient(recorder)
    _ = await both_workflow.submit(id="workflow", client="explicit")
    await client.submit(id="client", workflow="explicit")
    assert source_calls == ["shared", "workflow", "shared", "client"]
    assert type(workflow_recorder.calls[0][1]) is both_models.SubmitRequest
    assert type(recorder.calls[0][1]) is both_models.SubmitRequest

    await both_client.Job(recorder, job_id="client-job").restart()
    await WorkflowJob(job_id="workflow-job").restart()
    assert source_calls[-2:] == ["shared", "shared"]
    assert recorder.calls[-1] == (
        "Restart",
        both_models.RestartRequest(job_id="client-job", shared="shared"),
    )
    assert workflow_recorder.calls[-1] == (
        "Restart",
        both_models.RestartRequest(job_id="workflow-job", shared="shared"),
    )


async def test_target_packages_do_not_conflict_with_operation_names(
    workflow_recorder: WorkflowRecorder,
) -> None:
    recorder = Recorder()
    await BothClient(recorder).client()
    _ = await both_workflow.client()
    assert recorder.calls == [("Client", both_models.CreateMarkerRequest())]
    assert workflow_recorder.calls == [("Client", both_models.CreateMarkerRequest())]
    assert both_client.SourceServiceClientExtension is not None


@pytest.mark.parametrize(
    ("package", "forbidden"),
    [
        ("wit.target_sources.both.client", "wit.target_sources.both.workflow"),
        ("wit.target_sources.both.workflow", "wit.target_sources.both.client"),
        ("wit.target_sources.client.client", "wit.target_sources.client.workflow"),
    ],
)
def test_api_packages_import_without_the_other_target(
    package: str, forbidden: str
) -> None:
    script = """
import importlib
import importlib.abc
import sys

package, forbidden = sys.argv[1:]

class RejectOtherTarget(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path=None, target=None):
        if fullname == forbidden or fullname.startswith(forbidden + "."):
            raise AssertionError("API imported another target: " + fullname)
        return None

sys.meta_path.insert(0, RejectOtherTarget())
api = importlib.import_module(package)
importlib.import_module(package + ".services")
resources = importlib.import_module(package + "._resources")
assert resources.Job
"""
    result = subprocess.run(
        [sys.executable, "-c", script, package, forbidden],
        check=False,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize(
    "package",
    [
        "wit.target_sources.models",
        "wit.target_sources.client",
        "wit.target_sources.both",
    ],
)
def test_root_package_has_only_shared_definitions(package: str) -> None:
    shared = importlib.import_module(package)
    assert shared.SubmitRequest
    with pytest.raises(ModuleNotFoundError):
        _ = importlib.import_module(package + "._resources")
    with pytest.raises(ModuleNotFoundError):
        _ = importlib.import_module(package + ".services")


@pytest.mark.parametrize(
    "models",
    [models_only, client_only, workflow_models, both_models],
)
def test_shared_models_never_evaluate_sources(
    source_calls: list[str], models: types.ModuleType
) -> None:
    request = models.SubmitRequest(
        id="id", shared="explicit", workflow="explicit", client="explicit"
    )
    converter = temporalio.converter.PayloadConverter.default
    decoded = converter.from_payload(
        converter.to_payload(request), models.SubmitRequest
    )
    assert decoded == request
    assert dataclasses.replace(request, id="copy").id == "copy"
    with pytest.raises(TypeError):
        _ = models.SubmitRequest(id="missing")
    assert source_calls == []


@pytest.mark.parametrize(
    "models", [models_only, client_only, workflow_models, both_models]
)
async def test_shared_wire_models_are_independent_of_api_targets(
    models: types.ModuleType,
) -> None:
    converter = temporalio.converter.DataConverter.default
    for wire, model_type, fields in [
        (
            StartWorkflowExecutionResponse(run_id="run"),
            models.RunResult,
            {"run_id": "run"},
        ),
        (
            SignalWithStartWorkflowExecutionResponse(run_id="run", started=True),
            models.SignalWithStartWorkflowExecutionResponse,
            {"run_id": "run", "started": True},
        ),
    ]:
        payloads = await converter.encode([wire])
        result = (await converter.decode(payloads, [model_type]))[0]
        assert dataclasses.asdict(result) == fields
        assert (
            await converter.decode(await converter.encode([result]), [type(wire)])
        ) == [wire]
