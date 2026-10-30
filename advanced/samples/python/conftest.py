from __future__ import annotations

from collections.abc import AsyncIterator
import importlib
import sys
from typing import cast

import pytest
import pytest_asyncio
from temporalio.client import Client
from temporalio.testing import WorkflowEnvironment


DEV_SERVER_DOWNLOAD_VERSION = "v1.7.1-system-nexus-operations"
DEV_SERVER_EXTRA_ARGS = [
    "--dynamic-config-value",
    "history.enableSignalWithStartFromWorkflow=true",
]


def install_stream_protos() -> None:
    """Make the Stream protos importable where the generated Stream service imports them.

    No temporalio release has the Stream protos yet, so ``streamprotos/`` holds them.
    """
    for name in (
        "temporalio.api.enums.v1.stream_pb2",
        "temporalio.api.stream",
        "temporalio.api.stream.v1",
        "temporalio.api.stream.v1.message_pb2",
        "temporalio.api.streamservice",
        "temporalio.api.streamservice.v1",
        "temporalio.api.streamservice.v1.request_response_pb2",
    ):
        module = importlib.import_module(
            "streamprotos.temporal." + name.removeprefix("temporalio.")
        )
        sys.modules[name] = module
        parent, _, child = name.rpartition(".")
        setattr(importlib.import_module(parent), child, module)


install_stream_protos()


def pytest_addoption(parser: pytest.Parser) -> None:
    parser.addoption(
        "-E",
        "--workflow-environment",
        default="local",
        help=(
            "Workflow environment to use: 'local' to start a dev server, "
            "or host:port for an existing Temporal server"
        ),
    )


@pytest.fixture(scope="session")
def env_type(request: pytest.FixtureRequest) -> str:
    option = cast(object, request.config.getoption("--workflow-environment"))
    if not isinstance(option, str):
        raise TypeError("--workflow-environment must be a string")
    return option


@pytest_asyncio.fixture(scope="session")
async def env(env_type: str) -> AsyncIterator[WorkflowEnvironment]:
    if env_type == "local":
        workflow_environment = await WorkflowEnvironment.start_local(
            dev_server_download_version=DEV_SERVER_DOWNLOAD_VERSION,
            dev_server_extra_args=DEV_SERVER_EXTRA_ARGS,
        )
    else:
        workflow_environment = WorkflowEnvironment.from_client(
            await Client.connect(env_type)
        )

    try:
        yield workflow_environment
    finally:
        await workflow_environment.shutdown()
