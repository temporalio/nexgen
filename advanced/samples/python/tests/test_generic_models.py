from __future__ import annotations

import json
import typing

from temporalio.converter import PayloadConverter
from typing_extensions import assert_type

from wit.generic_models.models import (
    GenericRequest,
    GenericResponse,
    Inner,
    OperationCompletionResult,
    OperationCompletionResultSuccess,
)


def test_generic_model_runtime_shapes() -> None:
    inner = Inner(value="nested")
    request = GenericRequest(
        context="context",
        contexts=["first"],
        by_name={"primary": "value"},
        nested=inner,
    )
    result: OperationCompletionResult[int] = OperationCompletionResultSuccess(42)
    response: GenericResponse[str, int, bool] = GenericResponse(
        context="context", completion=result, metadata=True
    )

    _ = assert_type(request, GenericRequest[str])
    _ = assert_type(response, GenericResponse[str, int, bool])
    _ = assert_type(response.completion, OperationCompletionResult[int])
    assert request.nested.value == "nested"
    assert isinstance(response.completion, OperationCompletionResultSuccess)
    assert response.completion.value == 42


def test_generic_variant_payload_converter_round_trip() -> None:
    first_response = GenericResponse(
        context="context",
        completion=OperationCompletionResultSuccess(42),
        metadata=["metadata"],
    )
    second_response = GenericResponse(
        context=7,
        completion=OperationCompletionResultSuccess(True),
        metadata={"count": 3},
    )
    converter = PayloadConverter.default

    first_payload = converter.to_payloads([first_response])[0]

    assert first_payload.metadata["encoding"] == b"json/plain"
    assert json.loads(first_payload.data)["completion"] == {
        "tag": "success",
        "value": 42,
    }
    first_decoded = converter.from_payloads(
        [first_payload], [GenericResponse[str, int, list[str]]]
    )[0]
    operation_decoded = converter.from_payloads(
        [first_payload], [GenericResponse[typing.Any, typing.Any, typing.Any]]
    )[0]
    second_payload = converter.to_payloads([second_response])[0]
    second_decoded = converter.from_payloads(
        [second_payload], [GenericResponse[int, bool, dict[str, int]]]
    )[0]

    assert first_decoded == first_response
    assert operation_decoded == first_response
    assert second_decoded == second_response
