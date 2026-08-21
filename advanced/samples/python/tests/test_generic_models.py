from __future__ import annotations

from typing_extensions import assert_type

from wit.generic_models.models import (
    GenericRequest,
    GenericResponse,
    Inner,
    OperationCompletionResult,
    OperationCompletionSuccess,
)


def test_generic_model_runtime_shapes() -> None:
    inner = Inner(value="nested")
    request = GenericRequest(
        context="context",
        contexts=["first"],
        by_name={"primary": "value"},
        nested=inner,
    )
    completion = OperationCompletionSuccess(output=42)
    result: OperationCompletionResult[int] = ("success", completion)
    response = GenericResponse(context="context", completion=result, metadata=True)

    _ = assert_type(request, GenericRequest[str])
    _ = assert_type(completion, OperationCompletionSuccess[int])
    _ = assert_type(response, GenericResponse[str, int, bool])
    _ = assert_type(response.completion, OperationCompletionResult[int])
    assert request.nested.value == "nested"
    assert response.completion[0] == "success"
    assert isinstance(response.completion[1], OperationCompletionSuccess)
    assert response.completion[1].output == 42
