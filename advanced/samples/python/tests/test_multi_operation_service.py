from __future__ import annotations

import typing

import temporalio.converter
from temporalio import workflow
from temporalio.worker._interceptor import StartNexusOperationInput
from typing_extensions import override
import wit.multi_operation_service.models as models
from wit.multi_operation_service._system_nexus_interceptor import (
    _SystemNexusWorkflowOutboundInterceptorTerminal,
)


class CapturingSystemNexusTerminal(_SystemNexusWorkflowOutboundInterceptorTerminal):
    def __init__(self) -> None:
        self.input: StartNexusOperationInput[typing.Any, typing.Any] | None = None

    @override
    async def _intercept_system_nexus_operation(
        self,
        input: StartNexusOperationInput[typing.Any, typing.Any],
    ) -> workflow.NexusOperationHandle[typing.Any]:
        self.input = input
        return typing.cast(workflow.NexusOperationHandle[typing.Any], object())


async def test_the_terminal_decodes_a_generic_result_with_its_output_type() -> None:
    terminal = CapturingSystemNexusTerminal()

    _ = await terminal.start_value_operation(models.ValueRequest(id="v1", value="x"))

    # The SDK decodes the operation's result with the output type that the terminal passes.
    assert terminal.input is not None
    output_type = terminal.input.output_type
    assert output_type is not None
    converter = temporalio.converter.PayloadConverter.default
    payload = converter.to_payload({"values": ["x"]})
    decoded = converter.from_payload(payload, output_type)
    assert decoded == models.ValueResponse(values=["x"])
