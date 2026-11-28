"""Poll helper for the catalog sample."""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from wit.page_poll.models import Page, PageRequest


def next_page_request(_request: PageRequest, response: Page) -> PageRequest | None:
    return response.next
