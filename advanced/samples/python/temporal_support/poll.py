"""The poll helpers of the stream-service sample."""

import collections.abc
import dataclasses
import typing

_BodyT = typing.TypeVar("_BodyT")


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


@dataclasses.dataclass
class Record(typing.Generic[_BodyT]):
    """A Record that a read gives."""

    offset: int
    body: _BodyT


@dataclasses.dataclass
class Superseded:
    """A Superseded item, with the offsets of the earlier Records that it withdraws."""

    offset: int
    withdrawn: list[int]


def read_item_transform() -> collections.abc.Callable[
    [typing.Any], list[Record[typing.Any] | Superseded]
]:
    """A new transform for the items of one read.

    A Superseded item withdraws the earlier Records of its Producer in the same read. The Store's
    rule also compares epochs and withdraws a retried run. This sample doesn't.
    """
    offsets: dict[str, list[int]] = {}

    def transform(record: typing.Any) -> list[Record[typing.Any] | Superseded]:
        # A newer server can send an item kind that this SDK doesn't know. Then the item is empty.
        if record.item is None:
            return []
        value = record.item.value
        offset = record.offset or 0
        # Only a Superseded item has a producer id.
        if hasattr(value, "producer_id"):
            withdrawn = offsets.pop(value.producer_id, [])
            return [Superseded(offset=offset, withdrawn=withdrawn)]
        if value.producer is not None:
            offsets.setdefault(value.producer.id, []).append(offset)
        return [Record(offset=offset, body=value.body)]

    return transform
