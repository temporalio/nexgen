"""The start of a Workflow call at the call, for sync Workflow code."""

from __future__ import annotations

import asyncio
import collections.abc
import sys
import typing

T = typing.TypeVar("T")

_EAGER_START = sys.version_info >= (3, 12)


def start_now(call: collections.abc.Awaitable[T]) -> asyncio.Task[T]:
    """Run ``call`` up to its first wait now, in a task that gives its result.

    A call adds its command before it first waits, so the command keeps its place in the order of
    the commands that the caller adds, as ``workflow.start_activity`` does.
    """

    async def run() -> T:
        return await call

    loop = asyncio.get_running_loop()
    # Python 3.12 runs a task to its first wait at its creation. A type checker that targets an
    # older Python has no `eager_start`, so the call goes through a plain callable.
    if _EAGER_START:
        start = typing.cast("collections.abc.Callable[..., object]", asyncio.Task)
        return typing.cast("asyncio.Task[T]", start(run(), loop=loop, eager_start=True))
    # Before Python 3.12, step the call to its first wait here, and a task drives the rest.
    coroutine = run()
    try:
        waiting = coroutine.send(None)
    except StopIteration as finished:
        return loop.create_task(_finished(typing.cast(T, finished.value)))
    except BaseException as error:
        return loop.create_task(_raised(error))
    return loop.create_task(_resume(coroutine, waiting))


async def _finished(value: T) -> T:
    return value


async def _raised(error: BaseException) -> typing.NoReturn:
    raise error


async def _resume(
    coroutine: collections.abc.Coroutine[object, None, T], waiting: object
) -> T:
    """Drive a started coroutine as a task does. A cancel goes to the future that it waits on."""
    while True:
        cancelled: asyncio.CancelledError | None = None
        try:
            if isinstance(waiting, asyncio.Future):
                future = typing.cast("asyncio.Future[object]", waiting)
                done = asyncio.get_running_loop().create_future()
                future.add_done_callback(lambda _: done.done() or done.set_result(None))
                await done
            else:
                await asyncio.sleep(0)
        except asyncio.CancelledError as error:
            if isinstance(waiting, asyncio.Future):
                _ = typing.cast("asyncio.Future[object]", waiting).cancel()
            cancelled = error
        try:
            waiting = (
                coroutine.send(None)
                if cancelled is None
                else coroutine.throw(cancelled)
            )
        except StopIteration as finished:
            return typing.cast(T, finished.value)
