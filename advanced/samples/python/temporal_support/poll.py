"""The poll helper of the stream-service sample."""

from __future__ import annotations

import dataclasses
import typing

if typing.TYPE_CHECKING:
    import collections.abc
    import datetime

    from _typeshed import DataclassInstance

    # The generated models import this package, and basedpyright reports even a type-only import
    # of them here as a cycle. These Protocols name the fields that the helper reads.
    class _ReadRequest(DataclassInstance, typing.Protocol):
        @property
        def from_offset(self) -> int | None: ...

        @property
        def wait(self) -> datetime.timedelta | None: ...

    class _ReadResponse(typing.Protocol):
        @property
        def records(self) -> collections.abc.Sequence[object] | None: ...

        @property
        def next_offset(self) -> int | None: ...

        @property
        def done(self) -> bool | None: ...

    _ReadRequestT = typing.TypeVar("_ReadRequestT", bound=_ReadRequest)


def next_read_request(
    request: _ReadRequestT, response: _ReadResponse
) -> _ReadRequestT | None:
    if response.done:
        return None
    next_offset = response.next_offset
    # A read that doesn't wait ends at the newest Record.
    if (
        not response.records
        and not request.wait
        and next_offset in (None, request.from_offset)
    ):
        return None
    if next_offset is None:
        # `last` reads the newest Records only once.
        return dataclasses.replace(request, last=None)
    # `last` takes no `from_offset`, and a `from_offset` overrides `start`.
    return dataclasses.replace(request, from_offset=next_offset, start=None, last=None)
