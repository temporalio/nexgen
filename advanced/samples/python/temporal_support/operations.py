"""The operations of a client resource handle that has none.

The SDK's helper also gives the client of an Activity. The sample sets the client in a context.
"""

import collections.abc
import contextlib
import contextvars
import typing

_current_client: contextvars.ContextVar[object | None] = contextvars.ContextVar(
    "current_client", default=None
)


def shared_source() -> str:
    return "shared"


def workflow_source() -> str:
    return "workflow"


def client_source() -> str:
    return "client"


def current_operations() -> typing.Any:
    client = _current_client.get()
    if client is None:
        raise RuntimeError("no Temporal client for a handle with no operations")
    return client


@contextlib.contextmanager
def current_client(client: object) -> collections.abc.Iterator[None]:
    token = _current_client.set(client)
    try:
        yield
    finally:
        _current_client.reset(token)
