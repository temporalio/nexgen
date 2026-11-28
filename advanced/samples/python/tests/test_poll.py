import asyncio
import typing
from unittest.mock import AsyncMock, Mock

import pytest
import temporalio.client
import temporalio.service

from wit.page_poll.client import CatalogClientExtension
from wit.page_poll.models import Page, PageRequest


class CatalogClient(temporalio.client.Client, CatalogClientExtension):
    pass


@pytest.fixture
def catalog(monkeypatch: pytest.MonkeyPatch) -> tuple[CatalogClient, AsyncMock]:
    client = CatalogClient(typing.cast(temporalio.service.ServiceClient, object()))
    handle = AsyncMock(spec=temporalio.client.NexusOperationHandle)
    handle.result.side_effect = [
        Page(values=[], next=PageRequest(cursor=1)),
        Page(values=["last"]),
    ]
    start = AsyncMock(return_value=handle)
    nexus = Mock(spec=temporalio.client.NexusClient, start_operation=start)
    monkeypatch.setattr(client, "create_nexus_client", Mock(return_value=nexus))
    return client, start


async def test_a_page_only_poll_can_be_cancelled_before_the_next_request(
    catalog: tuple[CatalogClient, AsyncMock],
) -> None:
    client, start = catalog
    pages = client.iter_list_pages(cursor=0)
    assert (await anext(pages)).next == PageRequest(cursor=1)
    with pytest.raises(TimeoutError):
        async with asyncio.timeout(0):
            _ = await anext(pages)
    start.assert_awaited_once()


async def test_an_item_poll_can_be_cancelled_after_an_immediate_empty_page(
    catalog: tuple[CatalogClient, AsyncMock],
) -> None:
    client, start = catalog
    items = client.iter_list_items_items(cursor=0)
    with pytest.raises(TimeoutError):
        async with asyncio.timeout(0):
            _ = await anext(items)
    start.assert_awaited_once()


async def test_changing_an_empty_page_still_allows_cancellation(
    catalog: tuple[CatalogClient, AsyncMock],
) -> None:
    client, start = catalog
    pages = client.iter_list_items(cursor=0)
    page = await anext(pages)
    page.values.append("local")
    with pytest.raises(TimeoutError):
        async with asyncio.timeout(0):
            _ = await anext(pages)
    start.assert_awaited_once()
