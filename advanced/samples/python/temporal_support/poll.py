"""The poll helper of the stream-service sample."""

import dataclasses
import sys
import typing


# The generated models import this package, so a typed import of them here would be a cycle. The
# helper only reads a few fields and copies the request, so Any is enough.
def next_read_request(request: typing.Any, response: typing.Any) -> typing.Any:
    if response.done:
        return None
    next_offset = response.next_offset
    # A read that doesn't wait ends at the newest Record.
    if (
        not response.records
        and not request.wait
        and next_offset in (None, _from_offset(request))
    ):
        return None
    if next_offset is None:
        # `last` reads the newest Records only once.
        return dataclasses.replace(request, last=None)
    return _read_after(request, next_offset)


def resume_read_request(request: typing.Any, item: typing.Any) -> typing.Any:
    return _read_after(request, item.offset)


def _read_after(request: typing.Any, offset: bytes) -> typing.Any:
    # A from-offset is exclusive, so a read after an offset passes that offset, with no
    # arithmetic. `last` takes no position, so the read doesn't read `last` again.
    position = _models(request).ReadPositionFromOffset(offset)
    return dataclasses.replace(request, position=position, last=None)


def _from_offset(request: typing.Any) -> bytes | None:
    position = request.position
    if isinstance(position, _models(request).ReadPositionFromOffset):
        return position.value
    return None


def _models(request: typing.Any) -> typing.Any:
    # Each generated package has its own position classes, and its converter matches only those.
    return sys.modules[type(request).__module__]
