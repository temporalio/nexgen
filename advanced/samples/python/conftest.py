from __future__ import annotations

from collections.abc import AsyncIterator
from datetime import timedelta
from pathlib import Path
from typing import cast

import pytest
import pytest_asyncio
from temporalio.client import Client
from temporalio.testing import WorkflowEnvironment


DEV_SERVER_DOWNLOAD_VERSION = "v1.7.1-system-nexus-operations"
DEV_SERVER_DOWNLOAD_DIR = (
    Path(__file__).resolve().parents[3] / "target" / "temporal-cli"
)
DEV_SERVER_STARTUP_TIMEOUT = timedelta(seconds=60)
DEV_SERVER_EXTRA_ARGS = [
    "--dynamic-config-value",
    "history.enableSignalWithStartFromWorkflow=true",
]


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
        DEV_SERVER_DOWNLOAD_DIR.mkdir(parents=True, exist_ok=True)
        workflow_environment = await WorkflowEnvironment.start_local(
            download_dest_dir=str(DEV_SERVER_DOWNLOAD_DIR),
            dev_server_download_version=DEV_SERVER_DOWNLOAD_VERSION,
            dev_server_extra_args=DEV_SERVER_EXTRA_ARGS,
            dev_server_startup_timeout=DEV_SERVER_STARTUP_TIMEOUT,
        )
    else:
        workflow_environment = WorkflowEnvironment.from_client(
            await Client.connect(env_type)
        )

    try:
        yield workflow_environment
    finally:
        await workflow_environment.shutdown()
