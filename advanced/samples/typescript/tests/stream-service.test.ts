import * as common from "@temporalio/common";
import type { temporal } from "@temporalio/proto";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

import * as support from "../support/temporal_model_converters.ts";
import {
  type CreateStreamRequest,
  type Producer,
  type ReadRecordsResponse,
  type StreamRef,
  StreamStatus,
  appendRecordsRequestTransferTypeConverter,
  createCreateStreamRequestTransferTypeConverter,
  createReadRecordsResponseTransferTypeConverter,
  describeStreamResponseTransferTypeConverter,
  readRecordsRequestTransferTypeConverter,
  readRecordsResponseTransferTypeConverter,
} from "../wit/stream-service/workflow/models.ts";

// The operation modules import the SDK's Nexus module, which this sample doesn't have. A
// Stream method sends its request through these functions.
const sent = vi.hoisted(() => ({ requests: [] as unknown[] }));

vi.mock("../wit/stream-service/workflow/operations/append-records.ts", () => ({
  _appendRecords: async (request: unknown) => {
    sent.requests.push(request);
    return { result: async () => ({}) };
  },
}));
vi.mock("../wit/stream-service/workflow/operations/close-stream.ts", () => ({
  _closeStream: async (request: unknown) => {
    sent.requests.push(request);
    return { result: async () => undefined };
  },
}));
vi.mock("../wit/stream-service/workflow/operations/read-records.ts", () => ({}));
vi.mock("../wit/stream-service/workflow/operations/describe-stream.ts", () => ({}));

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

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  sent.requests.length = 0;
});

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
  test("maps the records of a read through their proto", () => {
    const model: ReadRecordsResponse<string> = {
      records: [
        {
          body: "hi",
          producer: { id: "wf:r1", kind: { tag: "workflow", value: { runId: "r1" } } },
          offset: _offset(1),
        },
      ],
      lastOffset: _offset(1),
      done: true,
    };

    const proto: temporal.api.streamservice.v1.IReadRecordsResponse =
      readRecordsResponseTransferTypeConverter.toTransferType(model);

    expect(proto.records?.[0].producer?.kind).toBe("workflow");
    expect(readRecordsResponseTransferTypeConverter.fromTransferType(proto)).toEqual(
      model,
    );
  });

  // A Stream<Turn> read gives its runtime type to this converter.
  test("decodes the bodies of a read with the runtime type of its stream", () => {
    const proto: temporal.api.streamservice.v1.IReadRecordsResponse = {
      records: [
        {
          body: common.defaultPayloadConverter.toPayload({ role: "user" }),
          offset: _offset(1),
        },
      ],
    };

    const model = createReadRecordsResponseTransferTypeConverter({
      bodyT: turnType,
    }).fromTransferType(proto);

    expect(model.records?.[0].body).toStrictEqual(new Turn("user"));
  });

  test("maps every field of a read request through its proto", () => {
    const proto: temporal.api.streamservice.v1.IReadRecordsRequest = {
      ref: { id: "stream-1" },
      fromOffset: _offset(7),
      maxRecords: 10,
    };

    const model = readRecordsRequestTransferTypeConverter.fromTransferType(proto);

    expect(model.ref.id).toBe("stream-1");
    expect(model.fromOffset).toEqual(_offset(7));
    expect(model.maxRecords).toBe(10);
    const wire = readRecordsRequestTransferTypeConverter.toTransferType(model);
    expect(wire.ref?.id).toBe("stream-1");
    expect(wire.fromOffset).toEqual(_offset(7));
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

  test("takes the producer of a call, or its source when the call passes none", async () => {
    const own = { id: "own", kind: { tag: "client", value: {} } } as const;
    const sourced = {
      id: "wf:r1",
      kind: { tag: "workflow", value: { runId: "r1" } },
    } as const;
    const source = vi.spyOn(support, "workflowProducer").mockReturnValue(sourced);
    // The tsconfig excludes the stream output, which doesn't typecheck here, so a test loads it
    // at run time.
    const resources = "../wit/stream-service/workflow/resources.ts";
    const { Stream } = (await import(resources)) as {
      Stream: new (
        ref: StreamRef<string>,
        created: undefined,
      ) => {
        append(records: string[], producer?: Producer): Promise<unknown>;
        close(producer?: Producer): Promise<void>;
      };
    };
    const stream = new Stream({ id: "stream-1" }, undefined);

    await stream.append(["a"], own);
    await stream.close(own);
    // A call that passes a producer never evaluates the source.
    expect(source).not.toHaveBeenCalled();

    await stream.append(["b"]);
    await stream.close();

    expect(source).toHaveBeenCalledTimes(2);
    expect(
      sent.requests.map((request) => (request as { producer: unknown }).producer),
    ).toEqual([own, own, sourced, sourced]);
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
