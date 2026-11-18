from __future__ import annotations

from collections.abc import AsyncGenerator, Callable
from contextlib import aclosing
from datetime import timedelta
from inspect import unwrap
from pathlib import Path
from typing import cast
from unittest.mock import AsyncMock, Mock, patch

import pytest
from temporalio.client import Client
from temporalio.testing import WorkflowEnvironment

import conftest


def environment(env_type: str) -> AsyncGenerator[WorkflowEnvironment, None]:
    fixture = cast(
        Callable[[str], AsyncGenerator[WorkflowEnvironment, None]], unwrap(conftest.env)
    )
    return fixture(env_type)


async def test_local_environment_configures_and_reuses_download_cache(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    cache = tmp_path / "cli"
    monkeypatch.setattr(conftest, "DEV_SERVER_DOWNLOAD_DIR", cache)
    server = AsyncMock(spec=WorkflowEnvironment)
    start_local = AsyncMock(return_value=server)
    with patch.object(WorkflowEnvironment, "start_local", start_local):
        async with aclosing(environment("local")) as fixture:
            assert await anext(fixture) is server
            assert cache.is_dir()
            marker = cache / "cached-download"
            _ = marker.write_text("keep across sessions")
        async with aclosing(environment("local")) as fixture:
            assert await anext(fixture) is server
            assert marker.read_text() == "keep across sessions"

    assert start_local.await_count == 2
    start_local.assert_awaited_with(
        download_dest_dir=str(cache),
        dev_server_download_version="v1.7.1-system-nexus-operations",
        dev_server_extra_args=[
            "--dynamic-config-value",
            "history.enableSignalWithStartFromWorkflow=true",
        ],
        dev_server_startup_timeout=timedelta(seconds=60),
    )
    assert server.shutdown.await_count == 2


async def test_existing_environment_does_not_start_or_cache_a_server(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    cache = tmp_path / "unused"
    monkeypatch.setattr(conftest, "DEV_SERVER_DOWNLOAD_DIR", cache)
    client = Mock()
    server = AsyncMock(spec=WorkflowEnvironment)
    connect = AsyncMock(return_value=client)
    from_client = Mock(return_value=server)
    start_local = AsyncMock()
    with (
        patch.object(Client, "connect", connect),
        patch.object(WorkflowEnvironment, "from_client", from_client),
        patch.object(WorkflowEnvironment, "start_local", start_local),
    ):
        async with aclosing(environment("localhost:7233")) as fixture:
            assert await anext(fixture) is server
        assert not cache.exists()

    connect.assert_awaited_once_with("localhost:7233")
    from_client.assert_called_once_with(client)
    start_local.assert_not_awaited()
    server.shutdown.assert_awaited_once()
