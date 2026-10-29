from __future__ import annotations

import asyncio
import collections.abc
import dataclasses
import importlib
import subprocess
import sys
import types
import typing
import uuid

import nexusrpc
import pytest
from temporalio.api.enums.v1 import stream_pb2
from temporalio.api.stream.v1 import message_pb2
from temporalio.api.streamservice.v1 import request_response_pb2
from temporalio.converter import PayloadConverter
import temporalio.nexus.system
import temporalio.workflow

from wit.stream_service.models import models as models_only
from wit.stream_service.workflow import models as workflow_models
from wit.stream_service.client import models as client_models
from wit.stream_service.both import models as both_models
from wit.stream_service import workflow as workflow_only
from wit.stream_service.both import workflow as both_workflow


@dataclasses.dataclass
class FakeWorkflowInfo:
    run_id: str
    attempt: int = 1


@pytest.fixture(
    params=[models_only, workflow_models, client_models, both_models],
    ids=["models", "workflow", "client", "both"],
)
def models(request: pytest.FixtureRequest) -> types.ModuleType:
    return typing.cast(types.ModuleType, request.param)


@pytest.fixture(autouse=True)
def user_payload_converter(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        temporalio.nexus.system,
        "_current_user_payload_converter",
        lambda: PayloadConverter.default,
    )


def test_read_items_round_trip_as_their_records(models: types.ModuleType) -> None:
    converter = models._ReadRecordsResponseTransferTypeConverter[dict[str, str]]()
    model = models.ReadRecordsResponse(
        records=[
            models.StreamRecord(
                item=models.StreamItemData(
                    models.StreamData(
                        body={"text": "hi"},
                        producer=models.Producer(
                            id="la:r1:1",
                            epoch=models.ProducerEpoch(
                                kind=models.ProducerEpochKindLocalActivity(
                                    models.LocalActivityEpoch(
                                        task=models.WorkflowTaskEpoch(
                                            started_event_id=4
                                        ),
                                        attempt=2,
                                    )
                                )
                            ),
                        ),
                    )
                ),
                offset=1,
            ),
            models.StreamRecord(
                item=models.StreamItemSuperseded(
                    models.StreamSuperseded(producer_id="workflow-1", failed_tasks=[7])
                ),
                offset=2,
            ),
        ],
        next_offset=3,
        done=True,
    )

    wire = converter.to_transfer_type(model)

    # The wire messages come from `temporalio-stream-protos`, since no temporalio release has them.
    assert isinstance(wire, request_response_pb2.ReadRecordsResponse)
    assert all(isinstance(record, message_pb2.StreamRecord) for record in wire.records)
    assert [record.WhichOneof("item") for record in wire.records] == [
        "data",
        "superseded",
    ]
    assert wire.records[0].data.producer.epoch.WhichOneof("kind") == "local_activity"
    assert wire.records[1].superseded.failed_tasks == [7]
    assert converter.from_transfer_type(wire, models.ReadRecordsResponse) == model


def test_a_record_of_an_unknown_item_kind_has_no_item(models: types.ModuleType) -> None:
    converter = models._StreamRecordTransferTypeConverter[object]()
    # Offset 1, and an item in field 5, which only a newer server knows.
    wire = message_pb2.StreamRecord.FromString(b"\x08\x01\x2a\x00")

    decoded = converter.from_transfer_type(wire, models.StreamRecord)

    assert decoded == models.StreamRecord(item=None, offset=1)


def test_a_request_without_its_handle_is_refused(models: types.ModuleType) -> None:
    converter = models._AppendRecordsRequestTransferTypeConverter[object]()
    wire = converter.to_transfer_type(
        models.AppendRecordsRequest(ref=models.StreamRef(id="s-1"), records=[])
    )
    assert wire.ref.id == "s-1"

    wire.ClearField("ref")
    with pytest.raises(
        ValueError, match="missing required field AppendRecordsRequest.ref"
    ):
        _ = converter.from_transfer_type(wire, models.AppendRecordsRequest)


def test_an_unknown_enum_number_decodes_as_the_fallback_case(
    models: types.ModuleType,
) -> None:
    converter = models._DescribeStreamResponseTransferTypeConverter()
    # A newer server can send a Stream status that this code doesn't know.
    wire = request_response_pb2.DescribeStreamResponse(
        status=stream_pb2.StreamStatus.ValueType(9)
    )

    decoded = converter.from_transfer_type(wire, models.DescribeStreamResponse)

    assert decoded.status is models.StreamStatus.StreamStatusUnspecified


