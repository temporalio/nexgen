from __future__ import annotations

from temporalio.api.command.v1 import Command as WireCommand
from temporalio.api.enums.v1 import CommandType, NamespaceState, Severity
from temporalio.api.namespace.v1 import NamespaceInfo as WireNamespaceInfo
from temporalio.api.version.v1 import Alert as WireAlert

from wit.proto_enums import models


def test_ordered_enum_round_trips() -> None:
    converter = models._NamespaceInfoTransferTypeConverter()
    for member, number in zip(models.NamespaceState, (0, 1, 2, 3), strict=True):
        assert member.value == number
        wire = WireNamespaceInfo(state=NamespaceState.ValueType(number))
        model = converter.from_transfer_type(wire, models.NamespaceInfo)
        assert model.state is member
        assert converter.to_transfer_type(model) == wire
    assert converter.to_transfer_type(models.NamespaceInfo()).state == 1


def test_reordered_enum_round_trips() -> None:
    converter = models._AlertTransferTypeConverter()
    for member, number in zip(models.AlertLevel, (3, 1, 2, 0), strict=True):
        assert member.value == number
        wire = WireAlert(message="test", severity=Severity.ValueType(number))
        model = converter.from_transfer_type(wire, models.Alert)
        assert model.severity is member
        assert converter.to_transfer_type(model) == wire
    assert converter.to_transfer_type(models.Alert(message="test")).severity == 3


def test_sparse_enum_round_trips() -> None:
    converter = models._CommandModelTransferTypeConverter()
    numbers = (*range(15), 16, 17, 18)
    for member, number in zip(models.CommandType, numbers, strict=True):
        assert member.value == number
        wire = WireCommand(command_type=CommandType.ValueType(number))
        model = converter.from_transfer_type(wire, models.CommandModel)
        assert model.command_type is member
        assert converter.to_transfer_type(model) == wire
    assert converter.to_transfer_type(models.CommandModel()).command_type == 16
