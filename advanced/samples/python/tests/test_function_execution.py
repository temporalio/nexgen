from __future__ import annotations

import typing
from unittest.mock import AsyncMock, Mock, patch

import pytest
import wit.function_execution as function_execution
from wit.function_execution.models import ExecuteCallbackRequest, ExecuteFunctionResult


def valid_function(name: str, enabled: bool) -> str:
    return f"{name},{enabled}"


def valid_counted_function(name: str, count: int) -> str:
    return f"{name},{count}"


def valid_varargs_function(*values: str) -> str:
    return ",".join(values)


def wrong_name_function(name: int, enabled: bool) -> str:
    return f"{name},{enabled}"


def wrong_count_function(name: str, count: str) -> str:
    return f"{name},{count}"


def one_arg_function(name: str) -> str:
    return name


@pytest.mark.asyncio
async def test_secondary_fixed_argument_callable_builds_request() -> None:
    handle = object()
    start_operation = AsyncMock(return_value=handle)
    client = Mock(start_operation=start_operation)
    with patch("temporalio.workflow.create_nexus_client", return_value=client):
        result = await function_execution.execute_callback(
            function=valid_function,
            name="one",
            enabled=True,
        )

    assert result is handle
    start_operation.assert_awaited_once_with(
        operation="ExecuteCallback",
        input=ExecuteCallbackRequest(
            function=valid_function,
            name="one",
            enabled=True,
        ),
        output_type=ExecuteFunctionResult,
    )


if typing.TYPE_CHECKING:
    _ = function_execution.execute_function(
        valid_function,
        "one",
        True,
    )

    _ = function_execution.execute_counted_function(
        valid_counted_function,
        "one",
        1,
    )

    _ = function_execution.execute_named_function(
        "named-function",
        "one",
        True,
    )

    _ = function_execution.execute_named_function(
        valid_function,
        "one",
        True,
    )

    _ = function_execution.execute_varargs_function(
        valid_varargs_function,
        "one",
        "two",
    )

    _ = function_execution.execute_varargs_function(
        valid_varargs_function,
        args=["one", "two"],
    )

    _ = function_execution.execute_named_varargs_function(
        "named-varargs-function",
        "one",
        "two",
    )

    _ = function_execution.execute_named_varargs_function(
        valid_varargs_function,
        "one",
        "two",
    )

    _ = function_execution.execute_named_varargs_function(
        "named-varargs-function",
        args=["one", "two"],
    )

    function_execution.execute_named_varargs_function(  # pyright: ignore[reportCallIssue]
        "named-varargs-function",
        "one",
        args=["two"],
    )

    _ = function_execution.execute_function(
        wrong_name_function,  # pyright: ignore[reportArgumentType]
        "one",
        True,
    )

    _ = function_execution.execute_function(
        one_arg_function,  # pyright: ignore[reportArgumentType]
        "one",
        True,
    )

    _ = function_execution.execute_function(
        valid_function,
        "one",
        "true",  # pyright: ignore[reportArgumentType]
    )

    _ = function_execution.execute_counted_function(
        wrong_count_function,  # pyright: ignore[reportArgumentType]
        "one",
        1,
    )