def test_an_unknown_number_of_an_enum_with_no_fallback_fails_the_message(
    models: types.ModuleType,
) -> None:
    converter = models._ReadRecordsRequestTransferTypeConverter()
    wire = request_response_pb2.ReadRecordsRequest(
        ref=message_pb2.StreamRef(id="s-1"),
        start=stream_pb2.StreamReadStart.ValueType(9),
    )

    with pytest.raises(ValueError, match="9 is not a valid StreamReadStart"):
        _ = converter.from_transfer_type(wire, models.ReadRecordsRequest)


@pytest.mark.parametrize(
    ("api", "model_types"),
    [(workflow_only, workflow_models), (both_workflow, both_models)],
    ids=["workflow", "both"],
)
async def test_workflow_apis_call_the_stream_service(
    monkeypatch: pytest.MonkeyPatch,
    api: types.ModuleType,
    model_types: types.ModuleType,
) -> None:
    ref = model_types.StreamRef(id="workflow-stream")
    replies = {
        "CreateStream": model_types.CreateStreamResult(ref=ref, created=True),
        "AppendRecords": model_types.AppendRecordsResponse(last_offset=3),
    }
    calls: list[tuple[str, typing.Any]] = []

    class NexusClient:
        async def start_operation(
            self,
            operation: str,
            input: typing.Any,
            *,
            output_type: type[typing.Any] | None = None,
        ) -> collections.abc.Awaitable[typing.Any]:
            del output_type
            calls.append((operation, input))
            future: asyncio.Future[typing.Any] = (
                asyncio.get_running_loop().create_future()
            )
            future.set_result(replies[operation])
            return future

    def create_client(*, service: str, endpoint: str) -> NexusClient:
        assert service == "temporal.api.streamservice.v1.StreamService"
        assert endpoint == "__temporal_system"
        return NexusClient()

    monkeypatch.setattr(temporalio.workflow, "create_nexus_client", create_client)
    monkeypatch.setattr(temporalio.workflow, "info", lambda: FakeWorkflowInfo("r2"))
    monkeypatch.setattr(temporalio.workflow, "in_workflow", lambda: True)
    request_id = uuid.UUID(int=7)
    monkeypatch.setattr(temporalio.workflow, "uuid4", lambda: request_id)
    stream = await api.create_stream(str, stream_ref=ref)
    response = await stream.publish(records=[])
    assert response.last_offset == 3
    # The Workflow API fills the request id and the producer from its sources.
    run = model_types.Producer(id="wf:r2", run_id="r2")
    expected = [
        (
            "CreateStream",
            model_types.CreateStreamRequest(stream_ref=ref, request_id=str(request_id)),
        ),
        (
            "AppendRecords",
            model_types.AppendRecordsRequest(ref=ref, records=[], producer=run),
        ),
    ]
    # The sources build the models of one package, so the calls compare as data.
    assert [(name, dataclasses.asdict(arg)) for name, arg in calls] == [
        (name, dataclasses.asdict(arg)) for name, arg in expected
    ]


@pytest.mark.parametrize(
    ("package", "forbidden"),
    [
        ("wit.stream_service.both.client", "wit.stream_service.both.workflow"),
        ("wit.stream_service.both.workflow", "wit.stream_service.both.client"),
        ("wit.stream_service.client.client", "wit.stream_service.client.workflow"),
    ],
)
def test_api_packages_do_not_import_another_target(
    package: str, forbidden: str
) -> None:
    script = """
import importlib
import importlib.abc
import sys

package, forbidden = sys.argv[1:]

class RejectOtherTarget(importlib.abc.MetaPathFinder):
    def find_spec(self, fullname, path=None, target=None):
        if fullname == forbidden or fullname.startswith(forbidden + "."):
            raise AssertionError("API imported another target: " + fullname)
        return None

sys.meta_path.insert(0, RejectOtherTarget())
api = importlib.import_module(package)
shared = importlib.import_module(package.rsplit(".", 1)[0])
services = importlib.import_module(shared.__name__ + ".services")
assert shared.StreamService is services.StreamService
resources = importlib.import_module(package + "._resources.stream")
assert resources.Stream
"""
    result = subprocess.run(
        [sys.executable, "-c", script, package, forbidden],
        check=False,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize(
    "package",
    [
        "wit.stream_service.models",
        "wit.stream_service.client",
        "wit.stream_service.both",
    ],
)
def test_root_service_contract_uses_shared_wire_models(package: str) -> None:
    shared = importlib.import_module(package)
    definition = nexusrpc.get_service_definition(shared.StreamService)
    assert definition is not None
    operation = definition.operation_definitions["CreateStream"]
    assert operation.input_type is shared.CreateStreamRequest
    assert operation.output_type is shared.CreateStreamResult
