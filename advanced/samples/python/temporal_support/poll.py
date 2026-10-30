"""The poll helper of the stream-service sample."""

import dataclasses
import typing


class _Request(typing.Protocol):
    __dataclass_fields__: typing.ClassVar[dict[str, dataclasses.Field[typing.Any]]]


class _ReadRequest(_Request, typing.Protocol):
    @property
    def from_offset(self) -> int | None: ...


class _ReadResponse(typing.Protocol):
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
    # An empty page has no next offset, so the next read starts at the same offset.
    from_offset = (
        request.from_offset if response.next_offset is None else response.next_offset
    )
    # `last` reads the newest Records once. The next reads continue from the offset.
    return dataclasses.replace(request, from_offset=from_offset, last=None)
