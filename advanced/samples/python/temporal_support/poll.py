"""The poll helper of the stream-service sample."""

import dataclasses
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
        and next_offset in (None, request.from_offset)
    ):
        return None
    if next_offset is None:
        # `last` reads the newest Records only once.
        return dataclasses.replace(request, last=None)
    # `last` takes no `from_offset`, and a `from_offset` overrides `start`.
    return dataclasses.replace(request, from_offset=next_offset, start=None, last=None)


def resume_read_request(request: typing.Any, item: typing.Any) -> typing.Any:
    # A read that resumes after an item starts past its offset, and doesn't read `last` again.
    return dataclasses.replace(
        request, from_offset=(item.offset or 0) + 1, start=None, last=None
    )
