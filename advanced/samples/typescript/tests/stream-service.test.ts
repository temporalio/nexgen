import * as common from "@temporalio/common";
import type { temporal } from "@temporalio/proto";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import {
  type CreateStreamRequest,
  type ReadRecordsRequest,
  type ReadRecordsResponse,
  StreamCloseStatus,
  type StreamRef,
  StreamReadStart,
  StreamStatus,
  appendRecordsRequestTransferTypeConverter,
  createCreateStreamRequestTransferTypeConverter,
  createReadRecordsResponseTransferTypeConverter,
  describeStreamResponseTransferTypeConverter,
  readRecordsRequestTransferTypeConverter,
  readRecordsResponseTransferTypeConverter,
} from "../wit/stream-service/workflow/models.ts";
function _offset(n: number): Uint8Array {
  // Big-endian bytes, so a bytes compare keeps the int order these tests assert.
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigUint64(0, BigInt(n));
  return bytes;
}

class Turn {
  constructor(readonly role: string) {}
}

const turnType: common.TypeInfo<Turn, { role: string }> = {
  transferTypeConverter: {
    toTransferType: (turn) => ({ role: turn.role }),
    fromTransferType: (value) => new Turn(value.role),
  },
};

// A body goes through the payload converter of the Workflow.
beforeEach(() => {
  vi.stubGlobal("__TEMPORAL_ACTIVATOR__", {
    payloadConverter: common.defaultPayloadConverter,
  });
});

afterEach(() => vi.unstubAllGlobals());

describe("stream-service generated output", () => {
  test("carries a declared ref parameter through a create request", () => {
    const ref: StreamRef<Turn> = { id: "typed-stream" };
    const request: CreateStreamRequest<Turn> = {
      streamRef: ref,
      requestId: "request-1",
    };
    const converter = createCreateStreamRequestTransferTypeConverter<Turn>();
    const decoded: CreateStreamRequest<Turn> = converter.fromTransferType(
      converter.toTransferType(request),
    );
    expect(decoded.streamRef).toEqual(ref);
  });

  // No SDK has these protos yet, so proto/streams.d.ts declares them. The converters fail to
  // compile when those declarations drift from what they read and write.
  test("maps each read item kind through the proto oneof", () => {
    const model: ReadRecordsResponse<string> = {
      records: [
        {
          item: {
            tag: "data",
            value: { body: "hi", offset: _offset(1) },
          },
        },
        {
          item: {
            tag: "close",
            value: {
              producer: {
                id: "wf:r1",
                kind: { tag: "workflow", value: { runId: "r1" } },
              },
              status: StreamCloseStatus.StreamCloseStatusClosed,
              offset: _offset(2),
            },
          },
        },
      ],
      lastOffset: _offset(3),
      done: true,
    };

    const proto: temporal.api.streamservice.v1.IReadRecordsResponse =
      readRecordsResponseTransferTypeConverter.toTransferType(model);

    expect(proto.records?.map((record) => record.item)).toEqual(["data", "close"]);
    expect(proto.records?.[1].close?.status).toBe(
      StreamCloseStatus.StreamCloseStatusClosed,
    );
    expect(readRecordsResponseTransferTypeConverter.fromTransferType(proto)).toEqual(
      model,
    );
  });

  // A Stream<Turn> read gives its runtime type to this converter.
  test("decodes the bodies of a read with the runtime type of its stream", () => {
    const proto: temporal.api.streamservice.v1.IReadRecordsResponse = {
      records: [
        {
          item: "data",
          data: {
            body: common.defaultPayloadConverter.toPayload({ role: "user" }),
            offset: _offset(1),
          },
        },
      ],
    };

    const model = createReadRecordsResponseTransferTypeConverter({
      bodyT: turnType,
    }).fromTransferType(proto);

    const item = model.records?.[0].item;
    expect(item?.tag === "data" ? item.value.body : undefined).toStrictEqual(
      new Turn("user"),
    );
  });

  test("gives no item for a read record whose item kind only a newer server knows", () => {
    const proto: temporal.api.streamservice.v1.IReadRecordsResponse = {
      records: [{}],
    };

    const model = readRecordsResponseTransferTypeConverter.fromTransferType(proto);

    expect(model.records?.[0].item).toBeUndefined();
  });

  test("maps every field of a read request through its proto", () => {
    const proto: temporal.api.streamservice.v1.IReadRecordsRequest = {
      ref: { id: "stream-1" },
      position: "fromOffset",
      fromOffset: _offset(7),
      maxRecords: 10,
    };

    const model = readRecordsRequestTransferTypeConverter.fromTransferType(proto);

    expect(model.ref.id).toBe("stream-1");
    expect(model.position).toEqual({ tag: "from-offset", value: _offset(7) });
    expect(model.maxRecords).toBe(10);
    const wire = readRecordsRequestTransferTypeConverter.toTransferType(model);
    expect(wire.ref?.id).toBe("stream-1");
    expect(wire.position).toBe("fromOffset");
    expect(wire.fromOffset).toEqual(_offset(7));
  });

  test("maps the start case of a read position through the proto oneof", () => {
    const request: ReadRecordsRequest<string> = {
      ref: { id: "stream-1" },
      position: { tag: "start", value: StreamReadStart.StreamReadStartEnd },
    };

    const wire = readRecordsRequestTransferTypeConverter.toTransferType(request);

    expect(wire.position).toBe("start");
    expect(wire.start).toBe(StreamReadStart.StreamReadStartEnd);
    expect(
      readRecordsRequestTransferTypeConverter.fromTransferType(wire).position,
    ).toEqual(request.position);
  });

  test("wraps each value of an append in its record on the wire", () => {
    const proto = appendRecordsRequestTransferTypeConverter.toTransferType({
      ref: { id: "stream-1" },
      records: ["a", "b"],
      producer: { id: "client:sample" },
    });

    // `append-record` flattens, so the model holds the values and the proto holds the records.
    expect(
      proto.records?.map((record) =>
        common.defaultPayloadConverter.fromPayload(record.body!),
      ),
    ).toEqual(["a", "b"]);
    expect(
      appendRecordsRequestTransferTypeConverter.fromTransferType(proto).records,
    ).toEqual(["a", "b"]);
  });

  test("decodes an enum number that it doesn't know as the fallback case", () => {
    // A newer server can send a Stream status that this code doesn't know.
    const proto: temporal.api.streamservice.v1.IDescribeStreamResponse = {
      status: 9 as temporal.api.enums.v1.StreamStatus,
    };

    const model = describeStreamResponseTransferTypeConverter.fromTransferType(proto);

    expect(model.status).toBe(StreamStatus.StreamStatusUnspecified);
  });
});
