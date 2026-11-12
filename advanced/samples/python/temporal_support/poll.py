"""The poll helper of the stream-service sample."""

import dataclasses
import typing


# The generated models import this package, so a typed import of them here would be a cycle. The
# helper only reads a few fields and copies the request, so Any is enough.
def next_read_request(request: typing.Any, response: typing.Any) -> typing.Any:
    if response.done:
        return None
    last_offset = response.last_offset
    # A read that doesn't wait ends at the newest Record.
    if (
        not response.records
        and not request.wait
        and last_offset in (None, request.from_offset)
    ):
        return None
    if last_offset is None:
        # The read covered no Record, so the next read starts at the same offset.
        return request
    return _read_after(request, last_offset)


def resume_read_request(request: typing.Any, record: typing.Any) -> typing.Any:
    return _read_after(request, record.offset)


def _read_after(request: typing.Any, offset: bytes) -> typing.Any:
    # A from-offset is exclusive, so a read after an offset passes that offset, with no
    # arithmetic.
    return dataclasses.replace(request, from_offset=offset)
